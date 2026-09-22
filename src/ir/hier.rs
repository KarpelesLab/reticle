//! Hierarchy passes: flattening, unique-ification and path queries.
//!
//! A lowered design is a tree of modules. Most back ends want that tree
//! collapsed into one module ([`Design::flatten`]), some passes want the
//! opposite ([`Design::uniquify`], which gives every instantiation its own
//! copy of a module so per-instance constraints, attributes and formal
//! properties can be attached without affecting the other instantiations),
//! and both want to ask where an object lives ([`Design::hier_paths`],
//! [`Design::resolve_path`], [`Design::instance_count`]).
//!
//! # Flattening
//!
//! [`Design::flatten`] rewrites one module so that it contains everything
//! its sub-tree contained. Each inlined instance contributes a copy of the
//! child's nets, memories, expressions, assigns, processes and cells, named
//! `instance<sep>object` (the separator is a [`FlattenOptions`] knob, `.` by
//! default). Copies keep the child's [`Span`]s, so a diagnostic raised on
//! flattened logic still points into the source the child was written in;
//! with [`FlattenOptions::annotate_paths`] every copied object also gets an
//! [`ORIGIN`] attribute holding the instance path.
//!
//! Port connections become:
//!
//! | Port    | Connection                | Result                                  |
//! |---------|---------------------------|-----------------------------------------|
//! | `In`    | a plain net of equal type | the two nets are merged, no assign       |
//! | `In`    | anything else             | `assign <child port net> = <expression>` |
//! | `Out`   | net, slice or concat      | `assign <connection> = <child port net>` |
//! | `InOut` | a plain net of equal type | the two nets are merged                  |
//!
//! Merging keeps the parent's net, so a chain of direct connections through
//! several levels collapses onto the top-level net and adds nothing. An
//! output connected to an expression that is not an lvalue, and an inout
//! that is not connected to a plain net, are errors: flattening cannot
//! express them.
//!
//! Instances are left in place, under their hierarchical name, when they
//! are unresolved (a black box the target flow supplies), when their target
//! module is a [`Module::blackbox`], when the instance or the target module
//! carries the [`KEEP_HIERARCHY`] attribute and
//! [`FlattenOptions::keep_hierarchy_attr`] is set, or when
//! [`FlattenOptions::max_depth`] cuts the recursion off.
//!
//! # Unique-ification
//!
//! [`Design::uniquify`] gives every instantiation of a multiply
//! instantiated module its own module, named `<name>$1`, `<name>$2`, ... and
//! carrying [`UNIQUIFIED_FROM`] with the original name. The first copy
//! reuses the original module, so ids stay valid and nothing is left
//! dangling. [`Design::dedup`] is the inverse: it merges modules whose text
//! rendering is identical once their name and [`UNIQUIFIED_FROM`] attribute
//! are ignored, and renames a lone survivor back to the name it was
//! uniquified from, so `uniquify` followed by `dedup` reproduces the design
//! it started from. [`Design::remove_unused_modules`] drops what the top no
//! longer reaches.
//!
//! # Diagnostics
//!
//! | Code    | Meaning                                                            |
//! |---------|--------------------------------------------------------------------|
//! | `I0030` | The module to flatten, or an instance target, is not in the design |
//! | `I0031` | The hierarchy is recursive and cannot be flattened                 |
//! | `I0032` | An output port is connected to something that cannot be driven     |
//! | `I0033` | An inout port is not connected to a plain net of the same type     |
//!
//! Flattening assumes the design passes [`validate`](super::validate);
//! it reports the problems it cannot work around rather than checking the
//! design again.

use std::collections::{BTreeMap, HashSet};

use super::Name;
use super::arena::{Arena, Id};
use super::attr::Attrs;
use super::cell::CellKind;
use super::design::{
    Assign, Design, Instance, InstanceId, MemoryId, Module, ModuleId, ModuleRef, NetId, PortDir,
};
use super::expr::{Expr, ExprId, ExprKind};
use super::process::{Lvalue, StmtKind};
use crate::diag::{Diagnostic, Diagnostics};
use crate::source::Span;

/// Attribute that keeps an instance, or every instance of a module, out of
/// [`Design::flatten`].
pub const KEEP_HIERARCHY: &str = "keep_hierarchy";

/// Attribute holding the instance path an object came from, written by
/// [`FlattenOptions::annotate_paths`].
pub const ORIGIN: &str = "origin";

/// Attribute holding the name a module was copied from, written by
/// [`Design::uniquify`].
pub const UNIQUIFIED_FROM: &str = "uniquified_from";

/// A module referenced here is not part of the design.
const MISSING: &str = "I0030";
/// The hierarchy is recursive.
const RECURSIVE: &str = "I0031";
/// An output port connection cannot be turned into an assignment target.
const NOT_LVALUE: &str = "I0032";
/// An inout port connection cannot be merged.
const BAD_INOUT: &str = "I0033";

/// Knobs for [`Design::flatten`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlattenOptions {
    /// Respect the [`KEEP_HIERARCHY`] attribute on instances and modules.
    pub keep_hierarchy_attr: bool,
    /// Placed between an instance path and the name of a copied object.
    pub separator: String,
    /// Deepest instance level to inline: `Some(0)` inlines nothing,
    /// `Some(1)` only the top's own instances. `None` means no limit.
    pub max_depth: Option<u32>,
    /// Record the instance path of every copied object in an [`ORIGIN`]
    /// attribute.
    pub annotate_paths: bool,
}

impl Default for FlattenOptions {
    /// Honours [`KEEP_HIERARCHY`], separates with `.`, inlines the whole
    /// hierarchy and writes no [`ORIGIN`] attributes.
    fn default() -> Self {
        FlattenOptions {
            keep_hierarchy_attr: true,
            separator: ".".to_owned(),
            max_depth: None,
            annotate_paths: false,
        }
    }
}

/// What [`Design::flatten`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlattenReport {
    /// How many instances of each module were inlined, by module name, in
    /// name order.
    pub inlined: Vec<(Name, usize)>,
    /// Total number of instances inlined.
    pub inlined_total: usize,
    /// Instances left in place: black boxes, [`KEEP_HIERARCHY`] and
    /// whatever [`FlattenOptions::max_depth`] cut off.
    pub kept: usize,
    /// The deepest instance level inlined; zero when nothing was.
    pub depth: u32,
    /// Nets in the flattened module.
    pub nets: usize,
    /// Cells in the flattened module.
    pub cells: usize,
    /// Processes in the flattened module.
    pub processes: usize,
    /// Continuous assignments in the flattened module.
    pub assigns: usize,
}

impl FlattenReport {
    /// How many instances of the named module were inlined.
    pub fn inlined_count(&self, module: &str) -> usize {
        self.inlined
            .iter()
            .find(|(n, _)| n.as_str() == module)
            .map_or(0, |(_, c)| *c)
    }
}

/// What [`Design::uniquify`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UniquifyReport {
    /// One entry per module that was copied: the name it had, and the names
    /// of the copies in instantiation order. The first copy is the original
    /// module renamed, unless the module is the design's top.
    pub split: Vec<(Name, Vec<Name>)>,
}

