//! AST-level lint rules for Verilog and SystemVerilog.
//!
//! The linter runs on a parsed [`ast::SourceFile`] and needs neither
//! elaboration nor synthesis: everything it knows comes from one traversal
//! that collects [`facts::FileFacts`] (declarations, reads, writes,
//! processes, instances) plus the lexer's comment table and the source
//! text. Rules then query those facts, so a run is linear in the size of
//! the tree.
//!
//! # Using it
//!
//! ```
//! use reticle::diag::Diagnostics;
//! use reticle::source::SourceMap;
//! use reticle::verilog::lint::{LintConfig, run};
//! use reticle::verilog::{Dialect, NoIncludes, lex_source_full, parse_source};
//!
//! let src = "module m(input a, output y);\n  wire unused;\n  assign y = a;\nendmodule\n";
//! let mut map = SourceMap::new();
//! let id = map.add("m.v", src).unwrap();
//! let mut diags = Diagnostics::new();
//! let lexed = lex_source_full(&mut map, id, Dialect::Verilog2005, &mut NoIncludes, &mut diags);
//! let file = parse_source(&mut map, id, Dialect::Verilog2005, &mut NoIncludes, &mut diags);
//!
//! let config = LintConfig::new();
//! let mut lints = Diagnostics::new();
//! run(&config, &map, id, &file, &lexed.comments, Dialect::Verilog2005, &mut lints);
//! assert!(lints.iter().any(|d| d.message.contains("`unused`")));
//! ```
//!
//! # Rules
//!
//! Every rule implements [`Lint`]: a stable code (`L0001`), a kebab-case
//! name (`unused-signal`), a default [`Level`] and a `check`. [`LintSet`]
//! is the registry; [`LintConfig`] overrides levels by name and carries the
//! few options rules take. `docs/lints.md` documents each rule with an
//! example.
//!
//! # Turning rules off
//!
//! Besides the configuration, a source file can silence a rule itself:
//!
//! - `(* lint_off *)` or `(* lint_off = "unused-signal, tabs" *)` on an
//!   item, port or statement silences the named rules (or all of them)
//!   inside it;
//! - `// reticle-lint: off <names>` silences them from that comment to a
//!   matching `// reticle-lint: on <names>` or to the end of the file;
//! - `// reticle-lint: off-line <names>` and
//!   `// reticle-lint: off-next-line <names>` silence one line.
//!
//! An empty name list means every rule.

pub mod facts;
pub mod width;

mod control;
mod procedural;
mod signals;
mod style;
mod text;
mod widths;

use std::collections::BTreeSet;

use crate::diag::{Diagnostic, Diagnostics, Severity};
use crate::source::{SourceId, SourceMap, Span};

use super::ast;
use super::lex::CommentKind;
use super::token::Dialect;

use facts::FileFacts;

/// The level a rule reports at, or [`Level::Off`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// The rule does not run.
    Off,
    /// The rule reports at this severity.
    On(Severity),
}

impl Level {
    /// Reports as [`Severity::Note`].
    pub const NOTE: Level = Level::On(Severity::Note);
    /// Reports as [`Severity::Warning`].
    pub const WARN: Level = Level::On(Severity::Warning);
    /// Reports as [`Severity::Error`].
    pub const ERROR: Level = Level::On(Severity::Error);

    /// The severity, or `None` when off.
    pub fn severity(self) -> Option<Severity> {
        match self {
            Level::Off => None,
            Level::On(s) => Some(s),
        }
    }

    /// The name used in configuration text (`off`, `note`, `warn`,
    /// `error`).
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::On(Severity::Help) | Level::On(Severity::Note) => "note",
            Level::On(Severity::Warning) => "warn",
            Level::On(Severity::Error) => "error",
        }
    }

    /// Parses a level name; `allow`, `info` and `deny` are accepted as
    /// aliases of `off`, `note` and `error`.
    pub fn parse(text: &str) -> Option<Level> {
        match text {
            "off" | "allow" => Some(Level::Off),
            "note" | "info" => Some(Level::NOTE),
            "warn" | "warning" => Some(Level::WARN),
            "error" | "deny" => Some(Level::ERROR),
            _ => None,
        }
    }
}

/// One lint rule.
///
/// Rules are stateless: everything they need is in the [`LintContext`] and
/// the tree they are handed. They report through [`LintContext::report`],
/// which applies the effective level and the source suppressions.
pub trait Lint {
    /// A stable code such as `L0001`, shown in the diagnostic and used for
    /// documentation links.
    fn id(&self) -> &'static str;

