//! The IP and project manifest formats.
//!
//! Two files share one grammar and one parser:
//!
//! - **`reticle.ip`**, an [`IpManifest`], sits in an IP package's
//!   directory and describes what the package is: its name, its
//!   [`Version`], its licence, its sources, its top entity, its
//!   parameters, the bus interfaces it exposes, the devices it supports
//!   and the IP it depends on.
//! - **`reticle.proj`**, a [`Project`], sits in the user's design and
//!   describes what to build: a name, a top, a target device, the
//!   sources, constraints and testbenches, and the dependencies with
//!   where to get each one from.
//!
//! `ROADMAP.md` calls the project file `reticle.toml`. It is spelled
//! `reticle.proj` here because it is *not* TOML: the crate has no TOML
//! parser and will not grow one (see `CONTRIBUTING.md`), and the two
//! files must read the same way. Calling a line-oriented file `.toml`
//! would be a promise the format does not keep.
//!
//! # The grammar
//!
//! One declaration per line, keyword first, words separated by
//! whitespace, `#` or `//` starting a comment, double quotes around a
//! word containing spaces. Blank lines are free. This is the shape of the
//! `.rcf` constraints and `.dev` device files, and of the IR's own
//! `.rtl`: line-oriented text diffs, reviews and merges better than a
//! nested document, and every line carries a [`Span`] so a diagnostic can
//! point at exactly the word that is wrong.
//!
//! ```text
//! # reticle.ip
//! name        uart_lite
//! version     1.2.0
//! license     MIT
//! description "AXI4-Lite UART with a 16-byte FIFO"
//!
//! top         uart_lite
//! target      ice40
//!
//! source      rtl/uart_lite.v
//! source      rtl/uart_regs.vhd language vhdl
//! source      rtl/uart_phy.vp encrypted
//! model       sim/uart_lite_model.v
//!
//! param       DATA_WIDTH int 32 8..64
//! param       PARITY string none
//!
//! port        clk in
//! port        rst_n in
//! interface   s_axi axi4lite subordinate prefix s_axi_
//!
//! constraints board/ice40.rcf
//! testbench   tb/uart_tb.v
//!
//! depends     fifo_sync ^1.0.0
//! ```
//!
//! ```text
//! # reticle.proj
//! name        blinky
//! top         top
//! device      ice40-hx1k-tq144
//!
//! source      rtl/top.v
//! constraints board/ice40.rcf
//! testbench   tb/top_tb.v
//!
//! depends     uart_lite ^1.2.0 path ../ip/uart_lite
//! depends     fifo_sync >=1.0.0 git https://example.invalid/fifo.git rev v1.0.4
//! depends     cdc_sync  *       registry
//! ```
//!
//! # Round-tripping
//!
//! [`IpManifest::to_text`] and [`Project::to_text`] render the canonical
//! form: the declarations grouped by keyword in a fixed order, one blank
//! line between groups, every word quoted only when it has to be.
//! Parsing that output and rendering it again is the identity, which the
//! tests check both ways; comments and the original line order are not
//! preserved, since they belong to the file the user edits, not to the
//! data.
//!
//! # Diagnostics
//!
//! Everything reported here carries a `P00nn` code and a span:
//!
//! | Code | Meaning |
//! |------|---------|
//! | [`UNKNOWN_KEY`] | a keyword the manifest kind does not have, with a "did you mean" |
//! | [`SYNTAX`] | a known keyword with the wrong number of words |
//! | [`BAD_VERSION`] | a version that is not `major.minor.patch[-pre]` |
//! | [`BAD_REQUIREMENT`] | a dependency requirement outside the grammar |
//! | [`DUPLICATE`] | a second value for a keyword that takes one |
//! | [`MISSING`] | a required keyword that is absent |
//! | [`BAD_WORD`] | an unknown language, role, parameter type or source kind |
//! | [`BAD_PARAM`] | a parameter default outside its declared range |

use std::fmt;

use super::bus::{BusRole, Width};
use super::text::{Line, Token, closest, quote, tokenize};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::PortDir;
use crate::source::{SourceId, Span};

/// Diagnostic code for a keyword the manifest kind does not define.
pub const UNKNOWN_KEY: &str = "P0001";
/// Diagnostic code for a known keyword used with the wrong words.
pub const SYNTAX: &str = "P0002";
/// Diagnostic code for a malformed version.
pub const BAD_VERSION: &str = "P0003";
/// Diagnostic code for a malformed dependency requirement.
pub const BAD_REQUIREMENT: &str = "P0004";
/// Diagnostic code for a second value for a single-valued keyword.
pub const DUPLICATE: &str = "P0005";
/// Diagnostic code for a required keyword that is missing.
pub const MISSING: &str = "P0006";
/// Diagnostic code for an unknown enumerated word.
pub const BAD_WORD: &str = "P0007";
/// Diagnostic code for a parameter whose default does not fit its range.
pub const BAD_PARAM: &str = "P0008";

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

/// A semantic version: `major.minor.patch` with an optional pre-release.
///
/// Ordering follows semver: the three numbers compare numerically, and a
/// pre-release sorts *before* the release it leads to, so `1.0.0-rc1 <
/// 1.0.0`. Build metadata is not part of the format; a version that needs
/// it is one the IP should have bumped.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    /// The major number: bumped for an incompatible change.
    pub major: u64,
    /// The minor number: bumped for a compatible addition.
    pub minor: u64,
    /// The patch number: bumped for a fix.
    pub patch: u64,
    /// The pre-release tag, without its leading `-`.
    pub pre: Option<String>,
}