impl UniquifyReport {
    /// Total number of module copies created, the original ones included.
    pub fn copies(&self) -> usize {
        self.split.iter().map(|(_, c)| c.len()).sum()
    }
}

/// What [`Design::dedup`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DedupReport {
    /// One entry per group of identical modules: the name of the module
    /// that survived and the names of those folded into it.
    pub merged: Vec<(Name, Vec<Name>)>,
}

impl DedupReport {
    /// Total number of modules removed.
    pub fn removed(&self) -> usize {
        self.merged.iter().map(|(_, m)| m.len()).sum()
    }
}

// ---------------------------------------------------------------------------
// Flattening
// ---------------------------------------------------------------------------

/// An instance waiting to be inlined or kept.
struct Pending {
    /// The instance itself; its connections already point into the
    /// flattened module's expression arena.
    inst: Instance,
    /// Hierarchical path of the instance, separator included.
    path: String,
    /// Instance level, counting the top's own instances as one.
    depth: u32,
}

/// The names already taken in one namespace of the flattened module.
#[derive(Default)]
struct NameSet(HashSet<String>);

impl NameSet {
    /// A set holding every name of `names`.
    fn of<'a>(names: impl Iterator<Item = &'a str>) -> Self {
        NameSet(names.map(str::to_owned).collect())
    }

    /// Reserves `base`, or the first free `base$2`, `base$3`, ... when it is
    /// taken, and returns the reserved name.
    fn unique(&mut self, base: String) -> Name {
        if self.0.insert(base.clone()) {
            return Name::new(base);
        }
        let mut n = 2u32;
        loop {
            let candidate = format!("{base}${n}");
            if self.0.insert(candidate.clone()) {
                return Name::new(candidate);
            }
            n += 1;
        }
    }
}

/// Every namespace of the module being built.
#[derive(Default)]
struct Names {
    nets: NameSet,
    memories: NameSet,
    instances: NameSet,
    cells: NameSet,
    processes: NameSet,
}

impl Names {
    /// The names already used by `module`.
    fn of(module: &Module) -> Self {
        Names {
            nets: NameSet::of(module.nets.values().map(|n| n.name.as_str())),
            memories: NameSet::of(module.memories.values().map(|m| m.name.as_str())),
            instances: NameSet::of(module.instances.values().map(|i| i.name.as_str())),
            cells: NameSet::of(module.cells.values().map(|c| c.name.as_str())),
            processes: NameSet::of(
                module
                    .processes
                    .values()
                    .filter_map(|p| p.name.as_ref())
                    .map(Name::as_str),
            ),
        }
    }
}

/// The state of one [`Design::flatten`] run.
struct Flattener<'a> {
    design: &'a Design,
    options: &'a FlattenOptions,
    flat: Module,
    names: Names,
    diags: Diagnostics,
    counts: BTreeMap<String, usize>,
    report: FlattenReport,
}

impl<'a> Flattener<'a> {
    /// Starts from a copy of `top` with its instances removed; they go
    /// through the work list like every other instance.
    fn new(design: &'a Design, top: ModuleId, options: &'a FlattenOptions) -> Self {
        let mut flat = design.modules[top].clone();
        flat.instances = Arena::new();
        let names = Names::of(&flat);
        Flattener {
            design,
            options,
            flat,
            names,
            diags: Diagnostics::new(),
            counts: BTreeMap::new(),
            report: FlattenReport::default(),
        }
    }

    /// Inlines the sub-tree of `top` depth first, in declaration order.
    fn run(&mut self, top: ModuleId) {
        let design = self.design;
        let mut stack: Vec<Pending> = design.modules[top]
            .instances
            .values()
            .rev()
            .map(|inst| Pending {
                path: inst.name.as_str().to_owned(),
                inst: inst.clone(),
                depth: 1,
            })
            .collect();
        while let Some(pending) = stack.pop() {
            let target = self.target_of(&pending);
            if let Some(id) = target
                && !self.keeps(id, &pending)
            {
                let children = self.inline(id, &pending);
                self.report.inlined_total += 1;
                self.report.depth = self.report.depth.max(pending.depth);
                *self
                    .counts
                    .entry(design.modules[id].name.as_str().to_owned())
                    .or_default() += 1;
                stack.extend(children.into_iter().rev());
            } else {
                self.keep(pending);
            }
        }
    }

    /// The module an instance targets, or `None` when it is unresolved.
    /// [`Design::flatten`] has already reported every dangling reference,
    /// so a missing target simply keeps the instance.
    fn target_of(&self, pending: &Pending) -> Option<ModuleId> {
        let id = pending.inst.module.id()?;
        self.design.modules.contains(id).then_some(id)
    }

    /// True when the instance must stay a boundary.
    fn keeps(&self, target: ModuleId, pending: &Pending) -> bool {
        let child = &self.design.modules[target];
        child.blackbox
            || self
                .options
                .max_depth
                .is_some_and(|max| pending.depth > max)
            || (self.options.keep_hierarchy_attr
                && (pending.inst.attrs.is_set(KEEP_HIERARCHY)
                    || child.attrs.is_set(KEEP_HIERARCHY)))
    }

    /// Moves an instance into the flattened module under its hierarchical
    /// name.
    fn keep(&mut self, pending: Pending) {
        let Pending { mut inst, path, .. } = pending;
        inst.name = self.names.instances.unique(path.clone());
        if self.options.annotate_paths {
            inst.attrs.set(ORIGIN, path.as_str());
        }
        self.flat.instances.push(inst);
        self.report.kept += 1;
    }

