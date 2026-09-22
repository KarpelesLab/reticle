//! The per-document index the features answer from.
//!
//! Every feature except formatting needs the same three things: what a
//! document declares, where each of those declarations is used, and which
//! declarations are visible at a given offset. The two frontends reach
//! those facts by very different routes — the Verilog side walks the AST
//! with its own scope stack ([`super::verilog`]), the VHDL side reads the
//! side tables of [`crate::vhdl::sema::Analysis`] ([`super::vhdl`]) — so
//! they meet here, in one language-neutral [`Index`], and the features are
//! written once.
//!
//! The index is deliberately *shallow*. It records what the source says
//! (`wire [7:0] q`, `signal count : std_logic_vector(W-1 downto 0)`), not
//! what elaboration would make of it, because a language server runs on
//! text that is half-written most of the time. Where a fact needs
//! elaboration to be certain — the width of a parameterised vector, say —
//! the index leaves it out rather than guessing, and the feature answers
//! with what it does know.
//!
//! Spans are byte offsets into the one document the index describes;
//! [`super::protocol::LineIndex`] turns them into LSP positions.

use crate::source::Span;

use super::protocol::{CompletionItemKind, SymbolKind};

/// What a declaration is, across both languages.
///
/// The two languages' vocabularies are kept apart where they differ in
/// kind (a Verilog `reg` is not a VHDL `signal`) and merged where they do
/// not, since the features only ever need the distinctions that reach the
/// user: an icon in the outline, a word in a hover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeclClass {
    /// A Verilog `module`, `macromodule` or `primitive`.
    Module,
    /// A SystemVerilog `interface` or `program`.
    Interface,
    /// A VHDL entity.
    Entity,
    /// A VHDL architecture.
    Architecture,
    /// A Verilog or VHDL package.
    Package,
    /// A VHDL component declaration.
    Component,
    /// A port of a module, entity, component or subprogram.
    Port,
    /// A VHDL generic or a Verilog parameter.
    Parameter,
    /// A Verilog net (`wire`, `tri`, ...).
    Net,
    /// A Verilog variable (`reg`, `logic`, `int`, ...).
    Variable,
    /// A VHDL signal.
    Signal,
    /// A VHDL constant or variable, or a Verilog `localparam`.
    Constant,
    /// A type, subtype or `typedef`.
    Type,
    /// A function.
    Function,
    /// A Verilog task or a VHDL procedure.
    Procedure,
    /// A module, entity or component instantiation.
    Instance,
    /// A VHDL process or a Verilog `always` / `initial` block.
    Process,
    /// A named block: a `begin : name`, a generate block or a VHDL block.
    Block,
    /// An enumeration literal.
    EnumLiteral,
    /// A `genvar` or a VHDL generate parameter.
    Genvar,
}

