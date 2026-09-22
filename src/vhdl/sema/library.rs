//! Design libraries, design units and analysis ordering (IEEE 1076-2008
//! clause 13).
//!
//! A [`Design`] gathers parsed files, each compiled into a named library
//! (`work` by default; the bundled sources go into `std` and `ieee`). Before
//! analysis it builds one [`Unit`] per library unit and orders them so that
//! every unit comes after the units it depends on, which clause 13.5
//! requires (a package body needs its package, an architecture its entity,
//! and any unit the packages named in its `use` clauses). Dependencies are
//! taken from the context clause, from the primary/secondary relation, and
//! from `library.name` selected names inside the unit, so a file set may be
//! given in any order. Independent units keep their source order, which
//! keeps diagnostics deterministic.
//!
//! Units in different libraries may share a name; a unit is identified by
//! `(library, name, kind)`. Re-declaring a primary unit in the same library
//! is reported and the first declaration wins.

use crate::diag::{Diagnostic, Diagnostics};
use crate::intern::{Interner, Symbol};
use crate::source::{SourceId, SourceMap, Span};
use crate::vhdl::ast::{self, ContextItem, LibraryUnit, Name, Suffix};
use crate::vhdl::{Standard, parse_source, stdlib};

use super::{Analysis, AnalyzedFile, DeclId, RegionId};

/// Index of a [`Unit`] in [`Analysis::units`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitId(u32);

impl UnitId {
    /// The raw index.
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(i: usize) -> UnitId {
        UnitId(u32::try_from(i).expect("unit count"))
    }
}

/// The kind of a library unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryUnitKind {
    /// An entity declaration.
    Entity,
    /// An architecture body.
    Architecture,
    /// A package declaration.
    Package,
    /// A package body.
    PackageBody,
    /// A package instantiation declaration.
    PackageInstantiation,
    /// A configuration declaration.
    Configuration,
    /// A context declaration.
    Context,
}

impl LibraryUnitKind {
    /// True for primary units (everything but architectures and package
    /// bodies).
    pub fn is_primary(self) -> bool {
        !matches!(
            self,
            LibraryUnitKind::Architecture | LibraryUnitKind::PackageBody
        )
    }

    /// The unit kind as a lowercase phrase.
    pub fn as_str(self) -> &'static str {
        match self {
            LibraryUnitKind::Entity => "entity",
            LibraryUnitKind::Architecture => "architecture",
            LibraryUnitKind::Package => "package",
            LibraryUnitKind::PackageBody => "package body",
            LibraryUnitKind::PackageInstantiation => "package instantiation",
            LibraryUnitKind::Configuration => "configuration",
            LibraryUnitKind::Context => "context",
        }
    }
}

/// One design unit of the analysed design.
#[derive(Clone, Debug)]
pub struct Unit {
    /// The library it was compiled into.
    pub library: Symbol,
    /// Its name (an architecture's own name, not its entity's).
    pub name: Symbol,
    /// Its kind.
    pub kind: LibraryUnitKind,
    /// For an architecture the entity name, for a package body the package
    /// name.
    pub primary: Option<Symbol>,
    /// Index into [`Analysis::files`].
    pub file: usize,
    /// Index into that file's `units`.
    pub index: usize,
    /// The unit's span.
    pub span: Span,
    /// The declaration naming the unit, once analysed.
    pub decl: Option<DeclId>,
    /// The unit's declarative region, once analysed.
    pub region: Option<RegionId>,
    /// True once analysed (false for units skipped after an error in a
    /// unit they depend on).
    pub analyzed: bool,
}

/// A file waiting for analysis.
#[derive(Debug)]
struct PendingFile {
    source: SourceId,
    library: String,
    ast: ast::DesignFile,
}

/// A set of parsed files, each assigned to a library, ready for analysis.
#[derive(Debug)]
pub struct Design {
    standard: Standard,
    files: Vec<PendingFile>,
    stdlib: bool,
}

