//! IP-XACT import: an existing catalogue's component becomes a
//! `reticle.ip`.
//!
//! Vendors and in-house catalogues describe IP in IP-XACT, so the way
//! into Reticle for IP nobody is going to rewrite a manifest for is to
//! read theirs. [`import`] takes one **component** description and
//! produces an [`IpManifest`] plus a [`ImportReport`] of what happened to
//! every part of it.
//!
//! # Which standards
//!
//! The root element's namespace decides, because the prefix is only a
//! spelling and catalogues in the wild are a mix of both:
//!
//! | Namespace | [`Standard`] |
//! |-----------|--------------|
//! | `http://www.spiritconsortium.org/XMLSchema/SPIRIT/1685-2009` | IEEE 1685-2009, usually spelled `spirit:` |
//! | `http://www.accellera.org/XMLSchema/IPXACT/1685-2014` | IEEE 1685-2014, `ipxact:` |
//! | `http://www.accellera.org/XMLSchema/IPXACT/1685-2022` | IEEE 1685-2022, `ipxact:` |
//!
//! An older SPIRIT namespace (1.2 to 1.5) is read as 1685-2009 and said
//! to be; anything else is [`UNKNOWN_STANDARD`], since guessing at a
//! schema nobody has named is how an importer silently loses half a
//! component.
//!
//! The differences that matter are handled in both spellings: a vector
//! is `wire/vector` in 2009 and `wire/vectors/vector` from 2014, HDL
//! parameters are `model/modelParameters` in 2009 and
//! `model/instantiations/componentInstantiation/moduleParameters` from
//! 2014, port maps hang off the `busInterface` in 2009 and off its
//! `abstractionType` from 2014, and a bus interface's side is
//! `master`/`slave` in 2009 and 2014 and `initiator`/`target` in 2022.
//!
//! # What is translated
//!
//! | IP-XACT | `reticle.ip` |
//! |---------|--------------|
//! | VLNV `name` and `version` | `name` and `version` (see [`map_version`]) |
//! | `description` | `description`, with the memory maps appended |
//! | `model/views` model or module name | `top` |
//! | `model/ports/port` (wire ports) | `port <name> <dir> [width]` |
//! | `fileSets/fileSet/file` | `source <path> [language <lang>]` |
//! | `parameters`, `modelParameters`, `moduleParameters` | `param <name> <type> [default] [lo..hi]` |
//! | `busInterfaces/busInterface` | `interface <name> <bus> <role> [prefix <p>]` |
//! | `memoryMaps` | a phrase in `description` |
//!
//! Vector bounds are expressions over the component's parameters
//! (`ADDR_WIDTH-1 downto 0`), so they are evaluated against the
//! parameters' resolved values. A bound that will not evaluate but has
//! the shape `PARAM-1` or `PARAM/n-1` becomes the manifest's own
//! [`Width::Derived`], which is exactly what a `wstrb` needs; anything
//! else is reported and the port is dropped rather than given an invented
//! width.
//!
//! # The bus mapping
//!
//! A `busType` is a VLNV of its own. Its `name` is normalised (upper
//! case, everything but letters and digits removed) and looked up:
//!
//! | `busType` name | Reticle bus | Note |
//! |----------------|-------------|------|
//! | `AXI4LITE`, `AXILITE` | `axi4lite` | |
//! | `AXI4STREAM`, `AXISTREAM`, `AXIS` | `axi4stream` | |
//! | `AXI4` | `axi4` | |
//! | `AXI3`, `AXI` | `axi4` | approximated: AXI4 is the closest built-in |
//! | `APB`, `APB2`, `APB3`, `APB4` | `apb` | |
//! | `WISHBONE`, `WB`, `WISHBONEB4` | `wishbone` | |
//! | `WISHBONEPIPELINED`, `WBPIPELINED` | `wishbone_pipelined` | |
//! | `AVALON`, `AVALONMM`, `AVALONMEMORYMAPPED` | `avalon_mm` | |
//!
//! [`ImportOptions::bus_map`] adds to that table, for a site with its own
//! `.bus` files. A bus nothing matches is **not** guessed at: the
//! interface is dropped with [`UNKNOWN_BUS`] naming the whole VLNV, and
//! its ports stay in the manifest as plain `port` lines so nothing is
//! lost.
//!
//! # Silence is the enemy
//!
//! Everything that was not translated exactly is in
//! [`ImportedIp::report`], under `approximated` or `dropped`, and the
//! interesting ones are diagnostics as well. An import that quietly
//! halves a component is the failure that bites months later, when the
//! synthesised design is missing an interrupt line nobody noticed had
//! gone.
//!
//! # What is not read
//!
//! Designs, design configurations, abstraction definitions, bus
//! definitions, catalogues, abstractors and generators: [`import`] reads
//! a `component` and says so when handed anything else. Within a
//! component, transactional (TLM) ports, `vendorExtensions`, `choices`,
//! `whiteboxElements`, `cpus`, `channels`, `resetTypes`,
//! `indirectInterfaces`, address spaces, register field details and a
//! view's file-set references are all skipped; everything in that list
//! that the component actually has gets a line in the report, so the
//! reader learns what was there. Every file set is imported, in document
//! order, since which of them a view refers to is not followed.
//!
//! ```
//! use reticle::diag::Diagnostics;
//! use reticle::ip::ipxact::{self, ImportOptions};
//! use reticle::source::SourceMap;
//!
//! let xml = r#"<?xml version="1.0"?>
//! <ipxact:component xmlns:ipxact="http://www.accellera.org/XMLSchema/IPXACT/1685-2014">
//!   <ipxact:vendor>example.com</ipxact:vendor>
//!   <ipxact:library>ip</ipxact:library>
//!   <ipxact:name>counter</ipxact:name>
//!   <ipxact:version>1.0</ipxact:version>
//!   <ipxact:model><ipxact:ports>
//!     <ipxact:port><ipxact:name>clk</ipxact:name>
//!       <ipxact:wire><ipxact:direction>in</ipxact:direction></ipxact:wire>
//!     </ipxact:port>
//!   </ipxact:ports></ipxact:model>
//! </ipxact:component>"#;
//!
//! let mut map = SourceMap::new();
//! let file = map.add("counter.xml", xml).unwrap();
//! let mut diags = Diagnostics::new();
//! let imported = ipxact::import(xml, &ImportOptions::new(file), &mut diags).unwrap();
//! assert_eq!(imported.manifest.name, "counter");
//! assert_eq!(imported.manifest.version.to_string(), "1.0.0");
//! assert_eq!(imported.manifest.ports.len(), 1);
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::bus::{self, BusRole, Width};
use super::manifest::{
    InterfaceDecl, IpManifest, Language, ParamDecl, ParamType, PortDecl, SourceEntry, Version,
};
use super::xml::{Document, Element};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::PortDir;
use crate::source::{SourceId, Span};

/// Diagnostic code for a root namespace that is not a known IP-XACT one.
pub const UNKNOWN_STANDARD: &str = "P0601";
/// Diagnostic code for a document that is not a component description.
pub const NOT_A_COMPONENT: &str = "P0602";
/// Diagnostic code for a component missing something it must have.
pub const MISSING_ELEMENT: &str = "P0603";
/// Diagnostic code for a VLNV version that is not a semantic version.
pub const ODD_VERSION: &str = "P0604";
/// Diagnostic code for a vector bound that will not evaluate.
pub const UNRESOLVED_EXPRESSION: &str = "P0605";
/// Diagnostic code for a `busType` no Reticle bus matches.
pub const UNKNOWN_BUS: &str = "P0606";
/// Diagnostic code for a file whose type is not an HDL source.
pub const UNKNOWN_FILE_TYPE: &str = "P0607";
/// Diagnostic code for a port direction that is not `in`, `out` or
/// `inout`.
pub const UNKNOWN_DIRECTION: &str = "P0608";

// ---------------------------------------------------------------------------
// Standards and identities
// ---------------------------------------------------------------------------

/// Which revision of the standard a document follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Standard {
    /// IEEE 1685-2009, the SPIRIT Consortium's namespace.
    Spirit2009,
    /// IEEE 1685-2014, Accellera's namespace.
    Ipxact2014,
    /// IEEE 1685-2022, Accellera's namespace.
    Ipxact2022,
}

impl Standard {
    /// The namespace URI of this revision.
    pub fn namespace(self) -> &'static str {
        match self {
            Standard::Spirit2009 => "http://www.spiritconsortium.org/XMLSchema/SPIRIT/1685-2009",
            Standard::Ipxact2014 => "http://www.accellera.org/XMLSchema/IPXACT/1685-2014",
            Standard::Ipxact2022 => "http://www.accellera.org/XMLSchema/IPXACT/1685-2022",
        }
    }

    /// Every revision, oldest first.
    pub const ALL: [Standard; 3] = [
        Standard::Spirit2009,
        Standard::Ipxact2014,
        Standard::Ipxact2022,
    ];

    /// The revision a namespace URI names, exactly.
    pub fn from_namespace(uri: &str) -> Option<Standard> {
        Standard::ALL.into_iter().find(|s| s.namespace() == uri)
    }

    /// How a report and a diagnostic name it.
    pub fn describe(self) -> &'static str {
        match self {
            Standard::Spirit2009 => "IEEE 1685-2009 (spirit)",
            Standard::Ipxact2014 => "IEEE 1685-2014 (ipxact)",
            Standard::Ipxact2022 => "IEEE 1685-2022 (ipxact)",
        }
    }
}

impl fmt::Display for Standard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.describe())
    }
}

/// An IP-XACT identity: vendor, library, name and version.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Vlnv {
    /// The vendor, usually a domain name.
    pub vendor: String,
    /// The library within the vendor.
    pub library: String,
    /// The name of the thing itself.
    pub name: String,
    /// Its version, which IP-XACT leaves as free text.
    pub version: String,
}

impl Vlnv {
    /// Reads a VLNV written as four child elements, as a component's own
    /// identity is.
    pub fn from_children(element: &Element) -> Option<Vlnv> {
        Some(Vlnv {
            vendor: element.child_text("vendor")?,
            library: element.child_text("library")?,
            name: element.child_text("name")?,
            version: element.child_text("version")?,
        })
    }

    /// Reads a VLNV written as four attributes, as a `busType` is.
    pub fn from_attributes(element: &Element) -> Option<Vlnv> {
        Some(Vlnv {
            vendor: element.attr("vendor")?.to_owned(),
            library: element.attr("library")?.to_owned(),
            name: element.attr("name")?.to_owned(),
            version: element.attr("version")?.to_owned(),
        })
    }
}

