//! Hierarchical references into instances below the module.
//!
//! A testbench reads, waits on and forces nets of the design it tests by
//! hierarchical name — `U_dut.oadc.U_fifo.error_flag` — and the frontend
//! used to resolve a name only within the current module, its generate
//! blocks and its interfaces. Such a name now becomes a net of this
//! module, recorded as an [`ir::HierRef`] that the simulator maps onto the
//! target's signal, so everything that reads a net reads it.
//!
//! The aliases are made before the module's behaviour is lowered, because
//! width inference resolves names before any expression is built, and a
//! testbench may name an instance above the line that declares it, as
//! NewAE's Husky testbench names `U_dut` from blocks above it. Each
//! instance on the path is elaborated as its instantiation would elaborate
//! it (the variant is cached, so this is the same module), and the path is
//! walked down the elaborated IR by instance name to the net.

use crate::ir::{self, ModuleId, ModuleRef, Type};
use crate::source::Span;
use crate::verilog::ast::{self, Expr, ExprKind, Item, ItemKind};
use crate::verilog::lint::facts::{stmt_exprs, walk_expr, walk_items, walk_stmt};

use super::super::types::{Packed, VType};
use super::Lowerer;

/// A name made only of identifiers joined by `.`, outermost first, with
/// the span of the whole; `None` for anything else.
fn dotted(e: &Expr) -> Option<Vec<String>> {
    match &e.kind {
        ExprKind::Ident(id) => Some(vec![id.name.clone()]),
        ExprKind::Member { base, name } => {
            let mut path = dotted(base)?;
            path.push(name.name.clone());
            Some(path)
        }
        _ => None,
    }
}

/// The frontend type of an IR net's type, for the alias that stands for it.
fn vtype_of(ty: &Type) -> Option<VType> {
    match ty {
        Type::Bits { width, signed } => Some(VType::Packed(if *signed {
            Packed::sbits(*width)
        } else {
            Packed::bits(*width)
        })),
        Type::Real => Some(VType::Real),
        _ => None,
    }
}

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    /// Makes an alias net for every hierarchical name in `items` that
    /// starts at one of this module's instances and ends at a net; see the
    /// module documentation. A name that does not resolve is left for the
    /// ordinary resolution to report.
    pub(super) fn hier_aliases(&mut self, items: &'ast [Item]) {
        // This module's instances by name, as written at its top level.
        let mut instances: Vec<(&'ast str, &'ast ast::Instantiation, Span)> = Vec::new();
        for item in items {
            if let ItemKind::Instance(inst) = &item.kind {
                for one in &inst.instances {
                    if let Some(name) = &one.name
                        && one.dims.is_empty()
                    {
                        instances.push((name.name.as_str(), inst, name.span));
                    }
                }
            }
        }
        if instances.is_empty() {
            return;
        }
        // Every name in the module that could be one.
        let mut names: Vec<(Vec<String>, Span)> = Vec::new();
        let mut take = |e: &'ast Expr| {
            walk_expr(e, &mut |sub| {
                if matches!(sub.kind, ExprKind::Member { .. })
                    && let Some(path) = dotted(sub)
                    && path.len() >= 2
                {
                    names.push((path, sub.span));
                }
            });
        };
        walk_items(items, &mut |item| match &item.kind {
            ItemKind::Always(_, stmt) | ItemKind::Initial(stmt) | ItemKind::Final(stmt) => {
                walk_stmt(stmt, &mut |s| {
                    for e in stmt_exprs(s) {
                        take(e);
                    }
                    // `stmt_exprs` leaves out what an event control
                    // watches: `@(posedge U_dut.x)` names one too.
                    if let ast::StmtKind::Timing(tc, _) = &s.kind {
                        let ev = match &tc.kind {
                            ast::TimingKind::Event(ev) => Some(ev),
                            ast::TimingKind::RepeatEvent(count, ev) => {
                                take(count);
                                Some(ev)
                            }
                            ast::TimingKind::Delay(_) => None,
                        };
                        if let Some(ast::EventControl {
                            kind: ast::EventControlKind::List(list),
                            ..
                        }) = ev
                        {
                            for item in list {
                                take(&item.expr);
                            }
                        }
                    }
                });
            }
            ItemKind::ContAssign(ca) => {
                for pair in &ca.assigns {
                    take(&pair.lhs);
                    take(&pair.rhs);
                }
            }
            ItemKind::Net(n) => {
                for d in &n.decls {
                    if let Some(init) = &d.init {
                        take(init);
                    }
                }
            }
            ItemKind::Var(v) => {
                for d in &v.decls {
                    if let Some(init) = &d.init {
                        take(init);
                    }
                }
            }
            _ => {}
        });
        for (path, span) in names {
            let key = path.join(".");
            if self.env.hier.contains_key(&key) {
                continue;
            }
            let Some(&(_, inst, inst_span)) = instances.iter().find(|(n, _, _)| *n == path[0])
            else {
                continue;
            };
            let Some((ty, instance_path, net_name)) = self.hier_target(inst, inst_span, &path)
            else {
                continue;
            };
            let Some(vty) = vtype_of(&ty) else {
                continue;
            };
            let Some(local) = self.new_net(key.clone(), vty.clone(), false, span) else {
                continue;
            };
            self.b.module_mut().hier_refs.push(ir::HierRef {
                local,
                path: instance_path,
                net: net_name,
            });
            self.env.hier.insert(key, (local, vty));
        }
    }

    /// The type of the net `path` names, the instance names down to it and
    /// its name there, when `path[0]` is the instance `inst` declares and
    /// the rest leads through its instances to one of its nets.
    fn hier_target(
        &mut self,
        inst: &'ast ast::Instantiation,
        inst_span: Span,
        path: &[String],
    ) -> Option<(Type, Vec<ir::Name>, ir::Name)> {
        let module = inst.module.name.as_str();
        if !self.env.cx.table.modules.contains_key(module) {
            return None;
        }
        let overrides = self.build_overrides(&inst.params, &path[0]);
        let mut id: ModuleId = super::elaborate_module(self.env.cx, module, overrides, inst_span)?;
        let (last, middle) = path[1..].split_last()?;
        for name in middle {
            let m = self.env.cx.design.modules.get(id)?;
            let (_, child) = m.instances.iter().find(|(_, i)| i.name.as_str() == name)?;
            let ModuleRef::Resolved(next) = child.module else {
                return None;
            };
            id = next;
        }
        let m = self.env.cx.design.modules.get(id)?;
        let (_, net) = m.nets.iter().find(|(_, n)| n.name.as_str() == last)?;
        let ty = net.ty.clone();
        let instances = path[..path.len() - 1]
            .iter()
            .map(|n| ir::Name::new(n.as_str()))
            .collect();
        Some((ty, instances, ir::Name::new(last.as_str())))
    }
}
