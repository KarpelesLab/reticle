//! Vendor and encrypted IP as black boxes.
//!
//! A vendor core often arrives as ciphertext: an `.vp` or `.vhdp` file
//! that only that vendor's tools can read, or a netlist that is not
//! shipped at all until the customer's licence server says so. A
//! toolchain that stops at such a file is useless, because the design
//! *around* it is the part the user is writing.
//!
//! So Reticle does not stop. From the package's `reticle.ip` — which is
//! plain text even when the sources are not — [`stub`] builds an
//! [`ir::Module`] with the declared interface, [`Module::blackbox`] set,
//! and no contents. Everything downstream already knows what that means:
//! the elaborator instantiates it, the validator checks the connections,
//! the linter sees the widths, the emitters write the instantiation for
//! the vendor tool to fill in, and the simulator warns that the box is
//! empty rather than failing.
//!
//! [`ir::Module`]: crate::ir::Module
//! [`Module::blackbox`]: crate::ir::Module::blackbox
//!
//! # Where the ports come from
//!
//! Two kinds of manifest line:
//!
//! - `port <name> <dir> [width]` for a port that stands alone: a clock,
//!   a reset, an interrupt.
//! - `interface <name> <bus> <role> [prefix <p>]` for a whole bus, which
//!   expands through [`super::bus`] into every signal of that bus at the
//!   role and prefix declared. One line becomes nineteen ports for
//!   AXI4-Lite.
//!
//! Widths come from the bus's own parameter defaults, overridden by any
//! `param` of the same name in the manifest, so
//! `param DATA_WIDTH int 64` widens `wdata` and `wstrb` together.
//!
//! # The behavioural model
//!
//! A vendor who cannot ship the implementation can still ship a model:
//! `model <path>` in the manifest. When there is one, simulation uses it
//! and the design is *not* a black box for that run — the synthesised
//! build still is. [`source_for`] makes that choice for one package and
//! [`BlackBoxReport`] states it in words, because a simulation that
//! quietly ran a model instead of the real core is a result nobody
//! should have to guess at.

use std::collections::BTreeMap;
use std::fmt;

use super::bus;
use super::manifest::{IpManifest, ParamType};
use super::resolve::{LoadedSource, Package};
use crate::diag::Diagnostics;
use crate::ir::builder::ModuleBuilder;
use crate::ir::{AttrValue, Attrs, Instance, InstanceId, Module, ModuleRef, Name, PortDir, Type};
use crate::source::Span;

/// What a black-boxed package is being represented by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlackBoxSource {
    /// Every source is marked `encrypted`; the vendor tool reads them.
    Encrypted,
    /// The manifest declares no source Reticle can read.
    Missing,
    /// A behavioural model is available and was used instead.
    Model(String),
    /// The real sources are readable; no black box is needed.
    Sources,
}

impl BlackBoxSource {
    /// True when the package will be a black box in the elaborated
    /// design.
    pub fn is_blackbox(&self) -> bool {
        !matches!(self, BlackBoxSource::Model(_) | BlackBoxSource::Sources)
    }
}

impl fmt::Display for BlackBoxSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlackBoxSource::Encrypted => f.write_str("encrypted sources"),
            BlackBoxSource::Missing => f.write_str("no readable sources"),
            BlackBoxSource::Model(path) => write!(f, "the behavioural model `{path}`"),
            BlackBoxSource::Sources => f.write_str("its own sources"),
        }
    }
}

/// What [`stub`] built, in a form a report can print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlackBoxReport {
    /// The generated module's name.
    pub module: String,
    /// The package's name and version, for the report line.
    pub package: String,
    /// Why the package is a black box.
    pub source: BlackBoxSource,
    /// The ports the stub declares.
    pub ports: Vec<String>,
    /// The bus interfaces that were expanded, with how many ports each
    /// contributed.
    pub interfaces: Vec<(String, usize)>,
    /// Interfaces naming a bus nobody defines, which contributed
    /// nothing.
    pub unknown_buses: Vec<String>,
}

impl BlackBoxReport {
    /// A deterministic, multi-line summary.
    ///
    /// This is what a build prints for every black box it made, so that
    /// "it simulated" and "it simulated the *model*" are never confused.
    pub fn describe(&self) -> String {
        let mut out = format!(
            "black box `{}` from {} ({})\n",
            self.module, self.package, self.source
        );
        for (name, count) in &self.interfaces {
            out.push_str(&format!("  interface {name}: {count} ports\n"));
        }
        for name in &self.unknown_buses {
            out.push_str(&format!("  interface {name}: unknown bus, skipped\n"));
        }
        out.push_str(&format!("  {} ports in total\n", self.ports.len()));
        out
    }
}

