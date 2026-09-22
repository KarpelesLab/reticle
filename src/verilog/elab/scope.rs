//! Symbol tables: scopes, symbols and name resolution.
//!
//! Elaboration keeps one arena of [`Scope`]s for a whole run. Every scope
//! has a parent (the enclosing scope, or nothing for the compilation-unit
//! scope `$unit`), a list of wildcard-imported packages, and an ordered map
//! from names to [`Symbol`]s. The chain is:
//!
//! ```text
//! $unit ─┬─ package scopes            (constants, types, functions)
//!        └─ module scope              (params, nets, functions, instances)
//!             └─ generate block scope (`g_loop[3]`: params, nets, ...)
//!                  └─ function / task inlining scope (locals)
//!                       └─ named begin/end block scope
//! ```
//!
//! `Scopes::lookup` walks the chain, consulting a scope's own names,
//! then its wildcard imports, then its parent (IEEE 1800-2017 §26.3). A
//! generate loop leaves one scope per iteration, recorded in the enclosing
//! scope under the loop's label as `Symbol::GenArray`, so hierarchical
//! names such as `g_loop[3].t` resolve by walking those entries; the same
//! mechanism resolves interface instance members (`bus.addr`) through
//! `Symbol::Iface`. Interface and module *instances* are recorded as
//! `Symbol::Instance` so a reference into another module can be reported
//! precisely rather than as an unknown name.
//!
//! Names are unique inside one scope; declaring a name twice is reported
//! by the caller with the earlier declaration's span, which every symbol
//! records.

use std::collections::HashMap;

use crate::ir::{MemoryId, NetId};
use crate::source::Span;

use super::constant::Value;
use super::types::VType;
use crate::verilog::ast;

/// Index of a scope in [`Scopes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeId(pub u32);

impl ScopeId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

/// A function or task with the scope it was declared in, so its body can
/// be evaluated or inlined with the right names visible.
#[derive(Clone, Copy, Debug)]
pub struct FnRef<'ast> {
    /// The declaration.
    pub def: &'ast ast::Subroutine,
    /// The scope the body's free names resolve in.
    pub home: ScopeId,
}

impl PartialEq for FnRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.def, other.def) && self.home == other.home
    }
}

/// What a name denotes.
#[derive(Clone, Debug, PartialEq)]
pub enum Symbol<'ast> {
    /// A constant: parameter, localparam, enum variant, or a genvar bound
    /// inside a generate loop iteration.
    Const {
        /// The value.
        value: Value,
        /// The declared type; `None` for an untyped parameter, whose type
        /// is that of its value.
        ty: Option<VType>,
    },
    /// A named type from a `typedef` or a type parameter.
    Type(VType),
    /// A net or variable lowered to an IR net.
    Net {
        /// The IR net.
        net: NetId,
        /// The declared type.
        ty: VType,
    },
    /// A one-dimensional unpacked variable lowered to an IR memory.
    Memory {
        /// The IR memory.
        mem: MemoryId,
        /// The declared type (an `Unpacked` over the element type).
        ty: VType,
    },
    /// A function.
    Function(FnRef<'ast>),
    /// A task.
    Task(FnRef<'ast>),
    /// A genvar outside any loop iteration: declared, not bound.
    Genvar,
    /// A named generate block (from `if` / `case` generate or a bare
    /// labelled block).
    GenBlock(ScopeId),
    /// A generate loop: one scope per iteration, keyed by the genvar value.
    GenArray(Vec<(i64, ScopeId)>),
    /// An interface instance or interface port: the scope holding the
    /// bundle's signals, parameters and subroutines.
    Iface {
        /// The interface's scope.
        scope: ScopeId,
        /// The interface (module) name.
        name: String,
        /// The modport selected on an interface port, if any.
        modport: Option<String>,
    },
    /// A module instance; only its existence is recorded.
    Instance {
        /// The instantiated module's name.
        module: String,
    },
    /// A package, for `pkg::name` references written without `import`.
    Package(ScopeId),
}

/// One scope: its names in declaration order, its parent and its imports.
#[derive(Debug)]
pub struct Scope<'ast> {
    /// The enclosing scope.
    pub parent: Option<ScopeId>,
    /// Hierarchical prefix for the IR names of objects declared here
    /// (`g_loop[3].`, `fn$2.`); empty at module level.
    pub prefix: String,
    /// Packages whose every name is visible here (`import pkg::*`), in
    /// import order.
    pub imports: Vec<ScopeId>,
    names: HashMap<String, usize>,
    entries: Vec<(String, Symbol<'ast>, Span)>,
}

impl<'ast> Scope<'ast> {
    /// The symbol declared here under `name`, with its span.
    pub fn get(&self, name: &str) -> Option<(&Symbol<'ast>, Span)> {
        self.names.get(name).map(|&i| {
            let (_, sym, span) = &self.entries[i];
            (sym, *span)
        })
    }

    /// Every name declared here, in declaration order.
    pub fn names(&self) -> impl Iterator<Item = (&str, &Symbol<'ast>)> {
        self.entries.iter().map(|(n, s, _)| (n.as_str(), s))
    }

    /// The span of the declaration of `name`.
    pub fn span_of(&self, name: &str) -> Option<Span> {
        self.get(name).map(|(_, s)| s)
    }
}

/// The scope arena of one elaboration.
#[derive(Debug, Default)]
pub struct Scopes<'ast> {
    scopes: Vec<Scope<'ast>>,
}