    /// Copies the contents of `target` into the flattened module and
    /// returns the instances the copy brought with it.
    fn inline(&mut self, target: ModuleId, pending: &Pending) -> Vec<Pending> {
        let design = self.design;
        let child = &design.modules[target];
        let inst = &pending.inst;
        let path = pending.path.as_str();
        let sep = self.options.separator.clone();

        // Ports connected to a plain net of the same type share that net
        // with the parent instead of getting a copy of their own.
        let mut net_map: Vec<Option<NetId>> = vec![None; child.nets.len()];
        let mut merged = vec![false; child.ports.len()];
        for (i, port) in child.ports.iter().enumerate() {
            let Some(conn) = inst.connection(port.name.as_str()) else {
                continue;
            };
            let parent = self.flat.exprs.get(conn).and_then(Expr::as_net);
            let same_type = match (
                parent.and_then(|n| self.flat.nets.get(n)),
                child.nets.get(port.net),
            ) {
                (Some(p), Some(c)) => p.ty == c.ty,
                _ => false,
            };
            let free = net_map.get(port.net.index()).is_some_and(Option::is_none);
            if matches!(port.dir, PortDir::In | PortDir::InOut) && same_type && free {
                net_map[port.net.index()] = parent;
                merged[i] = true;
            } else if port.dir == PortDir::InOut {
                self.diags.push(
                    Diagnostic::error(format!(
                        "instance `{path}` connects inout port `{}` of module `{}` to something \
                         that is not a net of the port's type",
                        port.name, child.name
                    ))
                    .with_code(BAD_INOUT)
                    .with_span(inst.span)
                    .with_note("an inout port must share a net with its connection to be inlined"),
                );
            }
        }

        // Everything else is copied under the instance path.
        for (id, net) in child.nets.iter() {
            if net_map[id.index()].is_some() {
                continue;
            }
            let mut copy = net.clone();
            copy.name = self.names.nets.unique(format!("{path}{sep}{}", net.name));
            self.annotate(&mut copy.attrs, path);
            net_map[id.index()] = Some(self.flat.nets.push(copy));
        }
        let mem_base = self.flat.memories.len();
        for (_, mem) in child.memories.iter() {
            let mut copy = mem.clone();
            copy.name = self
                .names
                .memories
                .unique(format!("{path}{sep}{}", mem.name));
            self.annotate(&mut copy.attrs, path);
            self.flat.memories.push(copy);
        }

        // Renumber a private copy of the child into the flattened module's
        // id space, then move its contents over. Expressions are copied
        // wholesale, so a child id maps to `expr_base + id`.
        let expr_base = self.flat.exprs.len();
        let mut copy = child.clone();
        copy.map_nets(|old| net_map.get(old.index()).copied().flatten().unwrap_or(old));
        copy.map_exprs(|old| ExprId::from_index(expr_base + old.index()));
        if mem_base > 0 {
            remap_memories(&mut copy, mem_base);
        }
        for expr in copy.exprs.values() {
            self.flat.exprs.push(expr.clone());
        }
        for assign in &copy.assigns {
            let mut assign = assign.clone();
            self.annotate(&mut assign.attrs, path);
            self.flat.assigns.push(assign);
        }
        for process in copy.processes.values() {
            let mut process = process.clone();
            if let Some(name) = &process.name {
                process.name = Some(self.names.processes.unique(format!("{path}{sep}{name}")));
            }
            self.annotate(&mut process.attrs, path);
            self.flat.processes.push(process);
        }
        for cell in copy.cells.values() {
            let mut cell = cell.clone();
            cell.name = self.names.cells.unique(format!("{path}{sep}{}", cell.name));
            self.annotate(&mut cell.attrs, path);
            self.flat.cells.push(cell);
        }

        // Connections that are not merged become continuous assignments.
        for (i, port) in child.ports.iter().enumerate() {
            if merged[i] {
                continue;
            }
            let Some(conn) = inst.connection(port.name.as_str()) else {
                continue;
            };
            let Some(net) = net_map.get(port.net.index()).copied().flatten() else {
                continue;
            };
            match port.dir {
                PortDir::In => {
                    let assign = self.new_assign(Lvalue::Net(net), conn, inst.span, path);
                    self.flat.assigns.push(assign);
                }
                PortDir::Out => match expr_to_lvalue(&self.flat, conn) {
                    Some(lvalue) => {
                        let ty = self.flat.nets[net].ty.clone();
                        let value =
                            self.flat
                                .exprs
                                .push(Expr::new(ExprKind::Net(net), ty, port.span));
                        let assign = self.new_assign(lvalue, value, inst.span, path);
                        self.flat.assigns.push(assign);
                    }
                    None => self.diags.push(
                        Diagnostic::error(format!(
                            "instance `{path}` connects output port `{}` of module `{}` to an \
                             expression that cannot be driven",
                            port.name, child.name
                        ))
                        .with_code(NOT_LVALUE)
                        .with_span(inst.span)
                        .with_note(
                            "an output must be connected to a net, a slice of a net or a \
                             concatenation of those",
                        ),
                    ),
                },
                // Reported above when the merge was impossible.
                PortDir::InOut => {}
            }
        }

        copy.instances
            .values()
            .map(|child_inst| Pending {
                path: format!("{path}{sep}{}", child_inst.name),
                inst: child_inst.clone(),
                depth: pending.depth + 1,
            })
            .collect()
    }

    /// Records the instance path when [`FlattenOptions::annotate_paths`] is
    /// on.
    fn annotate(&self, attrs: &mut Attrs, path: &str) {
        if self.options.annotate_paths {
            attrs.set(ORIGIN, path);
        }
    }

    /// A connection assignment, annotated like the objects around it.
    fn new_assign(&self, target: Lvalue, value: ExprId, span: Span, path: &str) -> Assign {
        let mut attrs = Attrs::new();
        self.annotate(&mut attrs, path);
        Assign {
            target,
            value,
            delay: None,
            attrs,
            span,
        }
    }

    /// The flattened module, the report and everything reported on the way.
    fn finish(mut self) -> (Module, FlattenReport, Diagnostics) {
        self.flat.gc_exprs();
        self.report.nets = self.flat.nets.len();
        self.report.cells = self.flat.cells.len();
        self.report.processes = self.flat.processes.len();
        self.report.assigns = self.flat.assigns.len();
        self.report.inlined = self
            .counts
            .into_iter()
            .map(|(name, count)| (Name::new(name), count))
            .collect();
        (self.flat, self.report, self.diags)
    }
}

/// The lvalue an instance connection denotes, if it denotes one: a net, a
/// constant slice of one, or a concatenation of those. Nested slices are
/// composed, so `%a[7:4][1:0]` becomes `%a[5:4]`.
fn expr_to_lvalue(module: &Module, id: ExprId) -> Option<Lvalue> {
    match &module.exprs.get(id)?.kind {
        ExprKind::Net(net) => Some(Lvalue::Net(*net)),
        ExprKind::Slice { base, hi, lo } => match expr_to_lvalue(module, *base)? {
            Lvalue::Net(net) => Some(Lvalue::Slice {
                net,
                hi: *hi,
                lo: *lo,
            }),
            Lvalue::Slice {
                net, lo: base_lo, ..
            } => Some(Lvalue::Slice {
                net,
                hi: base_lo.checked_add(*hi)?,
                lo: base_lo.checked_add(*lo)?,
            }),
            _ => None,
        },
        ExprKind::Concat(parts) => parts
            .iter()
            .map(|part| expr_to_lvalue(module, *part))
            .collect::<Option<Vec<_>>>()
            .map(Lvalue::Concat),
        _ => None,
    }
}

/// Shifts every memory id in `module` by `base`, the way [`Module::map_nets`]
/// shifts net ids.
fn remap_memories(module: &mut Module, base: usize) {
    let shift = |id: &mut MemoryId| *id = MemoryId::from_index(base + id.index());
    for (_, expr) in module.exprs.iter_mut() {
        if let ExprKind::MemRead { mem, .. } = &mut expr.kind {
            shift(mem);
        }
    }
    for assign in &mut module.assigns {
        lvalue_memories(&mut assign.target, &shift);
    }
    module.for_each_stmt_mut(|stmt| match &mut stmt.kind {
        StmtKind::Assign { target, .. } => lvalue_memories(target, &shift),
        StmtKind::For { init, step, .. } => {
            for (lvalue, _) in init.iter_mut().chain(step.iter_mut()) {
                lvalue_memories(lvalue, &shift);
            }
        }
        StmtKind::MemWrite { mem, .. } | StmtKind::MemFile { mem, .. } => shift(mem),
        _ => {}
    });
    for (_, cell) in module.cells.iter_mut() {
        match &mut cell.kind {
            CellKind::MemRdPort { mem, .. } | CellKind::MemWrPort { mem, .. } => shift(mem),
            _ => {}
        }
    }
}