/// Which representation of `package` an elaboration should use.
///
/// A readable source wins over everything; a behavioural model is the
/// fallback for a package that has none; a package with neither becomes
/// an empty black box.
pub fn source_for(package: &Package) -> BlackBoxSource {
    if package.sources.iter().any(LoadedSource::is_readable) {
        return BlackBoxSource::Sources;
    }
    if let Some(model) = &package.model
        && model.is_readable()
    {
        return BlackBoxSource::Model(model.path.clone());
    }
    if !package.manifest.sources.is_empty() && package.manifest.sources.iter().all(|s| s.encrypted)
    {
        BlackBoxSource::Encrypted
    } else {
        BlackBoxSource::Missing
    }
}

/// The widths every bus parameter has for this package: the bus's own
/// defaults, overridden by a `param` of the same name.
fn bindings(manifest: &IpManifest, bus: &bus::BusInterface) -> BTreeMap<String, u32> {
    let mut out = bus.defaults();
    for (name, value) in &mut out {
        if let Some(param) = manifest.param(name)
            && param.ty == ParamType::Int
            && let Some(declared) = param.default_int()
            && let Ok(declared) = u32::try_from(declared)
            && declared > 0
        {
            *value = declared;
        }
    }
    out
}

/// The widths the manifest's own `param` lines bind, for a `port` line
/// whose width names a parameter.
fn param_widths(manifest: &IpManifest) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for param in &manifest.params {
        if param.ty == ParamType::Int
            && let Some(value) = param.default_int()
            && let Ok(value) = u32::try_from(value)
            && value > 0
        {
            out.insert(param.name.clone(), value);
        }
    }
    out
}

/// Builds the black-box module for `manifest`.
///
/// `source` says why the package is being black-boxed, and ends up in
/// the report and in the module's attributes. Interfaces naming an
/// unknown bus are reported through `diags` and skipped, so a manifest
/// with one typo still produces a usable stub for the rest.
///
/// ```
/// use reticle::diag::Diagnostics;
/// use reticle::ip::blackbox::{self, BlackBoxSource};
/// use reticle::ip::IpManifest;
/// use reticle::source::SourceMap;
///
/// let text = "\
/// name vendor_ddr
/// version 2.1.0
///
/// top ddr_phy
///
/// source rtl/ddr_phy.vp encrypted
///
/// param DATA_WIDTH int 64
///
/// port clk in
/// port rst_n in
/// interface s_axi axi4lite subordinate prefix s_axi_
/// ";
/// let mut map = SourceMap::new();
/// let file = map.add("reticle.ip", text).unwrap();
/// let mut diags = Diagnostics::new();
/// let manifest = IpManifest::parse(text, file, &mut diags).unwrap();
/// let span = manifest.span;
///
/// let (module, report) =
///     blackbox::stub(&manifest, BlackBoxSource::Encrypted, span, &mut diags);
/// assert!(module.blackbox);
/// assert_eq!(module.name, "ddr_phy");
/// // The `interface` line became the whole of AXI4-Lite, at 64 bits.
/// assert_eq!(module.ports.len(), 2 + 19);
/// assert_eq!(
///     module.nets[module.port("s_axi_wdata").unwrap().net].ty.width(),
///     Some(64)
/// );
/// assert!(report.describe().contains("encrypted sources"));
/// ```
pub fn stub(
    manifest: &IpManifest,
    source: BlackBoxSource,
    span: Span,
    diags: &mut Diagnostics,
) -> (Module, BlackBoxReport) {
    let mut b = ModuleBuilder::new(manifest.top_name(), span);
    b.blackbox();
    b.attr("blackbox", 1i64);
    b.attr("ip_name", manifest.name.clone());
    b.attr("ip_version", manifest.version.to_string());
    if let Some(license) = &manifest.license {
        b.attr("ip_license", license.clone());
    }
    b.attr("ip_source", source.to_string());
    for param in &manifest.params {
        let value: AttrValue = match (&param.default, param.ty) {
            (Some(text), ParamType::Int) => text
                .parse::<i64>()
                .map_or_else(|_| AttrValue::String(text.clone()), AttrValue::Int),
            (Some(text), ParamType::Bool) => AttrValue::Int(i64::from(text == "true")),
            (Some(text), _) => AttrValue::String(text.clone()),
            (None, _) => AttrValue::String(String::new()),
        };
        b.param(param.name.clone(), value);
    }

    let widths = param_widths(manifest);
    let mut ports = Vec::new();
    for port in &manifest.ports {
        let width = port.width.resolve(&widths).unwrap_or(1);
        match port.dir {
            PortDir::In => b.input(port.name.clone(), Type::bits(width)),
            PortDir::Out => b.output(port.name.clone(), Type::bits(width)),
            PortDir::InOut => b.inout(port.name.clone(), Type::bits(width)),
        };
        ports.push(port.name.clone());
    }

    let mut interfaces = Vec::new();
    let mut unknown_buses = Vec::new();
    for declared in &manifest.interfaces {
        let Some(interface) = bus::builtin(&declared.bus) else {
            diags.push(bus::no_such_bus(&declared.bus, declared.span));
            unknown_buses.push(declared.name.clone());
            continue;
        };
        let bindings = bindings(manifest, interface);
        let prefix = declared.prefix();
        let mut count = 0;
        for signal in &interface.signals {
            let name = format!("{prefix}{}", signal.name);
            if b.module().port(&name).is_some() {
                // A `port` line already declared it by hand; the
                // explicit declaration wins.
                continue;
            }
            let width = signal.width.resolve(&bindings).unwrap_or(1);
            match signal.direction(declared.role) {
                PortDir::In => b.input(name.clone(), Type::bits(width)),
                PortDir::Out => b.output(name.clone(), Type::bits(width)),
                PortDir::InOut => b.inout(name.clone(), Type::bits(width)),
            };
            ports.push(name);
            count += 1;
        }
        interfaces.push((declared.name.clone(), count));
    }

    let report = BlackBoxReport {
        module: manifest.top_name().to_owned(),
        package: format!("{} {}", manifest.name, manifest.version),
        source,
        ports,
        interfaces,
        unknown_buses,
    };
    (b.finish(), report)
}