impl<'ast> Scopes<'ast> {
    /// Creates an empty arena.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a scope under `parent` with the given IR name prefix.
    pub fn push(&mut self, parent: Option<ScopeId>, prefix: impl Into<String>) -> ScopeId {
        let id = ScopeId(u32::try_from(self.scopes.len()).expect("scope count fits u32"));
        self.scopes.push(Scope {
            parent,
            prefix: prefix.into(),
            imports: Vec::new(),
            names: HashMap::new(),
            entries: Vec::new(),
        });
        id
    }

    /// The scope with the given id.
    pub fn get(&self, id: ScopeId) -> &Scope<'ast> {
        &self.scopes[id.index()]
    }

    /// Mutable access to the scope with the given id.
    pub fn get_mut(&mut self, id: ScopeId) -> &mut Scope<'ast> {
        &mut self.scopes[id.index()]
    }

    /// Declares `name` in `scope`. Returns the span of the earlier
    /// declaration when the name is already taken there (the new symbol
    /// then replaces it, so elaboration can continue).
    pub fn declare(
        &mut self,
        scope: ScopeId,
        name: impl Into<String>,
        sym: Symbol<'ast>,
        span: Span,
    ) -> Option<Span> {
        let name = name.into();
        let s = self.get_mut(scope);
        if let Some(&i) = s.names.get(&name) {
            let old = s.entries[i].2;
            s.entries[i] = (name, sym, span);
            return Some(old);
        }
        s.names.insert(name.clone(), s.entries.len());
        s.entries.push((name, sym, span));
        None
    }

    /// Declares `name` in `scope`, silently replacing an earlier symbol.
    pub fn redeclare(
        &mut self,
        scope: ScopeId,
        name: impl Into<String>,
        sym: Symbol<'ast>,
        span: Span,
    ) {
        let _ = self.declare(scope, name, sym, span);
    }

    /// Adds a wildcard package import to `scope`.
    pub fn import_all(&mut self, scope: ScopeId, package: ScopeId) {
        let s = self.get_mut(scope);
        if !s.imports.contains(&package) {
            s.imports.push(package);
        }
    }

    /// Resolves `name` from `scope` outwards: own names, then wildcard
    /// imports, then the parent chain. Returns the symbol, the span of its
    /// declaration, and the scope it was found in.
    pub fn lookup(&self, scope: ScopeId, name: &str) -> Option<(&Symbol<'ast>, Span, ScopeId)> {
        let mut cur = Some(scope);
        while let Some(id) = cur {
            let s = self.get(id);
            if let Some((sym, span)) = s.get(name) {
                return Some((sym, span, id));
            }
            for &pkg in &s.imports {
                if let Some((sym, span)) = self.get(pkg).get(name) {
                    return Some((sym, span, pkg));
                }
            }
            cur = s.parent;
        }
        None
    }

    /// The IR name of an object called `name` declared in `scope`: the
    /// concatenated prefixes of the scope chain plus the name.
    pub fn qualified(&self, scope: ScopeId, name: &str) -> String {
        let mut parts = Vec::new();
        let mut cur = Some(scope);
        while let Some(id) = cur {
            let s = self.get(id);
            if !s.prefix.is_empty() {
                parts.push(s.prefix.as_str());
            }
            cur = s.parent;
        }
        let mut out = String::new();
        for p in parts.iter().rev() {
            out.push_str(p);
        }
        out.push_str(name);
        out
    }

    /// Every name visible from `scope`, nearest first, for "did you mean"
    /// suggestions. Deterministic: declaration order within each scope.
    pub fn visible_names(&self, scope: ScopeId) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = Some(scope);
        while let Some(id) = cur {
            let s = self.get(id);
            out.extend(s.names().map(|(n, _)| n.to_owned()));
            for &pkg in &s.imports {
                out.extend(self.get(pkg).names().map(|(n, _)| n.to_owned()));
            }
            cur = s.parent;
        }
        out
    }

    /// The closest name to `name` among `candidates` under the edit
    /// distance heuristic used for suggestions: distance at most 2, and
    /// less than half the name's length, and not the name itself.
    pub fn suggest<'a>(
        name: &str,
        candidates: impl IntoIterator<Item = &'a str>,
    ) -> Option<&'a str> {
        let mut best: Option<(usize, &'a str)> = None;
        for c in candidates {
            if c == name {
                continue;
            }
            let d = edit_distance(name, c);
            let limit = (name.chars().count() / 2).clamp(1, 2);
            if d <= limit && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, c));
            }
        }
        best.map(|(_, c)| c)
    }
}

