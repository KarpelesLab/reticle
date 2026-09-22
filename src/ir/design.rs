//! The design database: modules, nets, memories, instances and drivers.
//!
//! A [`Design`] is an arena of [`Module`]s plus an optional top. A module
//! owns everything inside it in arenas keyed by typed ids; an id from one
//! module means nothing in another. Instances refer to other modules by
//! [`ModuleRef`], which is either a resolved [`ModuleId`] or the name of a
//! module the design does not contain (a black box supplied later by the
//! target flow).

use super::Name;
use super::arena::{Arena, define_id};
use super::attr::{AttrValue, Attrs};
use super::cell::{Cell, CellId};
use super::expr::{Expr, ExprId};
use super::process::{Delay, Lvalue, Process, ProcessId, Timescale};
use super::types::{Const, Type};
use crate::source::Span;

define_id!(
    /// Identifies a module inside a design.
    ModuleId,
    "m"
);

define_id!(
    /// Identifies a net inside a module.
    NetId,
    "n"
);

define_id!(
    /// Identifies a memory inside a module.
    MemoryId,
    "mem"
);

define_id!(
    /// Identifies an instance inside a module.
    InstanceId,
    "i"
);

/// Direction of a port, seen from inside the module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortDir {
    /// Driven from outside.
    In,
    /// Driven from inside.
    Out,
    /// Bidirectional; drivers on both sides, resolved.
    InOut,
}

impl PortDir {
    /// The keyword in the text format.
    pub fn keyword(self) -> &'static str {
        match self {
            PortDir::In => "in",
            PortDir::Out => "out",
            PortDir::InOut => "inout",
        }
    }

    /// The direction with the given keyword.
    pub fn from_keyword(name: &str) -> Option<PortDir> {
        match name {
            "in" => Some(PortDir::In),
            "out" => Some(PortDir::Out),
            "inout" => Some(PortDir::InOut),
            _ => None,
        }
    }
}

/// A connection point of a module: a name, a direction and the net it
/// exposes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    /// The port name, unique among the module's ports. Usually the net's
    /// name, but the two may diverge after renaming passes.
    pub name: Name,
    /// The direction.
    pub dir: PortDir,
    /// The net inside the module carrying the port's value.
    pub net: NetId,
    /// Where the port was declared.
    pub span: Span,
}

/// What kind of storage a net models.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetKind {
    /// A wire: no storage, its value is whatever drives it.
    Wire,
    /// A signal updated by processes (Verilog `reg`/`logic` written from
    /// `always`, VHDL signals); updates are scheduled, not immediate.
    Reg,
    /// A process-local variable (VHDL `variable`, SystemVerilog automatic
    /// variables); updates take effect immediately.
    Variable,
}

impl NetKind {
    /// The keyword in the text format.
    pub fn keyword(self) -> &'static str {
        match self {
            NetKind::Wire => "wire",
            NetKind::Reg => "reg",
            NetKind::Variable => "var",
        }
    }

    /// The kind with the given keyword.
    pub fn from_keyword(name: &str) -> Option<NetKind> {
        match name {
            "wire" => Some(NetKind::Wire),
            "reg" => Some(NetKind::Reg),
            "var" => Some(NetKind::Variable),
            _ => None,
        }
    }
}

/// A named, typed value inside a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Net {
    /// Unique among the module's nets.
    pub name: Name,
    /// The type of the value carried.
    pub ty: Type,
    /// What the net models.
    pub kind: NetKind,
    /// Source attributes.
    pub attrs: Attrs,
    /// Where the net was declared.
    pub span: Span,
}

/// An array with ports: a RAM or ROM candidate.
///
/// Memories are separate from array-typed nets so inference and simulation
/// can treat them as storage with a fixed set of read and write ports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Memory {
    /// Unique among the module's memories.
    pub name: Name,
    /// Type of one element.
    pub elem: Type,
    /// Number of elements.
    pub size: u64,
    /// Initial contents, element 0 first; shorter than `size` leaves the
    /// rest undefined, `None` leaves everything undefined.
    pub init: Option<Vec<Const>>,
    /// Source attributes (`ram_style` and friends).
    pub attrs: Attrs,
    /// Where the memory was declared.
    pub span: Span,
}

/// The module an instance refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleRef {
    /// A module in the same design.
    Resolved(ModuleId),
    /// A module not in the design, by name; the instance is a black box.
    Unresolved(Name),
}

impl ModuleRef {
    /// The resolved id, if any.
    pub fn id(&self) -> Option<ModuleId> {
        match self {
            ModuleRef::Resolved(id) => Some(*id),
            ModuleRef::Unresolved(_) => None,
        }
    }
}