    /// The kebab-case name used in configuration and `lint_off`.
    fn name(&self) -> &'static str;

    /// The level the rule reports at when nothing overrides it.
    fn default_level(&self) -> Level;

    /// Runs the rule over one file.
    fn check(&self, ctx: &LintContext<'_>, file: &ast::SourceFile, diags: &mut Diagnostics);
}

/// The registry of rules and their levels.
///
/// [`LintSet::all`] builds the set with every rule at its default level;
/// [`LintSet::set_level`], [`LintSet::enable`] and [`LintSet::disable`]
/// change one by name.
pub struct LintSet {
    rules: Vec<Registered>,
}

/// One entry of a [`LintSet`].
struct Registered {
    rule: Box<dyn Lint>,
    level: Level,
    /// The level was set explicitly rather than taken from the rule.
    explicit: bool,
}

impl std::fmt::Debug for LintSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.rules.iter().map(|r| (r.rule.name(), r.level)))
            .finish()
    }
}

impl Default for LintSet {
    fn default() -> Self {
        Self::all()
    }
}

impl LintSet {
    /// Every rule, at its default level, in code order.
    pub fn all() -> Self {
        let rules: Vec<Box<dyn Lint>> = vec![
            Box::new(signals::UnusedSignal),
            Box::new(signals::UnusedInput),
            Box::new(signals::UndrivenSignal),
            Box::new(signals::UndrivenOutput),
            Box::new(signals::MultipleDrivers),
            Box::new(signals::ImplicitNet),
            Box::new(procedural::BlockingInSequential),
            Box::new(procedural::NonBlockingInComb),
            Box::new(procedural::MixedAssignmentStyles),
            Box::new(procedural::SensitivityList),
            Box::new(procedural::ResetStyle),
            Box::new(control::IncompleteCase),
            Box::new(control::LatchInferred),
            Box::new(control::CaseXZ),
            Box::new(control::UnreachableStatement),
            Box::new(control::ConstantCondition),
            Box::new(widths::WidthMismatch),
            Box::new(widths::UnsizedLiteralInConcat),
            Box::new(style::PortConnection),
            Box::new(style::NonAnsiPorts),
            Box::new(style::MissingDefaultNettype),
            Box::new(style::KeywordAsIdentifier),
            Box::new(style::DeprecatedConstruct),
            Box::new(style::TodoComment),
            Box::new(style::Naming),
            Box::new(text::LineLength),
            Box::new(text::TrailingWhitespace),
            Box::new(text::Tabs),
        ];
        LintSet {
            rules: rules
                .into_iter()
                .map(|rule| Registered {
                    level: rule.default_level(),
                    rule,
                    explicit: false,
                })
                .collect(),
        }
    }

    /// An empty set, for a caller that registers only what it wants.
    pub fn empty() -> Self {
        LintSet { rules: Vec::new() }
    }

    /// Adds a rule at its default level.
    pub fn register(&mut self, rule: Box<dyn Lint>) {
        self.rules.push(Registered {
            level: rule.default_level(),
            rule,
            explicit: false,
        });
    }

    /// The names of every registered rule, in code order.
    pub fn names(&self) -> impl Iterator<Item = &'static str> {
        self.rules.iter().map(|r| r.rule.name())
    }

    /// The rules, with the level each will report at.
    pub fn rules(&self) -> impl Iterator<Item = (&dyn Lint, Level)> {
        self.rules.iter().map(|r| (&*r.rule, r.level))
    }

    /// True when a rule of that name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.rules.iter().any(|r| r.rule.name() == name)
    }

    /// Sets one rule's level, or every rule's when `name` is `all`.
    ///
    /// Returns `false` when no such rule is registered.
    pub fn set_level(&mut self, name: &str, level: Level) -> bool {
        if name == "all" {
            for r in &mut self.rules {
                r.level = level;
                r.explicit = true;
            }
            return true;
        }
        let mut found = false;
        for r in self.rules.iter_mut().filter(|r| r.rule.name() == name) {
            r.level = level;
            r.explicit = true;
            found = true;
        }
        found
    }

    /// Turns a rule on at its default level (or at a warning when its
    /// default is `off`).
    pub fn enable(&mut self, name: &str) -> bool {
        let mut found = false;
        for r in self
            .rules
            .iter_mut()
            .filter(|r| name == "all" || r.rule.name() == name)
        {
            r.level = match r.rule.default_level() {
                Level::Off => Level::WARN,
                on => on,
            };
            r.explicit = true;
            found = true;
        }
        found
    }

    /// Turns a rule off.
    pub fn disable(&mut self, name: &str) -> bool {
        self.set_level(name, Level::Off)
    }

    /// Applies a configuration's overrides.
    pub fn apply(&mut self, config: &LintConfig) {
        for (name, level) in &config.levels {
            self.set_level(name, *level);
        }
    }
}