/// Levenshtein distance between two strings, by character.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SourceId, SourceMap};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id: SourceId = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    #[test]
    fn lookup_walks_parents_and_imports() {
        let mut scopes = Scopes::new();
        let unit = scopes.push(None, "");
        let pkg = scopes.push(Some(unit), "");
        let module = scopes.push(Some(unit), "");
        let block = scopes.push(Some(module), "blk.");
        scopes.declare(pkg, "P", Symbol::Genvar, span());
        scopes.declare(unit, "U", Symbol::Genvar, span());
        scopes.declare(module, "M", Symbol::Genvar, span());
        assert!(scopes.lookup(block, "P").is_none());
        scopes.import_all(module, pkg);
        scopes.import_all(module, pkg);
        assert_eq!(scopes.get(module).imports.len(), 1);
        assert_eq!(scopes.lookup(block, "P").map(|r| r.2), Some(pkg));
        assert_eq!(scopes.lookup(block, "U").map(|r| r.2), Some(unit));
        assert_eq!(scopes.lookup(block, "M").map(|r| r.2), Some(module));
        assert!(scopes.lookup(block, "X").is_none());
        assert_eq!(scopes.qualified(block, "x"), "blk.x");
        assert_eq!(scopes.qualified(module, "x"), "x");
        let dup = scopes.declare(module, "M", Symbol::Genvar, span());
        assert!(dup.is_some());
        scopes.redeclare(module, "M", Symbol::Genvar, span());
        assert_eq!(scopes.get(module).span_of("M"), Some(span()));
        let names = scopes.visible_names(block);
        assert_eq!(names, ["M", "P", "U"]);
    }

    #[test]
    fn suggestions_use_edit_distance() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "ab"), 2);
        assert_eq!(
            Scopes::suggest("rst_m", ["clk", "rst_n", "rst_m"]),
            Some("rst_n")
        );
        assert_eq!(Scopes::suggest("a", ["b"]), Some("b"));
        assert_eq!(Scopes::suggest("clock", ["data", "enable"]), None);
    }
}