impl Version {
    /// A release version with no pre-release tag.
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Version {
            major,
            minor,
            patch,
            pre: None,
        }
    }

    /// Parses `major.minor.patch` or `major.minor.patch-pre`.
    ///
    /// ```
    /// use reticle::ip::Version;
    /// assert_eq!(Version::parse("1.2.3"), Some(Version::new(1, 2, 3)));
    /// assert!(Version::parse("1.2").is_none());
    /// assert!(Version::parse("1.2.3-rc1").unwrap() < Version::new(1, 2, 3));
    /// ```
    pub fn parse(text: &str) -> Option<Version> {
        let (numbers, pre) = match text.split_once('-') {
            Some((n, p)) if !p.is_empty() => (n, Some(p.to_owned())),
            Some(_) => return None,
            None => (text, None),
        };
        let mut parts = numbers.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// True when the version has a pre-release tag.
    pub fn is_prerelease(&self) -> bool {
        self.pre.is_some()
    }

    /// The three numbers, ignoring any pre-release tag.
    fn numbers(&self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Version) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        self.numbers()
            .cmp(&other.numbers())
            .then_with(|| match (&self.pre, &other.pre) {
                // A release outranks any pre-release of the same numbers.
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

/// What versions of a dependency a manifest will accept.
///
/// The grammar is deliberately four forms and no more, because a
/// requirement nobody can read is a requirement nobody can audit:
///
/// | Written | Variant | Accepts |
/// |---------|---------|---------|
/// | `1.2.3` | [`VersionReq::Exact`] | exactly `1.2.3` |
/// | `^1.2.3` | [`VersionReq::Caret`] | `>=1.2.3` up to the next breaking change |
/// | `>=1.2.3` | [`VersionReq::AtLeast`] | `1.2.3` and anything above it |
/// | `*` | [`VersionReq::Any`] | any release |
///
/// The caret rule is Cargo's: the leftmost non-zero number may not
/// change, so `^1.2.3` accepts `1.9.0` but not `2.0.0`, `^0.2.3` accepts
/// `0.2.9` but not `0.3.0`, and `^0.0.3` accepts only `0.0.3`.
///
/// A pre-release only ever satisfies a requirement that names the same
/// three numbers, so `^1.0.0` does not quietly pick up `1.1.0-rc1`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VersionReq {
    /// `*`: any release.
    Any,
    /// `1.2.3`: that version and no other.
    Exact(Version),
    /// `^1.2.3`: compatible with that version.
    Caret(Version),
    /// `>=1.2.3`: that version or newer.
    AtLeast(Version),
}

impl VersionReq {
    /// Parses one of the four forms.
    ///
    /// ```
    /// use reticle::ip::{Version, VersionReq};
    /// let req = VersionReq::parse("^0.2.3").unwrap();
    /// assert!(req.matches(&Version::new(0, 2, 9)));
    /// assert!(!req.matches(&Version::new(0, 3, 0)));
    /// ```
    pub fn parse(text: &str) -> Option<VersionReq> {
        if text == "*" {
            return Some(VersionReq::Any);
        }
        if let Some(rest) = text.strip_prefix('^') {
            return Some(VersionReq::Caret(Version::parse(rest)?));
        }
        if let Some(rest) = text.strip_prefix(">=") {
            return Some(VersionReq::AtLeast(Version::parse(rest)?));
        }
        Some(VersionReq::Exact(Version::parse(text)?))
    }

    /// True when `version` satisfies the requirement.
    pub fn matches(&self, version: &Version) -> bool {
        let base = match self {
            VersionReq::Any => {
                return !version.is_prerelease();
            }
            VersionReq::Exact(v) => return version == v,
            VersionReq::Caret(v) | VersionReq::AtLeast(v) => v,
        };
        // A pre-release is only ever picked when it was asked for by
        // name, which `Exact` above has already handled.
        if version.is_prerelease() {
            return false;
        }
        match self {
            VersionReq::AtLeast(_) => version >= base,
            VersionReq::Caret(_) => {
                if version < base {
                    return false;
                }
                if base.major > 0 {
                    version.major == base.major
                } else if base.minor > 0 {
                    version.major == 0 && version.minor == base.minor
                } else {
                    version.numbers() == base.numbers()
                }
            }
            VersionReq::Any | VersionReq::Exact(_) => unreachable!("handled above"),
        }
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionReq::Any => f.write_str("*"),
            VersionReq::Exact(v) => write!(f, "{v}"),
            VersionReq::Caret(v) => write!(f, "^{v}"),
            VersionReq::AtLeast(v) => write!(f, ">={v}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// The language a source file is written in.
///
/// A `source` line without a `language` word takes the language from the
/// file extension; the word exists for the file whose extension lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    /// Verilog-2005 (`.v`, `.vh`).
    Verilog,
    /// SystemVerilog (`.sv`, `.svh`).
    SystemVerilog,
    /// VHDL (`.vhd`, `.vhdl`).
    Vhdl,
    /// A design already in the IR's `.rtl` text format.
    Rtl,
}

impl Language {
    /// Every language, in the order the keyword table lists them.
    pub const ALL: [Language; 4] = [
        Language::Verilog,
        Language::SystemVerilog,
        Language::Vhdl,
        Language::Rtl,
    ];

    /// The word used in a manifest.
    pub fn keyword(self) -> &'static str {
        match self {
            Language::Verilog => "verilog",
            Language::SystemVerilog => "systemverilog",
            Language::Vhdl => "vhdl",
            Language::Rtl => "rtl",
        }
    }

    /// The language named by `word`.
    pub fn from_keyword(word: &str) -> Option<Language> {
        Language::ALL
            .into_iter()
            .find(|l| l.keyword() == word.to_ascii_lowercase())
    }

    /// The language a path's extension implies.
    ///
    /// ```
    /// use reticle::ip::Language;
    /// assert_eq!(Language::from_path("rtl/uart.sv"), Some(Language::SystemVerilog));
    /// assert_eq!(Language::from_path("rtl/uart.vhd"), Some(Language::Vhdl));
    /// assert_eq!(Language::from_path("notes.txt"), None);
    /// ```
    pub fn from_path(path: &str) -> Option<Language> {
        let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "v" | "vh" => Some(Language::Verilog),
            "sv" | "svh" => Some(Language::SystemVerilog),
            "vhd" | "vhdl" => Some(Language::Vhdl),
            "rtl" => Some(Language::Rtl),
            _ => None,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.keyword())
    }
}

/// One `source` line: a path, optionally with a language and an
/// `encrypted` marker.
///
/// An encrypted source is one the vendor ships as ciphertext. Reticle
/// never tries to read it; the package becomes a black box (see
/// [`super::blackbox`]) and the file is passed on to the vendor tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceEntry {
    /// The path, relative to the manifest's directory.
    pub path: String,
    /// The language override, if the line gave one.
    pub language: Option<Language>,
    /// True when the file is encrypted and must be treated as a black box.
    pub encrypted: bool,
    /// Where the line was written.
    pub span: Span,
}

impl SourceEntry {
    /// The language: the override, else what the extension implies.
    pub fn language(&self) -> Option<Language> {
        self.language.or_else(|| Language::from_path(&self.path))
    }
}

// ---------------------------------------------------------------------------
// Parameters and ports
// ---------------------------------------------------------------------------

/// The type of a declared parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamType {
    /// An integer, the usual width or depth knob.
    Int,
    /// A boolean written `true` or `false`.
    Bool,
    /// A free string, for a mode or a file name.
    Str,
    /// A sized bit vector written as a Verilog literal (`8'hff`).
    Bits,
}

impl ParamType {
    /// Every type, in the order the keyword table lists them.
    pub const ALL: [ParamType; 4] = [
        ParamType::Int,
        ParamType::Bool,
        ParamType::Str,
        ParamType::Bits,
    ];

    /// The word used in a manifest.
    pub fn keyword(self) -> &'static str {
        match self {
            ParamType::Int => "int",
            ParamType::Bool => "bool",
            ParamType::Str => "string",
            ParamType::Bits => "bits",
        }
    }

    /// The type named by `word`.
    pub fn from_keyword(word: &str) -> Option<ParamType> {
        ParamType::ALL.into_iter().find(|t| t.keyword() == word)
    }
}

impl fmt::Display for ParamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.keyword())
    }
}

/// One `param` line: `param <name> <type> [default] [lo..hi]`.
///
/// The default is kept as the text the manifest wrote, so it round-trips
/// exactly and so a `bits` default keeps its Verilog spelling; the range
/// is only meaningful for [`ParamType::Int`] and is checked against the
/// default at parse time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamDecl {
    /// The parameter name, as the HDL spells it.
    pub name: String,
    /// Its type.
    pub ty: ParamType,
    /// The default value as written, if the line gave one.
    pub default: Option<String>,
    /// The inclusive range an integer parameter must stay inside.
    pub range: Option<(i64, i64)>,
    /// Where the line was written.
    pub span: Span,
}

impl ParamDecl {
    /// The default as an integer, for an `int` parameter that has one.
    pub fn default_int(&self) -> Option<i64> {
        self.default.as_ref()?.parse().ok()
    }
}

/// One `port` line: `port <name> <in|out|inout> [width]`.
///
/// A port that belongs to a bus is declared once through an
/// [`InterfaceDecl`] instead; `port` is for the clock, the reset, an
/// interrupt line and anything else that stands alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortDecl {
    /// The port name, as the HDL spells it.
    pub name: String,
    /// The direction, seen from inside the IP.
    pub dir: PortDir,
    /// The width: a number, or a parameter name, possibly divided.
    pub width: Width,
    /// Where the line was written.
    pub span: Span,
}