/// An instance of a black box, referring to it by name.
///
/// The reference is [`ModuleRef::Unresolved`] even when the stub module
/// is in the same design, because that is what "the target flow supplies
/// the contents" means in the IR; [`crate::ir::Design::resolve_instances`]
/// binds it to the stub when the design holds one.
pub fn instance(manifest: &IpManifest, name: impl Into<Name>, span: Span) -> Instance {
    Instance {
        name: name.into(),
        module: ModuleRef::Unresolved(Name::new(manifest.top_name())),
        connections: Vec::new(),
        params: Attrs::new(),
        attrs: Attrs::new(),
        span,
    }
}

/// Instantiates a black box inside `b`, creating one net per port and
/// connecting it.
///
/// The nets are named `<instance>_<port>`, so two instances of the same
/// core do not collide. Returns the new instance.
pub fn instantiate(
    b: &mut ModuleBuilder,
    stub: &Module,
    manifest: &IpManifest,
    name: &str,
) -> InstanceId {
    let ports: Vec<(Name, Type)> = stub
        .ports
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                stub.nets
                    .get(p.net)
                    .map_or_else(Type::bit, |net| net.ty.clone()),
            )
        })
        .collect();
    let mut connections = Vec::with_capacity(ports.len());
    for (port, ty) in ports {
        let net_name = format!("{name}_{port}");
        let net = match b.module().net_by_name(&net_name) {
            Some(existing) => existing,
            None => b.add_net(net_name, ty),
        };
        let expr = b.net(net);
        connections.push((port, expr));
    }
    let span = b.span;
    let id = b.instance(
        name,
        ModuleRef::Unresolved(Name::new(manifest.top_name())),
        connections,
    );
    b.module_mut().instances[id].span = span;
    id
}

#[cfg(test)]
mod tests {
    use super::super::manifest::Version;
    use super::super::resolve::{PathProvider, Resolved, Resolver};
    use super::super::{Project, bus::BusRole};
    use super::*;
    use crate::ir::Design;
    use crate::ir::validate::validate;
    use crate::source::SourceMap;

    fn parse(text: &str) -> (IpManifest, Span, SourceMap) {
        let mut map = SourceMap::new();
        let file = map.add("reticle.ip", text).unwrap();
        let mut diags = Diagnostics::new();
        let manifest = IpManifest::parse(text, file, &mut diags).expect("parses");
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        let span = manifest.span;
        (manifest, span, map)
    }

    const VENDOR: &str = "\
name vendor_ddr
version 2.1.0
license proprietary

top ddr_phy

source rtl/ddr_phy.vp language verilog encrypted

param DATA_WIDTH int 64
param ADDR_WIDTH int 30
param MODE string fast
param PIPELINE bool true
param INIT bits 8'hff

port clk in
port rst_n in
port irq out
port dq inout DATA_WIDTH
interface s_axi axi4lite subordinate prefix s_axi_
";