/// An error in configuration text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintConfigError {
    /// What is wrong.
    pub message: String,
}

impl std::fmt::Display for LintConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LintConfigError {}

/// Level overrides and rule options.
///
/// [`LintConfig::parse`] reads the text form: whitespace-, comma- or
/// newline-separated items, `#` starting a comment.
///
/// ```
/// use reticle::verilog::lint::{Level, LintConfig};
///
/// let config = LintConfig::parse(
///     "# project settings\n\
///      error:multiple-drivers, off:unused-signal\n\
///      warn:naming max-line-length=100",
/// )
/// .unwrap();
/// assert_eq!(config.level("multiple-drivers"), Some(Level::ERROR));
/// assert_eq!(config.level("unused-signal"), Some(Level::Off));
/// assert_eq!(config.max_line_length, 100);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintConfig {
    /// Level overrides, in the order they were given; `all` is allowed as
    /// a name.
    levels: Vec<(String, Level)>,
    /// The column at which `line-length` reports, counted in characters.
    pub max_line_length: usize,
    /// The dialect whose reserved words `keyword-as-identifier` checks
    /// against; identifiers reserved in it but not in the file's own
    /// dialect are reported.
    pub strict_dialect: Dialect,
}

impl Default for LintConfig {
    fn default() -> Self {
        LintConfig {
            levels: Vec::new(),
            max_line_length: 100,
            strict_dialect: Dialect::SystemVerilog,
        }
    }
}

impl LintConfig {
    /// The default configuration: every rule at its default level.
    pub fn new() -> Self {
        Self::default()
    }

    /// The level explicitly set for `name`, if any.
    pub fn level(&self, name: &str) -> Option<Level> {
        self.levels
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, l)| *l)
    }

    /// Overrides one rule's level, or every rule's when `name` is `all`.
    ///
    /// Fails when no rule of that name exists, so a typo in a project's
    /// configuration is reported rather than silently ignored.
    pub fn set_level(&mut self, name: &str, level: Level) -> Result<(), LintConfigError> {
        if name != "all" && !LintSet::all().contains(name) {
            return Err(LintConfigError {
                message: format!("unknown lint `{name}`"),
            });
        }
        self.levels.push((name.to_string(), level));
        Ok(())
    }

    /// Parses the text form.
    ///
    /// Items are `<level>:<name>` (`off`, `note`, `warn`, `error`, with
    /// `allow`, `info` and `deny` as aliases; `all` as a name sets every
    /// rule) or `<option>=<value>` (`max-line-length`, `strict-dialect`).
    /// `#` starts a comment.
    pub fn parse(text: &str) -> Result<Self, LintConfigError> {
        let mut config = LintConfig::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("");
            for item in line.split([',', ' ', '\t', ';']).filter(|s| !s.is_empty()) {
                config.parse_item(item)?;
            }
        }
        Ok(config)
    }

    fn parse_item(&mut self, item: &str) -> Result<(), LintConfigError> {
        if let Some((key, value)) = item.split_once('=') {
            return self.parse_option(key, value);
        }
        let Some((level, name)) = item.split_once(':') else {
            return Err(LintConfigError {
                message: format!("expected `<level>:<lint>` or `<option>=<value>`, found `{item}`"),
            });
        };
        let Some(level) = Level::parse(level) else {
            return Err(LintConfigError {
                message: format!("unknown lint level `{level}`; expected off, note, warn or error"),
            });
        };
        self.set_level(name, level)
    }

    fn parse_option(&mut self, key: &str, value: &str) -> Result<(), LintConfigError> {
        match key {
            "max-line-length" => {
                self.max_line_length = value.parse().map_err(|_| LintConfigError {
                    message: format!("`max-line-length` expects a number, found `{value}`"),
                })?;
                Ok(())
            }
            "strict-dialect" => {
                self.strict_dialect = match value {
                    "verilog2001" | "1364-2001" => Dialect::Verilog2001,
                    "verilog2005" | "1364-2005" => Dialect::Verilog2005,
                    "systemverilog" | "sv" | "1800" => Dialect::SystemVerilog,
                    other => {
                        return Err(LintConfigError {
                            message: format!("unknown dialect `{other}`"),
                        });
                    }
                };
                Ok(())
            }
            other => Err(LintConfigError {
                message: format!("unknown lint option `{other}`"),
            }),
        }
    }
}