/// One `interface` line: `interface <name> <bus> <role> [prefix <p>]`.
///
/// This is the declaration `ROADMAP.md` calls "bus interfaces exposed":
/// it names a bus ([`super::bus`] has the built-in definitions) and the
/// role the IP plays on it, and it stands for every port of that bus at
/// once. [`super::bus::match_ports`] checks the IP's real ports against
/// it, and [`super::blackbox`] expands it into the ports of a stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceDecl {
    /// The interface's name inside the IP (`s_axi`, `m_axi`).
    pub name: String,
    /// The bus definition's name (`axi4lite`, `wishbone`).
    pub bus: String,
    /// Which side of the bus the IP is.
    pub role: BusRole,
    /// The prefix the IP's port names carry; the interface name plus `_`
    /// when the line does not say.
    pub prefix: Option<String>,
    /// Where the line was written.
    pub span: Span,
}

impl InterfaceDecl {
    /// The port-name prefix: the declared one, else `<name>_`.
    pub fn prefix(&self) -> String {
        self.prefix
            .clone()
            .unwrap_or_else(|| format!("{}_", self.name))
    }
}

// ---------------------------------------------------------------------------
// Dependencies
// ---------------------------------------------------------------------------

/// Where a dependency comes from.
///
/// An IP manifest's `depends` line never carries one: an IP says *what*
/// it needs, and the project it is built into says *where* that comes
/// from, exactly as a Cargo dependency's source is the workspace's
/// business and not the library's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepSource {
    /// `registry`: the index, once there is one.
    Registry,
    /// `path <dir>`: a directory, relative to the manifest.
    Path(String),
    /// `git <url> [rev <r>]`: a checkout of a repository.
    Git {
        /// The repository URL.
        url: String,
        /// The revision (tag, branch or commit) to check out.
        rev: Option<String>,
    },
}

impl DepSource {
    /// The words this source is written as, for [`to_text`](Project::to_text).
    fn words(&self) -> Vec<String> {
        match self {
            DepSource::Registry => vec!["registry".to_owned()],
            DepSource::Path(dir) => vec!["path".to_owned(), dir.clone()],
            DepSource::Git { url, rev } => {
                let mut out = vec!["git".to_owned(), url.clone()];
                if let Some(rev) = rev {
                    out.push("rev".to_owned());
                    out.push(rev.clone());
                }
                out
            }
        }
    }

    /// A short phrase for a diagnostic.
    pub fn describe(&self) -> String {
        match self {
            DepSource::Registry => "the registry".to_owned(),
            DepSource::Path(dir) => format!("the directory `{dir}`"),
            DepSource::Git { url, rev: None } => format!("the repository `{url}`"),
            DepSource::Git {
                url,
                rev: Some(rev),
            } => format!("the repository `{url}` at `{rev}`"),
        }
    }
}

impl fmt::Display for DepSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.words().join(" "))
    }
}

/// One `depends` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    /// The package name.
    pub name: String,
    /// What versions of it are acceptable.
    pub req: VersionReq,
    /// Where to get it, for a project manifest.
    pub source: Option<DepSource>,
    /// Where the line was written.
    pub span: Span,
}

impl Dependency {
    /// A dependency with no source, as an IP manifest writes it.
    pub fn new(name: impl Into<String>, req: VersionReq, span: Span) -> Self {
        Dependency {
            name: name.into(),
            req,
            source: None,
            span,
        }
    }

    /// The words after the keyword, for `to_text`.
    fn words(&self) -> Vec<String> {
        let mut out = vec![self.name.clone(), self.req.to_string()];
        if let Some(source) = &self.source {
            out.extend(source.words());
        }
        out
    }
}

// ---------------------------------------------------------------------------
// The manifests
// ---------------------------------------------------------------------------

/// An IP package's `reticle.ip`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpManifest {
    /// The package name, unique in a dependency graph.
    pub name: String,
    /// The package version.
    pub version: Version,
    /// The licence, as an SPDX identifier when there is one.
    pub license: Option<String>,
    /// One line about what the package is.
    pub description: Option<String>,
    /// The top entity or module of the package.
    pub top: Option<String>,
    /// The HDL sources, in the order they must be analysed.
    pub sources: Vec<SourceEntry>,
    /// A behavioural model used for simulation when the real sources are
    /// encrypted or absent.
    pub model: Option<SourceEntry>,
    /// The parameters the package takes.
    pub params: Vec<ParamDecl>,
    /// The ports that do not belong to a bus.
    pub ports: Vec<PortDecl>,
    /// The bus interfaces the package exposes.
    pub interfaces: Vec<InterfaceDecl>,
    /// Devices or families the package supports; empty means "any".
    pub targets: Vec<String>,
    /// Constraints files the package ships.
    pub constraints: Vec<String>,
    /// Testbenches the package ships.
    pub testbenches: Vec<String>,
    /// The IP this package needs.
    pub depends: Vec<Dependency>,
    /// The span of the whole manifest file.
    pub span: Span,
}

/// The keywords an IP manifest accepts, for the "did you mean" note.
const IP_KEYS: [&str; 14] = [
    "name",
    "version",
    "license",
    "description",
    "top",
    "source",
    "model",
    "param",
    "port",
    "interface",
    "target",
    "constraints",
    "testbench",
    "depends",
];

/// The keywords a project manifest accepts.
const PROJECT_KEYS: [&str; 7] = [
    "name",
    "top",
    "device",
    "source",
    "constraints",
    "testbench",
    "depends",
];

impl IpManifest {
    /// An empty manifest, for building one from Rust.
    pub fn new(name: impl Into<String>, version: Version, span: Span) -> Self {
        IpManifest {
            name: name.into(),
            version,
            license: None,
            description: None,
            top: None,
            sources: Vec::new(),
            model: None,
            params: Vec::new(),
            ports: Vec::new(),
            interfaces: Vec::new(),
            targets: Vec::new(),
            constraints: Vec::new(),
            testbenches: Vec::new(),
            depends: Vec::new(),
            span,
        }
    }

    /// Parses a `reticle.ip`.
    ///
    /// Returns `None` only when a required keyword (`name`, `version`) is
    /// missing, since without those there is no package to speak of.
    /// Every other problem is reported into `diags` and the line skipped,
    /// so one bad line does not lose the file.
    ///
    /// ```
    /// use reticle::diag::Diagnostics;
    /// use reticle::ip::IpManifest;
    /// use reticle::source::SourceMap;
    ///
    /// let text = "name fifo\nversion 1.0.0\n\nsource rtl/fifo.v\n";
    /// let mut map = SourceMap::new();
    /// let file = map.add("reticle.ip", text).unwrap();
    /// let mut diags = Diagnostics::new();
    /// let ip = IpManifest::parse(text, file, &mut diags).unwrap();
    /// assert!(!diags.has_errors());
    /// assert_eq!(ip.name, "fifo");
    /// assert_eq!(ip.sources.len(), 1);
    /// assert_eq!(ip.to_text(), text);
    /// ```
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> Option<IpManifest> {
        let span = whole(text, file);
        let mut p = ManifestParser::new(text, file, diags, Kind::Ip);
        let mut out = IpManifest::new(String::new(), Version::new(0, 0, 0), span);
        let mut name = None;
        let mut version = None;
        for line in &p.lines.clone() {
            p.line = line.clone();
            match line.keyword() {
                "name" => {
                    p.once(&mut name, "name");
                }
                "version" => {
                    if let Some(text) = p.one_word("version") {
                        match Version::parse(text.as_str()) {
                            Some(v) => {
                                if version.is_some() {
                                    p.duplicate("version");
                                } else {
                                    version = Some(v);
                                }
                            }
                            None => p.bad_version(&text),
                        }
                    }
                }
                "license" => {
                    p.once(&mut out.license, "license");
                }
                "description" => {
                    p.once(&mut out.description, "description");
                }
                "top" => {
                    p.once(&mut out.top, "top");
                }
                "source" => {
                    if let Some(entry) = p.source_entry() {
                        out.sources.push(entry);
                    }
                }
                "model" => {
                    if let Some(entry) = p.source_entry() {
                        if out.model.is_some() {
                            p.duplicate("model");
                        } else {
                            out.model = Some(entry);
                        }
                    }
                }
                "param" => {
                    if let Some(decl) = p.param_decl() {
                        out.params.push(decl);
                    }
                }
                "port" => {
                    if let Some(decl) = p.port_decl() {
                        out.ports.push(decl);
                    }
                }
                "interface" => {
                    if let Some(decl) = p.interface_decl() {
                        out.interfaces.push(decl);
                    }
                }
                "target" => p.push_word("target", &mut out.targets),
                "constraints" => p.push_word("constraints", &mut out.constraints),
                "testbench" => p.push_word("testbench", &mut out.testbenches),
                "depends" => {
                    if let Some(dep) = p.dependency(false) {
                        out.depends.push(dep);
                    }
                }
                other => p.unknown_key(other, &IP_KEYS),
            }
        }
        out.name = p.required(name, "name", span)?;
        out.version = match version {
            Some(v) => v,
            None => {
                p.missing("version", span);
                return None;
            }
        };
        Some(out)
    }

