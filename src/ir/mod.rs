//! The unified design IR.
//!
//! Both frontends lower into the types defined here, and every later stage
//! (simulation, synthesis, formal, emission) consumes them. The IR is the
//! product of the project; everything else is a producer or a consumer.
//!
//! # Shape
//!
//! A [`Design`] is an arena of [`Module`]s with an optional top. A module
//! holds, each in its own arena keyed by a `Copy` id:
//!
//! | Object        | Id             | Role                                              |
//! |---------------|----------------|---------------------------------------------------|
//! | [`Net`]       | [`NetId`]      | A typed value: wire, register or variable         |
//! | [`Memory`]    | [`MemoryId`]   | An array with ports (RAM / ROM candidate)         |
//! | [`Expr`]      | [`ExprId`]     | A typed expression node                           |
//! | [`Instance`]  | [`InstanceId`] | A sub-module instantiation                        |
//! | [`Process`]   | [`ProcessId`]  | Structured behavioural code with a trigger        |
//! | [`Cell`]      | [`CellId`]     | A primitive from the fixed post-synthesis set     |
//!
//! plus its [`Port`]s, resolved [`Param`]s (metadata), continuous
//! [`Assign`]s, [`Attrs`] and an optional [`Timescale`]. The *process form*
//! (processes and assigns) and the *cell form* (cells) coexist in one
//! module: a freshly lowered module has only processes, a synthesised one
//! only cells, and passes may leave a mix (a black-box instance next to
//! lowered logic, a behavioural model next to a mapped datapath).
//!
//! Design rules that hold everywhere:
//!
//! - **Every object has a [`Span`]** back to the source that produced it.
//! - **Everything is typed.** Nets and expressions carry a [`Type`];
//!   operators have fixed width rules ([`expr`]) that [`validate`] checks.
//!   Bit vectors are the main path; `Integer`, `Real` and `String` exist for
//!   simulation-only values.
//! - **Attributes on everything.** Every object carries [`Attrs`], an
//!   ordered map, so `(* keep *)` and friends survive the pipeline.
//! - **Ids, not references.** Objects are addressed through arena ids so
//!   passes can mutate freely; [`walk`] has the helpers that keep ids
//!   consistent across removals.
//! - **Names are unique per kind per module** (nets, memories, instances,
//!   cells, named processes) and modules are unique per design, because the
//!   text format refers to objects by name.
//!
//! # Text format
//!
//! [`Design::to_text`] and [`Design::parse_text`] convert to and from the
//! `.rtl` text format described in [`text`] and in `docs/ir.md`. It
//! round-trips exactly and is what golden tests compare.
//!
//! # Building
//!
//! [`builder::ModuleBuilder`] is the ergonomic way to construct modules
//! from Rust, used by the frontends' lowering, by tests and by future IP
//! generators.
//!
//! # Placeholders
//!
//! [`Name`] wraps a `String` and will become an interned `Symbol` once the
//! interner lands; [`Const`] is a small 4-state constant that will become
//! `logic::Logic`. Both are used only through their small public APIs so the
//! swap stays local.

use std::borrow::Borrow;
use std::fmt;

pub mod arena;
pub mod attr;
pub mod builder;
pub mod cell;
pub mod design;
pub mod expr;
pub mod process;
pub mod text;
pub mod types;
pub mod validate;
pub mod walk;

pub use arena::{Arena, Id};
pub use attr::{AttrValue, Attrs};
pub use cell::{Cell, CellId, CellKind, Reset};
pub use design::{
    Assign, Design, Instance, InstanceId, Memory, MemoryId, Module, ModuleId, ModuleRef, Net,
    NetId, NetKind, Param, Port, PortDir,
};
pub use expr::{BinaryOp, Expr, ExprId, ExprKind, TypeError, UnaryOp, infer_type};
pub use process::{
    AssignKind, Block, CaseArm, CaseKind, CaseQualifier, Delay, Edge, Lvalue, Polarity, Process,
    ProcessId, ProcessKind, ReportSeverity, Stmt, StmtKind, TimeUnit, Timescale, WaitKind,
};
pub use types::{Bit, Const, Type};

#[doc(no_inline)]
pub use crate::source::Span;

/// An identifier: the name of a module, net, port, cell, attribute and so
/// on.
///
/// Currently a `String`; it will become an interned `Symbol` once the
/// interner exists. Code should treat it as opaque and go through
/// [`Name::new`] and [`Name::as_str`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(String);

impl Name {
    /// Builds a name.
    pub fn new(name: impl Into<String>) -> Self {
        Name(name.into())
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Self {
        Name(s.to_owned())
    }
}

impl From<String> for Name {
    fn from(s: String) -> Self {
        Name(s)
    }
}

impl From<&Name> for Name {
    fn from(n: &Name) -> Self {
        n.clone()
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_conversions() {
        let n = Name::new("clk");
        assert_eq!(n, "clk");
        assert_eq!(n, *"clk");
        assert_eq!(n.as_str(), "clk");
        assert_eq!(n.to_string(), "clk");
        assert_eq!(format!("{n:?}"), "\"clk\"");
        assert_eq!(Name::from(String::from("a")), Name::from("a"));
        assert_eq!(Name::from(&n), n);
        let s: &str = n.borrow();
        assert_eq!(s, "clk");
    }
}