impl fmt::Display for Vlnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}:{}",
            self.vendor, self.library, self.name, self.version
        )
    }
}

// ---------------------------------------------------------------------------
// Options, report and result
// ---------------------------------------------------------------------------

/// What an import should do with the choices the format leaves open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportOptions {
    /// The file the XML was added to the source map under, so that every
    /// diagnostic points into it.
    pub file: SourceId,
    /// The package name, overriding the one derived from the VLNV.
    pub name: Option<String>,
    /// The package version, overriding the one derived from the VLNV.
    pub version: Option<Version>,
    /// The licence to record; IP-XACT has nowhere to keep one.
    pub license: Option<String>,
    /// Extra `busType` name to Reticle bus mappings, tried before the
    /// built-in table. The key is matched after the same normalisation
    /// the table uses, so `"AHB-Lite"` and `"ahblite"` are one key.
    pub bus_map: Vec<(String, String)>,
}

impl ImportOptions {
    /// The defaults: names and versions from the VLNV, no licence, no
    /// extra buses.
    pub fn new(file: SourceId) -> Self {
        ImportOptions {
            file,
            name: None,
            version: None,
            license: None,
            bus_map: Vec::new(),
        }
    }

    /// Sets the package name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the package version.
    pub fn with_version(mut self, version: Version) -> Self {
        self.version = Some(version);
        self
    }

    /// Sets the licence recorded in the manifest.
    pub fn with_license(mut self, license: impl Into<String>) -> Self {
        self.license = Some(license.into());
        self
    }

    /// Teaches the importer one more bus.
    pub fn with_bus(mut self, ipxact_name: impl Into<String>, bus: impl Into<String>) -> Self {
        self.bus_map.push((ipxact_name.into(), bus.into()));
        self
    }

    /// The Reticle bus this component's options map `name` to.
    fn mapped_bus(&self, name: &str) -> Option<&str> {
        let key = normalise_bus_name(name);
        self.bus_map
            .iter()
            .find(|(from, _)| normalise_bus_name(from) == key)
            .map(|(_, to)| to.as_str())
    }
}

/// What an import did, in three lists.
///
/// The split is the point: `translated` is what came across exactly,
/// `approximated` is what came across differently and `dropped` is what
/// did not come across at all. A reader who checks only one list should
/// check the third.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// What was translated exactly, in document order.
    pub translated: Vec<String>,
    /// What was translated into something close but not equal.
    pub approximated: Vec<String>,
    /// What the manifest does not describe at all.
    pub dropped: Vec<String>,
}

impl ImportReport {
    /// True when nothing was approximated and nothing dropped.
    pub fn is_lossless(&self) -> bool {
        self.approximated.is_empty() && self.dropped.is_empty()
    }

    /// A deterministic, multi-line summary, in the shape
    /// [`super::Elaboration::report`] uses.
    pub fn describe(&self) -> String {
        let mut out = String::new();
        for (heading, entries) in [
            ("translated", &self.translated),
            ("approximated", &self.approximated),
            ("dropped", &self.dropped),
        ] {
            if entries.is_empty() {
                continue;
            }
            out.push_str(heading);
            out.push('\n');
            for entry in entries {
                out.push_str("  ");
                out.push_str(entry);
                out.push('\n');
            }
        }
        out
    }

    fn note_translated(&mut self, what: impl Into<String>) {
        self.translated.push(what.into());
    }

    fn note_approximated(&mut self, what: impl Into<String>) {
        self.approximated.push(what.into());
    }

    fn note_dropped(&mut self, what: impl Into<String>) {
        self.dropped.push(what.into());
    }
}

/// One imported component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedIp {
    /// The manifest, ready to be written as `reticle.ip`.
    pub manifest: IpManifest,
    /// The component's IP-XACT identity, kept because the manifest has
    /// nowhere to put a vendor or a library.
    pub vlnv: Vlnv,
    /// Which revision the document followed.
    pub standard: Standard,
    /// What was translated, approximated and dropped.
    pub report: ImportReport,
}

impl ImportedIp {
    /// The manifest text, for writing next to the sources.
    pub fn to_text(&self) -> String {
        self.manifest.to_text()
    }
}

// ---------------------------------------------------------------------------
// The import
// ---------------------------------------------------------------------------

/// Reads an IP-XACT component description into a manifest.
///
/// Returns `None` when the document is not well-formed XML, is not a
/// component, follows no namespace this understands, or has no VLNV;
/// every other problem is a diagnostic and a line in the
/// [`ImportReport`], and the manifest still comes back.
pub fn import(xml: &str, options: &ImportOptions, diags: &mut Diagnostics) -> Option<ImportedIp> {
    let document = Document::parse(xml, options.file, diags)?;
    let root = &document.root;
    let span = document.span;

    if !root.is("component") {
        let what = root.name.local.clone();
        diags.push(
            Diagnostic::error(format!(
                "this is an IP-XACT `{what}` description, not a `component`"
            ))
            .with_code(NOT_A_COMPONENT)
            .with_span(root.name.span)
            .with_note(
                "Reticle imports components; a design, catalogue or bus definition \
                        describes no package",
            ),
        );
        return None;
    }

    let mut report = ImportReport::default();
    let standard = match root.namespace_of() {
        Some(uri) => match Standard::from_namespace(uri) {
            Some(standard) => standard,
            None if is_old_spirit(uri) => {
                report.note_approximated(format!(
                    "namespace {uri} read as {}",
                    Standard::Spirit2009.describe()
                ));
                Standard::Spirit2009
            }
            None => {
                diags.push(unknown_standard(Some(uri), root.name.span));
                return None;
            }
        },
        None => {
            diags.push(unknown_standard(None, root.name.span));
            return None;
        }
    };
    report.note_translated(format!("standard {}", standard.describe()));

    let Some(vlnv) = Vlnv::from_children(root) else {
        diags.push(
            Diagnostic::error("the component has no complete VLNV")
                .with_code(MISSING_ELEMENT)
                .with_span(root.name.span)
                .with_note("a component needs `vendor`, `library`, `name` and `version`"),
        );
        return None;
    };

    let name = match &options.name {
        Some(name) => name.clone(),
        None => {
            let derived = sanitise_name(&vlnv.name);
            if derived != vlnv.name {
                report.note_approximated(format!(
                    "name `{}` spelled `{derived}` for a package name",
                    vlnv.name
                ));
            }
            derived
        }
    };
    let version = match &options.version {
        Some(version) => version.clone(),
        None => {
            let (version, note) = map_version(&vlnv.version);
            if let Some(note) = note {
                report.note_approximated(format!("version `{}`: {note}", vlnv.version));
                diags.push(
                    Diagnostic::warning(format!(
                        "the version `{}` is not a semantic version; using {version}",
                        vlnv.version
                    ))
                    .with_code(ODD_VERSION)
                    .with_span(
                        root.child("version")
                            .map_or(root.name.span, |e| e.name.span),
                    )
                    .with_note(note),
                );
            }
            version
        }
    };
    report.note_translated(format!("component {vlnv} -> package {name} {version}"));

    let mut manifest = IpManifest::new(name, version, span);
    manifest.license.clone_from(&options.license);

    let mut importer = Importer {
        options,
        diags,
        report,
        parameters: BTreeMap::new(),
        covered: BTreeSet::new(),
    };

    importer.read_parameters(root, &mut manifest);
    importer.read_top(root, &mut manifest, &vlnv);
    let ports = importer.read_ports(root);
    importer.read_interfaces(root, &mut manifest, &ports);
    importer.read_sources(root, &mut manifest);
    importer.read_description(root, &mut manifest);
    importer.note_leftovers(root);

    // Ports a recognised interface already stands for are not written
    // again; `port` is for what stands alone.
    let mut kept = 0usize;
    for (port, _) in &ports {
        if importer.covered.contains(&port.name.to_ascii_lowercase()) {
            continue;
        }
        kept += 1;
        manifest.ports.push(port.clone());
    }
    if kept > 0 {
        importer.report.note_translated(format!("ports: {kept}"));
    }

    let Importer { report, .. } = importer;
    Some(ImportedIp {
        manifest,
        vlnv,
        standard,
        report,
    })
}

/// True for the SPIRIT namespaces that came before 1685-2009.
fn is_old_spirit(uri: &str) -> bool {
    uri.starts_with("http://www.spiritconsortium.org/XMLSchema/SPIRIT/")
}

fn unknown_standard(uri: Option<&str>, span: Span) -> Diagnostic {
    let message = match uri {
        Some(uri) => format!("`{uri}` is not an IP-XACT namespace"),
        None => "the root element is in no namespace".to_owned(),
    };
    let known: Vec<&str> = Standard::ALL.iter().map(|s| s.namespace()).collect();
    Diagnostic::error(message)
        .with_code(UNKNOWN_STANDARD)
        .with_span(span)
        .with_note(format!("Reticle reads {}", known.join(", ")))
        .with_note(
            "the prefix may be anything; it is the namespace URI that says which \
                    revision the document follows",
        )
}

/// The state one import carries between its steps.
///
/// The revision is deliberately not in here: every step reads both
/// spellings of whatever it is looking for, so a component that mixes
/// them — which a catalogue converted by a tool often does — is read
/// rather than half-read.
struct Importer<'a> {
    options: &'a ImportOptions,
    diags: &'a mut Diagnostics,
    report: ImportReport,
    /// Every parameter's resolved value, under its name and its id.
    parameters: BTreeMap<String, i64>,
    /// The lower-cased names of the ports a bus interface stands for.
    covered: BTreeSet<String>,
}

/// A port as the model declares it, with the element it came from.
type ModelPort = (PortDecl, Span);