/// An instantiation of another module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    /// Unique among the module's instances.
    pub name: Name,
    /// The instantiated module.
    pub module: ModuleRef,
    /// Port connections: the target's port name and the expression connected
    /// to it. Output and inout ports must connect to nets, slices or
    /// concatenations of nets.
    pub connections: Vec<(Name, ExprId)>,
    /// Parameter overrides, kept for black boxes and for emitters; resolved
    /// modules have already been specialised.
    pub params: Attrs,
    /// Source attributes.
    pub attrs: Attrs,
    /// Where the instance was written.
    pub span: Span,
}

impl Instance {
    /// The expression connected to `port`.
    pub fn connection(&self, port: &str) -> Option<ExprId> {
        self.connections
            .iter()
            .find(|(n, _)| n.as_str() == port)
            .map(|(_, e)| *e)
    }
}

/// A resolved parameter or generic, kept as metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    /// The parameter name.
    pub name: Name,
    /// Its value after elaboration.
    pub value: AttrValue,
    /// Source attributes.
    pub attrs: Attrs,
    /// Where the parameter was declared.
    pub span: Span,
}

/// A continuous driver: `assign target = value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assign {
    /// What is driven.
    pub target: Lvalue,
    /// The driving expression.
    pub value: ExprId,
    /// Transport delay, if any.
    pub delay: Option<Delay>,
    /// Source attributes.
    pub attrs: Attrs,
    /// Where the assignment was written.
    pub span: Span,
}

/// One module: an interface and an implementation in process and cell
/// form.
#[derive(Clone, Debug)]
pub struct Module {
    /// Unique among the design's modules.
    pub name: Name,
    /// The interface, in declaration order.
    pub ports: Vec<Port>,
    /// Resolved parameters, for reports and emitters.
    pub params: Vec<Param>,
    /// Every net.
    pub nets: Arena<NetId, Net>,
    /// Every memory.
    pub memories: Arena<MemoryId, Memory>,
    /// Every expression referenced from this module.
    pub exprs: Arena<ExprId, Expr>,
    /// Sub-module instances.
    pub instances: Arena<InstanceId, Instance>,
    /// The process form.
    pub processes: Arena<ProcessId, Process>,
    /// The cell form.
    pub cells: Arena<CellId, Cell>,
    /// Continuous assignments.
    pub assigns: Vec<Assign>,
    /// Source attributes.
    pub attrs: Attrs,
    /// True when the module has no implementation: only its interface is
    /// known and the target flow supplies the contents.
    pub blackbox: bool,
    /// The module's time unit and precision.
    pub timescale: Option<Timescale>,
    /// Where the module was declared.
    pub span: Span,
}

impl Module {
    /// Creates an empty module.
    pub fn new(name: impl Into<Name>, span: Span) -> Self {
        Module {
            name: name.into(),
            ports: Vec::new(),
            params: Vec::new(),
            nets: Arena::new(),
            memories: Arena::new(),
            exprs: Arena::new(),
            instances: Arena::new(),
            processes: Arena::new(),
            cells: Arena::new(),
            assigns: Vec::new(),
            attrs: Attrs::new(),
            blackbox: false,
            timescale: None,
            span,
        }
    }

    /// Adds an expression node and returns its id.
    pub fn add_expr(&mut self, expr: Expr) -> ExprId {
        self.exprs.push(expr)
    }

    /// The expression node with the given id.
    ///
    /// Ids come from this module's arena, so a miss is a programming error.
    pub fn expr(&self, id: ExprId) -> &Expr {
        &self.exprs[id]
    }

    /// The net with the given name.
    pub fn net_by_name(&self, name: &str) -> Option<NetId> {
        self.nets.find(|n| n.name.as_str() == name)
    }

    /// The memory with the given name.
    pub fn memory_by_name(&self, name: &str) -> Option<MemoryId> {
        self.memories.find(|m| m.name.as_str() == name)
    }

    /// The port with the given name.
    pub fn port(&self, name: &str) -> Option<&Port> {
        self.ports.iter().find(|p| p.name.as_str() == name)
    }

    /// The port exposing `net`, if any.
    pub fn port_of_net(&self, net: NetId) -> Option<&Port> {
        self.ports.iter().find(|p| p.net == net)
    }

    /// The instance with the given name.
    pub fn instance_by_name(&self, name: &str) -> Option<InstanceId> {
        self.instances.find(|i| i.name.as_str() == name)
    }