    /// Renders the manifest in the canonical form.
    pub fn to_text(&self) -> String {
        let mut w = Writer::new();
        w.word("name", &self.name);
        w.word("version", &self.version.to_string());
        w.opt("license", self.license.as_deref());
        w.opt("description", self.description.as_deref());
        w.group();
        w.opt("top", self.top.as_deref());
        for target in &self.targets {
            w.word("target", target);
        }
        w.group();
        for source in &self.sources {
            w.line("source", &source_words(source));
        }
        if let Some(model) = &self.model {
            w.line("model", &source_words(model));
        }
        w.group();
        for param in &self.params {
            w.line("param", &param_words(param));
        }
        for port in &self.ports {
            w.line("port", &port_words(port));
        }
        for iface in &self.interfaces {
            w.line("interface", &interface_words(iface));
        }
        w.group();
        for path in &self.constraints {
            w.word("constraints", path);
        }
        for path in &self.testbenches {
            w.word("testbench", path);
        }
        w.group();
        for dep in &self.depends {
            w.line("depends", &dep.words());
        }
        w.finish()
    }

    /// The parameter with the given name.
    pub fn param(&self, name: &str) -> Option<&ParamDecl> {
        self.params.iter().find(|p| p.name == name)
    }

    /// The bus interface with the given name.
    pub fn interface(&self, name: &str) -> Option<&InterfaceDecl> {
        self.interfaces.iter().find(|i| i.name == name)
    }

    /// The module name a design instantiates this package as: the
    /// declared `top`, else the package name.
    pub fn top_name(&self) -> &str {
        self.top.as_deref().unwrap_or(&self.name)
    }

    /// True when every source is encrypted, or there are no sources at
    /// all: the package can only be a black box.
    ///
    /// A package with a behavioural `model` is still a black box for
    /// synthesis; [`super::blackbox`] uses the model for simulation and
    /// says so in its report.
    pub fn is_blackbox(&self) -> bool {
        self.sources.is_empty() || self.sources.iter().all(|s| s.encrypted)
    }
}

/// The user's `reticle.proj`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    /// The project name.
    pub name: String,
    /// The top module of the design.
    pub top: Option<String>,
    /// The target device, as [`crate::fpga::target`] names them.
    pub device: Option<String>,
    /// The project's own sources.
    pub sources: Vec<SourceEntry>,
    /// Constraints files.
    pub constraints: Vec<String>,
    /// Testbenches.
    pub testbenches: Vec<String>,
    /// The IP the project pulls in, each with where it comes from.
    pub depends: Vec<Dependency>,
    /// The span of the `top` line, for a diagnostic about the top.
    pub top_span: Option<Span>,
    /// The span of the whole manifest file.
    pub span: Span,
}

impl Project {
    /// An empty project, for building one from Rust.
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Project {
            name: name.into(),
            top: None,
            device: None,
            sources: Vec::new(),
            constraints: Vec::new(),
            testbenches: Vec::new(),
            depends: Vec::new(),
            top_span: None,
            span,
        }
    }

    /// Parses a `reticle.proj`.
    ///
    /// Returns `None` only when `name` is missing.
    ///
    /// ```
    /// use reticle::diag::Diagnostics;
    /// use reticle::ip::Project;
    /// use reticle::source::SourceMap;
    ///
    /// let text = "name blinky\ntop top\n\nsource rtl/top.v\n\ndepends fifo ^1.0.0 path ../fifo\n";
    /// let mut map = SourceMap::new();
    /// let file = map.add("reticle.proj", text).unwrap();
    /// let mut diags = Diagnostics::new();
    /// let project = Project::parse(text, file, &mut diags).unwrap();
    /// assert!(!diags.has_errors());
    /// assert_eq!(project.depends[0].name, "fifo");
    /// assert_eq!(project.to_text(), text);
    /// ```
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> Option<Project> {
        let span = whole(text, file);
        let mut p = ManifestParser::new(text, file, diags, Kind::Project);
        let mut out = Project::new(String::new(), span);
        let mut name = None;
        for line in &p.lines.clone() {
            p.line = line.clone();
            match line.keyword() {
                "name" => {
                    p.once(&mut name, "name");
                }
                "top" => out.top_span = p.once(&mut out.top, "top"),
                "device" => {
                    p.once(&mut out.device, "device");
                }
                "source" => {
                    if let Some(entry) = p.source_entry() {
                        out.sources.push(entry);
                    }
                }
                "constraints" => p.push_word("constraints", &mut out.constraints),
                "testbench" => p.push_word("testbench", &mut out.testbenches),
                "depends" => {
                    if let Some(dep) = p.dependency(true) {
                        out.depends.push(dep);
                    }
                }
                other => p.unknown_key(other, &PROJECT_KEYS),
            }
        }
        out.name = p.required(name, "name", span)?;
        Some(out)
    }

    /// Renders the project in the canonical form.
    pub fn to_text(&self) -> String {
        let mut w = Writer::new();
        w.word("name", &self.name);
        w.opt("top", self.top.as_deref());
        w.opt("device", self.device.as_deref());
        w.group();
        for source in &self.sources {
            w.line("source", &source_words(source));
        }
        for path in &self.constraints {
            w.word("constraints", path);
        }
        for path in &self.testbenches {
            w.word("testbench", path);
        }
        w.group();
        for dep in &self.depends {
            w.line("depends", &dep.words());
        }
        w.finish()
    }

    /// The dependency with the given name.
    pub fn dependency(&self, name: &str) -> Option<&Dependency> {
        self.depends.iter().find(|d| d.name == name)
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn source_words(entry: &SourceEntry) -> Vec<String> {
    let mut out = vec![entry.path.clone()];
    if let Some(language) = entry.language {
        out.push("language".to_owned());
        out.push(language.keyword().to_owned());
    }
    if entry.encrypted {
        out.push("encrypted".to_owned());
    }
    out
}

fn param_words(param: &ParamDecl) -> Vec<String> {
    let mut out = vec![param.name.clone(), param.ty.keyword().to_owned()];
    if let Some(default) = &param.default {
        out.push(default.clone());
    }
    if let Some((lo, hi)) = param.range {
        out.push(format!("{lo}..{hi}"));
    }
    out
}

fn port_words(port: &PortDecl) -> Vec<String> {
    let mut out = vec![port.name.clone(), port.dir.keyword().to_owned()];
    if port.width != Width::Fixed(1) {
        out.push(port.width.to_string());
    }
    out
}

fn interface_words(iface: &InterfaceDecl) -> Vec<String> {
    let mut out = vec![
        iface.name.clone(),
        iface.bus.clone(),
        iface.role.keyword().to_owned(),
    ];
    if let Some(prefix) = &iface.prefix {
        out.push("prefix".to_owned());
        out.push(prefix.clone());
    }
    out
}

/// Accumulates the canonical text: lines in groups, one blank line
/// between two non-empty groups and none at the top or bottom.
struct Writer {
    out: String,
    /// True when something has been written since the last [`group`].
    ///
    /// [`group`]: Writer::group
    pending_blank: bool,
    wrote_any: bool,
}

impl Writer {
    fn new() -> Self {
        Writer {
            out: String::new(),
            pending_blank: false,
            wrote_any: false,
        }
    }

    fn line(&mut self, keyword: &str, words: &[String]) {
        if self.pending_blank && self.wrote_any {
            self.out.push('\n');
        }
        self.pending_blank = false;
        self.wrote_any = true;
        self.out.push_str(keyword);
        for word in words {
            self.out.push(' ');
            self.out.push_str(&quote(word));
        }
        self.out.push('\n');
    }

    fn word(&mut self, keyword: &str, word: &str) {
        self.line(keyword, std::slice::from_ref(&word.to_owned()));
    }

    fn opt(&mut self, keyword: &str, word: Option<&str>) {
        if let Some(word) = word {
            self.word(keyword, word);
        }
    }

    /// Ends a group: the next line written starts a new paragraph.
    fn group(&mut self) {
        self.pending_blank = true;
    }

    fn finish(self) -> String {
        self.out
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Which manifest is being parsed, for the wording of diagnostics.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ip,
    Project,
}

impl Kind {
    fn describe(self) -> &'static str {
        match self {
            Kind::Ip => "an IP manifest",
            Kind::Project => "a project manifest",
        }
    }
}

/// The span covering a whole file, for diagnostics about what is absent.
fn whole(text: &str, file: SourceId) -> Span {
    let end = u32::try_from(text.len()).expect("manifest length fits u32");
    Span::new(file, 0, end)
}

struct ManifestParser<'a> {
    lines: Vec<Line>,
    line: Line,
    diags: &'a mut Diagnostics,
    kind: Kind,
}