impl DeclClass {
    /// The word used when describing the declaration to the user.
    pub fn describe(self) -> &'static str {
        match self {
            DeclClass::Module => "module",
            DeclClass::Interface => "interface",
            DeclClass::Entity => "entity",
            DeclClass::Architecture => "architecture",
            DeclClass::Package => "package",
            DeclClass::Component => "component",
            DeclClass::Port => "port",
            DeclClass::Parameter => "parameter",
            DeclClass::Net => "net",
            DeclClass::Variable => "variable",
            DeclClass::Signal => "signal",
            DeclClass::Constant => "constant",
            DeclClass::Type => "type",
            DeclClass::Function => "function",
            DeclClass::Procedure => "procedure",
            DeclClass::Instance => "instance",
            DeclClass::Process => "process",
            DeclClass::Block => "block",
            DeclClass::EnumLiteral => "enumeration literal",
            DeclClass::Genvar => "generate parameter",
        }
    }

    /// The LSP symbol kind for the outline.
    pub fn symbol_kind(self) -> SymbolKind {
        match self {
            DeclClass::Module | DeclClass::Entity | DeclClass::Component => SymbolKind::Module,
            DeclClass::Interface | DeclClass::Architecture => SymbolKind::Namespace,
            DeclClass::Package => SymbolKind::Package,
            DeclClass::Port => SymbolKind::Field,
            DeclClass::Parameter | DeclClass::Constant | DeclClass::Genvar => SymbolKind::Constant,
            DeclClass::Net | DeclClass::Variable | DeclClass::Signal => SymbolKind::Variable,
            DeclClass::Type => SymbolKind::Struct,
            DeclClass::Function | DeclClass::Procedure => SymbolKind::Function,
            DeclClass::Instance => SymbolKind::Object,
            DeclClass::Process | DeclClass::Block => SymbolKind::Event,
            DeclClass::EnumLiteral => SymbolKind::EnumMember,
        }
    }

    /// The LSP completion kind.
    pub fn completion_kind(self) -> CompletionItemKind {
        match self {
            DeclClass::Module
            | DeclClass::Entity
            | DeclClass::Interface
            | DeclClass::Component
            | DeclClass::Architecture
            | DeclClass::Package => CompletionItemKind::Module,
            DeclClass::Port => CompletionItemKind::Field,
            DeclClass::Parameter | DeclClass::Constant | DeclClass::Genvar => {
                CompletionItemKind::Constant
            }
            DeclClass::Net | DeclClass::Variable | DeclClass::Signal => {
                CompletionItemKind::Variable
            }
            DeclClass::Type => CompletionItemKind::Struct,
            DeclClass::Function | DeclClass::Procedure => CompletionItemKind::Function,
            DeclClass::Instance | DeclClass::Process | DeclClass::Block => CompletionItemKind::Unit,
            DeclClass::EnumLiteral => CompletionItemKind::EnumMember,
        }
    }

    /// True for the declarations whose names other files can see, and which
    /// a rename therefore cannot be sure to have found every use of.
    pub fn is_file_global(self) -> bool {
        matches!(
            self,
            DeclClass::Module
                | DeclClass::Interface
                | DeclClass::Entity
                | DeclClass::Architecture
                | DeclClass::Package
                | DeclClass::Component
        )
    }
}

/// One port of a module, entity or component, for hovers and for
/// completing a connection list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    /// The port's name, as written.
    pub name: String,
    /// The direction and type, as far as the source says them.
    pub detail: String,
}

/// One declaration the document makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclInfo {
    /// The declared name, as written.
    pub name: String,
    /// What kind of declaration it is.
    pub class: DeclClass,
    /// The type as the source gives it, empty when nothing was written.
    pub detail: String,
    /// The width in bits, when it follows from the source alone.
    pub width: Option<u32>,
    /// The declaration as written, collapsed to one line.
    pub text: String,
    /// The name alone.
    pub name_span: Span,
    /// The whole declaration, which for a module or entity is its body
    /// too, and which therefore doubles as the scope of what it declares.
    pub full_span: Span,
    /// The ports, for a module, entity or component.
    pub ports: Vec<PortInfo>,
    /// The declaration this one sits inside, if any.
    pub parent: Option<usize>,
    /// True when the declaration belongs in the document outline.
    pub in_outline: bool,
}

impl DeclInfo {
    /// A short description: the type if there is one, else the kind.
    pub fn summary(&self) -> String {
        if self.detail.is_empty() {
            self.class.describe().to_string()
        } else {
            self.detail.clone()
        }
    }
}

/// One use of a declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ref {
    /// The name as it appears at the use site.
    pub span: Span,
    /// Which declaration it denotes.
    pub decl: usize,
}

/// A connection list of an instantiation, where the port names of the
/// instantiated unit are what the user wants completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstSite {
    /// The parenthesised connection list, brackets included.
    pub span: Span,
    /// The instantiated module, entity or component, as written.
    pub target: String,
    /// The formals already written, each with the span of the name, so
    /// that the one the cursor is in is not mistaken for a port that has
    /// already been dealt with.
    pub connected: Vec<(String, Span)>,
}

impl InstSite {
    /// True when a port is connected by some formal other than the one the
    /// cursor is inside.
    ///
    /// The formal being typed is already in the tree — `.q` parses as a
    /// connection the moment it is written — so completing it must not
    /// treat it as taken.
    pub fn taken(&self, name: &str, offset: u32, case_insensitive: bool) -> bool {
        self.connected.iter().any(|(formal, span)| {
            !covers(*span, offset)
                && if case_insensitive {
                    formal.eq_ignore_ascii_case(name)
                } else {
                    formal == name
                }
        })
    }
}

/// Everything one document declares and uses.
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// The declarations, in the order they were found.
    pub decls: Vec<DeclInfo>,
    /// Every use of a declaration, sorted by position.
    pub refs: Vec<Ref>,
    /// The instantiation connection lists.
    pub instances: Vec<InstSite>,
    /// True when names are compared without regard to case (VHDL).
    pub case_insensitive: bool,
}