impl Design {
    /// An empty design for the given standard. Without the bundled
    /// libraries even `boolean` is unknown; use [`Design::with_stdlib`]
    /// unless the caller supplies its own `std.standard`.
    pub fn new(standard: Standard) -> Self {
        Design {
            standard,
            files: Vec::new(),
            stdlib: false,
        }
    }

    /// A design pre-loaded with the bundled `std` and `ieee` libraries
    /// from [`stdlib`]. The sources are added to `map` under
    /// `<reticle>/...` names so diagnostics inside them (which should not
    /// happen) can be located.
    pub fn with_stdlib(map: &mut SourceMap, standard: Standard, diags: &mut Diagnostics) -> Self {
        let mut d = Design::new(standard);
        for src in stdlib::SOURCES {
            let id = map
                .add(format!("<reticle>/{}/{}", src.library, src.name), src.text)
                .expect("bundled source fits");
            // The bundled sources use VHDL-2008 syntax throughout; they
            // are parsed in 2008 mode whatever the user's standard.
            let ast = parse_source(map, id, Standard::Vhdl2008, diags);
            d.files.push(PendingFile {
                source: id,
                library: src.library.to_owned(),
                ast,
            });
        }
        d.stdlib = true;
        d
    }

    /// The standard the design is analysed against.
    pub fn standard(&self) -> Standard {
        self.standard
    }

    /// Parses `id` under the design's standard and adds it to `library`.
    pub fn add_source(
        &mut self,
        map: &SourceMap,
        id: SourceId,
        library: &str,
        diags: &mut Diagnostics,
    ) {
        let ast = parse_source(map, id, self.standard, diags);
        self.add_file(id, library, ast);
    }

    /// Adds an already parsed file to `library`.
    pub fn add_file(&mut self, source: SourceId, library: &str, ast: ast::DesignFile) {
        self.files.push(PendingFile {
            source,
            library: library.to_owned(),
            ast,
        });
    }

    /// Analyses every unit, in dependency order, and returns the annotated
    /// result. Diagnostics go to `diags`.
    pub fn analyze(self, map: &SourceMap, diags: &mut Diagnostics) -> Analysis {
        let mut interner = Interner::new();
        let mut analysis = Analysis::new_empty(Interner::new());
        let mut files = Vec::new();
        for f in self.files {
            let lib = interner.intern_ci(&f.library);
            files.push(AnalyzedFile {
                source: f.source,
                library: lib,
                ast: f.ast,
            });
        }
        let (units, order) = collect_units(&files, &mut interner, map, diags);
        analysis.interner = interner;
        analysis.files = files;
        analysis.units = units;
        super::check::run(&mut analysis, &order, map, self.standard, diags);
        analysis
    }
}