impl<'a> ManifestParser<'a> {
    fn new(text: &str, file: SourceId, diags: &'a mut Diagnostics, kind: Kind) -> Self {
        let lines = tokenize(text, file);
        // A parser always has a current line while it is iterating; the
        // placeholder covers the moment before the first one.
        let line = lines.first().cloned().unwrap_or(Line {
            tokens: vec![Token {
                text: String::new(),
                span: whole(text, file),
            }],
            span: whole(text, file),
        });
        ManifestParser {
            lines,
            line,
            diags,
            kind,
        }
    }

    fn error(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        self.diags
            .push(Diagnostic::error(message).with_code(code).with_span(span));
    }

    fn unknown_key(&mut self, key: &str, known: &[&str]) {
        let mut d = Diagnostic::error(format!("unknown key `{key}` in {}", self.kind.describe()))
            .with_code(UNKNOWN_KEY)
            .with_span(self.line.keyword_span());
        if let Some(suggestion) = closest(key, known) {
            d = d.with_note(format!("did you mean `{suggestion}`?"));
        } else {
            d = d.with_note(format!("known keys are: {}", known.join(", ")));
        }
        self.diags.push(d);
    }

    fn duplicate(&mut self, key: &str) {
        self.error(
            DUPLICATE,
            self.line.span,
            format!("`{key}` is given more than once"),
        );
    }

    fn missing(&mut self, key: &str, span: Span) {
        self.diags.push(
            Diagnostic::error(format!("{} has no `{key}`", self.kind.describe()))
                .with_code(MISSING)
                .with_span(span),
        );
    }

    fn required(&mut self, value: Option<String>, key: &str, span: Span) -> Option<String> {
        match value {
            Some(v) => Some(v),
            None => {
                self.missing(key, span);
                None
            }
        }
    }

    fn bad_version(&mut self, token: &Token) {
        self.diags.push(
            Diagnostic::error(format!("`{token}` is not a version"))
                .with_code(BAD_VERSION)
                .with_span(token.span)
                .with_note("a version is `major.minor.patch`, optionally `-pre`"),
        );
    }

    fn shape(&mut self, expected: &str) {
        let keyword = self.line.keyword().to_owned();
        self.error(
            SYNTAX,
            self.line.span,
            format!("`{keyword}` takes {expected}"),
        );
    }

    /// The single word a keyword takes, or `None` after reporting.
    fn one_word(&mut self, key: &str) -> Option<Token> {
        match self.line.args() {
            [token] => Some(token.clone()),
            _ => {
                self.shape(&format!("one word: `{key} <value>`"));
                None
            }
        }
    }

    /// Reads a single-valued keyword into `slot`, reporting a second one.
    /// Returns the line's span when it set the slot, for the keywords
    /// whose line a later diagnostic wants to point at.
    fn once(&mut self, slot: &mut Option<String>, key: &str) -> Option<Span> {
        let token = self.one_word(key)?;
        if slot.is_some() {
            self.duplicate(key);
            return None;
        }
        *slot = Some(token.as_str().to_owned());
        Some(self.line.span)
    }

    /// Reads a repeatable single-word keyword into `list`.
    fn push_word(&mut self, key: &str, list: &mut Vec<String>) {
        if let Some(token) = self.one_word(key) {
            list.push(token.as_str().to_owned());
        }
    }

    /// `<path> [language <lang>] [encrypted]`.
    fn source_entry(&mut self) -> Option<SourceEntry> {
        let args = self.line.args().to_vec();
        let Some((path, rest)) = args.split_first() else {
            self.shape("a path: `source <path> [language <lang>] [encrypted]`");
            return None;
        };
        let mut language = None;
        let mut encrypted = false;
        let mut i = 0;
        while i < rest.len() {
            if rest[i].is("encrypted") {
                encrypted = true;
                i += 1;
            } else if rest[i].is("language") {
                let Some(word) = rest.get(i + 1) else {
                    self.shape("a language after `language`");
                    return None;
                };
                match Language::from_keyword(word.as_str()) {
                    Some(l) => language = Some(l),
                    None => {
                        self.bad_word(word, "language", &languages());
                        return None;
                    }
                }
                i += 2;
            } else {
                self.shape("a path: `source <path> [language <lang>] [encrypted]`");
                return None;
            }
        }
        Some(SourceEntry {
            path: path.as_str().to_owned(),
            language,
            encrypted,
            span: self.line.span,
        })
    }

    fn bad_word(&mut self, token: &Token, what: &str, known: &[&str]) {
        let mut d = Diagnostic::error(format!("`{token}` is not a known {what}"))
            .with_code(BAD_WORD)
            .with_span(token.span);
        if let Some(suggestion) = closest(token.as_str(), known) {
            d = d.with_note(format!("did you mean `{suggestion}`?"));
        } else {
            d = d.with_note(format!("known {what}s are: {}", known.join(", ")));
        }
        self.diags.push(d);
    }