impl Index {
    /// The declaration at `offset`, whether the offset is on the name in
    /// the declaration or on a use of it.
    pub fn decl_at(&self, offset: u32) -> Option<usize> {
        if let Some(r) = self.ref_at(offset) {
            return Some(r.decl);
        }
        self.decls.iter().position(|d| covers(d.name_span, offset))
    }

    /// The use at `offset`, if the offset is on one.
    pub fn ref_at(&self, offset: u32) -> Option<&Ref> {
        self.refs.iter().find(|r| covers(r.span, offset))
    }

    /// The name span at `offset`: the use, or the declaration's own name.
    pub fn name_span_at(&self, offset: u32) -> Option<Span> {
        if let Some(r) = self.ref_at(offset) {
            return Some(r.span);
        }
        self.decls
            .iter()
            .find(|d| covers(d.name_span, offset))
            .map(|d| d.name_span)
    }

    /// Every occurrence of `decl`: its own name and all its uses, sorted
    /// and without duplicates.
    pub fn occurrences(&self, decl: usize) -> Vec<Span> {
        let mut spans: Vec<Span> = self
            .refs
            .iter()
            .filter(|r| r.decl == decl)
            .map(|r| r.span)
            .collect();
        if let Some(d) = self.decls.get(decl) {
            spans.push(d.name_span);
        }
        spans.sort_by_key(|s| (s.start, s.end));
        spans.dedup();
        spans
    }

    /// The innermost declaration whose body contains `offset`.
    pub fn enclosing(&self, offset: u32) -> Option<usize> {
        self.decls
            .iter()
            .enumerate()
            .filter(|(_, d)| covers(d.full_span, offset))
            .min_by_key(|(_, d)| d.full_span.end - d.full_span.start)
            .map(|(i, _)| i)
    }

    /// The declarations visible at `offset`: the ones declared at file
    /// level, and the ones whose enclosing declarations all contain it.
    ///
    /// The result is sorted by name so completion is deterministic.
    pub fn visible(&self, offset: u32) -> Vec<usize> {
        let mut out: Vec<usize> = (0..self.decls.len())
            .filter(|&i| self.is_visible(i, offset))
            .collect();
        out.sort_by(|&a, &b| {
            self.decls[a].name.cmp(&self.decls[b].name).then(
                self.decls[a]
                    .name_span
                    .start
                    .cmp(&self.decls[b].name_span.start),
            )
        });
        out
    }

    /// True when the declaration's scope contains `offset`.
    fn is_visible(&self, decl: usize, offset: u32) -> bool {
        let mut parent = self.decls[decl].parent;
        // A chain longer than the arena cannot exist, but bound the walk
        // anyway so a builder bug cannot hang the server.
        for _ in 0..=self.decls.len() {
            let Some(p) = parent else { return true };
            let Some(d) = self.decls.get(p) else {
                return false;
            };
            if !covers(d.full_span, offset) {
                return false;
            }
            parent = d.parent;
        }
        false
    }

    /// The instantiation connection list containing `offset`, innermost
    /// first.
    pub fn instance_at(&self, offset: u32) -> Option<&InstSite> {
        self.instances
            .iter()
            .filter(|i| covers(i.span, offset))
            .min_by_key(|i| i.span.end - i.span.start)
    }

    /// The declaration named `name` with one of the given classes.
    pub fn find(&self, name: &str, classes: &[DeclClass]) -> Option<usize> {
        self.decls
            .iter()
            .position(|d| classes.contains(&d.class) && self.same_name(&d.name, name))
    }