    #[test]
    fn a_stub_carries_the_declared_interface() {
        let (manifest, span, map) = parse(VENDOR);
        let mut diags = Diagnostics::new();
        let (module, report) = stub(&manifest, BlackBoxSource::Encrypted, span, &mut diags);
        assert_eq!(diags.render(&map), "");
        assert!(module.blackbox);
        assert_eq!(module.name, "ddr_phy");
        assert_eq!(module.ports.len(), 4 + 19);
        assert_eq!(report.interfaces, [("s_axi".to_owned(), 19)]);
        assert!(report.unknown_buses.is_empty());

        let width = |name: &str| {
            module.nets[module.port(name).unwrap_or_else(|| panic!("{name}")).net]
                .ty
                .width()
        };
        assert_eq!(width("dq"), Some(64));
        assert_eq!(width("s_axi_wdata"), Some(64));
        assert_eq!(width("s_axi_wstrb"), Some(8));
        assert_eq!(width("s_axi_awaddr"), Some(30));
        assert_eq!(module.port("irq").unwrap().dir, PortDir::Out);
        assert_eq!(module.port("dq").unwrap().dir, PortDir::InOut);
        assert_eq!(module.port("s_axi_awvalid").unwrap().dir, PortDir::In);

        // The stub is recognised as the interface it says it is.
        let axi = bus::builtin("axi4lite").unwrap();
        bus::match_ports(&module, axi, BusRole::Subordinate, "s_axi_").expect("matches");

        assert_eq!(
            module.attrs.get("ip_version").and_then(|v| v.as_str()),
            Some("2.1.0")
        );
        assert_eq!(
            module.attrs.get("ip_source").and_then(|v| v.as_str()),
            Some("encrypted sources")
        );
        assert_eq!(module.param("DATA_WIDTH").unwrap().value.as_int(), Some(64));
        assert_eq!(module.param("PIPELINE").unwrap().value.as_int(), Some(1));
        assert_eq!(module.param("MODE").unwrap().value.as_str(), Some("fast"));
        assert_eq!(module.param("INIT").unwrap().value.as_str(), Some("8'hff"));
    }

    #[test]
    fn the_report_says_what_was_used() {
        let (manifest, span, _) = parse(VENDOR);
        let mut diags = Diagnostics::new();
        let (_, report) = stub(&manifest, BlackBoxSource::Encrypted, span, &mut diags);
        let text = report.describe();
        assert_eq!(
            text,
            "black box `ddr_phy` from vendor_ddr 2.1.0 (encrypted sources)\n  \
             interface s_axi: 19 ports\n  23 ports in total\n"
        );
        let (_, report) = stub(
            &manifest,
            BlackBoxSource::Model("sim/ddr_model.v".to_owned()),
            span,
            &mut diags,
        );
        assert!(
            report
                .describe()
                .contains("the behavioural model `sim/ddr_model.v`")
        );
        assert!(!BlackBoxSource::Model(String::new()).is_blackbox());
        assert!(!BlackBoxSource::Sources.is_blackbox());
        assert!(BlackBoxSource::Missing.is_blackbox());
        assert_eq!(BlackBoxSource::Sources.to_string(), "its own sources");
    }

    #[test]
    fn an_unknown_bus_is_reported_and_skipped() {
        let text = "\
name x
version 1.0.0

port clk in
interface s axi4lit subordinate
interface t apb subordinate prefix t_
";
        let (manifest, span, map) = parse(text);
        let mut diags = Diagnostics::new();
        let (module, report) = stub(&manifest, BlackBoxSource::Missing, span, &mut diags);
        let rendered = diags.render(&map);
        assert!(rendered.contains("unknown bus `axi4lit`"), "{rendered}");
        assert!(rendered.contains("did you mean `axi4lite`?"), "{rendered}");
        assert_eq!(report.unknown_buses, ["s"]);
        assert_eq!(report.interfaces, [("t".to_owned(), 10)]);
        assert_eq!(module.ports.len(), 1 + 10);
        assert!(
            report
                .describe()
                .contains("interface s: unknown bus, skipped")
        );
    }

    #[test]
    fn an_explicit_port_line_beats_the_bus_expansion() {
        let text = "\
name x
version 1.0.0

port s_pslverr in 1
interface s apb subordinate prefix s_
";
        let (manifest, span, _) = parse(text);
        let mut diags = Diagnostics::new();
        let (module, report) = stub(&manifest, BlackBoxSource::Missing, span, &mut diags);
        // The hand-written line stays, and the interface contributes one
        // port fewer.
        assert_eq!(report.interfaces, [("s".to_owned(), 9)]);
        assert_eq!(module.ports.len(), 10);
        assert_eq!(module.port("s_pslverr").unwrap().dir, PortDir::In);
    }