    /// `<name> <type> [default] [lo..hi]`.
    fn param_decl(&mut self) -> Option<ParamDecl> {
        let args = self.line.args().to_vec();
        if args.len() < 2 || args.len() > 4 {
            self.shape("`param <name> <type> [default] [lo..hi]`");
            return None;
        }
        let name = args[0].as_str().to_owned();
        let Some(ty) = ParamType::from_keyword(args[1].as_str()) else {
            self.bad_word(&args[1], "parameter type", &param_types());
            return None;
        };
        let mut default = None;
        let mut range = None;
        for token in &args[2..] {
            if let Some((lo, hi)) = token.as_str().split_once("..") {
                let (Ok(lo), Ok(hi)) = (lo.parse::<i64>(), hi.parse::<i64>()) else {
                    self.error(
                        BAD_PARAM,
                        token.span,
                        format!("`{token}` is not a range of integers"),
                    );
                    return None;
                };
                if lo > hi {
                    self.error(
                        BAD_PARAM,
                        token.span,
                        format!("the range `{token}` is empty"),
                    );
                    return None;
                }
                if range.is_some() {
                    self.shape("`param <name> <type> [default] [lo..hi]`");
                    return None;
                }
                range = Some((lo, hi));
            } else {
                if default.is_some() || range.is_some() {
                    self.shape("`param <name> <type> [default] [lo..hi]`");
                    return None;
                }
                default = Some(token.as_str().to_owned());
            }
        }
        if range.is_some() && ty != ParamType::Int {
            self.error(
                BAD_PARAM,
                self.line.span,
                format!("only an `int` parameter takes a range, not `{ty}`"),
            );
            return None;
        }
        let decl = ParamDecl {
            name,
            ty,
            default,
            range,
            span: self.line.span,
        };
        if let (Some((lo, hi)), Some(value)) = (decl.range, decl.default_int())
            && (value < lo || value > hi)
        {
            self.error(
                BAD_PARAM,
                self.line.span,
                format!(
                    "the default {value} of `{}` is outside {lo}..{hi}",
                    decl.name
                ),
            );
            return None;
        }
        Some(decl)
    }

    /// `<name> <in|out|inout> [width]`.
    fn port_decl(&mut self) -> Option<PortDecl> {
        let args = self.line.args().to_vec();
        if args.is_empty() || args.len() > 3 {
            self.shape("`port <name> <in|out|inout> [width]`");
            return None;
        }
        let Some(dir) = args.get(1).and_then(|t| PortDir::from_keyword(t.as_str())) else {
            let token = args.get(1).cloned().unwrap_or_else(|| args[0].clone());
            self.bad_word(&token, "direction", &["in", "out", "inout"]);
            return None;
        };
        let width = match args.get(2) {
            None => Width::Fixed(1),
            Some(token) => match Width::parse(token.as_str()) {
                Some(w) => w,
                None => {
                    self.error(
                        SYNTAX,
                        token.span,
                        format!("`{token}` is not a width: a number, `PARAM` or `PARAM/n`"),
                    );
                    return None;
                }
            },
        };
        Some(PortDecl {
            name: args[0].as_str().to_owned(),
            dir,
            width,
            span: self.line.span,
        })
    }

    /// `<name> <bus> <role> [prefix <p>]`.
    fn interface_decl(&mut self) -> Option<InterfaceDecl> {
        let args = self.line.args().to_vec();
        if args.len() != 3 && args.len() != 5 {
            self.shape("`interface <name> <bus> <role> [prefix <p>]`");
            return None;
        }
        let Some(role) = BusRole::from_keyword(args[2].as_str()) else {
            self.bad_word(&args[2], "bus role", &BusRole::KEYWORDS);
            return None;
        };
        let mut prefix = None;
        if args.len() == 5 {
            if !args[3].is("prefix") {
                self.shape("`interface <name> <bus> <role> [prefix <p>]`");
                return None;
            }
            prefix = Some(args[4].as_str().to_owned());
        }
        Some(InterfaceDecl {
            name: args[0].as_str().to_owned(),
            bus: args[1].as_str().to_owned(),
            role,
            prefix,
            span: self.line.span,
        })
    }

    /// `<name> <requirement> [path <dir> | git <url> [rev <r>] | registry]`.
    fn dependency(&mut self, allow_source: bool) -> Option<Dependency> {
        let args = self.line.args().to_vec();
        if args.len() < 2 {
            self.shape("`depends <name> <requirement>`");
            return None;
        }
        let Some(req) = VersionReq::parse(args[1].as_str()) else {
            self.diags.push(
                Diagnostic::error(format!("`{}` is not a version requirement", args[1]))
                    .with_code(BAD_REQUIREMENT)
                    .with_span(args[1].span)
                    .with_note("write `1.2.3`, `^1.2.3`, `>=1.2.3` or `*`"),
            );
            return None;
        };
        let rest = &args[2..];
        let source = if rest.is_empty() {
            None
        } else if !allow_source {
            self.error(
                SYNTAX,
                rest[0].span,
                "an IP manifest states what it needs, not where it comes from",
            );
            return None;
        } else {
            Some(self.dep_source(rest)?)
        };
        Some(Dependency {
            name: args[0].as_str().to_owned(),
            req,
            source,
            span: self.line.span,
        })
    }

    fn dep_source(&mut self, words: &[Token]) -> Option<DepSource> {
        match words[0].as_str() {
            "registry" if words.len() == 1 => Some(DepSource::Registry),
            "path" if words.len() == 2 => Some(DepSource::Path(words[1].as_str().to_owned())),
            "git" if words.len() == 2 => Some(DepSource::Git {
                url: words[1].as_str().to_owned(),
                rev: None,
            }),
            "git" if words.len() == 4 && words[2].is("rev") => Some(DepSource::Git {
                url: words[1].as_str().to_owned(),
                rev: Some(words[3].as_str().to_owned()),
            }),
            "registry" | "path" | "git" => {
                self.shape(
                    "`path <dir>`, `git <url> [rev <r>]` or `registry` after the requirement",
                );
                None
            }
            _ => {
                self.bad_word(&words[0], "dependency source", &["path", "git", "registry"]);
                None
            }
        }
    }
}

fn languages() -> Vec<&'static str> {
    Language::ALL.iter().map(|l| l.keyword()).collect()
}