    /// Compares two names under the document's case rules.
    pub fn same_name(&self, a: &str, b: &str) -> bool {
        if self.case_insensitive {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    }

    /// The outline: the declarations marked for it, as a tree in source
    /// order.
    ///
    /// Returns pairs of `(declaration index, children)`, which the feature
    /// layer turns into [`super::protocol::DocumentSymbol`]s once it has a
    /// line index to convert the spans with.
    pub fn outline(&self) -> Vec<OutlineNode> {
        let mut nodes: Vec<Option<OutlineNode>> = self
            .decls
            .iter()
            .enumerate()
            .map(|(i, d)| {
                d.in_outline.then(|| OutlineNode {
                    decl: i,
                    children: Vec::new(),
                })
            })
            .collect();
        let mut roots = Vec::new();
        // Children before parents, so a node is complete when it is moved
        // into its parent.
        for i in (0..nodes.len()).rev() {
            let Some(node) = nodes[i].take() else {
                continue;
            };
            match self.outline_parent(i) {
                Some(p) => match &mut nodes[p] {
                    Some(parent) => parent.children.push(node),
                    None => roots.push(node),
                },
                None => roots.push(node),
            }
        }
        sort_outline(self, &mut roots);
        roots
    }

    /// The nearest ancestor of `decl` that is itself in the outline.
    fn outline_parent(&self, decl: usize) -> Option<usize> {
        let mut parent = self.decls[decl].parent;
        for _ in 0..=self.decls.len() {
            let p = parent?;
            if self.decls.get(p)?.in_outline {
                return Some(p);
            }
            parent = self.decls[p].parent;
        }
        None
    }
}

/// One node of the outline tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineNode {
    /// The declaration this node stands for.
    pub decl: usize,
    /// Nested declarations, in source order.
    pub children: Vec<OutlineNode>,
}

/// Sorts an outline level (and its children) into source order.
fn sort_outline(index: &Index, nodes: &mut Vec<OutlineNode>) {
    nodes.sort_by_key(|n| {
        let d = &index.decls[n.decl];
        (d.full_span.start, d.name_span.start)
    });
    for node in nodes {
        sort_outline(index, &mut node.children);
    }
}

/// True when `span` covers `offset`, counting the position just past the
/// end so that a cursor at the end of a name still finds it.
fn covers(span: Span, offset: u32) -> bool {
    offset >= span.start && offset <= span.end
}

/// Builds an [`Index`], resolving names against a stack of scopes.
///
/// The Verilog side uses the scopes; the VHDL side, whose name resolution
/// has already been done by [`crate::vhdl::sema`], only uses
/// [`Builder::declare`] and [`Builder::add_ref`].
#[derive(Debug)]
pub struct Builder {
    index: Index,
    scopes: Vec<Scope>,
    stack: Vec<usize>,
    pending: Vec<Pending>,
}

/// One declarative region.
#[derive(Debug)]
struct Scope {
    parent: Option<usize>,
    names: Vec<(String, usize)>,
}

/// A name whose declaration is looked up once the whole file is walked, so
/// that a use before a declaration still resolves.
#[derive(Debug)]
struct Pending {
    span: Span,
    name: String,
    scope: usize,
}

impl Builder {
    /// Starts a builder with one root scope.
    pub fn new(case_insensitive: bool) -> Builder {
        Builder {
            index: Index {
                case_insensitive,
                ..Index::default()
            },
            scopes: vec![Scope {
                parent: None,
                names: Vec::new(),
            }],
            stack: vec![0],
            pending: Vec::new(),
        }
    }

    /// The innermost scope.
    fn current(&self) -> usize {
        *self.stack.last().unwrap_or(&0)
    }

    /// Opens a scope nested in the innermost one.
    pub fn push_scope(&mut self) {
        let parent = Some(self.current());
        self.scopes.push(Scope {
            parent,
            names: Vec::new(),
        });
        let id = self.scopes.len() - 1;
        self.stack.push(id);
    }