/// A region of the file in which some rules are silenced.
#[derive(Clone, Debug)]
struct Region {
    start: u32,
    end: u32,
    /// The silenced rules; `None` for all of them.
    names: Option<Vec<String>>,
}

/// The `lint_off` attributes and `reticle-lint` comments of one file.
#[derive(Clone, Debug, Default)]
struct Suppressions {
    regions: Vec<Region>,
}

impl Suppressions {
    /// Collects the attribute regions and the comment directives.
    fn collect(
        map: &SourceMap,
        source: SourceId,
        comments: &[(Span, CommentKind)],
        facts: &FileFacts<'_>,
    ) -> Self {
        let mut out = Suppressions::default();
        for off in &facts.lint_off {
            if off.span.file != source {
                continue;
            }
            out.regions.push(Region {
                start: off.span.start,
                end: off.span.end,
                names: off.names.clone(),
            });
        }

        let file = map.file(source);
        let text = file.text();
        let end_of_file = u32::try_from(text.len()).unwrap_or(u32::MAX);
        // Open `off` regions awaiting an `on`.
        let mut open: Vec<Region> = Vec::new();
        for (span, _) in comments {
            if span.file != source {
                continue;
            }
            let body = comment_body(text, *span);
            let Some(rest) = body.trim().strip_prefix("reticle-lint:") else {
                continue;
            };
            let mut words = rest
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty());
            let Some(directive) = words.next() else {
                continue;
            };
            let names: Vec<String> = words.map(str::to_string).collect();
            let names = if names.is_empty() { None } else { Some(names) };
            let line = file.loc(span.start).line;
            match directive {
                "off" => open.push(Region {
                    start: span.start,
                    end: end_of_file,
                    names,
                }),
                "on" => {
                    let mut still_open = Vec::new();
                    for mut region in open.drain(..) {
                        if overlaps(&region.names, &names) {
                            region.end = span.end;
                            out.regions.push(region);
                        } else {
                            still_open.push(region);
                        }
                    }
                    open = still_open;
                }
                "off-line" | "off-next-line" => {
                    let target = if directive == "off-line" {
                        line
                    } else {
                        line + 1
                    };
                    if let Some((start, end)) = line_bounds(map, source, target) {
                        out.regions.push(Region { start, end, names });
                    }
                }
                _ => {}
            }
        }
        out.regions.extend(open);
        out
    }

    /// True when `name` is silenced at `span`.
    fn silences(&self, name: &str, span: Span, source: SourceId) -> bool {
        if span.file != source {
            return false;
        }
        self.regions.iter().any(|r| {
            r.start <= span.start
                && span.start < r.end.max(r.start + 1)
                && r.names
                    .as_ref()
                    .is_none_or(|names| names.iter().any(|n| n == name))
        })
    }
}

/// True when two name sets intersect; `None` (all) intersects everything.
fn overlaps(a: &Option<Vec<String>>, b: &Option<Vec<String>>) -> bool {
    match (a, b) {
        (None, _) | (_, None) => true,
        (Some(a), Some(b)) => a.iter().any(|x| b.contains(x)),
    }
}

/// The byte range of a 1-based line, newline excluded.
fn line_bounds(map: &SourceMap, source: SourceId, line: u32) -> Option<(u32, u32)> {
    let file = map.file(source);
    let text = file.line_text(line)?;
    // `loc` maps an offset to a line; walk back from the line's start by
    // asking for the first column.
    let start = line_start(map, source, line)?;
    let len = u32::try_from(text.len()).unwrap_or(0);
    Some((start, start + len + 1))
}

/// The byte offset at which a 1-based line starts.
fn line_start(map: &SourceMap, source: SourceId, line: u32) -> Option<u32> {
    let file = map.file(source);
    if line == 0 || line > file.line_count() {
        return None;
    }
    let text = file.text();
    let mut offset = 0usize;
    for _ in 1..line {
        let rest = &text[offset..];
        let nl = rest.find('\n')?;
        offset += nl + 1;
    }
    u32::try_from(offset).ok()
}