impl Importer<'_> {
    // -- parameters ----------------------------------------------------

    /// Reads the component's parameters from all three places the two
    /// revisions keep them.
    fn read_parameters(&mut self, root: &Element, manifest: &mut IpManifest) {
        let mut groups: Vec<&Element> = Vec::new();
        if let Some(element) = root.child("parameters") {
            groups.push(element);
        }
        if let Some(element) = root.find(&["model", "modelParameters"]) {
            groups.push(element);
        }
        if let Some(instantiations) = root.find(&["model", "instantiations"]) {
            for instantiation in instantiations.children_named("componentInstantiation") {
                if let Some(element) = instantiation.child("moduleParameters") {
                    groups.push(element);
                }
            }
        }

        let mut count = 0usize;
        for group in groups {
            for element in group.elements() {
                if !element.is("parameter")
                    && !element.is("modelParameter")
                    && !element.is("moduleParameter")
                {
                    continue;
                }
                let Some(name) = element.child_text("name").filter(|n| !n.is_empty()) else {
                    self.report
                        .note_dropped("a parameter with no name".to_owned());
                    continue;
                };
                if manifest.param(&name).is_some() {
                    continue;
                }
                let value_element = element.child("value");
                let value = value_element.map(Element::trimmed_text);

                // The identifier an expression elsewhere refers to it by:
                // an attribute on the parameter from 2014, on the value
                // from 2009.
                let id = element
                    .attr("parameterId")
                    .or_else(|| value_element.and_then(|v| v.attr("id")))
                    .map(str::to_owned);
                if let Some(text) = &value
                    && let Some(number) = eval(text, &self.parameters)
                {
                    self.parameters.insert(name.clone(), number);
                    if let Some(id) = &id {
                        self.parameters.insert(id.clone(), number);
                    }
                }

                // 2014 writes the type as an attribute, 2009's
                // `modelParameter` as a `dataType` child holding the HDL
                // type.
                let declared = element
                    .attr("type")
                    .map(str::to_owned)
                    .or_else(|| element.child_text("dataType"));
                let ty = param_type(declared.as_deref(), value.as_deref());

                let mut default = value.clone();
                if let Some(text) = &default
                    && ty != ParamType::Int
                    && text.contains("..")
                {
                    // A `..` in a word is a range to the manifest
                    // tokenizer, so a default holding one could not be
                    // read back.
                    self.report.note_dropped(format!(
                        "parameter {name}: the default `{text}` is not writable"
                    ));
                    default = None;
                }
                // An integer default written as an expression is
                // resolved here, because a manifest's default is a
                // value: nothing downstream evaluates `ADDR_WIDTH-4`.
                if ty == ParamType::Int
                    && let Some(text) = &default
                    && text.parse::<i64>().is_err()
                    && let Some(number) = eval(text, &self.parameters)
                {
                    self.report.note_approximated(format!(
                        "parameter {name}: the default `{text}` evaluates to {number}"
                    ));
                    default = Some(number.to_string());
                }
                let range = self.param_range(element, &name, ty, default.as_deref());

                manifest.params.push(ParamDecl {
                    name,
                    ty,
                    default,
                    range,
                    span: element.span,
                });
                count += 1;
            }
        }
        if count > 0 {
            self.report.note_translated(format!("parameters: {count}"));
        }
    }

    /// The `minimum`/`maximum` of an `int` parameter, when both are
    /// there and the default fits between them.
    fn param_range(
        &mut self,
        element: &Element,
        name: &str,
        ty: ParamType,
        default: Option<&str>,
    ) -> Option<(i64, i64)> {
        let read = |key: &str| -> Option<String> {
            element
                .attr(key)
                .map(str::to_owned)
                .or_else(|| element.child_text(key))
        };
        let (lo, hi) = (read("minimum")?, read("maximum")?);
        if ty != ParamType::Int {
            self.report.note_dropped(format!(
                "parameter {name}: the range {lo}..{hi} is only written for an `int`"
            ));
            return None;
        }
        let (Some(lo), Some(hi)) = (eval(&lo, &self.parameters), eval(&hi, &self.parameters))
        else {
            self.report.note_dropped(format!(
                "parameter {name}: the range `{lo}`..`{hi}` does not evaluate"
            ));
            return None;
        };
        if lo > hi {
            self.report
                .note_dropped(format!("parameter {name}: the range {lo}..{hi} is empty"));
            return None;
        }
        if let Some(value) = default.and_then(|d| d.parse::<i64>().ok())
            && (value < lo || value > hi)
        {
            self.report.note_dropped(format!(
                "parameter {name}: the range {lo}..{hi} excludes the default {value}"
            ));
            return None;
        }
        Some((lo, hi))
    }

    // -- the top -------------------------------------------------------

    /// The HDL name the package is instantiated as.
    fn read_top(&mut self, root: &Element, manifest: &mut IpManifest, vlnv: &Vlnv) {
        // 2014 and later: a component instantiation names the module.
        if let Some(instantiations) = root.find(&["model", "instantiations"]) {
            for instantiation in instantiations.children_named("componentInstantiation") {
                if let Some(module) = instantiation
                    .child_text("moduleName")
                    .filter(|m| !m.is_empty())
                {
                    manifest.top = Some(module);
                    return;
                }
            }
        }
        // 2009: the view carries the model name, sometimes as
        // `entity(architecture)`.
        if let Some(views) = root.find(&["model", "views"]) {
            for view in views.children_named("view") {
                if let Some(model) = view.child_text("modelName").filter(|m| !m.is_empty()) {
                    let top = model
                        .split_once('(')
                        .map_or(model.as_str(), |(e, _)| e)
                        .trim();
                    manifest.top = Some(top.to_owned());
                    return;
                }
            }
        }
        manifest.top = Some(vlnv.name.clone());
        self.report.note_approximated(format!(
            "top: no view names a module, using `{}`",
            vlnv.name
        ));
    }

    // -- ports ---------------------------------------------------------

    /// Every wire port of the model, in document order.
    fn read_ports(&mut self, root: &Element) -> Vec<ModelPort> {
        let mut out = Vec::new();
        let Some(ports) = root.find(&["model", "ports"]) else {
            return out;
        };
        for port in ports.children_named("port") {
            let Some(name) = port.child_text("name").filter(|n| !n.is_empty()) else {
                self.report.note_dropped("a port with no name".to_owned());
                continue;
            };
            let Some(wire) = port.child("wire") else {
                let kind = if port.child("transactional").is_some() {
                    "transactional"
                } else {
                    "neither wire nor transactional"
                };
                self.report
                    .note_dropped(format!("port {name}: {kind} ports have no HDL shape here"));
                continue;
            };
            let direction = wire.child_text("direction").unwrap_or_default();
            let dir = match direction.as_str() {
                "in" => PortDir::In,
                "out" => PortDir::Out,
                "inout" => PortDir::InOut,
                "phantom" => {
                    self.report
                        .note_dropped(format!("port {name}: a phantom port is not in the HDL"));
                    continue;
                }
                other => {
                    self.diags.push(
                        Diagnostic::warning(format!(
                            "the port `{name}` has the direction `{other}`"
                        ))
                        .with_code(UNKNOWN_DIRECTION)
                        .with_span(wire.span)
                        .with_note("a wire port is `in`, `out`, `inout` or `phantom`"),
                    );
                    self.report
                        .note_dropped(format!("port {name}: unknown direction `{other}`"));
                    continue;
                }
            };
            let width = match self.port_width(wire, &name) {
                Ok(width) => width,
                Err(()) => continue,
            };
            out.push((
                PortDecl {
                    name,
                    dir,
                    width,
                    span: port.span,
                },
                port.span,
            ));
        }
        out
    }

    /// The width a `wire` declares, from its vector bounds.
    fn port_width(&mut self, wire: &Element, name: &str) -> Result<Width, ()> {
        // 2009 writes `wire/vector`; 2014 and later wrap it in `vectors`.
        let vector = wire
            .child("vector")
            .or_else(|| wire.find(&["vectors", "vector"]));
        let Some(vector) = vector else {
            return Ok(Width::Fixed(1));
        };
        let left = vector.child_text("left").unwrap_or_default();
        let right = vector.child_text("right").unwrap_or_default();
        if left.is_empty() && right.is_empty() {
            return Ok(Width::Fixed(1));
        }
        match self.vector_width(&left, &right) {
            Ok(width) => {
                if matches!(width, Width::Derived { .. }) {
                    self.report.note_approximated(format!(
                        "port {name}: the width stays the expression `{width}`, since \
                         `{left}` has no value here"
                    ));
                }
                Ok(width)
            }
            Err(reason) => {
                self.diags.push(
                    Diagnostic::warning(format!(
                        "the width of the port `{name}` does not evaluate: {reason}"
                    ))
                    .with_code(UNRESOLVED_EXPRESSION)
                    .with_span(vector.span)
                    .with_note(
                        "give the parameters it uses a value, or declare the port by \
                                hand: a width Reticle had to guess would be worse than none",
                    ),
                );
                self.report.note_dropped(format!(
                    "port {name}: the width `{left}:{right}` does not evaluate"
                ));
                Err(())
            }
        }
    }

    /// The width between two vector bounds.
    ///
    /// Both bounds are evaluated when they can be; when they cannot, the
    /// `PARAM-1 : 0` and `PARAM/n-1 : 0` shapes still become a manifest
    /// [`Width::Derived`], which is what carries a `DATA_WIDTH/8` strobe
    /// across.
    fn vector_width(&self, left: &str, right: &str) -> Result<Width, String> {
        let left = if left.is_empty() { "0" } else { left };
        let right = if right.is_empty() { "0" } else { right };
        let high = eval(left, &self.parameters);
        let low = eval(right, &self.parameters);
        if let (Some(high), Some(low)) = (high, low) {
            // Either way round: IP-XACT allows an ascending vector, and
            // `[0:7]` is eight bits just as `[7:0]` is.
            let bits = high.abs_diff(low) + 1;
            return match u32::try_from(bits) {
                Ok(bits) => Ok(Width::Fixed(bits)),
                Err(_) => Err(format!("{bits} bits is not a width")),
            };
        }
        if low == Some(0)
            && let Some(width) = derived_width(left)
        {
            return Ok(width);
        }
        Err(format!("`{left}` and `{right}` are not both constant"))
    }

    // -- bus interfaces ------------------------------------------------

    /// Maps every bus interface, recording which ports each consumes.
    fn read_interfaces(&mut self, root: &Element, manifest: &mut IpManifest, ports: &[ModelPort]) {
        let Some(interfaces) = root.child("busInterfaces") else {
            return;
        };
        for interface in interfaces.children_named("busInterface") {
            let Some(name) = interface.child_text("name").filter(|n| !n.is_empty()) else {
                self.report
                    .note_dropped("a bus interface with no name".to_owned());
                continue;
            };
            let Some(bus_type) = interface.child("busType") else {
                self.report.note_dropped(format!(
                    "interface {name}: no `busType` says what bus it is"
                ));
                continue;
            };
            let Some(vlnv) = Vlnv::from_attributes(bus_type) else {
                self.report.note_dropped(format!(
                    "interface {name}: its `busType` has no complete VLNV"
                ));
                continue;
            };

            let maps = port_maps(interface);
            let (bus, note) = match self.map_bus(&vlnv) {
                Some(mapped) => mapped,
                None => {
                    self.diags.push(
                        Diagnostic::warning(format!(
                            "no Reticle bus matches the `busType` `{vlnv}` of `{name}`"
                        ))
                        .with_code(UNKNOWN_BUS)
                        .with_span(bus_type.span)
                        .with_note(format!(
                            "the built-in buses are {}; write a `.bus` file, or pass the \
                             mapping in `ImportOptions::bus_map`",
                            builtin_names().join(", ")
                        )),
                    );
                    // The ports themselves are not lost: they simply fall
                    // through to the `port` lines below, since nothing
                    // marked them as covered.
                    let kept = match maps.len() {
                        0 => String::new(),
                        1 => " (its port is kept as a plain port)".to_owned(),
                        n => format!(" (its {n} ports are kept as plain ports)"),
                    };
                    self.report
                        .note_dropped(format!("interface {name}: no bus matches {vlnv}{kept}"));
                    continue;
                }
            };
            if let Some(note) = note {
                self.report
                    .note_approximated(format!("interface {name}: {note}"));
            }

            let Some((role, role_note)) = self.map_role(interface, &name) else {
                continue;
            };
            if let Some(note) = role_note {
                self.report
                    .note_approximated(format!("interface {name}: {note}"));
            }

            let local = sanitise_name(&name);
            let (prefix, inferred) = self.prefix_of(&local, &maps, &bus, ports);
            if inferred {
                // With no port map there is nothing to measure the
                // prefix against, so say which of the two guesses was
                // made rather than printing an empty one.
                let how = if prefix.is_empty() {
                    "its ports carry no prefix".to_owned()
                } else {
                    format!("the prefix `{prefix}` comes from its name")
                };
                self.report
                    .note_approximated(format!("interface {name}: no port map, so {how}"));
            }

            for (_, physical) in &maps {
                self.covered.insert(physical.to_ascii_lowercase());
            }
            let signals = self.cover_by_prefix(&name, &bus, &prefix, role, ports, maps.is_empty());

            let declared = if prefix == format!("{local}_") {
                None
            } else {
                Some(prefix.clone())
            };
            manifest.interfaces.push(InterfaceDecl {
                name: local,
                bus: bus.clone(),
                role,
                prefix: declared,
                span: interface.span,
            });
            self.report.note_translated(format!(
                "interface {name}: {vlnv} -> {bus} {role} (prefix `{prefix}`, {signals} signals)"
            ));
        }
    }

    /// The Reticle bus a `busType` names, with a note when it is not an
    /// exact match.
    fn map_bus(&self, vlnv: &Vlnv) -> Option<(String, Option<String>)> {
        if let Some(bus) = self.options.mapped_bus(&vlnv.name) {
            return Some((bus.to_owned(), None));
        }
        let key = normalise_bus_name(&vlnv.name);
        if let Some((_, bus, note)) = BUS_TABLE.iter().find(|(from, _, _)| *from == key) {
            return Some((
                (*bus).to_owned(),
                note.map(|note| format!("{} read as {bus}: {note}", vlnv.name)),
            ));
        }
        let (_, bus) = BUS_ALIASES.iter().find(|(from, _)| *from == key)?;
        Some(((*bus).to_owned(), None))
    }

    /// Which side of the bus the interface is on.
    fn map_role(&mut self, interface: &Element, name: &str) -> Option<(BusRole, Option<String>)> {
        for (word, role, note) in ROLE_TABLE {
            if interface.child(word).is_some() {
                return Some((role, note.map(str::to_owned)));
            }
        }
        for word in ["system", "mirroredSystem"] {
            if interface.child(word).is_some() {
                self.report.note_dropped(format!(
                    "interface {name}: a `{word}` interface has no Reticle role"
                ));
                return None;
            }
        }
        self.report.note_dropped(format!(
            "interface {name}: nothing says which side of the bus it is"
        ));
        None
    }

    /// The port-name prefix the interface's ports carry.
    ///
    /// The port maps say it exactly: a physical port `s_axi_awaddr` for
    /// the logical signal `AWADDR` leaves `s_axi_`. With no port maps
    /// there is nothing to measure, so the interface's own name is used,
    /// and the caller reports that it was.
    fn prefix_of(
        &self,
        local: &str,
        maps: &[(String, String)],
        bus: &str,
        ports: &[ModelPort],
    ) -> (String, bool) {
        let mut votes: BTreeMap<String, usize> = BTreeMap::new();
        for (logical, physical) in maps {
            let logical = logical.to_ascii_lowercase();
            let lower = physical.to_ascii_lowercase();
            if let Some(prefix) = lower.strip_suffix(&logical) {
                *votes.entry(prefix.to_owned()).or_default() += 1;
            }
        }
        // Ties break towards the longer prefix, then alphabetically, so
        // two equally popular readings of the same port maps always pick
        // the same one.
        let best = votes
            .iter()
            .max_by(|a, b| {
                a.1.cmp(b.1)
                    .then_with(|| a.0.len().cmp(&b.0.len()))
                    .then_with(|| b.0.cmp(a.0))
            })
            .map(|(prefix, _)| prefix.clone());
        if let Some(prefix) = best {
            return (prefix, false);
        }
        // No usable map: the conventional prefix is the interface name,
        // unless the ports say the bus signals carry none at all.
        let bare = bus::builtin(bus).is_some_and(|definition| {
            definition.signals.iter().filter(|s| s.required).all(|s| {
                ports
                    .iter()
                    .any(|(port, _)| port.name.eq_ignore_ascii_case(&s.name))
            })
        });
        if bare {
            (String::new(), true)
        } else {
            (format!("{local}_"), true)
        }
    }

    /// Marks the ports a recognised interface stands for and counts the
    /// bus signals that were found.
    fn cover_by_prefix(
        &mut self,
        name: &str,
        bus: &str,
        prefix: &str,
        role: BusRole,
        ports: &[ModelPort],
        cover: bool,
    ) -> usize {
        let Some(definition) = bus::builtin(bus) else {
            return 0;
        };
        let mut found = 0usize;
        let mut missing = Vec::new();
        for signal in &definition.signals {
            let wanted = format!("{prefix}{}", signal.name);
            let port = ports
                .iter()
                .find(|(port, _)| port.name.eq_ignore_ascii_case(&wanted));
            match port {
                Some((port, _)) => {
                    found += 1;
                    if cover {
                        self.covered.insert(port.name.to_ascii_lowercase());
                    }
                    if port.dir != signal.direction(role) {
                        self.report.note_approximated(format!(
                            "port {}: the bus wants it {}, the component declares it {}",
                            port.name,
                            signal.direction(role).keyword(),
                            port.dir.keyword()
                        ));
                    }
                }
                None if signal.required => missing.push(signal.name.clone()),
                None => {}
            }
        }
        if !missing.is_empty() {
            self.report.note_approximated(format!(
                "interface {name}: no port for the required {bus} signals {}",
                missing.join(", ")
            ));
        }
        found
    }

    // -- file sets -----------------------------------------------------

    /// Every file set's files, as `source` lines.
    fn read_sources(&mut self, root: &Element, manifest: &mut IpManifest) {
        let Some(sets) = root.child("fileSets") else {
            return;
        };
        for set in sets.children_named("fileSet") {
            let set_name = set
                .child_text("name")
                .unwrap_or_else(|| "<unnamed>".to_owned());
            let mut count = 0usize;
            for file in set.children_named("file") {
                let Some(path) = file.child_text("name").filter(|p| !p.is_empty()) else {
                    self.report
                        .note_dropped(format!("file set {set_name}: a file with no name"));
                    continue;
                };
                if file.child_text("isIncludeFile").as_deref() == Some("true") {
                    self.report.note_dropped(format!(
                        "file {path}: an include file, and Reticle does not follow `include`"
                    ));
                    continue;
                }
                let types: Vec<String> = file
                    .children_named("fileType")
                    .chain(file.children_named("userFileType"))
                    .map(Element::trimmed_text)
                    .collect();
                let language = types.iter().find_map(|t| language_of(t)).or_else(|| {
                    let implied = Language::from_path(&path);
                    if implied.is_some() {
                        self.report.note_approximated(format!(
                            "file {path}: the file type {} says nothing, the extension does",
                            types
                                .first()
                                .map_or_else(|| "(none)".to_owned(), |t| format!("`{t}`"))
                        ));
                    }
                    implied
                });
                let Some(language) = language else {
                    self.diags.push(
                        Diagnostic::warning(format!("`{path}` is not an HDL source"))
                            .with_code(UNKNOWN_FILE_TYPE)
                            .with_span(file.span)
                            .with_note(format!(
                                "its file type is {}; Reticle imports Verilog, \
                                 SystemVerilog and VHDL sources",
                                types
                                    .first()
                                    .map_or_else(|| "absent".to_owned(), |t| format!("`{t}`"))
                            )),
                    );
                    self.report.note_dropped(format!(
                        "file {path}: {} is not an HDL source",
                        types.first().map_or_else(
                            || "no file type".to_owned(),
                            |t| format!("the type `{t}`")
                        )
                    ));
                    continue;
                };
                // The `language` word is only written when the extension
                // does not already say the same thing.
                let explicit = (Language::from_path(&path) != Some(language)).then_some(language);
                manifest.sources.push(SourceEntry {
                    path,
                    language: explicit,
                    encrypted: false,
                    span: file.span,
                });
                count += 1;
            }
            self.report
                .note_translated(format!("file set {set_name}: {count} sources"));
        }
    }

    // -- description and memory maps -----------------------------------

    /// The one-line description, with the memory maps described into it.
    fn read_description(&mut self, root: &Element, manifest: &mut IpManifest) {
        let mut parts = Vec::new();
        if let Some(text) = root.child_text("description").filter(|d| !d.is_empty()) {
            parts.push(collapse(&text));
        }
        if let Some(maps) = root.child("memoryMaps") {
            for map in maps.children_named("memoryMap") {
                let name = map
                    .child_text("name")
                    .unwrap_or_else(|| "<unnamed>".to_owned());
                let mut blocks = Vec::new();
                for block in map.children_named("addressBlock") {
                    let block_name = block
                        .child_text("name")
                        .unwrap_or_else(|| "<unnamed>".to_owned());
                    let base = block.child_text("baseAddress").unwrap_or_default();
                    let range = block.child_text("range").unwrap_or_default();
                    let width = block.child_text("width").unwrap_or_default();
                    let mut phrase = block_name;
                    if !base.is_empty() {
                        phrase.push_str(&format!(" at {}", collapse(&base)));
                    }
                    if !range.is_empty() {
                        phrase.push_str(&format!(" range {}", collapse(&range)));
                    }
                    if !width.is_empty() {
                        phrase.push_str(&format!(" width {}", collapse(&width)));
                    }
                    blocks.push(phrase);
                }
                let described = if blocks.is_empty() {
                    format!("memory map {name}")
                } else {
                    format!("memory map {name}: {}", blocks.join(", "))
                };
                self.report
                    .note_translated(format!("{described} (described, not turned into logic)"));
                parts.push(described);
            }
        }
        if !parts.is_empty() {
            // A description that already ends in a full stop does not
            // want a semicolon after it as well.
            let last = parts.len() - 1;
            for (i, part) in parts.iter_mut().enumerate() {
                if i != last {
                    *part = part.trim_end_matches('.').to_owned();
                }
            }
            manifest.description = Some(parts.join("; "));
        }
    }

    /// Notes the parts of a component nothing here reads.
    fn note_leftovers(&mut self, root: &Element) {
        for (local, why) in [
            ("vendorExtensions", "vendor extensions are not read"),
            (
                "addressSpaces",
                "address spaces describe a processor's view, not a port",
            ),
            ("cpus", "a CPU description has no manifest line"),
            ("channels", "channels connect interfaces inside a design"),
            (
                "whiteboxElements",
                "white-box elements are a verification aid",
            ),
            (
                "indirectInterfaces",
                "indirect interfaces have no manifest line",
            ),
            ("choices", "choices constrain a configuration GUI"),
        ] {
            if root.child(local).is_some() {
                self.report.note_dropped(format!("{local}: {why}"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

/// `busType` names, normalised, and the Reticle bus each becomes.
///
/// The third column is the note that makes the mapping an approximation
/// rather than a translation.
const BUS_TABLE: [(&str, &str, Option<&str>); 17] = [
    ("AXI4LITE", "axi4lite", None),
    ("AXILITE", "axi4lite", None),
    ("AXI4STREAM", "axi4stream", None),
    ("AXISTREAM", "axi4stream", None),
    ("AXIS", "axi4stream", None),
    ("AXI4", "axi4", None),
    ("AXI3", "axi4", Some("AXI4 is the closest built-in")),
    (
        "AXI",
        "axi4",
        Some("the revision is not named; AXI4 assumed"),
    ),
    ("APB", "apb", None),
    ("APB2", "apb", None),
    ("APB3", "apb", None),
    ("APB4", "apb", None),
    ("WISHBONE", "wishbone", None),
    ("WISHBONEB4", "wishbone", None),
    ("WB", "wishbone", None),
    ("WISHBONEPIPELINED", "wishbone_pipelined", None),
    ("AVALONMM", "avalon_mm", None),
];

/// The extra names that are not simply the bus spelled differently.
///
/// Kept apart from [`BUS_TABLE`] only because the table above is already
/// the documented one; both are searched.
const BUS_ALIASES: [(&str, &str); 3] = [
    ("AVALON", "avalon_mm"),
    ("AVALONMEMORYMAPPED", "avalon_mm"),
    ("WBPIPELINED", "wishbone_pipelined"),
];

/// The element that says which side of a bus an interface is on.
const ROLE_TABLE: [(&str, BusRole, Option<&str>); 7] = [
    ("master", BusRole::Manager, None),
    ("initiator", BusRole::Manager, None),
    ("slave", BusRole::Subordinate, None),
    ("target", BusRole::Subordinate, None),
    ("monitor", BusRole::Monitor, None),
    (
        "mirroredMaster",
        BusRole::Subordinate,
        Some("a mirrored master drives the subordinate's directions"),
    ),
    (
        "mirroredSlave",
        BusRole::Manager,
        Some("a mirrored slave drives the manager's directions"),
    ),
];

/// IP-XACT file types and the language each means.
const FILE_TYPES: [(&str, Language); 13] = [
    ("verilogsource", Language::Verilog),
    ("verilogsource-95", Language::Verilog),
    ("verilogsource-2001", Language::Verilog),
    ("verilogsource-2005", Language::Verilog),
    ("systemverilogsource", Language::SystemVerilog),
    ("systemverilogsource-3.0", Language::SystemVerilog),
    ("systemverilogsource-3.1", Language::SystemVerilog),
    ("systemverilogsource-3.1a", Language::SystemVerilog),
    ("vhdlsource", Language::Vhdl),
    ("vhdlsource-87", Language::Vhdl),
    ("vhdlsource-93", Language::Vhdl),
    ("vhdlsource-2002", Language::Vhdl),
    ("vhdlsource-2008", Language::Vhdl),
];

/// The language an IP-XACT file type names.
fn language_of(file_type: &str) -> Option<Language> {
    let key = file_type.trim().to_ascii_lowercase();
    if let Some((_, language)) = FILE_TYPES.iter().find(|(name, _)| *name == key) {
        return Some(*language);
    }
    // Revisions of these three keep arriving; the stem is what matters.
    if key.starts_with("systemverilogsource") {
        return Some(Language::SystemVerilog);
    }
    if key.starts_with("verilogsource") {
        return Some(Language::Verilog);
    }
    if key.starts_with("vhdlsource") {
        return Some(Language::Vhdl);
    }
    None
}

/// The built-in bus names, for the note on an unrecognised `busType`.
fn builtin_names() -> Vec<&'static str> {
    bus::builtin_buses()
        .iter()
        .map(|b| b.name.as_str())
        .collect()
}

/// Upper-cases a `busType` name and drops everything but letters and
/// digits, so `AXI4-Lite`, `axi4_lite` and `AXI4LITE` are one key.
fn normalise_bus_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

// ---------------------------------------------------------------------------
// Names, versions and expressions
// ---------------------------------------------------------------------------

/// Turns an IP-XACT name into a package name.
///
/// Package names end up in manifests, paths and diagnostics, so they are
/// lower case, and everything that is not a letter, a digit or an
/// underscore becomes one.
pub fn sanitise_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push_str("unnamed");
    } else if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert_str(0, "ip_");
    }
    out
}

/// Turns an IP-XACT version string into a [`Version`].
///
/// IP-XACT leaves the version as free text, and catalogues use all of
/// `1.2.3`, `1.2`, `2`, `r0p0_0` and `v1.0-beta`. The rules, in order:
///
/// 1. `major.minor.patch[-pre]` is taken as written.
/// 2. `major.minor` becomes `major.minor.0`, and `major` becomes
///    `major.0.0`.
/// 3. A longer dotted number keeps its first three parts.
/// 4. Anything else with a leading number keeps that number and puts the
///    rest in the pre-release tag.
/// 5. Anything else at all becomes `0.0.0` with the whole string as the
///    pre-release tag.
///
/// The second return value is the note for the report, `None` when rule
/// 1 applied.
///
/// ```
/// use reticle::ip::ipxact::map_version;
/// assert_eq!(map_version("1.2.3").0.to_string(), "1.2.3");
/// assert_eq!(map_version("1.2").0.to_string(), "1.2.0");
/// assert_eq!(map_version("r0p0_0").0.to_string(), "0.0.0-r0p0.0");
/// ```
pub fn map_version(text: &str) -> (Version, Option<String>) {
    let text = text.trim();
    if let Some(version) = Version::parse(text) {
        return (version, None);
    }
    let trimmed = text.strip_prefix(['v', 'V']).unwrap_or(text);
    // The longest leading run of digits and dots is the number; whatever
    // follows becomes the pre-release tag.
    let head: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let tail = &trimmed[head.len()..];
    let numbers: Vec<u64> = head
        .split('.')
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();
    let pre = sanitise_pre(tail);
    if numbers.is_empty() {
        let whole = sanitise_pre(text);
        let version = Version {
            major: 0,
            minor: 0,
            patch: 0,
            pre: Some(whole.clone().unwrap_or_else(|| "unversioned".to_owned())),
        };
        return (
            version,
            Some("it names no number at all, so it sorts below every release".to_owned()),
        );
    }
    let version = Version {
        major: numbers[0],
        minor: numbers.get(1).copied().unwrap_or(0),
        patch: numbers.get(2).copied().unwrap_or(0),
        pre: pre.clone(),
    };
    let note = if numbers.len() > 3 {
        format!("only the first three numbers are kept, giving {version}")
    } else if pre.is_some() {
        format!("the part after the numbers became a pre-release tag, giving {version}")
    } else {
        format!("the missing numbers are zero, giving {version}")
    };
    (version, Some(note))
}

/// Makes a pre-release tag out of arbitrary text.
///
/// A tag has to survive being written to a manifest and read back, so
/// everything but letters, digits and dots becomes a dot, leading and
/// trailing dots are dropped, and an empty result is no tag at all. The
/// `-` is excluded because [`Version::parse`] splits on the first one.
fn sanitise_pre(text: &str) -> Option<String> {
    let mapped: String = text
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '.' })
        .collect();
    let trimmed = mapped.trim_matches('.').to_owned();
    // Two dots in a row read no better than one.
    let mut out = String::with_capacity(trimmed.len());
    let mut last_dot = false;
    for c in trimmed.chars() {
        if c == '.' {
            if last_dot {
                continue;
            }
            last_dot = true;
        } else {
            last_dot = false;
        }
        out.push(c);
    }
    (!out.is_empty()).then_some(out)
}