/// Builds the unit table and its analysis order.
fn collect_units(
    files: &[AnalyzedFile],
    interner: &mut Interner,
    map: &SourceMap,
    diags: &mut Diagnostics,
) -> (Vec<Unit>, Vec<UnitId>) {
    let mut units: Vec<Unit> = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        for (ui, du) in f.ast.units.iter().enumerate() {
            let (kind, primary) = match &du.unit {
                LibraryUnit::Entity(_) => (LibraryUnitKind::Entity, None),
                LibraryUnit::Architecture(a) => (
                    LibraryUnitKind::Architecture,
                    Some(interner.intern_ci(&simple_name(&a.entity))),
                ),
                LibraryUnit::Package(_) => (LibraryUnitKind::Package, None),
                LibraryUnit::PackageBody(b) => (
                    LibraryUnitKind::PackageBody,
                    Some(intern_ident(interner, &b.name)),
                ),
                LibraryUnit::PackageInstantiation(_) => {
                    (LibraryUnitKind::PackageInstantiation, None)
                }
                LibraryUnit::Configuration(_) => (LibraryUnitKind::Configuration, None),
                LibraryUnit::Context(_) => (LibraryUnitKind::Context, None),
            };
            let name = intern_ident(interner, du.unit.name());
            let unit = Unit {
                library: f.library,
                name,
                kind,
                primary,
                file: fi,
                index: ui,
                span: du.unit.name().span,
                decl: None,
                region: None,
                analyzed: false,
            };
            // Duplicate primary units (or architectures of the same name
            // for one entity) in one library.
            let dup = units.iter().find(|u| {
                u.library == unit.library
                    && u.name == unit.name
                    && u.kind.is_primary() == unit.kind.is_primary()
                    && u.primary == unit.primary
                    && (u.kind == unit.kind || (u.kind.is_primary() && unit.kind.is_primary()))
            });
            if let Some(prev) = dup {
                diags.push(
                    Diagnostic::error(format!(
                        "{} `{}` is already declared in library `{}`",
                        unit.kind.as_str(),
                        du.unit.name().name,
                        interner.resolve(unit.library)
                    ))
                    .with_code("V0100")
                    .with_label(unit.span, "redeclared here")
                    .with_secondary(prev.span, "first declared here"),
                );
                continue;
            }
            units.push(unit);
        }
    }

    // Dependencies: indices into `units`.
    let library_names: Vec<Symbol> = {
        let mut v: Vec<Symbol> = units.iter().map(|u| u.library).collect();
        v.sort();
        v.dedup();
        v
    };
    let work = interner.intern_ci("work");
    let find = |units: &[Unit], lib: Symbol, name: Symbol, primary: bool| -> Vec<usize> {
        units
            .iter()
            .enumerate()
            .filter(|(_, u)| u.library == lib && u.name == name && u.kind.is_primary() == primary)
            .map(|(i, _)| i)
            .collect()
    };
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); units.len()];
    for i in 0..units.len() {
        let u = &units[i];
        let file = &files[u.file];
        let du = &file.ast.units[u.index];
        let add = |targets: Vec<usize>, deps: &mut Vec<usize>| {
            for t in targets {
                if t != i && !deps.contains(&t) {
                    deps.push(t);
                }
            }
        };
        let mut d = Vec::new();
        // Secondary units follow their primary.
        if let Some(p) = u.primary {
            add(find(&units, u.library, p, true), &mut d);
        }
        // Context clause: `use lib.pkg...` and `context lib.ctx`.
        let mut names: Vec<(Symbol, Symbol)> = Vec::new();
        for item in &du.context {
            match item {
                ContextItem::Use(uc) => {
                    for n in &uc.names {
                        if let Some(p) = library_prefix(n, interner) {
                            names.push(p);
                        }
                    }
                }
                ContextItem::Context(cr) => {
                    for n in &cr.names {
                        if let Some(p) = library_prefix(n, interner) {
                            names.push(p);
                        }
                    }
                }
                ContextItem::Library(_) => {}
            }
        }
        // Selected names anywhere in the unit's text: `work.pkg.t`,
        // `entity work.sub`. A textual scan is enough for ordering; the
        // checker resolves the names properly later.
        let text = map.file(file.source).text();
        let unit_text = &text[du.span.start as usize..du.span.end as usize];
        for lib in &library_names {
            let lib_name = interner.resolve(*lib).to_owned();
            for name in scan_selected(unit_text, &lib_name) {
                names.push((*lib, interner.intern_ci(&name)));
            }
        }
        for (lib, name) in names {
            let lib = if lib == work { u.library } else { lib };
            add(find(&units, lib, name, true), &mut d);
        }
        deps[i] = d;
    }

    // Kahn's algorithm, always taking the earliest ready unit so
    // independent units keep source order. A dependency cycle (which only
    // the textual scan can produce) is broken by taking the earliest
    // remaining unit.
    let n = units.len();
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let next = (0..n)
            .find(|&i| !done[i] && deps[i].iter().all(|&d| done[d]))
            .or_else(|| (0..n).find(|&i| !done[i]));
        let Some(i) = next else { break };
        done[i] = true;
        order.push(UnitId::from_index(i));
    }
    (units, order)
}