/// The text of a comment with its delimiters removed.
fn comment_body(text: &str, span: Span) -> &str {
    let start = span.start as usize;
    let end = (span.end as usize).min(text.len());
    if start >= end {
        return "";
    }
    let body = &text[start..end];
    if let Some(rest) = body.strip_prefix("//") {
        rest
    } else {
        body.strip_prefix("/*")
            .map(|r| r.strip_suffix("*/").unwrap_or(r))
            .unwrap_or(body)
    }
}

/// What a rule sees: the facts, the text and the machinery to report.
///
/// One context is built per rule, so [`LintContext::report`] knows the
/// name, code and effective level without the rule repeating them.
pub struct LintContext<'a> {
    /// The source map the file came from.
    pub map: &'a SourceMap,
    /// The linted file.
    pub source: SourceId,
    /// The facts collected from the tree.
    pub facts: &'a FileFacts<'a>,
    /// Every comment, in source order.
    pub comments: &'a [(Span, CommentKind)],
    /// The dialect the file was parsed in.
    pub dialect: Dialect,
    /// The configuration, for rules with options.
    pub config: &'a LintConfig,
    suppressions: &'a Suppressions,
    rule_name: &'static str,
    rule_id: &'static str,
    level: Severity,
    /// The level came from the configuration, so a rule must not adjust it.
    explicit: bool,
}

impl<'a> LintContext<'a> {
    /// The text of the linted file.
    pub fn text(&self) -> &'a str {
        self.map.file(self.source).text()
    }

    /// The name of the running rule.
    pub fn rule_name(&self) -> &'static str {
        self.rule_name
    }

    /// The severity the running rule reports at.
    pub fn level(&self) -> Severity {
        self.level
    }

    /// Starts a report at the rule's level, primary label on `span`.
    ///
    /// The returned builder is empty (and emits nothing) when the rule is
    /// silenced at that position.
    pub fn report(&self, span: Span, message: impl Into<String>) -> Report<'_> {
        self.report_at(self.level, span, message)
    }

    /// Starts a report at `severity` instead of the rule's own level,
    /// unless the configuration set that level explicitly.
    ///
    /// Used by the rules whose severity depends on the source, such as
    /// `implicit-net` under `` `default_nettype none ``.
    pub fn report_as(
        &self,
        severity: Severity,
        span: Span,
        message: impl Into<String>,
    ) -> Report<'_> {
        let severity = if self.explicit { self.level } else { severity };
        self.report_at(severity, span, message)
    }

    fn report_at(&self, severity: Severity, span: Span, message: impl Into<String>) -> Report<'_> {
        if self
            .suppressions
            .silences(self.rule_name, span, self.source)
        {
            return Report {
                ctx: self,
                diag: None,
            };
        }
        let diag = Diagnostic::new(severity, message)
            .with_code(self.rule_id)
            .with_span(span);
        Report {
            ctx: self,
            diag: Some(diag),
        }
    }

    /// The span of the whole file, for reports that have no better place.
    pub fn file_span(&self) -> Span {
        let len = u32::try_from(self.text().len()).unwrap_or(0);
        Span::new(self.source, 0, len.min(1))
    }

    /// The 1-based line a span starts on.
    pub fn line_of(&self, span: Span) -> u32 {
        self.map.file(span.file).loc(span.start).line
    }
}

/// A diagnostic under construction, or nothing when the rule is silenced.
#[must_use = "a report does nothing until `emit` is called"]
pub struct Report<'a> {
    ctx: &'a LintContext<'a>,
    diag: Option<Diagnostic>,
}

impl Report<'_> {
    /// True when the report will emit nothing.
    pub fn is_silenced(&self) -> bool {
        self.diag.is_none()
    }

    /// Adds a primary label.
    pub fn label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.diag = self.diag.map(|d| d.with_label(span, message));
        self
    }

    /// Adds a secondary label.
    pub fn secondary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.diag = self.diag.map(|d| d.with_secondary(span, message));
        self
    }

    /// Adds a note line.
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.diag = self.diag.map(|d| d.with_note(note));
        self
    }

    /// Pushes the diagnostic, with the trailing `lint:` note.
    pub fn emit(self, diags: &mut Diagnostics) {
        let Some(diag) = self.diag else { return };
        let name = self.ctx.rule_name;
        diags.push(diag.with_note(format!(
            "lint: {name}, disable with `// reticle-lint: off {name}` or `(* lint_off = \"{name}\" *)`"
        )));
    }
}