/// Applies `shift` to every memory id an lvalue holds.
fn lvalue_memories(lvalue: &mut Lvalue, shift: &impl Fn(&mut MemoryId)) {
    match lvalue {
        Lvalue::MemElem { mem, .. } => shift(mem),
        Lvalue::Concat(parts) => parts.iter_mut().for_each(|p| lvalue_memories(p, shift)),
        Lvalue::Net(_) | Lvalue::Slice { .. } | Lvalue::Index { .. } => {}
    }
}

// ---------------------------------------------------------------------------
// The design-level entry points
// ---------------------------------------------------------------------------

impl Design {
    /// Inlines the whole sub-tree of `top` into `top`.
    ///
    /// See the [module docs](self) for what happens to ports, names, spans
    /// and instances that stay boundaries. The design is left untouched
    /// when anything is reported; other modules are never modified, so
    /// [`Design::remove_unused_modules`] is the usual next call.
    ///
    /// # Errors
    ///
    /// Returns the diagnostics of every connection that cannot be inlined
    /// (`I0032`, `I0033`), of a missing module (`I0030`) and of a recursive
    /// hierarchy (`I0031`).
    pub fn flatten(
        &mut self,
        top: ModuleId,
        options: &FlattenOptions,
    ) -> Result<FlattenReport, Diagnostics> {
        let mut diags = Diagnostics::new();
        if !self.modules.contains(top) {
            diags.push(
                Diagnostic::error(format!("cannot flatten: module {top} is not in the design"))
                    .with_code(MISSING),
            );
            return Err(diags);
        }
        // Every walk below indexes modules by id, so dangling references
        // are reported before anything else looks at the tree.
        for (_, module) in self.modules.iter() {
            for (_, inst) in module.instances.iter() {
                if let ModuleRef::Resolved(id) = inst.module
                    && !self.modules.contains(id)
                {
                    diags.push(
                        Diagnostic::error(format!(
                            "instance `{}` of module `{}` refers to module {id}, which is not in \
                             the design",
                            inst.name, module.name
                        ))
                        .with_code(MISSING)
                        .with_span(inst.span),
                    );
                }
            }
        }
        if diags.has_errors() {
            return Err(diags);
        }
        if let Err(cycle) = self.topological_order() {
            let module = &self.modules[cycle];
            diags.push(
                Diagnostic::error(format!(
                    "cannot flatten: module `{}` is part of a recursive hierarchy",
                    module.name
                ))
                .with_code(RECURSIVE)
                .with_span(module.span),
            );
            return Err(diags);
        }
        let (module, report, diags) = {
            let mut flattener = Flattener::new(self, top, options);
            flattener.run(top);
            flattener.finish()
        };
        if diags.has_errors() {
            return Err(diags);
        }
        self.modules[top] = module;
        Ok(report)
    }

    /// Gives every instantiation of a multiply instantiated module its own
    /// copy, named `<name>$1`, `<name>$2`, ... and carrying
    /// [`UNIQUIFIED_FROM`].
    ///
    /// The first copy is the original module renamed, so existing
    /// [`ModuleId`]s stay valid and no module is left without instances;
    /// the design's top is never renamed and keeps its identity. Modules
    /// are visited from the top down, so copying a module and then its
    /// children leaves every instantiation with a private sub-tree. A
    /// recursive hierarchy is left alone.
    pub fn uniquify(&mut self) -> UniquifyReport {
        let mut report = UniquifyReport::default();
        if !self.refs_resolve() {
            return report;
        }
        let Ok(order) = self.topological_order() else {
            return report;
        };
        let mut used: HashSet<String> = self
            .modules
            .values()
            .map(|m| m.name.as_str().to_owned())
            .collect();
        // Leaves last: a module's instance count is final once every module
        // above it has been split.
        for module in order.into_iter().rev() {
            let sites = self.instances_of(module);
            if sites.len() < 2 {
                continue;
            }
            let original = self.modules[module].name.clone();
            let reuse_original = self.top != Some(module);
            let mut names = Vec::with_capacity(sites.len());
            for (i, (parent, inst)) in sites.iter().enumerate() {
                let name = unique_module_name(&mut used, &original, i + 1);
                let target = if i == 0 && reuse_original {
                    self.modules[module].name = name.clone();
                    self.modules[module]
                        .attrs
                        .set(UNIQUIFIED_FROM, original.as_str());
                    module
                } else {
                    let mut copy = self.modules[module].clone();
                    copy.name = name.clone();
                    copy.attrs.set(UNIQUIFIED_FROM, original.as_str());
                    self.add_module(copy)
                };
                self.modules[*parent].instances[*inst].module = ModuleRef::Resolved(target);
                names.push(name);
            }
            report.split.push((original, names));
        }
        report
    }

    /// Merges modules that render identically in the text format once their
    /// name and [`UNIQUIFIED_FROM`] attribute are ignored.
    ///
    /// This is the inverse of [`Design::uniquify`]: identical copies are
    /// folded back together, instances are repointed at the survivor, the
    /// duplicates are removed from the design, and a survivor that is the
    /// only module left of its [`UNIQUIFIED_FROM`] group takes its original
    /// name back. Merging is iterated to a fixed point, so identical
    /// parents are merged once their children have been.
    pub fn dedup(&mut self) -> DedupReport {
        let mut report = DedupReport::default();
        while self.dedup_round(&mut report) {}
        self.restore_uniquified_names();
        report
    }

    /// One merging pass; true when anything was merged.
    fn dedup_round(&mut self, report: &mut DedupReport) -> bool {
        let keys: Vec<String> = self.modules.values().map(dedup_key).collect();
        let count = self.modules.len();
        let mut group: Vec<Option<usize>> = vec![None; count];
        let mut merged = false;
        for i in 0..count {
            if group[i].is_some() {
                continue;
            }
            let members: Vec<usize> = (i..count)
                .filter(|j| group[*j].is_none() && keys[*j] == keys[i])
                .collect();
            if members.len() < 2 {
                continue;
            }
            // The design's top always survives; otherwise the lowest id
            // does, so the result does not depend on iteration order.
            let keeper = members
                .iter()
                .copied()
                .find(|m| self.top == Some(ModuleId::from_index(*m)))
                .unwrap_or(members[0]);
            let mut names = Vec::new();
            for member in members {
                group[member] = Some(keeper);
                if member != keeper {
                    names.push(self.modules[ModuleId::from_index(member)].name.clone());
                }
            }
            report.merged.push((
                self.modules[ModuleId::from_index(keeper)].name.clone(),
                names,
            ));
            merged = true;
        }
        if !merged {
            return false;
        }
        let keep: Vec<bool> = (0..count)
            .map(|i| group[i].is_none_or(|k| k == i))
            .collect();
        let repoint: Vec<ModuleId> = (0..count)
            .map(|i| ModuleId::from_index(group[i].unwrap_or(i)))
            .collect();
        for (_, module) in self.modules.iter_mut() {
            for (_, inst) in module.instances.iter_mut() {
                if let ModuleRef::Resolved(id) = inst.module
                    && let Some(new) = repoint.get(id.index())
                {
                    inst.module = ModuleRef::Resolved(*new);
                }
            }
        }
        self.retain_modules(&keep);
        true
    }