/// Collapses every run of whitespace into one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `PARAM-1` and `PARAM/n-1` shapes, as a manifest width.
fn derived_width(left: &str) -> Option<Width> {
    let body = left.trim().strip_suffix("-1")?.trim().to_owned();
    match body.split_once('/') {
        None => is_identifier(&body).then(|| Width::param(body.clone())),
        Some((param, divisor)) => {
            let param = param.trim();
            let divisor: u32 = divisor.trim().parse().ok()?;
            (is_identifier(param) && divisor != 0).then(|| Width::Derived {
                param: param.to_owned(),
                divisor,
            })
        }
    }
}

/// True for a word that could be an HDL parameter name.
fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Evaluates an IP-XACT expression against resolved parameters.
///
/// The grammar is integers (decimal, or `0x` hexadecimal), parameter
/// names and ids, `+ - * / %`, unary `+` and `-`, and parentheses. That
/// is what a vector bound and an address block's range are written with.
/// Anything else — a function call, a string, a name nothing resolves —
/// is `None`, which the caller turns into a report line rather than a
/// guess.
///
/// A parameter name containing `-` cannot be told from a subtraction and
/// is therefore not resolved; parameter *ids* generated as UUIDs are
/// usually spelled with underscores for exactly that reason.
///
/// ```
/// use std::collections::BTreeMap;
/// use reticle::ip::ipxact::eval;
///
/// let params = BTreeMap::from([("W".to_owned(), 32i64)]);
/// assert_eq!(eval("W/8-1", &params), Some(3));
/// assert_eq!(eval("clog2(W)", &params), None);
/// ```
pub fn eval(expr: &str, params: &BTreeMap<String, i64>) -> Option<i64> {
    // A bound is a handful of characters; anything longer is either
    // generated nonsense or an attempt to make the parser recurse.
    if expr.len() > 512 {
        return None;
    }
    let tokens = tokenize_expression(expr)?;
    let mut parser = ExprParser {
        tokens: &tokens,
        at: 0,
        params,
    };
    let value = parser.sum()?;
    (parser.at == tokens.len()).then_some(value)
}