    #[test]
    fn a_stub_instantiates_into_a_design_that_validates() {
        let (manifest, span, _) = parse(VENDOR);
        let mut diags = Diagnostics::new();
        let (module, _) = stub(&manifest, BlackBoxSource::Encrypted, span, &mut diags);
        let mut design = Design::new();
        let mut b = ModuleBuilder::new("top", span);
        let id = instantiate(&mut b, &module, &manifest, "u_ddr");
        // A second instance gets its own nets.
        instantiate(&mut b, &module, &manifest, "u_ddr2");
        let top = b.finish();
        assert_eq!(top.instances[id].connections.len(), module.ports.len());
        assert_eq!(top.nets.len(), 2 * module.ports.len());
        design.add_module(module);
        design.top = Some(design.add_module(top));
        assert_eq!(design.resolve_instances(), 2);
        let diags = validate(&design);
        assert!(diags.is_empty(), "{:?}", diags.iter().next());
    }

    #[test]
    fn a_bare_instance_is_unresolved() {
        let (manifest, span, _) = parse(VENDOR);
        let instance = instance(&manifest, "u_ddr", span);
        assert_eq!(instance.module, ModuleRef::Unresolved(Name::new("ddr_phy")));
        assert!(instance.connections.is_empty());
        assert_eq!(instance.name, "u_ddr");
    }

    /// Resolves a one-package project and returns the package.
    fn resolve_one(manifest: &str, files: &[(&str, &str)]) -> Resolved {
        let mut fs: BTreeMap<String, String> = files
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        fs.insert("ip/a/reticle.ip".to_owned(), manifest.to_owned());
        let project_text = "name p\n\ndepends a ^1.0.0 path ip/a\n";
        let mut map = SourceMap::new();
        let file = map.add("reticle.proj", project_text).unwrap();
        let mut diags = Diagnostics::new();
        let project = Project::parse(project_text, file, &mut diags).unwrap();
        let mut provider = PathProvider::new(".", |path: &str| fs.get(path).cloned());
        Resolver::new(map).resolve(&project, &mut provider, &mut diags)
    }

    #[test]
    fn the_source_choice_prefers_sources_then_a_model() {
        let resolved = resolve_one(
            "name a\nversion 1.0.0\n\nsource rtl/a.v\nmodel sim/a.v\n",
            &[
                ("ip/a/rtl/a.v", "module a; endmodule\n"),
                ("ip/a/sim/a.v", ""),
            ],
        );
        assert_eq!(
            source_for(resolved.package("a").unwrap()),
            BlackBoxSource::Sources
        );

        let resolved = resolve_one(
            "name a\nversion 1.0.0\n\nsource rtl/a.vp encrypted\nmodel sim/a.v\n",
            &[("ip/a/sim/a.v", "module a; endmodule\n")],
        );
        assert_eq!(
            source_for(resolved.package("a").unwrap()),
            BlackBoxSource::Model("sim/a.v".to_owned())
        );

        let resolved = resolve_one("name a\nversion 1.0.0\n\nsource rtl/a.vp encrypted\n", &[]);
        assert_eq!(
            source_for(resolved.package("a").unwrap()),
            BlackBoxSource::Encrypted
        );

        let resolved = resolve_one("name a\nversion 1.0.0\n", &[]);
        assert_eq!(
            source_for(resolved.package("a").unwrap()),
            BlackBoxSource::Missing
        );

        // A model the provider could not read is no model at all.
        let resolved = resolve_one(
            "name a\nversion 1.0.0\n\nsource rtl/a.vp encrypted\nmodel sim/a.v\n",
            &[],
        );
        assert_eq!(
            source_for(resolved.package("a").unwrap()),
            BlackBoxSource::Encrypted
        );
    }

    #[test]
    fn a_manifest_with_no_interface_still_stubs() {
        let mut map = SourceMap::new();
        let file = map.add("x", "").unwrap();
        let span = Span::new(file, 0, 0);
        let manifest = IpManifest::new("bare", Version::new(0, 1, 0), span);
        let mut diags = Diagnostics::new();
        let (module, report) = stub(&manifest, BlackBoxSource::Missing, span, &mut diags);
        assert!(module.ports.is_empty());
        assert_eq!(module.name, "bare");
        assert!(report.ports.is_empty());
        assert!(report.describe().contains("0 ports in total"));
    }
}