/// Runs every enabled rule of the default set over one parsed file.
///
/// `comments` is the lexer's comment table ([`super::Lexed::comments`]);
/// pass an empty slice when it is not available, at the cost of the
/// comment-driven rules and the `// reticle-lint:` directives. Diagnostics
/// are appended to `diags` sorted by position, so the output is
/// deterministic whatever order the rules ran in.
pub fn run(
    config: &LintConfig,
    map: &SourceMap,
    source: SourceId,
    file: &ast::SourceFile,
    comments: &[(Span, CommentKind)],
    dialect: Dialect,
    diags: &mut Diagnostics,
) {
    let mut set = LintSet::all();
    set.apply(config);
    run_with(&set, config, map, source, file, comments, dialect, diags);
}

/// Runs the rules of `set`, for a caller with its own registry.
#[allow(clippy::too_many_arguments)]
pub fn run_with(
    set: &LintSet,
    config: &LintConfig,
    map: &SourceMap,
    source: SourceId,
    file: &ast::SourceFile,
    comments: &[(Span, CommentKind)],
    dialect: Dialect,
    diags: &mut Diagnostics,
) {
    let facts = FileFacts::collect(file);
    let suppressions = Suppressions::collect(map, source, comments, &facts);
    let mut out = Diagnostics::new();
    for entry in &set.rules {
        let Some(level) = entry.level.severity() else {
            continue;
        };
        let ctx = LintContext {
            map,
            source,
            facts: &facts,
            comments,
            dialect,
            config,
            suppressions: &suppressions,
            rule_name: entry.rule.name(),
            rule_id: entry.rule.id(),
            level,
            explicit: entry.explicit,
        };
        entry.rule.check(&ctx, file, &mut out);
    }
    out.sort();
    diags.append(&mut out);
}