/// One token of an expression.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Number(i64),
    Name(String),
    Op(char),
}

fn tokenize_expression(expr: &str) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let bytes = expr.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
        } else if b.is_ascii_digit() {
            let start = i;
            if b == b'0' && matches!(bytes.get(i + 1), Some(b'x' | b'X')) {
                i += 2;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                out.push(Tok::Number(
                    i64::from_str_radix(&expr[start + 2..i], 16).ok()?,
                ));
            } else {
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                    i += 1;
                }
                out.push(Tok::Number(expr[start..i].replace('_', "").parse().ok()?));
            }
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            out.push(Tok::Name(expr[start..i].to_owned()));
        } else if matches!(b, b'+' | b'-' | b'*' | b'/' | b'%' | b'(' | b')') {
            out.push(Tok::Op(char::from(b)));
            i += 1;
        } else {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

struct ExprParser<'a> {
    tokens: &'a [Tok],
    at: usize,
    params: &'a BTreeMap<String, i64>,
}

impl ExprParser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Tok::Op(op)) {
            self.at += 1;
            return true;
        }
        false
    }

    fn sum(&mut self) -> Option<i64> {
        let mut value = self.product()?;
        loop {
            if self.eat('+') {
                value = value.checked_add(self.product()?)?;
            } else if self.eat('-') {
                value = value.checked_sub(self.product()?)?;
            } else {
                return Some(value);
            }
        }
    }

    fn product(&mut self) -> Option<i64> {
        let mut value = self.unary()?;
        loop {
            if self.eat('*') {
                value = value.checked_mul(self.unary()?)?;
            } else if self.eat('/') {
                value = value.checked_div(self.unary()?)?;
            } else if self.eat('%') {
                value = value.checked_rem(self.unary()?)?;
            } else {
                return Some(value);
            }
        }
    }

    fn unary(&mut self) -> Option<i64> {
        if self.eat('-') {
            return self.unary()?.checked_neg();
        }
        if self.eat('+') {
            return self.unary();
        }
        self.atom()
    }

    fn atom(&mut self) -> Option<i64> {
        match self.peek()?.clone() {
            Tok::Number(n) => {
                self.at += 1;
                Some(n)
            }
            Tok::Name(name) => {
                self.at += 1;
                // A call is not part of the grammar, and reading
                // `clog2(x)` as `clog2` times `x` would be a lie.
                if self.peek() == Some(&Tok::Op('(')) {
                    return None;
                }
                self.params.get(&name).copied()
            }
            Tok::Op('(') => {
                self.at += 1;
                let value = self.sum()?;
                self.eat(')').then_some(value)
            }
            Tok::Op(_) => None,
        }
    }
}