fn param_types() -> Vec<&'static str> {
    ParamType::ALL.iter().map(|t| t.keyword()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn ip(text: &str) -> (Option<IpManifest>, String) {
        let mut map = SourceMap::new();
        let file = map.add("reticle.ip", text).unwrap();
        let mut diags = Diagnostics::new();
        let manifest = IpManifest::parse(text, file, &mut diags);
        (manifest, diags.render(&map))
    }

    fn project(text: &str) -> (Option<Project>, String) {
        let mut map = SourceMap::new();
        let file = map.add("reticle.proj", text).unwrap();
        let mut diags = Diagnostics::new();
        let p = Project::parse(text, file, &mut diags);
        (p, diags.render(&map))
    }

    const FULL_IP: &str = "\
name uart_lite
version 1.2.0
license MIT
description \"AXI4-Lite UART with a 16-byte FIFO\"

top uart_lite
target ice40
target ecp5

source rtl/uart_lite.v
source rtl/uart_regs.vhd language vhdl
source rtl/uart_phy.vp language verilog encrypted
model sim/uart_model.v

param DATA_WIDTH int 32 8..64
param PARITY string none
param USE_FIFO bool true
param RESET_VALUE bits 8'h00
port clk in
port rst_n in
port irq out
port gpio inout 8
interface s_axi axi4lite subordinate prefix s_axi_
interface m_str axi4stream manager

constraints board/ice40.rcf
testbench tb/uart_tb.v

depends fifo_sync ^1.0.0
depends cdc_sync >=0.2.0
";

    const FULL_PROJECT: &str = "\
name blinky
top top
device ice40-hx1k-tq144

source rtl/top.v
source rtl/pll.vhd language vhdl
constraints board/ice40.rcf
testbench tb/top_tb.v

depends uart_lite ^1.2.0 path ../ip/uart_lite
depends fifo_sync >=1.0.0 git https://example.invalid/fifo.git rev v1.0.4
depends cdc_sync * registry
";

    #[test]
    fn ip_manifest_round_trips_byte_for_byte() {
        let (manifest, rendered) = ip(FULL_IP);
        let manifest = manifest.expect("parses");
        assert_eq!(rendered, "");
        assert_eq!(manifest.to_text(), FULL_IP);
        // And the parse of the canonical text renders the same again.
        let (again, _) = ip(&manifest.to_text());
        assert_eq!(again.unwrap().to_text(), FULL_IP);
    }

    #[test]
    fn ip_manifest_holds_what_it_declared() {
        let (manifest, _) = ip(FULL_IP);
        let m = manifest.unwrap();
        assert_eq!(m.version, Version::new(1, 2, 0));
        assert_eq!(m.license.as_deref(), Some("MIT"));
        assert_eq!(m.top_name(), "uart_lite");
        assert_eq!(m.sources.len(), 3);
        assert_eq!(m.sources[0].language(), Some(Language::Verilog));
        assert_eq!(m.sources[1].language(), Some(Language::Vhdl));
        assert!(m.sources[2].encrypted);
        assert_eq!(m.model.as_ref().unwrap().path, "sim/uart_model.v");
        assert_eq!(m.param("DATA_WIDTH").unwrap().range, Some((8, 64)));
        assert_eq!(m.param("DATA_WIDTH").unwrap().default_int(), Some(32));
        assert_eq!(m.ports.len(), 4);
        assert_eq!(m.ports[3].width, Width::Fixed(8));
        assert_eq!(m.ports[3].dir, PortDir::InOut);
        assert_eq!(m.interface("s_axi").unwrap().prefix(), "s_axi_");
        assert_eq!(m.interface("m_str").unwrap().prefix(), "m_str_");
        assert_eq!(m.interface("m_str").unwrap().role, BusRole::Manager);
        assert_eq!(m.targets, ["ice40", "ecp5"]);
        assert_eq!(m.depends.len(), 2);
        assert!(m.depends[0].source.is_none());
        assert!(!m.is_blackbox());
    }

    #[test]
    fn project_round_trips_byte_for_byte() {
        let (p, rendered) = project(FULL_PROJECT);
        let p = p.expect("parses");
        assert_eq!(rendered, "");
        assert_eq!(p.to_text(), FULL_PROJECT);
        assert_eq!(p.device.as_deref(), Some("ice40-hx1k-tq144"));
        assert_eq!(
            p.dependency("uart_lite").unwrap().source,
            Some(DepSource::Path("../ip/uart_lite".to_owned()))
        );
        assert_eq!(
            p.dependency("fifo_sync").unwrap().source,
            Some(DepSource::Git {
                url: "https://example.invalid/fifo.git".to_owned(),
                rev: Some("v1.0.4".to_owned()),
            })
        );
        assert_eq!(
            p.dependency("cdc_sync").unwrap().source,
            Some(DepSource::Registry)
        );
    }

    #[test]
    fn a_minimal_manifest_is_two_lines() {
        let (m, rendered) = ip("name fifo\nversion 0.1.0\n");
        assert_eq!(rendered, "");
        let m = m.unwrap();
        assert!(m.is_blackbox());
        assert_eq!(m.to_text(), "name fifo\nversion 0.1.0\n");
    }

    #[test]
    fn comments_and_blank_lines_are_free() {
        let (m, rendered) = ip("# a comment\n\nname fifo  // trailing\nversion 0.1.0\n\n");
        assert_eq!(rendered, "");
        assert_eq!(m.unwrap().name, "fifo");
    }

    #[test]
    fn unknown_key_suggests_the_closest() {
        let (m, rendered) = ip("name f\nversion 1.0.0\nversoin 2\n");
        assert!(m.is_some());
        assert!(rendered.contains("error[P0001]: unknown key `versoin` in an IP manifest"));
        assert!(rendered.contains("did you mean `version`?"));
    }

    #[test]
    fn unknown_key_without_a_neighbour_lists_the_keys() {
        let (_, rendered) = ip("name f\nversion 1.0.0\nwobbler x\n");
        assert!(
            rendered.contains("known keys are: name, version"),
            "{rendered}"
        );
    }

    #[test]
    fn a_project_key_is_unknown_in_an_ip_manifest() {
        let (_, rendered) = ip("name f\nversion 1.0.0\ndevice ice40\n");
        assert!(
            rendered.contains("unknown key `device` in an IP manifest"),
            "{rendered}"
        );
        let (_, rendered) = project("name p\nversion 1.0.0\n");
        assert!(
            rendered.contains("unknown key `version` in a project manifest"),
            "{rendered}"
        );
    }

    #[test]
    fn every_manifest_diagnostic_fires() {
        // P0002: wrong shape.
        let (_, r) = ip("name a b\nversion 1.0.0\n");
        assert!(r.contains("error[P0002]: `name` takes one word"), "{r}");
        // P0003: bad version.
        let (m, r) = ip("name f\nversion 1.2\n");
        assert!(m.is_none());
        assert!(r.contains("error[P0003]: `1.2` is not a version"), "{r}");
        // P0004: bad requirement.
        let (_, r) = ip("name f\nversion 1.0.0\ndepends g ~1.0\n");
        assert!(
            r.contains("error[P0004]: `~1.0` is not a version requirement"),
            "{r}"
        );
        // P0005: duplicate.
        let (_, r) = ip("name f\nname g\nversion 1.0.0\n");
        assert!(
            r.contains("error[P0005]: `name` is given more than once"),
            "{r}"
        );
        let (_, r) = ip("name f\nversion 1.0.0\nversion 1.0.1\n");
        assert!(
            r.contains("error[P0005]: `version` is given more than once"),
            "{r}"
        );
        // P0006: missing.
        let (m, r) = ip("version 1.0.0\n");
        assert!(m.is_none());
        assert!(
            r.contains("error[P0006]: an IP manifest has no `name`"),
            "{r}"
        );
        let (m, r) = project("top t\n");
        assert!(m.is_none());
        assert!(
            r.contains("error[P0006]: a project manifest has no `name`"),
            "{r}"
        );
        // P0007: bad enumerated word.
        let (_, r) = ip("name f\nversion 1.0.0\nsource a.x language verilag\n");
        assert!(
            r.contains("error[P0007]: `verilag` is not a known language"),
            "{r}"
        );
        assert!(r.contains("did you mean `verilog`?"), "{r}");
        let (_, r) = ip("name f\nversion 1.0.0\nport clk into\n");
        assert!(r.contains("`into` is not a known direction"), "{r}");
        let (_, r) = ip("name f\nversion 1.0.0\ninterface s axi4lite slave\n");
        assert!(r.contains("`slave` is not a known bus role"), "{r}");
        let (_, r) = project("name f\ndepends g 1.0.0 svn http://x\n");
        assert!(r.contains("`svn` is not a known dependency source"), "{r}");
        // P0008: parameter problems.
        let (_, r) = ip("name f\nversion 1.0.0\nparam W int 99 8..64\n");
        assert!(
            r.contains("error[P0008]: the default 99 of `W` is outside 8..64"),
            "{r}"
        );
        let (_, r) = ip("name f\nversion 1.0.0\nparam W string a 8..64\n");
        assert!(r.contains("only an `int` parameter takes a range"), "{r}");
        let (_, r) = ip("name f\nversion 1.0.0\nparam W int 3 64..8\n");
        assert!(r.contains("the range `64..8` is empty"), "{r}");
    }

    #[test]
    fn an_ip_manifest_may_not_say_where_a_dependency_lives() {
        let (_, r) = ip("name f\nversion 1.0.0\ndepends g 1.0.0 path ../g\n");
        assert!(
            r.contains("an IP manifest states what it needs, not where it comes from"),
            "{r}"
        );
    }

    #[test]
    fn source_lines_take_language_and_encrypted_in_any_order() {
        let (m, r) = ip("name f\nversion 1.0.0\nsource a.x encrypted language vhdl\n");
        assert_eq!(r, "");
        let s = &m.unwrap().sources[0];
        assert!(s.encrypted);
        assert_eq!(s.language, Some(Language::Vhdl));
        let (_, r) = ip("name f\nversion 1.0.0\nsource a.v wobble\n");
        assert!(r.contains("`source` takes a path"), "{r}");
        let (_, r) = ip("name f\nversion 1.0.0\nsource\n");
        assert!(r.contains("`source` takes a path"), "{r}");
        let (_, r) = ip("name f\nversion 1.0.0\nsource a.v language\n");
        assert!(
            r.contains("`source` takes a language after `language`"),
            "{r}"
        );
    }

    #[test]
    fn a_model_is_single_valued() {
        let (_, r) = ip("name f\nversion 1.0.0\nmodel a.v\nmodel b.v\n");
        assert!(r.contains("`model` is given more than once"), "{r}");
    }

    #[test]
    fn interface_and_port_shapes_are_checked() {
        let (_, r) = ip("name f\nversion 1.0.0\ninterface s axi4lite\n");
        assert!(
            r.contains("`interface` takes `interface <name> <bus> <role> [prefix <p>]`"),
            "{r}"
        );
        let (_, r) = ip("name f\nversion 1.0.0\ninterface s axi4lite manager pre x\n");
        assert!(r.contains("`interface` takes"), "{r}");
        let (_, r) = ip("name f\nversion 1.0.0\nport a in 8 9\n");
        assert!(
            r.contains("`port` takes `port <name> <in|out|inout> [width]`"),
            "{r}"
        );
        let (_, r) = ip("name f\nversion 1.0.0\nport a in ?\n");
        assert!(r.contains("is not a width"), "{r}");
    }

    #[test]
    fn dependency_source_shapes_are_checked() {
        let (_, r) = project("name f\ndepends g 1.0.0 path\n");
        assert!(r.contains("`depends` takes `path <dir>`"), "{r}");
        let (_, r) = project("name f\ndepends g\n");
        assert!(
            r.contains("`depends` takes `depends <name> <requirement>`"),
            "{r}"
        );
        let (m, r) = project("name f\ndepends g 1.0.0 git u\n");
        assert_eq!(r, "");
        assert_eq!(
            m.unwrap().depends[0].source,
            Some(DepSource::Git {
                url: "u".to_owned(),
                rev: None
            })
        );
    }

    #[test]
    fn versions_order_and_display() {
        assert!(Version::new(1, 0, 0) < Version::new(1, 0, 1));
        assert!(Version::new(1, 9, 0) < Version::new(2, 0, 0));
        assert!(Version::parse("1.0.0-alpha").unwrap() < Version::parse("1.0.0-beta").unwrap());
        assert!(Version::parse("1.0.0-beta").unwrap() < Version::new(1, 0, 0));
        assert_eq!(
            Version::parse("1.0.0-rc1").unwrap().to_string(),
            "1.0.0-rc1"
        );
        assert!(Version::parse("1.0.0-").is_none());
        assert!(Version::parse("1.0.0.0").is_none());
        assert!(Version::parse("x.0.0").is_none());
        assert!(Version::parse("1.0.0-rc1").unwrap().is_prerelease());
    }

    #[test]
    fn requirement_grammar_covers_the_four_forms() {
        let v = |s: &str| Version::parse(s).unwrap();
        let req = |s: &str| VersionReq::parse(s).unwrap();

        assert_eq!(req("*"), VersionReq::Any);
        assert!(req("*").matches(&v("9.9.9")));
        assert!(!req("*").matches(&v("1.0.0-rc1")));

        assert_eq!(req("1.2.3"), VersionReq::Exact(v("1.2.3")));
        assert!(req("1.2.3").matches(&v("1.2.3")));
        assert!(!req("1.2.3").matches(&v("1.2.4")));
        assert!(req("1.0.0-rc1").matches(&v("1.0.0-rc1")));

        assert!(req("^1.2.3").matches(&v("1.9.0")));
        assert!(!req("^1.2.3").matches(&v("1.2.2")));
        assert!(!req("^1.2.3").matches(&v("2.0.0")));
        assert!(req("^0.2.3").matches(&v("0.2.9")));
        assert!(!req("^0.2.3").matches(&v("0.3.0")));
        assert!(req("^0.0.3").matches(&v("0.0.3")));
        assert!(!req("^0.0.3").matches(&v("0.0.4")));
        assert!(!req("^1.0.0").matches(&v("1.1.0-rc1")));

        assert!(req(">=1.2.3").matches(&v("9.0.0")));
        assert!(!req(">=1.2.3").matches(&v("1.2.2")));

        assert!(VersionReq::parse("~1.2.3").is_none());
        assert!(VersionReq::parse(">1.2.3").is_none());
        for text in ["*", "1.2.3", "^1.2.3", ">=1.2.3"] {
            assert_eq!(VersionReq::parse(text).unwrap().to_string(), text);
        }
    }

    #[test]
    fn language_keywords_round_trip() {
        for language in Language::ALL {
            assert_eq!(Language::from_keyword(language.keyword()), Some(language));
        }
        assert_eq!(Language::from_keyword("VHDL"), Some(Language::Vhdl));
        assert_eq!(Language::from_path("a/b.RTL"), Some(Language::Rtl));
        assert_eq!(Language::from_path("noext"), None);
        for ty in ParamType::ALL {
            assert_eq!(ParamType::from_keyword(ty.keyword()), Some(ty));
        }
        assert_eq!(ParamType::Bits.to_string(), "bits");
        assert_eq!(Language::Vhdl.to_string(), "vhdl");
    }

    #[test]
    fn dependency_sources_describe_themselves() {
        assert_eq!(DepSource::Registry.to_string(), "registry");
        assert_eq!(DepSource::Registry.describe(), "the registry");
        assert_eq!(
            DepSource::Path("../a".to_owned()).describe(),
            "the directory `../a`"
        );
        let git = DepSource::Git {
            url: "u".to_owned(),
            rev: Some("r".to_owned()),
        };
        assert_eq!(git.to_string(), "git u rev r");
        assert_eq!(git.describe(), "the repository `u` at `r`");
        assert_eq!(
            DepSource::Git {
                url: "u".to_owned(),
                rev: None
            }
            .describe(),
            "the repository `u`"
        );
    }

    #[test]
    fn empty_manifests_report_both_missing_keys() {
        let (m, r) = ip("");
        assert!(m.is_none());
        assert!(r.contains("has no `name`"), "{r}");
        let (m, r) = ip("name f\n");
        assert!(m.is_none());
        assert!(r.contains("has no `version`"), "{r}");
    }

    #[test]
    fn builders_produce_parseable_text() {
        let mut map = SourceMap::new();
        let file = map.add("x", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut m = IpManifest::new("fifo", Version::new(2, 0, 0), span);
        m.depends
            .push(Dependency::new("cdc", VersionReq::Any, span));
        let text = m.to_text();
        assert_eq!(text, "name fifo\nversion 2.0.0\n\ndepends cdc *\n");
        let mut p = Project::new("top", span);
        p.top = Some("top".to_owned());
        assert_eq!(p.to_text(), "name top\ntop top\n");
    }
}