/// Joins names for a diagnostic message: `` `a`, `b` and `c` ``.
pub(crate) fn join_names(names: &BTreeSet<String>) -> String {
    let names: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        _ => format!(
            "{} and {}",
            names[..names.len() - 1].join(", "),
            names[names.len() - 1]
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verilog::{NoIncludes, lex_source_full, parse_source};

    /// Lints one source text with the given configuration.
    pub(crate) fn lint(src: &str, config: &LintConfig) -> String {
        let mut map = SourceMap::new();
        let id = map.add("t.sv", src).unwrap();
        let mut diags = Diagnostics::new();
        let lexed = lex_source_full(
            &mut map,
            id,
            Dialect::SystemVerilog,
            &mut NoIncludes,
            &mut diags,
        );
        let file = parse_source(
            &mut map,
            id,
            Dialect::SystemVerilog,
            &mut NoIncludes,
            &mut diags,
        );
        assert!(diags.is_empty(), "parse errors: {}", diags.render(&map));
        let mut lints = Diagnostics::new();
        run(
            config,
            &map,
            id,
            &file,
            &lexed.comments,
            Dialect::SystemVerilog,
            &mut lints,
        );
        lints.render(&map)
    }

    #[test]
    fn every_rule_has_a_unique_code_and_name() {
        let set = LintSet::all();
        let mut codes = BTreeSet::new();
        let mut names = BTreeSet::new();
        for (rule, _) in set.rules() {
            assert!(codes.insert(rule.id()), "duplicate code {}", rule.id());
            assert!(names.insert(rule.name()), "duplicate name {}", rule.name());
            assert_eq!(rule.id().len(), 5, "{} has an odd code", rule.name());
            assert!(rule.id().starts_with('L'));
            assert!(
                rule.name()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-'),
                "{} is not kebab-case",
                rule.name()
            );
        }
    }

    #[test]
    fn config_parses_levels_and_options() {
        let config = LintConfig::parse(
            "# comment\n\
             off:unused-signal warn:naming\n\
             deny:multiple-drivers, info:tabs\n\
             max-line-length=80 strict-dialect=verilog2001\n",
        )
        .unwrap();
        assert_eq!(config.level("unused-signal"), Some(Level::Off));
        assert_eq!(config.level("naming"), Some(Level::WARN));
        assert_eq!(config.level("multiple-drivers"), Some(Level::ERROR));
        assert_eq!(config.level("tabs"), Some(Level::NOTE));
        assert_eq!(config.level("line-length"), None);
        assert_eq!(config.max_line_length, 80);
        assert_eq!(config.strict_dialect, Dialect::Verilog2001);

        let mut set = LintSet::all();
        set.apply(&config);
        let levels: Vec<_> = set
            .rules()
            .filter(|(r, _)| r.name() == "unused-signal" || r.name() == "naming")
            .map(|(_, l)| l)
            .collect();
        assert_eq!(levels, [Level::Off, Level::WARN]);
    }

    #[test]
    fn config_rejects_typos() {
        assert_eq!(
            LintConfig::parse("off:unused-signl").unwrap_err().message,
            "unknown lint `unused-signl`"
        );
        assert!(
            LintConfig::parse("shout:tabs")
                .unwrap_err()
                .message
                .contains("unknown lint level")
        );
        assert!(
            LintConfig::parse("unused-signal")
                .unwrap_err()
                .message
                .contains("expected")
        );
        assert!(
            LintConfig::parse("max-line-length=wide")
                .unwrap_err()
                .message
                .contains("expects a number")
        );
        assert!(
            LintConfig::parse("tab-width=4")
                .unwrap_err()
                .message
                .contains("unknown lint option")
        );
    }

    #[test]
    fn all_sets_every_level() {
        let config = LintConfig::parse("off:all").unwrap();
        let mut set = LintSet::all();
        set.apply(&config);
        assert!(set.rules().all(|(_, l)| l == Level::Off));

        let mut set = LintSet::all();
        set.enable("all");
        assert!(set.rules().all(|(_, l)| l != Level::Off));
        assert!(!set.enable("nonesuch"));
        assert!(set.disable("tabs"));
    }

    /// A configuration with `unused-signal` as the only enabled rule.
    fn only_unused() -> LintConfig {
        LintConfig::parse("off:all warn:unused-signal").unwrap()
    }

    #[test]
    fn comment_directives_silence_rules() {
        let src = "module m;\n  wire a;\n  wire b; // reticle-lint: off-line unused-signal\n\
                   endmodule\n";
        let out = lint(src, &only_unused());
        assert!(out.contains("`a`"), "{out}");
        assert!(!out.contains("`b`"), "{out}");

        let src = "module m;\n// reticle-lint: off unused-signal\n  wire a;\n\
                   // reticle-lint: on unused-signal\n  wire b;\nendmodule\n";
        let out = lint(src, &only_unused());
        assert!(!out.contains("`a`"), "{out}");
        assert!(out.contains("`b`"), "{out}");

        // A bare `off` silences everything to the end of the file.
        let src = "module m;\n// reticle-lint: off\n  wire a;\n  wire b;\nendmodule\n";
        assert_eq!(lint(src, &only_unused()), "");

        // `off-next-line` covers the following line only.
        let src = "module m;\n  // reticle-lint: off-next-line\n  wire a;\n  wire b;\nendmodule\n";
        let out = lint(src, &only_unused());
        assert!(!out.contains("`a`"), "{out}");
        assert!(out.contains("`b`"), "{out}");
    }

    #[test]
    fn lint_off_attributes_silence_rules() {
        let src = "module m;\n  (* lint_off = \"unused-signal\" *) wire a;\n  wire b;\nendmodule\n";
        let out = lint(src, &only_unused());
        assert!(!out.contains("`a`"), "{out}");
        assert!(out.contains("`b`"), "{out}");

        let src = "module m;\n  (* lint_off *) wire a;\nendmodule\n";
        assert_eq!(lint(src, &only_unused()), "");

        // A different name does not silence this one.
        let src = "module m;\n  (* lint_off = \"tabs\" *) wire a;\nendmodule\n";
        assert!(lint(src, &only_unused()).contains("`a`"));
    }

    #[test]
    fn levels_are_configurable_per_rule() {
        let src = "module m;\n  wire a;\nendmodule\n";
        assert!(lint(src, &only_unused()).starts_with("warning[L0001]"));
        let config = LintConfig::parse("off:all error:unused-signal").unwrap();
        assert!(lint(src, &config).starts_with("error[L0001]"));
        let config = LintConfig::parse("off:all").unwrap();
        assert_eq!(lint(src, &config), "");
    }
}