/// The `(logical, physical)` pairs of a bus interface's port maps.
///
/// 2009 hangs `portMaps` off the interface; 2014 and later hang it off
/// each `abstractionType`.
fn port_maps(interface: &Element) -> Vec<(String, String)> {
    let mut groups: Vec<&Element> = Vec::new();
    if let Some(maps) = interface.child("portMaps") {
        groups.push(maps);
    }
    if let Some(types) = interface.child("abstractionTypes") {
        for abstraction in types.children_named("abstractionType") {
            if let Some(maps) = abstraction.child("portMaps") {
                groups.push(maps);
            }
        }
    }
    let mut out = Vec::new();
    for group in groups {
        for map in group.children_named("portMap") {
            let logical = map
                .find(&["logicalPort", "name"])
                .map(Element::trimmed_text)
                .unwrap_or_default();
            let physical = map
                .find(&["physicalPort", "name"])
                .map(Element::trimmed_text)
                .unwrap_or_default();
            if !logical.is_empty() && !physical.is_empty() {
                out.push((logical, physical));
            }
        }
    }
    out
}

/// The manifest parameter type an IP-XACT parameter has.
///
/// `declared` is the `type` attribute or the `dataType` element; when
/// there is neither, or it is a word nothing here knows, the value
/// decides, which is how an untyped `32` still becomes an `int`.
fn param_type(declared: Option<&str>, value: Option<&str>) -> ParamType {
    let word = declared.unwrap_or("").trim().to_ascii_lowercase();
    match word.as_str() {
        "int" | "integer" | "longint" | "shortint" | "byte" | "natural" | "positive" => {
            ParamType::Int
        }
        "bit" | "bool" | "boolean" => ParamType::Bool,
        "string" | "str" => ParamType::Str,
        _ => match value {
            Some(v) if v.parse::<i64>().is_ok() => ParamType::Int,
            Some(v) if matches!(v.to_ascii_lowercase().as_str(), "true" | "false") => {
                ParamType::Bool
            }
            _ => ParamType::Str,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    /// Wraps `body` in a 1685-2014 component with the given VLNV name.
    fn component(name: &str, version: &str, body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <ipxact:component \
             xmlns:ipxact=\"http://www.accellera.org/XMLSchema/IPXACT/1685-2014\">\n\
             <ipxact:vendor>example.com</ipxact:vendor>\n\
             <ipxact:library>ip</ipxact:library>\n\
             <ipxact:name>{name}</ipxact:name>\n\
             <ipxact:version>{version}</ipxact:version>\n\
             {body}\n\
             </ipxact:component>\n"
        )
    }

    /// Imports `xml`, returning the result and everything reported.
    fn run(xml: &str) -> (Option<ImportedIp>, String) {
        let mut map = SourceMap::new();
        let file = map.add("component.xml", xml).unwrap();
        let mut diags = Diagnostics::new();
        let imported = import(xml, &ImportOptions::new(file), &mut diags);
        (imported, diags.render(&map))
    }

    /// Imports `xml`, expecting it to succeed.
    fn imported(xml: &str) -> ImportedIp {
        run(xml).0.expect("the component imports")
    }

    /// A port element in the 2014 spelling.
    fn port(name: &str, dir: &str, vector: &str) -> String {
        format!(
            "<ipxact:port><ipxact:name>{name}</ipxact:name><ipxact:wire>\
             <ipxact:direction>{dir}</ipxact:direction>{vector}</ipxact:wire></ipxact:port>"
        )
    }

    /// A `vectors/vector` with the given bounds, as 2014 writes it.
    fn vector(left: &str, right: &str) -> String {
        format!(
            "<ipxact:vectors><ipxact:vector><ipxact:left>{left}</ipxact:left>\
             <ipxact:right>{right}</ipxact:right></ipxact:vector></ipxact:vectors>"
        )
    }

    /// True when the manifest's text parses to the same manifest again.
    fn reparses(manifest: &IpManifest) -> bool {
        let text = manifest.to_text();
        let mut map = SourceMap::new();
        let file = map.add("reticle.ip", &text).unwrap();
        let mut diags = Diagnostics::new();
        let again = IpManifest::parse(&text, file, &mut diags);
        !diags.has_errors() && again.is_some_and(|again| again.to_text() == text)
    }

    #[test]
    fn the_bus_table_names_only_built_in_buses() {
        for (from, bus, _) in BUS_TABLE {
            assert!(
                bus::builtin(bus).is_some(),
                "{from} maps to `{bus}`, which is not a built-in bus"
            );
            assert_eq!(normalise_bus_name(from), from, "{from} is not normalised");
        }
        for (from, bus) in BUS_ALIASES {
            assert!(bus::builtin(bus).is_some(), "{from} maps to `{bus}`");
            assert_eq!(normalise_bus_name(from), from);
        }
        // The normalisation is what makes one key serve every spelling.
        assert_eq!(normalise_bus_name("AXI4-Lite"), "AXI4LITE");
        assert_eq!(normalise_bus_name("axi4_lite"), "AXI4LITE");
    }

    #[test]
    fn maps_versions_of_every_shape() {
        let cases = [
            ("1.2.3", "1.2.3", false),
            ("1.2.3-rc1", "1.2.3-rc1", false),
            ("1.2", "1.2.0", true),
            ("2", "2.0.0", true),
            ("1.2.3.4", "1.2.3", true),
            ("v1.0", "1.0.0", true),
            ("2.0.1_beta", "2.0.1-beta", true),
            ("r0p0_0", "0.0.0-r0p0.0", true),
            ("", "0.0.0-unversioned", true),
        ];
        for (text, expected, approximate) in cases {
            let (version, note) = map_version(text);
            assert_eq!(version.to_string(), expected, "for `{text}`");
            assert_eq!(note.is_some(), approximate, "for `{text}`");
            // Whatever comes out has to survive a manifest round trip.
            assert_eq!(Version::parse(&version.to_string()), Some(version));
        }
    }

    #[test]
    fn evaluates_the_expressions_bounds_are_written_with() {
        let params = BTreeMap::from([("W".to_owned(), 32i64), ("N".to_owned(), 4i64)]);
        assert_eq!(eval("31", &params), Some(31));
        assert_eq!(eval(" W - 1 ", &params), Some(31));
        assert_eq!(eval("W/8-1", &params), Some(3));
        assert_eq!(eval("(W+N)*2", &params), Some(72));
        assert_eq!(eval("0x10", &params), Some(16));
        assert_eq!(eval("-N", &params), Some(-4));
        assert_eq!(eval("W%5", &params), Some(2));
        assert_eq!(eval("W/0", &params), None);
        assert_eq!(eval("clog2(W)", &params), None);
        assert_eq!(eval("UNKNOWN-1", &params), None);
        assert_eq!(eval("W +", &params), None);
        assert_eq!(eval("\"eight\"", &params), None);
        assert_eq!(eval("", &params), None);
        assert_eq!(eval(&"(".repeat(600), &params), None);
    }

    #[test]
    fn imports_a_component_with_a_bus_interface() {
        let ports = format!(
            "<ipxact:model><ipxact:ports>{}{}{}</ipxact:ports></ipxact:model>",
            port("clk", "in", ""),
            port("s_axi_awaddr", "in", &vector("31", "0")),
            port("s_axi_awvalid", "in", ""),
        );
        let xml = component(
            "axil_regs",
            "1.0.0",
            &format!(
                "<ipxact:busInterfaces><ipxact:busInterface>\
                   <ipxact:name>S_AXI</ipxact:name>\
                   <ipxact:busType vendor=\"amba.com\" library=\"AMBA4\" name=\"AXI4LITE\" \
                    version=\"r0p0_0\"/>\
                   <ipxact:slave/>\
                   <ipxact:abstractionTypes><ipxact:abstractionType><ipxact:portMaps>\
                     <ipxact:portMap><ipxact:logicalPort><ipxact:name>AWADDR</ipxact:name>\
                       </ipxact:logicalPort><ipxact:physicalPort>\
                       <ipxact:name>s_axi_awaddr</ipxact:name></ipxact:physicalPort>\
                       </ipxact:portMap>\
                     <ipxact:portMap><ipxact:logicalPort><ipxact:name>AWVALID</ipxact:name>\
                       </ipxact:logicalPort><ipxact:physicalPort>\
                       <ipxact:name>s_axi_awvalid</ipxact:name></ipxact:physicalPort>\
                       </ipxact:portMap>\
                   </ipxact:portMaps></ipxact:abstractionType></ipxact:abstractionTypes>\
                 </ipxact:busInterface></ipxact:busInterfaces>{ports}"
            ),
        );
        let result = imported(&xml);
        assert_eq!(result.standard, Standard::Ipxact2014);
        assert_eq!(result.vlnv.to_string(), "example.com:ip:axil_regs:1.0.0");

        let interfaces = &result.manifest.interfaces;
        assert_eq!(interfaces.len(), 1);
        assert_eq!(interfaces[0].name, "s_axi");
        assert_eq!(interfaces[0].bus, "axi4lite");
        assert_eq!(interfaces[0].role, BusRole::Subordinate);
        // `s_axi_` is what the interface's own name gives, so no
        // `prefix` word is needed.
        assert_eq!(interfaces[0].prefix, None);
        assert_eq!(interfaces[0].prefix(), "s_axi_");

        // The two bus ports are the interface's; the clock stands alone.
        let names: Vec<&str> = result
            .manifest
            .ports
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["clk"]);
        assert!(
            result
                .report
                .translated
                .iter()
                .any(|line| line.contains("S_AXI") && line.contains("axi4lite subordinate")),
            "{}",
            result.report.describe()
        );
        // Most of the nineteen signals have no port, and the report says
        // which required ones are missing.
        assert!(
            result
                .report
                .approximated
                .iter()
                .any(|line| line.contains("no port for the required axi4lite signals")),
            "{}",
            result.report.describe()
        );
        assert!(reparses(&result.manifest));
    }

    #[test]
    fn evaluates_parameterised_port_widths() {
        let xml = component(
            "widths",
            "1.0.0",
            &format!(
                "<ipxact:parameters>\
                   <ipxact:parameter parameterId=\"p_data\" type=\"int\" minimum=\"8\" \
                    maximum=\"64\"><ipxact:name>DATA_WIDTH</ipxact:name>\
                    <ipxact:value>32</ipxact:value></ipxact:parameter>\
                   <ipxact:parameter><ipxact:name>USER_WIDTH</ipxact:name>\
                    <ipxact:value>unset</ipxact:value></ipxact:parameter>\
                 </ipxact:parameters>\
                 <ipxact:model><ipxact:ports>{}{}{}{}</ipxact:ports></ipxact:model>",
                port("data", "out", &vector("DATA_WIDTH-1", "0")),
                port("strb", "out", &vector("p_data/8-1", "0")),
                port("user", "in", &vector("USER_WIDTH-1", "0")),
                port("odd", "in", &vector("clog2(DEPTH)-1", "0")),
            ),
        );
        let (result, rendered) = run(&xml);
        let result = result.expect("imports");
        let widths: BTreeMap<&str, String> = result
            .manifest
            .ports
            .iter()
            .map(|p| (p.name.as_str(), p.width.to_string()))
            .collect();
        // Both bounds resolve, through the name and through the id.
        assert_eq!(widths.get("data").map(String::as_str), Some("32"));
        assert_eq!(widths.get("strb").map(String::as_str), Some("4"));
        // Nothing gives `USER_WIDTH` a value, but the shape is a width
        // the manifest can write.
        assert_eq!(widths.get("user").map(String::as_str), Some("USER_WIDTH"));
        // And a bound with a function call in it is dropped, loudly.
        assert!(!widths.contains_key("odd"));
        assert!(rendered.contains("does not evaluate"), "{rendered}");
        assert!(
            result
                .report
                .dropped
                .iter()
                .any(|line| line.starts_with("port odd:")),
            "{}",
            result.report.describe()
        );
        // The parameter's range came across, and the untyped one did not
        // become an `int`.
        let data = result.manifest.param("DATA_WIDTH").expect("DATA_WIDTH");
        assert_eq!(data.ty, ParamType::Int);
        assert_eq!(data.range, Some((8, 64)));
        assert_eq!(data.default.as_deref(), Some("32"));
        assert_eq!(
            result.manifest.param("USER_WIDTH").expect("USER_WIDTH").ty,
            ParamType::Str
        );
        assert!(reparses(&result.manifest));
    }

    #[test]
    fn an_unrecognised_bus_is_reported_and_its_ports_kept() {
        let xml = component(
            "ahb_thing",
            "1.0.0",
            &format!(
                "<ipxact:busInterfaces><ipxact:busInterface>\
                   <ipxact:name>m_ahb</ipxact:name>\
                   <ipxact:busType vendor=\"amba.com\" library=\"AMBA3\" name=\"AHBLite\" \
                    version=\"r2p0_0\"/>\
                   <ipxact:master/>\
                   <ipxact:portMaps><ipxact:portMap>\
                     <ipxact:logicalPort><ipxact:name>HADDR</ipxact:name></ipxact:logicalPort>\
                     <ipxact:physicalPort><ipxact:name>m_ahb_haddr</ipxact:name>\
                       </ipxact:physicalPort></ipxact:portMap></ipxact:portMaps>\
                 </ipxact:busInterface></ipxact:busInterfaces>\
                 <ipxact:model><ipxact:ports>{}</ipxact:ports></ipxact:model>",
                port("m_ahb_haddr", "out", &vector("31", "0")),
            ),
        );
        let (result, rendered) = run(&xml);
        let result = result.expect("imports");
        assert!(result.manifest.interfaces.is_empty());
        assert!(rendered.contains("no Reticle bus matches"), "{rendered}");
        assert!(
            rendered.contains("amba.com:AMBA3:AHBLite:r2p0_0"),
            "{rendered}"
        );
        assert!(
            result.report.dropped.iter().any(
                |line| line.contains("no bus matches") && line.contains("kept as a plain port")
            ),
            "{}",
            result.report.describe()
        );
        // Nothing is lost: the port is still in the manifest.
        assert_eq!(result.manifest.ports.len(), 1);
        assert_eq!(result.manifest.ports[0].name, "m_ahb_haddr");
        assert_eq!(result.manifest.ports[0].width, Width::Fixed(32));

        // And a site that has its own AHB bus can say so.
        let mut map = SourceMap::new();
        let file = map.add("component.xml", &xml).unwrap();
        let mut diags = Diagnostics::new();
        let options = ImportOptions::new(file).with_bus("AHB-Lite", "ahblite");
        let taught = import(&xml, &options, &mut diags).expect("imports");
        assert_eq!(taught.manifest.interfaces.len(), 1);
        assert_eq!(taught.manifest.interfaces[0].bus, "ahblite");
        assert_eq!(taught.manifest.interfaces[0].role, BusRole::Manager);
    }

    #[test]
    fn an_unusual_vlnv_version_is_mapped_and_reported() {
        let (result, rendered) = run(&component("core", "r0p2_1", "<ipxact:model/>"));
        let result = result.expect("imports");
        assert_eq!(result.manifest.version.to_string(), "0.0.0-r0p2.1");
        assert_eq!(result.vlnv.version, "r0p2_1");
        assert!(rendered.contains("is not a semantic version"), "{rendered}");
        assert!(
            result
                .report
                .approximated
                .iter()
                .any(|line| line.starts_with("version `r0p2_1`")),
            "{}",
            result.report.describe()
        );
        // A name that is not a package name is spelled as one.
        let result = imported(&component("My Core.v2", "1.0.0", "<ipxact:model/>"));
        assert_eq!(result.manifest.name, "my_core_v2");
        assert!(reparses(&result.manifest));
    }

    #[test]
    fn reads_the_2009_spelling() {
        let xml = "<?xml version=\"1.0\"?>\n\
            <spirit:component \
             xmlns:spirit=\"http://www.spiritconsortium.org/XMLSchema/SPIRIT/1685-2009\">\
             <spirit:vendor>example.com</spirit:vendor>\
             <spirit:library>ip</spirit:library>\
             <spirit:name>legacy</spirit:name>\
             <spirit:version>1.0</spirit:version>\
             <spirit:busInterfaces><spirit:busInterface>\
               <spirit:name>apb</spirit:name>\
               <spirit:busType spirit:vendor=\"amba.com\" spirit:library=\"AMBA2\" \
                spirit:name=\"APB\" spirit:version=\"r1p0\"/>\
               <spirit:slave/>\
             </spirit:busInterface></spirit:busInterfaces>\
             <spirit:model>\
               <spirit:views><spirit:view><spirit:name>rtl</spirit:name>\
                 <spirit:modelName>legacy_top(rtl)</spirit:modelName></spirit:view></spirit:views>\
               <spirit:ports><spirit:port><spirit:name>paddr</spirit:name><spirit:wire>\
                 <spirit:direction>in</spirit:direction>\
                 <spirit:vector><spirit:left>ADDR_WIDTH-1</spirit:left>\
                   <spirit:right>0</spirit:right></spirit:vector>\
                 </spirit:wire></spirit:port></spirit:ports>\
               <spirit:modelParameters><spirit:modelParameter spirit:dataType=\"integer\">\
                 <spirit:name>ADDR_WIDTH</spirit:name>\
                 <spirit:value spirit:id=\"aw\">16</spirit:value>\
                 </spirit:modelParameter></spirit:modelParameters>\
             </spirit:model>\
             <spirit:fileSets><spirit:fileSet><spirit:name>rtl</spirit:name>\
               <spirit:file><spirit:name>rtl/legacy.vhd</spirit:name>\
                 <spirit:fileType>vhdlSource-93</spirit:fileType></spirit:file>\
               <spirit:file><spirit:name>doc/legacy.pdf</spirit:name>\
                 <spirit:fileType>unknown</spirit:fileType></spirit:file>\
             </spirit:fileSet></spirit:fileSets>\
             </spirit:component>";
        let (result, rendered) = run(xml);
        let result = result.expect("imports");
        assert_eq!(result.standard, Standard::Spirit2009);
        assert_eq!(result.manifest.top.as_deref(), Some("legacy_top"));
        // The 2009 `vector` sits straight under `wire`, and the
        // parameter it names is a `modelParameter`.
        assert_eq!(result.manifest.ports.len(), 1);
        assert_eq!(result.manifest.ports[0].width, Width::Fixed(16));
        assert_eq!(
            result.manifest.param("ADDR_WIDTH").expect("ADDR_WIDTH").ty,
            ParamType::Int
        );
        // The port maps are absent, so the prefix came from the name.
        let interface = &result.manifest.interfaces[0];
        assert_eq!(interface.bus, "apb");
        assert_eq!(interface.prefix(), "apb_");
        // One source, and the PDF said so.
        assert_eq!(result.manifest.sources.len(), 1);
        assert_eq!(result.manifest.sources[0].language(), Some(Language::Vhdl));
        assert!(rendered.contains("is not an HDL source"), "{rendered}");
        assert!(reparses(&result.manifest));
    }

    #[test]
    fn an_older_spirit_namespace_is_read_as_2009() {
        let xml = "<spirit:component \
             xmlns:spirit=\"http://www.spiritconsortium.org/XMLSchema/SPIRIT/1.5\">\
             <spirit:vendor>v</spirit:vendor><spirit:library>l</spirit:library>\
             <spirit:name>n</spirit:name><spirit:version>1.0.0</spirit:version>\
             </spirit:component>";
        let result = imported(xml);
        assert_eq!(result.standard, Standard::Spirit2009);
        assert!(
            result
                .report
                .approximated
                .iter()
                .any(|line| line.contains("SPIRIT/1.5")),
            "{}",
            result.report.describe()
        );
    }

    #[test]
    fn memory_maps_become_a_description() {
        let xml = component(
            "regs",
            "1.0.0",
            "<ipxact:description>A register block\n  with two lines</ipxact:description>\
             <ipxact:memoryMaps><ipxact:memoryMap><ipxact:name>REGS</ipxact:name>\
               <ipxact:addressBlock><ipxact:name>CTRL</ipxact:name>\
                 <ipxact:baseAddress>0x40000000</ipxact:baseAddress>\
                 <ipxact:range>0x1000</ipxact:range><ipxact:width>32</ipxact:width>\
               </ipxact:addressBlock></ipxact:memoryMap></ipxact:memoryMaps>\
             <ipxact:model/>",
        );
        let result = imported(&xml);
        assert_eq!(
            result.manifest.description.as_deref(),
            Some(
                "A register block with two lines; memory map REGS: CTRL at 0x40000000 \
                 range 0x1000 width 32"
            )
        );
        assert!(
            result
                .report
                .translated
                .iter()
                .any(|line| line.contains("not turned into logic")),
            "{}",
            result.report.describe()
        );
        assert!(reparses(&result.manifest));
    }

    #[test]
    fn mirrored_and_missing_roles() {
        let interface = |body: &str| {
            component(
                "roles",
                "1.0.0",
                &format!(
                    "<ipxact:busInterfaces><ipxact:busInterface>\
                       <ipxact:name>b</ipxact:name>\
                       <ipxact:busType vendor=\"v\" library=\"l\" name=\"APB\" version=\"1\"/>\
                       {body}</ipxact:busInterface></ipxact:busInterfaces><ipxact:model/>"
                ),
            )
        };
        let result = imported(&interface("<ipxact:mirroredMaster/>"));
        assert_eq!(result.manifest.interfaces[0].role, BusRole::Subordinate);
        assert!(
            result
                .report
                .approximated
                .iter()
                .any(|line| line.contains("mirrored master")),
            "{}",
            result.report.describe()
        );
        // 2022 spells the two sides differently.
        let result = imported(&interface("<ipxact:initiator/>"));
        assert_eq!(result.manifest.interfaces[0].role, BusRole::Manager);
        let result = imported(&interface("<ipxact:target/>"));
        assert_eq!(result.manifest.interfaces[0].role, BusRole::Subordinate);
        // A `system` interface and one with no side at all are dropped.
        for body in ["<ipxact:system/>", ""] {
            let result = imported(&interface(body));
            assert!(result.manifest.interfaces.is_empty());
            assert!(!result.report.dropped.is_empty());
        }
    }

    #[test]
    fn drops_what_it_cannot_describe() {
        let xml = component(
            "odds",
            "1.0.0",
            &format!(
                "<ipxact:vendorExtensions><x:thing xmlns:x=\"urn:x\"/></ipxact:vendorExtensions>\
                 <ipxact:cpus/>\
                 <ipxact:model><ipxact:ports>{}{}\
                   <ipxact:port><ipxact:name>tlm</ipxact:name><ipxact:transactional/>\
                     </ipxact:port>\
                 </ipxact:ports></ipxact:model>\
                 <ipxact:fileSets><ipxact:fileSet><ipxact:name>rtl</ipxact:name>\
                   <ipxact:file><ipxact:name>rtl/odds.v</ipxact:name>\
                     <ipxact:fileType>verilogSource</ipxact:fileType></ipxact:file>\
                   <ipxact:file><ipxact:name>rtl/odds.vh</ipxact:name>\
                     <ipxact:fileType>verilogSource</ipxact:fileType>\
                     <ipxact:isIncludeFile>true</ipxact:isIncludeFile></ipxact:file>\
                 </ipxact:fileSet></ipxact:fileSets>",
                port("ghost", "phantom", ""),
                port("weird", "sideways", ""),
            ),
        );
        let (result, rendered) = run(&xml);
        let result = result.expect("imports");
        let dropped = result.report.dropped.join("\n");
        for expected in [
            "vendorExtensions",
            "cpus",
            "port ghost",
            "port weird",
            "port tlm",
            "rtl/odds.vh",
        ] {
            assert!(
                dropped.contains(expected),
                "`{expected}` missing from:\n{dropped}"
            );
        }
        assert!(rendered.contains("direction `sideways`"), "{rendered}");
        assert_eq!(result.manifest.sources.len(), 1);
        assert!(result.manifest.ports.is_empty());
        assert!(!result.report.is_lossless());
    }

    #[test]
    fn options_override_the_identity() {
        let xml = component("thing", "9.9.9", "<ipxact:model/>");
        let mut map = SourceMap::new();
        let file = map.add("component.xml", &xml).unwrap();
        let mut diags = Diagnostics::new();
        let options = ImportOptions::new(file)
            .with_name("renamed")
            .with_version(Version::new(1, 0, 0))
            .with_license("MIT");
        let result = import(&xml, &options, &mut diags).expect("imports");
        assert_eq!(result.manifest.name, "renamed");
        assert_eq!(result.manifest.version, Version::new(1, 0, 0));
        assert_eq!(result.manifest.license.as_deref(), Some("MIT"));
        assert_eq!(result.vlnv.version, "9.9.9");
        assert!(!diags.has_errors());
    }

    #[test]
    fn refuses_what_is_not_an_importable_component() {
        // A design, not a component.
        let xml = "<ipxact:design \
            xmlns:ipxact=\"http://www.accellera.org/XMLSchema/IPXACT/1685-2014\"/>";
        let (result, rendered) = run(xml);
        assert!(result.is_none());
        assert!(rendered.contains("not a `component`"), "{rendered}");

        // A namespace nobody knows.
        let (result, rendered) = run("<component xmlns=\"urn:made:up\"/>");
        assert!(result.is_none());
        assert!(
            rendered.contains("is not an IP-XACT namespace"),
            "{rendered}"
        );

        // No namespace at all.
        let (result, rendered) = run("<component/>");
        assert!(result.is_none());
        assert!(rendered.contains("in no namespace"), "{rendered}");

        // A component with half a VLNV.
        let xml = "<ipxact:component \
            xmlns:ipxact=\"http://www.accellera.org/XMLSchema/IPXACT/1685-2014\">\
            <ipxact:name>x</ipxact:name></ipxact:component>";
        let (result, rendered) = run(xml);
        assert!(result.is_none());
        assert!(rendered.contains("no complete VLNV"), "{rendered}");

        // And XML that is not well formed at all: one diagnostic, from
        // the reader.
        let (result, rendered) = run("<ipxact:component>");
        assert!(result.is_none());
        assert!(rendered.contains("not declared"), "{rendered}");
    }

    #[test]
    fn a_component_with_no_view_falls_back_to_the_vlnv_name() {
        let result = imported(&component("fallback", "1.0.0", "<ipxact:model/>"));
        assert_eq!(result.manifest.top.as_deref(), Some("fallback"));
        assert!(
            result
                .report
                .approximated
                .iter()
                .any(|line| line.starts_with("top:")),
            "{}",
            result.report.describe()
        );
        // The 2014 module name wins when there is one.
        let result = imported(&component(
            "fallback",
            "1.0.0",
            "<ipxact:model><ipxact:instantiations><ipxact:componentInstantiation>\
               <ipxact:name>rtl</ipxact:name><ipxact:moduleName>fallback_rtl</ipxact:moduleName>\
             </ipxact:componentInstantiation></ipxact:instantiations></ipxact:model>",
        ));
        assert_eq!(result.manifest.top.as_deref(), Some("fallback_rtl"));
    }

    #[test]
    fn the_report_reads_in_three_parts() {
        let mut report = ImportReport::default();
        assert!(report.is_lossless());
        assert_eq!(report.describe(), "");
        report.note_translated("a");
        report.note_approximated("b");
        report.note_dropped("c");
        assert!(!report.is_lossless());
        assert_eq!(
            report.describe(),
            "translated\n  a\napproximated\n  b\ndropped\n  c\n"
        );
    }
}