fn intern_ident(interner: &mut Interner, id: &ast::Ident) -> Symbol {
    if id.extended {
        interner.intern(&format!("\\{}\\", id.name))
    } else {
        interner.intern_ci(&id.name)
    }
}

/// The last identifier of a name (`work.pkg` gives `pkg`).
fn simple_name(n: &Name) -> String {
    match n {
        Name::Simple(i) => i.name.clone(),
        Name::Selected {
            suffix: Suffix::Designator(ast::Designator::Ident(i)),
            ..
        } => i.name.clone(),
        _ => String::new(),
    }
}

/// `lib.unit[.rest]` as `(lib, unit)` when the name is selected.
fn library_prefix(n: &Name, interner: &mut Interner) -> Option<(Symbol, Symbol)> {
    let mut parts = Vec::new();
    let mut cur = n;
    loop {
        match cur {
            Name::Selected { prefix, suffix, .. } => {
                parts.push(match suffix {
                    Suffix::Designator(ast::Designator::Ident(i)) => Some(i.clone()),
                    _ => None,
                });
                cur = prefix;
            }
            Name::Simple(i) => {
                parts.push(Some(i.clone()));
                break;
            }
            _ => return None,
        }
    }
    parts.reverse();
    if parts.len() < 2 {
        return None;
    }
    let lib = parts[0].as_ref()?;
    let unit = parts[1].as_ref()?;
    Some((intern_ident(interner, lib), intern_ident(interner, unit)))
}

/// Finds the identifiers following `lib.` in `text` (case-insensitive,
/// at an identifier boundary).
fn scan_selected(text: &str, lib: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let needle = format!("{}.", lib.to_ascii_lowercase());
    let bytes = lower.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(&needle) {
        let at = from + pos;
        from = at + needle.len();
        // Must start at an identifier boundary.
        if at > 0 {
            let prev = bytes[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'.' {
                continue;
            }
        }
        let rest = &lower[from..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if end > 0 && rest.as_bytes()[0].is_ascii_alphabetic() {
            out.push(rest[..end].to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_selected_names() {
        let names = scan_selected(
            "use work.pkg.all; x <= WORK.other.f(1); a.work.no; mywork.no; work .no",
            "work",
        );
        assert_eq!(names, vec!["pkg", "other"]);
    }

    #[test]
    fn orders_units_by_dependency() {
        let mut map = SourceMap::new();
        let id = map
            .add(
                "t.vhd",
                "architecture a of e is begin end;\n\
                 package body p is end;\n\
                 entity e is end;\n\
                 use work.p.all;\n\
                 entity f is end;\n\
                 package p is end;\n",
            )
            .unwrap();
        let mut diags = Diagnostics::new();
        let ast = parse_source(&map, id, Standard::Vhdl2008, &mut diags);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        let mut interner = Interner::new();
        let files = vec![AnalyzedFile {
            source: id,
            library: interner.intern_ci("work"),
            ast,
        }];
        let (units, order) = collect_units(&files, &mut interner, &map, &mut diags);
        assert!(diags.is_empty());
        let names: Vec<String> = order
            .iter()
            .map(|u| {
                let u = &units[u.index()];
                format!("{}:{}", u.kind.as_str(), interner.resolve(u.name))
            })
            .collect();
        assert_eq!(
            names,
            [
                "entity:e",
                "architecture:a",
                "package:p",
                "package body:p",
                "entity:f"
            ]
        );
    }

    #[test]
    fn duplicate_units_are_reported() {
        let mut map = SourceMap::new();
        let id = map
            .add("t.vhd", "entity e is end; entity E is end;")
            .unwrap();
        let mut diags = Diagnostics::new();
        let ast = parse_source(&map, id, Standard::Vhdl2008, &mut diags);
        let mut interner = Interner::new();
        let files = vec![AnalyzedFile {
            source: id,
            library: interner.intern_ci("work"),
            ast,
        }];
        let (units, _) = collect_units(&files, &mut interner, &map, &mut diags);
        assert_eq!(units.len(), 1);
        assert_eq!(diags.error_count(), 1);
    }
}