    /// Renames a module back to the name it was uniquified from when it is
    /// the only one left of its group and the name is free.
    fn restore_uniquified_names(&mut self) {
        let mut groups: BTreeMap<String, usize> = BTreeMap::new();
        for module in self.modules.values() {
            if let Some(from) = module.attrs.get(UNIQUIFIED_FROM).and_then(|v| v.as_str()) {
                *groups.entry(from.to_owned()).or_default() += 1;
            }
        }
        let taken: HashSet<String> = self
            .modules
            .values()
            .map(|m| m.name.as_str().to_owned())
            .collect();
        for (_, module) in self.modules.iter_mut() {
            let Some(from) = module
                .attrs
                .get(UNIQUIFIED_FROM)
                .and_then(|v| v.as_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if groups.get(&from) == Some(&1) && !taken.contains(&from) {
                module.name = Name::new(from);
                module.attrs.remove(UNIQUIFIED_FROM);
            }
        }
    }

    /// Removes every module the hierarchy of `top` does not reach and
    /// returns how many were removed.
    pub fn remove_unused_modules(&mut self, top: ModuleId) -> usize {
        if !self.modules.contains(top) {
            return 0;
        }
        let mut keep = vec![false; self.modules.len()];
        keep[top.index()] = true;
        let mut stack = vec![top];
        while let Some(module) = stack.pop() {
            for child in self.children(module) {
                if let Some(mark) = keep.get_mut(child.index())
                    && !*mark
                {
                    *mark = true;
                    stack.push(child);
                }
            }
        }
        let before = self.modules.len();
        self.retain_modules(&keep);
        before - self.modules.len()
    }

    /// Drops the modules `keep` does not mark and renumbers every
    /// [`ModuleRef`] and the top. A reference to a removed module becomes
    /// [`ModuleRef::Unresolved`] under its name, so nothing dangles.
    fn retain_modules(&mut self, keep: &[bool]) {
        let names: Vec<Name> = self.modules.values().map(|m| m.name.clone()).collect();
        let remap = self
            .modules
            .retain(|id, _| keep.get(id.index()).copied().unwrap_or(true));
        self.top = self
            .top
            .and_then(|id| remap.get(id.index()).copied().flatten());
        for (_, module) in self.modules.iter_mut() {
            for (_, inst) in module.instances.iter_mut() {
                if let ModuleRef::Resolved(id) = inst.module {
                    inst.module = match remap.get(id.index()).copied().flatten() {
                        Some(new) => ModuleRef::Resolved(new),
                        // A removed, or already dangling, target keeps its
                        // name so nothing points at an unrelated module.
                        None => ModuleRef::Unresolved(
                            names
                                .get(id.index())
                                .cloned()
                                .unwrap_or_else(|| Name::new(format!("?{id}"))),
                        ),
                    };
                }
            }
        }
    }

    /// True when every resolved instance reference addresses a module of
    /// this design. The tree walks assume it; [`super::validate`] reports
    /// the violations (`I0007`).
    fn refs_resolve(&self) -> bool {
        self.modules.values().all(|module| {
            module.instances.values().all(|inst| match inst.module {
                ModuleRef::Resolved(id) => self.modules.contains(id),
                ModuleRef::Unresolved(_) => true,
            })
        })
    }

    /// Every instance path under `top`, depth first in declaration order,
    /// with the module each path names.
    ///
    /// Paths are dot-separated and do not include the top itself.
    /// Unresolved instances have no module and are skipped; a recursive
    /// hierarchy yields an empty list.
    pub fn hier_paths(&self, top: ModuleId) -> Vec<(String, ModuleId)> {
        let mut paths = Vec::new();
        if !self.modules.contains(top) || !self.refs_resolve() || self.topological_order().is_err()
        {
            return paths;
        }
        let mut stack: Vec<(String, ModuleId)> = children_of(&self.modules[top], "");
        stack.reverse();
        while let Some((path, module)) = stack.pop() {
            let mut nested = children_of(&self.modules[module], &path);
            paths.push((path, module));
            nested.reverse();
            stack.append(&mut nested);
        }
        paths
    }

    /// The number of instances in the hierarchy below `top`, unresolved
    /// ones included. Zero for a recursive hierarchy.
    pub fn instance_count(&self, top: ModuleId) -> usize {
        if !self.modules.contains(top) || !self.refs_resolve() || self.topological_order().is_err()
        {
            return 0;
        }
        let mut count = 0;
        let mut stack = vec![top];
        while let Some(module) = stack.pop() {
            for (_, inst) in self.modules[module].instances.iter() {
                count += 1;
                if let Some(id) = inst.module.id() {
                    stack.push(id);
                }
            }
        }
        count
    }

    /// Resolves a dotted path such as `u0.u1.state` to the instances walked
    /// through and the net named at the end.
    ///
    /// At every step the rest of the path is first tried as a net name, so
    /// the flattened net `u0.state` resolves just as well as the
    /// hierarchical one. Returns `None` when a component names neither a
    /// net nor a resolved instance.
    pub fn resolve_path(&self, top: ModuleId, path: &str) -> Option<(Vec<InstanceId>, NetId)> {
        let mut module = self.modules.get(top)?;
        let mut instances = Vec::new();
        let parts: Vec<&str> = path.split('.').collect();
        for (i, part) in parts.iter().enumerate() {
            if let Some(net) = module.net_by_name(&parts[i..].join(".")) {
                return Some((instances, net));
            }
            let id = module.instance_by_name(part)?;
            let target = module.instances[id].module.id()?;
            instances.push(id);
            module = self.modules.get(target)?;
        }
        None
    }
}

/// The `(path, module)` pairs of the resolved instances of `module`, under
/// `prefix`.
fn children_of(module: &Module, prefix: &str) -> Vec<(String, ModuleId)> {
    module
        .instances
        .values()
        .filter_map(|inst| {
            let id = inst.module.id()?;
            let path = if prefix.is_empty() {
                inst.name.as_str().to_owned()
            } else {
                format!("{prefix}.{}", inst.name)
            };
            Some((path, id))
        })
        .collect()
}

/// The text of a module with its identity removed, so two copies of the
/// same module compare equal.
fn dedup_key(module: &Module) -> String {
    let mut anonymous = module.clone();
    anonymous.name = Name::new("");
    anonymous.attrs.remove(UNIQUIFIED_FROM);
    anonymous.to_text()
}

/// `<base>$<index>`, or a further suffixed variant when that is taken.
fn unique_module_name(used: &mut HashSet<String>, base: &Name, index: usize) -> Name {
    let mut candidate = format!("{base}${index}");
    let mut extra = 1u32;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}${index}_{extra}");
        extra += 1;
    }
    Name::new(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::process::{AssignKind, ProcessKind};
    use crate::ir::types::Type;
    use crate::ir::validate::validate;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// `leaf`: `y = a + b`.
    fn leaf(span: Span) -> Module {
        let mut b = ModuleBuilder::new("leaf", span);
        let a = b.input("a", Type::bits(8));
        let bb = b.input("b", Type::bits(8));
        let y = b.output("y", Type::bits(8));
        let (av, bv) = (b.net(a), b.net(bb));
        let sum = b.add(av, bv);
        b.assign(y, sum);
        b.finish()
    }

    /// A design with `top` instantiating `leaf` twice: once with plain nets,
    /// once with an expression on an input.
    fn two_instance_design() -> (Design, ModuleId) {
        let span = span();
        let mut design = Design::new();
        let leaf_id = design.add_module(leaf(span));
        let mut b = ModuleBuilder::new("top", span);
        let x = b.input("x", Type::bits(8));
        let o0 = b.output("o0", Type::bits(8));
        let o1 = b.output("o1", Type::bits(8));
        let (xv, o0v, o1v) = (b.net(x), b.net(o0), b.net(o1));
        let one = b.const_u64(8, 1);
        let doubled = b.add(xv, xv);
        b.instance(
            "u0",
            ModuleRef::Resolved(leaf_id),
            vec![
                (Name::new("a"), xv),
                (Name::new("b"), one),
                (Name::new("y"), o0v),
            ],
        );
        b.instance(
            "u1",
            ModuleRef::Resolved(leaf_id),
            vec![
                (Name::new("a"), doubled),
                (Name::new("b"), xv),
                (Name::new("y"), o1v),
            ],
        );
        let top = design.add_module(b.finish());
        design.top = Some(top);
        (design, top)
    }

    fn assert_valid(design: &Design) {
        let diags = validate(design);
        assert!(!diags.has_errors(), "{}", diags.render(&SourceMap::new()));
    }

    #[test]
    fn flattens_merging_direct_connections() {
        let (mut design, top) = two_instance_design();
        let report = design.flatten(top, &FlattenOptions::default()).unwrap();
        assert_eq!(report.inlined_total, 2);
        assert_eq!(report.inlined_count("leaf"), 2);
        assert_eq!(report.inlined, [(Name::new("leaf"), 2)]);
        assert_eq!(report.kept, 0);
        assert_eq!(report.depth, 1);
        assert_valid(&design);

        let flat = design.module(top);
        assert!(flat.instances.is_empty());
        // `u0.a` and `u1.b` were direct connections and merged into `%x`.
        assert_eq!(flat.net_by_name("u0.a"), None);
        assert_eq!(flat.net_by_name("u1.b"), None);
        assert!(flat.net_by_name("u0.b").is_some());
        assert!(flat.net_by_name("u1.a").is_some());
        assert!(flat.net_by_name("u0.y").is_some());
        // Two copies of the child's assign, two output connections and the
        // expression connections of `u0.b` and `u1.a`.
        assert_eq!(flat.assigns.len(), 6);
        assert_eq!(report.assigns, 6);
        assert_eq!(report.nets, flat.nets.len());
    }

    #[test]
    fn keeps_black_boxes_and_marked_instances() {
        let span = span();
        let mut design = Design::new();
        let mut bb = ModuleBuilder::new("bb", span);
        bb.input("a", Type::bit());
        bb.blackbox();
        let bb_id = design.add_module(bb.finish());
        let leaf_id = design.add_module(leaf(span));

        let mut b = ModuleBuilder::new("top", span);
        let x = b.input("x", Type::bit());
        let xv = b.net(x);
        b.instance(
            "u_bb",
            ModuleRef::Resolved(bb_id),
            vec![(Name::new("a"), xv)],
        );
        b.instance("u_ext", ModuleRef::Unresolved(Name::new("ext")), Vec::new());
        let inst = b.instance("u_keep", ModuleRef::Resolved(leaf_id), Vec::new());
        b.module_mut().instances[inst].attrs.set(KEEP_HIERARCHY, 1);
        let top = design.add_module(b.finish());

        let report = design.flatten(top, &FlattenOptions::default()).unwrap();
        assert_eq!(report.kept, 3);
        assert_eq!(report.inlined_total, 0);
        assert_eq!(design.module(top).instances.len(), 3);

        // Without the attribute the marked instance is inlined.
        let (mut design2, top2) = (design.clone(), top);
        design2.modules[top2] = design2.modules[top2].clone();
        let options = FlattenOptions {
            keep_hierarchy_attr: false,
            ..FlattenOptions::default()
        };
        let report = design2.flatten(top2, &options).unwrap();
        assert_eq!(report.kept, 2);
        assert_eq!(report.inlined_count("leaf"), 1);
    }

    #[test]
    fn respects_the_depth_limit_and_annotates_paths() {
        let span = span();
        let mut design = Design::new();
        let leaf_id = design.add_module(leaf(span));
        let mut mid = ModuleBuilder::new("mid", span);
        let a = mid.input("a", Type::bits(8));
        let y = mid.output("y", Type::bits(8));
        let (av, yv) = (mid.net(a), mid.net(y));
        let one = mid.const_u64(8, 1);
        mid.instance(
            "u_leaf",
            ModuleRef::Resolved(leaf_id),
            vec![
                (Name::new("a"), av),
                (Name::new("b"), one),
                (Name::new("y"), yv),
            ],
        );
        let mid_id = design.add_module(mid.finish());
        let mut b = ModuleBuilder::new("top", span);
        let x = b.input("x", Type::bits(8));
        let o = b.output("o", Type::bits(8));
        let (xv, ov) = (b.net(x), b.net(o));
        b.instance(
            "u_mid",
            ModuleRef::Resolved(mid_id),
            vec![(Name::new("a"), xv), (Name::new("y"), ov)],
        );
        let top = design.add_module(b.finish());

        let shallow = FlattenOptions {
            max_depth: Some(1),
            ..FlattenOptions::default()
        };
        let mut one_level = design.clone();
        let report = one_level.flatten(top, &shallow).unwrap();
        assert_eq!(report.inlined_total, 1);
        assert_eq!(report.kept, 1);
        assert_eq!(
            one_level
                .module(top)
                .instances
                .values()
                .next()
                .unwrap()
                .name,
            "u_mid.u_leaf"
        );
        assert_valid(&one_level);

        let deep = FlattenOptions {
            annotate_paths: true,
            separator: "/".to_owned(),
            ..FlattenOptions::default()
        };
        let report = design.flatten(top, &deep).unwrap();
        assert_eq!(report.depth, 2);
        assert_eq!(report.inlined_total, 2);
        let flat = design.module(top);
        let net = flat.net_by_name("u_mid/u_leaf/b").expect("prefixed net");
        assert_eq!(
            flat.nets[net].attrs.get(ORIGIN).and_then(|v| v.as_str()),
            Some("u_mid/u_leaf")
        );
        assert_valid(&design);
    }

    #[test]
    fn merges_inout_ports_and_reports_the_rest() {
        let span = span();
        let mut design = Design::new();
        let mut pad = ModuleBuilder::new("pad", span);
        let io = pad.inout("io", Type::bit());
        let d = pad.input("d", Type::bit());
        let dv = pad.net(d);
        pad.assign(io, dv);
        let pad_id = design.add_module(pad.finish());

        let mut b = ModuleBuilder::new("top", span);
        let pin = b.inout("pin", Type::bit());
        let src = b.input("src", Type::bit());
        let (pinv, srcv) = (b.net(pin), b.net(src));
        b.instance(
            "u0",
            ModuleRef::Resolved(pad_id),
            vec![(Name::new("io"), pinv), (Name::new("d"), srcv)],
        );
        let top = design.add_module(b.finish());
        let mut merged = design.clone();
        merged.flatten(top, &FlattenOptions::default()).unwrap();
        assert_eq!(merged.module(top).net_by_name("u0.io"), None);
        assert_eq!(merged.module(top).assigns.len(), 1);
        assert_valid(&merged);

        // An inout connected to a slice cannot be merged.
        let sliced = {
            let module = design.module_mut(top);
            let wide = module.nets.push(crate::ir::Net {
                name: Name::new("bus"),
                ty: Type::bits(2),
                kind: crate::ir::NetKind::Wire,
                attrs: Attrs::new(),
                span,
            });
            let base = module.add_expr(Expr::new(ExprKind::Net(wide), Type::bits(2), span));
            module.add_expr(Expr::new(
                ExprKind::Slice { base, hi: 0, lo: 0 },
                Type::bit(),
                span,
            ))
        };
        design.module_mut(top).instances[InstanceId::from_index(0)].connections[0].1 = sliced;
        let diags = design.flatten(top, &FlattenOptions::default()).unwrap_err();
        assert_eq!(
            diags.iter().filter_map(|d| d.code).collect::<Vec<_>>(),
            [BAD_INOUT]
        );
    }

    #[test]
    fn reports_output_connections_that_are_not_lvalues() {
        let span = span();
        let mut design = Design::new();
        let leaf_id = design.add_module(leaf(span));
        let mut b = ModuleBuilder::new("top", span);
        let x = b.input("x", Type::bits(8));
        let lo = b.output("lo", Type::bits(4));
        let hi = b.output("hi", Type::bits(4));
        let (xv, lov, hiv) = (b.net(x), b.net(lo), b.net(hi));
        // `{%hi, %lo}[7:4]` is assignable to the validator but has no
        // lvalue form.
        let concat = b.concat(vec![hiv, lov]);
        let sliced = b.slice(concat, 7, 4);
        b.instance(
            "u0",
            ModuleRef::Resolved(leaf_id),
            vec![
                (Name::new("a"), xv),
                (Name::new("b"), xv),
                (Name::new("y"), sliced),
            ],
        );
        let top = design.add_module(b.finish());
        let diags = design.flatten(top, &FlattenOptions::default()).unwrap_err();
        assert_eq!(
            diags.iter().filter_map(|d| d.code).collect::<Vec<_>>(),
            [NOT_LVALUE]
        );
        // Nothing was committed.
        assert_eq!(design.module(top).instances.len(), 1);
    }

    #[test]
    fn flattens_processes_memories_and_cells() {
        let span = span();
        let mut design = Design::new();
        let mut child = ModuleBuilder::new("child", span);
        let clk = child.input("clk", Type::bit());
        let addr = child.input("addr", Type::bits(4));
        let q = child.output_reg("q", Type::bits(8));
        let mem = child.memory("ram", Type::bits(8), 16);
        let addrv = child.net(addr);
        let read = child.mem_read(mem, addrv);
        let mut p = child.process(Some("rd"), ProcessKind::posedge(clk));
        p.assign(q, read, AssignKind::NonBlocking);
        child.end_process(p);
        let child_id = design.add_module(child.finish());

        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let a = b.input("a", Type::bits(4));
        let o = b.output("o", Type::bits(8));
        let (clkv, av, ov) = (b.net(clk), b.net(a), b.net(o));
        let mem = b.memory("own", Type::bits(8), 4);
        let zero = b.const_u64(4, 0);
        let read = b.mem_read(mem, zero);
        let spare = b.add_net("spare", Type::bits(8));
        b.assign(spare, read);
        b.instance(
            "u0",
            ModuleRef::Resolved(child_id),
            vec![
                (Name::new("clk"), clkv),
                (Name::new("addr"), av),
                (Name::new("q"), ov),
            ],
        );
        let top = design.add_module(b.finish());
        design.flatten(top, &FlattenOptions::default()).unwrap();
        assert_valid(&design);
        let flat = design.module(top);
        assert_eq!(flat.processes.len(), 1);
        assert!(flat.process_by_name("u0.rd").is_some());
        assert_eq!(flat.memories.len(), 2);
        // The copied read still points at the copied memory.
        let copied = flat.memory_by_name("u0.ram").unwrap();
        assert!(
            flat.exprs
                .values()
                .any(|e| matches!(e.kind, ExprKind::MemRead { mem, .. } if mem == copied))
        );
    }

    #[test]
    fn uniquify_then_dedup_round_trips() {
        let (mut design, top) = two_instance_design();
        let before = design.to_text();
        let report = design.uniquify();
        assert_eq!(report.copies(), 2);
        assert_eq!(report.split.len(), 1);
        assert_eq!(report.split[0].0, "leaf");
        assert_eq!(
            report.split[0].1,
            [Name::new("leaf$1"), Name::new("leaf$2")]
        );
        assert_eq!(design.modules.len(), 3);
        assert_eq!(
            design.module_by_name("leaf$1").map(|id| design
                .module(id)
                .attrs
                .get(UNIQUIFIED_FROM)
                .is_some()),
            Some(true)
        );
        assert_valid(&design);
        // The two copies are now independent.
        let one = design.module_by_name("leaf$2").unwrap();
        design.module_mut(one).attrs.set("keep", 1);
        assert!(
            !design
                .module(design.module_by_name("leaf$1").unwrap())
                .attrs
                .is_set("keep")
        );
        design.module_mut(one).attrs.remove("keep");

        let report = design.dedup();
        assert_eq!(report.removed(), 1);
        assert_eq!(design.modules.len(), 2);
        assert_eq!(design.to_text(), before);
        assert_eq!(design.top, Some(top));
        assert_valid(&design);
    }

    #[test]
    fn uniquify_copies_nested_modules_and_skips_the_top() {
        let span = span();
        let mut design = Design::new();
        let leaf_id = design.add_module(leaf(span));
        let mut mid = ModuleBuilder::new("mid", span);
        let a = mid.input("a", Type::bits(8));
        let y = mid.output("y", Type::bits(8));
        let (av, yv) = (mid.net(a), mid.net(y));
        mid.instance(
            "u_leaf",
            ModuleRef::Resolved(leaf_id),
            vec![
                (Name::new("a"), av),
                (Name::new("b"), av),
                (Name::new("y"), yv),
            ],
        );
        let mid_id = design.add_module(mid.finish());
        let mut b = ModuleBuilder::new("top", span);
        let x = b.input("x", Type::bits(8));
        let o0 = b.output("o0", Type::bits(8));
        let o1 = b.output("o1", Type::bits(8));
        let (xv, o0v, o1v) = (b.net(x), b.net(o0), b.net(o1));
        for (name, out) in [("m0", o0v), ("m1", o1v)] {
            b.instance(
                name,
                ModuleRef::Resolved(mid_id),
                vec![(Name::new("a"), xv), (Name::new("y"), out)],
            );
        }
        let top = design.add_module(b.finish());
        design.top = Some(top);

        let report = design.uniquify();
        // `mid` is split in two, and so is the `leaf` under it; the first
        // copy of each reuses the original module.
        assert_eq!(report.split.len(), 2);
        assert_eq!(design.modules.len(), 5);
        assert_valid(&design);
        assert_eq!(design.instance_count(top), 4);
        let paths: Vec<String> = design.hier_paths(top).into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, ["m0", "m0.u_leaf", "m1", "m1.u_leaf"]);

        let report = design.dedup();
        assert_eq!(report.removed(), 2);
        assert_eq!(design.modules.len(), 3);
        // The survivors took their original names back.
        assert!(design.module_by_name("mid").is_some());
        assert!(design.module_by_name("leaf").is_some());
        assert_valid(&design);
    }

    #[test]
    fn removes_unused_modules() {
        let (mut design, _) = two_instance_design();
        let spare = design.add_module(Module::new("spare", span()));
        assert_eq!(design.modules.len(), 3);
        assert_eq!(design.remove_unused_modules(spare), 2);
        assert_eq!(design.modules.len(), 1);
        assert_eq!(design.top, None);
        assert_eq!(design.remove_unused_modules(ModuleId::from_index(9)), 0);

        let (mut design, top) = two_instance_design();
        design.add_module(Module::new("spare", span()));
        assert_eq!(design.remove_unused_modules(top), 1);
        assert_eq!(design.modules.len(), 2);
        assert_valid(&design);
    }

    #[test]
    fn resolves_paths_before_and_after_flattening() {
        let (mut design, top) = two_instance_design();
        let leaf_id = design.module_by_name("leaf").unwrap();
        assert_eq!(design.instance_count(top), 2);
        assert_eq!(
            design.hier_paths(top),
            [("u0".to_owned(), leaf_id), ("u1".to_owned(), leaf_id)]
        );
        let (path, net) = design.resolve_path(top, "u1.b").unwrap();
        assert_eq!(path, [InstanceId::from_index(1)]);
        assert_eq!(design.module(leaf_id).nets[net].name, "b");
        assert!(design.resolve_path(top, "u1.nope").is_none());
        assert!(design.resolve_path(top, "nope.b").is_none());
        let (path, net) = design.resolve_path(top, "x").unwrap();
        assert!(path.is_empty());
        assert_eq!(design.module(top).nets[net].name, "x");

        design.flatten(top, &FlattenOptions::default()).unwrap();
        assert!(design.hier_paths(top).is_empty());
        assert_eq!(design.instance_count(top), 0);
        // The flattened net carries the path in its name.
        let (path, net) = design.resolve_path(top, "u1.a").unwrap();
        assert!(path.is_empty());
        assert_eq!(design.module(top).nets[net].name, "u1.a");
    }

    #[test]
    fn refuses_recursive_and_missing_hierarchies() {
        let span = span();
        let mut design = Design::new();
        let a = design.add_module(Module::new("a", span));
        design.modules[a].instances.push(Instance {
            name: Name::new("self"),
            module: ModuleRef::Resolved(a),
            connections: Vec::new(),
            params: Attrs::new(),
            attrs: Attrs::new(),
            span,
        });
        let diags = design.flatten(a, &FlattenOptions::default()).unwrap_err();
        assert_eq!(
            diags.iter().filter_map(|d| d.code).collect::<Vec<_>>(),
            [RECURSIVE]
        );
        assert!(design.hier_paths(a).is_empty());
        assert_eq!(design.instance_count(a), 0);
        assert_eq!(design.uniquify(), UniquifyReport::default());

        let diags = design
            .flatten(ModuleId::from_index(7), &FlattenOptions::default())
            .unwrap_err();
        assert_eq!(
            diags.iter().filter_map(|d| d.code).collect::<Vec<_>>(),
            [MISSING]
        );
    }

    #[test]
    fn reports_instances_of_missing_modules() {
        let span = span();
        let mut design = Design::new();
        let mut b = ModuleBuilder::new("top", span);
        b.instance(
            "u0",
            ModuleRef::Resolved(ModuleId::from_index(9)),
            Vec::new(),
        );
        let top = design.add_module(b.finish());
        let diags = design.flatten(top, &FlattenOptions::default()).unwrap_err();
        assert_eq!(
            diags.iter().filter_map(|d| d.code).collect::<Vec<_>>(),
            [MISSING]
        );
        // The queries walk the same tree and must not panic on it either.
        assert!(design.hier_paths(top).is_empty());
        assert_eq!(design.instance_count(top), 0);
        assert_eq!(design.uniquify(), UniquifyReport::default());
        assert_eq!(design.remove_unused_modules(top), 0);
        assert_eq!(design.dedup(), DedupReport::default());
    }

    #[test]
    fn name_clashes_get_a_suffix() {
        let span = span();
        let mut design = Design::new();
        let leaf_id = design.add_module(leaf(span));
        let mut b = ModuleBuilder::new("top", span);
        let x = b.input("x", Type::bits(8));
        let o = b.output("o", Type::bits(8));
        // A net already called `u0.b`, the name the copy wants.
        b.add_net("u0.b", Type::bits(8));
        let (xv, ov) = (b.net(x), b.net(o));
        let one = b.const_u64(8, 1);
        b.instance(
            "u0",
            ModuleRef::Resolved(leaf_id),
            vec![
                (Name::new("a"), xv),
                (Name::new("b"), one),
                (Name::new("y"), ov),
            ],
        );
        let top = design.add_module(b.finish());
        design.flatten(top, &FlattenOptions::default()).unwrap();
        assert!(design.module(top).net_by_name("u0.b$2").is_some());
        assert_valid(&design);
    }

    #[test]
    fn dedup_merges_identical_modules() {
        let span = span();
        let mut design = Design::new();
        let a = design.add_module(leaf(span));
        let mut other = leaf(span);
        other.name = Name::new("leaf_copy");
        let b = design.add_module(other);
        let mut top = ModuleBuilder::new("top", span);
        let x = top.input("x", Type::bits(8));
        let o0 = top.output("o0", Type::bits(8));
        let o1 = top.output("o1", Type::bits(8));
        let (xv, o0v, o1v) = (top.net(x), top.net(o0), top.net(o1));
        top.instance(
            "u0",
            ModuleRef::Resolved(a),
            vec![
                (Name::new("a"), xv),
                (Name::new("b"), xv),
                (Name::new("y"), o0v),
            ],
        );
        top.instance(
            "u1",
            ModuleRef::Resolved(b),
            vec![
                (Name::new("a"), xv),
                (Name::new("b"), xv),
                (Name::new("y"), o1v),
            ],
        );
        let top_id = design.add_module(top.finish());
        design.top = Some(top_id);
        let report = design.dedup();
        assert_eq!(report.merged.len(), 1);
        assert_eq!(report.merged[0].0, "leaf");
        assert_eq!(report.merged[0].1, [Name::new("leaf_copy")]);
        assert_eq!(design.modules.len(), 2);
        assert_eq!(design.instance_count(design.top.unwrap()), 2);
        assert_valid(&design);
    }
}