    /// Closes the innermost scope.
    pub fn pop_scope(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Records a declaration and makes its name visible in the innermost
    /// scope.
    ///
    /// When the scope already declares that name — which is how Verilog
    /// writes a non-ANSI port, once as `input a;` and once as `wire a;` —
    /// the second declaration becomes a reference to the first, so that
    /// rename and references treat them as one object.
    pub fn declare(&mut self, decl: DeclInfo) -> usize {
        let scope = self.current();
        if let Some(existing) = self.lookup_in(scope, &decl.name) {
            self.index.refs.push(Ref {
                span: decl.name_span,
                decl: existing,
            });
            // Keep the richer of the two descriptions: `input a;` says the
            // direction, `wire [7:0] a;` says the width.
            if self.index.decls[existing].detail.is_empty() && !decl.detail.is_empty() {
                self.index.decls[existing].detail = decl.detail;
                self.index.decls[existing].width = decl.width;
            }
            return existing;
        }
        self.index.decls.push(decl);
        let id = self.index.decls.len() - 1;
        let name = self.index.decls[id].name.clone();
        self.scopes[scope].names.push((name, id));
        id
    }

    /// Records a declaration without making it visible by name, for the
    /// unnamed constructs that only the outline cares about.
    pub fn declare_anonymous(&mut self, decl: DeclInfo) -> usize {
        self.index.decls.push(decl);
        self.index.decls.len() - 1
    }

    /// Records a use of `name` at `span`, to be resolved when the walk is
    /// done.
    pub fn use_name(&mut self, span: Span, name: &str) {
        self.pending.push(Pending {
            span,
            name: name.to_string(),
            scope: self.current(),
        });
    }

    /// Records a use that is already resolved.
    pub fn add_ref(&mut self, span: Span, decl: usize) {
        self.index.refs.push(Ref { span, decl });
    }

    /// Records an instantiation connection list.
    pub fn add_instance(&mut self, site: InstSite) {
        self.index.instances.push(site);
    }

    /// A mutable handle on a declaration already recorded.
    pub fn decl_mut(&mut self, id: usize) -> &mut DeclInfo {
        &mut self.index.decls[id]
    }

    /// The declarations recorded so far.
    pub fn decls(&self) -> &[DeclInfo] {
        &self.index.decls
    }

    /// Resolves the pending names and returns the finished index.
    pub fn finish(mut self) -> Index {
        let pending = std::mem::take(&mut self.pending);
        for use_site in pending {
            let mut scope = Some(use_site.scope);
            while let Some(id) = scope {
                if let Some(decl) = self.lookup_in(id, &use_site.name) {
                    self.index.refs.push(Ref {
                        span: use_site.span,
                        decl,
                    });
                    break;
                }
                scope = self.scopes[id].parent;
            }
        }
        // A use inside an instantiation's connection list names a port of
        // the instantiated unit, which is only known once every module in
        // the file has been declared.
        self.index.refs.sort_by_key(|r| (r.span.start, r.span.end));
        self.index.refs.dedup_by_key(|r| (r.span.start, r.span.end));
        self.index
    }

    /// Looks a name up in one scope.
    fn lookup_in(&self, scope: usize, name: &str) -> Option<usize> {
        self.scopes[scope]
            .names
            .iter()
            .find(|(n, _)| self.index.same_name(n, name))
            .map(|(_, id)| *id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SourceId, SourceMap};

    fn file() -> SourceId {
        let mut map = SourceMap::new();
        map.add("t.v", "x".repeat(200)).unwrap()
    }

    fn decl(file: SourceId, name: &str, at: u32, body: (u32, u32)) -> DeclInfo {
        DeclInfo {
            name: name.to_string(),
            class: DeclClass::Net,
            detail: String::new(),
            width: None,
            text: String::new(),
            name_span: Span::new(file, at, at + 1),
            full_span: Span::new(file, body.0, body.1),
            ports: Vec::new(),
            parent: None,
            in_outline: true,
        }
    }

    #[test]
    fn resolves_uses_after_the_walk() {
        let f = file();
        let mut b = Builder::new(false);
        b.use_name(Span::new(f, 50, 51), "a");
        let a = b.declare(decl(f, "a", 10, (10, 100)));
        let index = b.finish();
        assert_eq!(index.refs.len(), 1);
        assert_eq!(index.refs[0].decl, a);
        assert_eq!(index.decl_at(50), Some(a));
        assert_eq!(index.decl_at(10), Some(a));
        assert_eq!(index.decl_at(150), None);
    }

    #[test]
    fn inner_scopes_shadow_outer_ones() {
        let f = file();
        let mut b = Builder::new(false);
        let outer = b.declare(decl(f, "a", 10, (10, 100)));
        b.push_scope();
        let inner = b.declare(decl(f, "a", 40, (40, 60)));
        b.use_name(Span::new(f, 50, 51), "a");
        b.pop_scope();
        b.use_name(Span::new(f, 80, 81), "a");
        let index = b.finish();
        assert_eq!(index.decl_at(50), Some(inner));
        assert_eq!(index.decl_at(80), Some(outer));
    }

    #[test]
    fn a_redeclaration_becomes_a_reference() {
        let f = file();
        let mut b = Builder::new(false);
        let first = b.declare(DeclInfo {
            class: DeclClass::Port,
            ..decl(f, "a", 10, (10, 20))
        });
        let again = b.declare(DeclInfo {
            detail: "wire [7:0]".to_string(),
            width: Some(8),
            ..decl(f, "a", 30, (30, 40))
        });
        assert_eq!(first, again);
        let index = b.finish();
        assert_eq!(index.decls.len(), 1);
        // The later, richer declaration filled the type in.
        assert_eq!(index.decls[0].detail, "wire [7:0]");
        assert_eq!(index.decls[0].width, Some(8));
        assert_eq!(
            index.occurrences(0),
            vec![index.decls[0].name_span, Span::new(f, 30, 31)]
        );
    }

    #[test]
    fn case_insensitive_lookup_is_opt_in() {
        let f = file();
        let mut b = Builder::new(true);
        let a = b.declare(decl(f, "Count", 10, (10, 100)));
        b.use_name(Span::new(f, 50, 55), "COUNT");
        let index = b.finish();
        assert_eq!(index.refs.len(), 1);
        assert_eq!(index.refs[0].decl, a);
        assert!(index.same_name("aB", "Ab"));

        let mut b = Builder::new(false);
        b.declare(decl(f, "Count", 10, (10, 100)));
        b.use_name(Span::new(f, 50, 55), "COUNT");
        assert!(b.finish().refs.is_empty());
    }

    #[test]
    fn visibility_follows_the_parent_chain() {
        let f = file();
        let mut b = Builder::new(false);
        let module = b.declare(decl(f, "m", 0, (0, 100)));
        let inner = b.declare_anonymous(DeclInfo {
            parent: Some(module),
            ..decl(f, "q", 20, (20, 30))
        });
        let outside = b.declare(decl(f, "other", 120, (120, 190)));
        let index = b.finish();
        // Visible declarations come back sorted by name: `m`, `other`, `q`.
        assert_eq!(index.visible(50), vec![module, outside, inner]);
        assert_eq!(index.visible(150), vec![module, outside]);
        assert_eq!(index.enclosing(25), Some(inner));
        assert_eq!(index.enclosing(50), Some(module));
        assert_eq!(index.enclosing(110), None);
    }

    #[test]
    fn the_outline_nests_and_sorts() {
        let f = file();
        let mut b = Builder::new(false);
        let m = b.declare(decl(f, "m", 0, (0, 100)));
        let late = b.declare_anonymous(DeclInfo {
            parent: Some(m),
            ..decl(f, "z", 60, (60, 70))
        });
        let early = b.declare_anonymous(DeclInfo {
            parent: Some(m),
            in_outline: false,
            ..decl(f, "a", 20, (20, 30))
        });
        let nested = b.declare_anonymous(DeclInfo {
            parent: Some(early),
            ..decl(f, "n", 22, (22, 26))
        });
        let index = b.finish();
        let outline = index.outline();
        assert_eq!(outline.len(), 1);
        assert_eq!(outline[0].decl, m);
        // `early` is not in the outline, so its child is lifted to `m`.
        let kids: Vec<usize> = outline[0].children.iter().map(|n| n.decl).collect();
        assert_eq!(kids, vec![nested, late]);
    }

    #[test]
    fn instance_sites_are_found_innermost_first() {
        let f = file();
        let mut b = Builder::new(false);
        b.add_instance(InstSite {
            span: Span::new(f, 10, 90),
            target: "outer".to_string(),
            connected: Vec::new(),
        });
        b.add_instance(InstSite {
            span: Span::new(f, 40, 60),
            target: "inner".to_string(),
            connected: vec![("clk".to_string(), Span::new(f, 44, 47))],
        });
        let index = b.finish();
        let inner = index.instance_at(50).unwrap();
        assert_eq!(inner.target, "inner");
        assert_eq!(index.instance_at(20).unwrap().target, "outer");
        assert!(index.instance_at(95).is_none());
        // `clk` is taken from elsewhere in the list, but not from inside
        // the formal that spells it.
        assert!(inner.taken("clk", 50, false));
        assert!(!inner.taken("clk", 45, false));
        assert!(inner.taken("CLK", 50, true));
        assert!(!inner.taken("CLK", 50, false));
    }

    #[test]
    fn classes_map_to_lsp_kinds() {
        assert_eq!(DeclClass::Module.symbol_kind(), SymbolKind::Module);
        assert_eq!(DeclClass::Port.completion_kind(), CompletionItemKind::Field);
        assert_eq!(DeclClass::Signal.describe(), "signal");
        assert!(DeclClass::Entity.is_file_global());
        assert!(!DeclClass::Net.is_file_global());
    }
}