    /// The cell with the given name.
    pub fn cell_by_name(&self, name: &str) -> Option<CellId> {
        self.cells.find(|c| c.name.as_str() == name)
    }

    /// The named process with the given name.
    pub fn process_by_name(&self, name: &str) -> Option<ProcessId> {
        self.processes
            .find(|p| p.name.as_ref().is_some_and(|n| n.as_str() == name))
    }

    /// The resolved parameter with the given name.
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name.as_str() == name)
    }
}

/// A whole design: every module plus the root of the hierarchy.
#[derive(Clone, Debug, Default)]
pub struct Design {
    /// Every module, in the order they were added.
    pub modules: Arena<ModuleId, Module>,
    /// The root of the hierarchy, if one has been selected.
    pub top: Option<ModuleId>,
}

impl Design {
    /// Creates an empty design.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a module and returns its id.
    pub fn add_module(&mut self, module: Module) -> ModuleId {
        self.modules.push(module)
    }

    /// The module with the given name.
    pub fn module_by_name(&self, name: &str) -> Option<ModuleId> {
        self.modules.find(|m| m.name.as_str() == name)
    }

    /// The module with the given id.
    ///
    /// Ids come from this design's arena, so a miss is a programming error.
    pub fn module(&self, id: ModuleId) -> &Module {
        &self.modules[id]
    }

    /// Mutable access to the module with the given id.
    pub fn module_mut(&mut self, id: ModuleId) -> &mut Module {
        &mut self.modules[id]
    }

    /// The top module, if selected.
    pub fn top_module(&self) -> Option<&Module> {
        self.top.and_then(|id| self.modules.get(id))
    }

    /// Resolves every [`ModuleRef::Unresolved`] whose name matches a module
    /// in the design, and returns how many references were resolved.
    pub fn resolve_instances(&mut self) -> usize {
        let names: Vec<(ModuleId, Name)> = self
            .modules
            .iter()
            .map(|(id, m)| (id, m.name.clone()))
            .collect();
        let mut count = 0;
        for module in self.modules.iter_mut().map(|(_, m)| m) {
            for inst in module.instances.iter_mut().map(|(_, i)| i) {
                if let ModuleRef::Unresolved(name) = &inst.module
                    && let Some((id, _)) = names.iter().find(|(_, n)| n == name)
                {
                    inst.module = ModuleRef::Resolved(*id);
                    count += 1;
                }
            }
        }
        count
    }
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
    fn keywords() {
        for dir in [PortDir::In, PortDir::Out, PortDir::InOut] {
            assert_eq!(PortDir::from_keyword(dir.keyword()), Some(dir));
        }
        for kind in [NetKind::Wire, NetKind::Reg, NetKind::Variable] {
            assert_eq!(NetKind::from_keyword(kind.keyword()), Some(kind));
        }
        assert_eq!(PortDir::from_keyword("x"), None);
        assert_eq!(NetKind::from_keyword("x"), None);
    }

    #[test]
    fn lookups_and_resolution() {
        let span = span();
        let mut design = Design::new();
        let mut top = Module::new("top", span);
        let a = top.nets.push(Net {
            name: Name::new("a"),
            ty: Type::bit(),
            kind: NetKind::Wire,
            attrs: Attrs::new(),
            span,
        });
        top.ports.push(Port {
            name: Name::new("a"),
            dir: PortDir::In,
            net: a,
            span,
        });
        top.instances.push(Instance {
            name: Name::new("u0"),
            module: ModuleRef::Unresolved(Name::new("leaf")),
            connections: Vec::new(),
            params: Attrs::new(),
            attrs: Attrs::new(),
            span,
        });
        let top_id = design.add_module(top);
        let leaf_id = design.add_module(Module::new("leaf", span));
        design.top = Some(top_id);
        assert_eq!(design.module_by_name("leaf"), Some(leaf_id));
        assert_eq!(design.module_by_name("nope"), None);
        assert_eq!(design.top_module().map(|m| m.name.as_str()), Some("top"));
        let top = design.module(top_id);
        assert_eq!(top.net_by_name("a"), Some(a));
        assert_eq!(top.port("a").map(|p| p.dir), Some(PortDir::In));
        assert_eq!(top.port_of_net(a).map(|p| p.name.as_str()), Some("a"));
        assert_eq!(top.instance_by_name("u0"), Some(InstanceId(0)));
        assert_eq!(top.process_by_name("p"), None);
        assert_eq!(design.resolve_instances(), 1);
        assert_eq!(
            design.module(top_id).instances[InstanceId(0)].module.id(),
            Some(leaf_id)
        );
        assert_eq!(design.resolve_instances(), 0);
    }
}
