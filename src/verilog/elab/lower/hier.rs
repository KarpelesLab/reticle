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
    pub(super) fn hier_aliases(&mut self, module: &str, items: &'ast [Item]) {
        // This module's instances by the name the IR gives them: as written
        // at its top level, and `blk.u` for `u` in a named generate block
        // `blk` (IEEE 1364-2005 §12.4: a generate block names a scope).
        let mut instances: Vec<(String, &'ast ast::Instantiation, Span)> = Vec::new();
        self.collect_instances(items, "", &mut instances);
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
            // `tb.m.x` inside `tb` is `m.x`: a path may start at the module
            // it is written in.
            let from = usize::from(path.len() > 2 && path[0] == module);
            // The instance is the longest leading part of the path that
            // names one: `blk.u` before `blk`.
            let found = (from + 1..path.len()).rev().find_map(|k| {
                let head = path[from..k].join(".");
                instances
                    .iter()
                    .find(|(n, _, _)| *n == head)
                    .map(|(n, inst, sp)| (k, n.clone(), *inst, *sp))
            });
            let Some((k, head, inst, inst_span)) = found else {
                continue;
            };
            let mut rest = vec![head];
            rest.extend(path[k..].iter().cloned());
            let Some((ty, instance_path, net_name, var)) = self.hier_target(inst, inst_span, &rest)
            else {
                continue;
            };
            let Some(vty) = vtype_of(&ty) else {
                continue;
            };
            // A variable below may be assigned from a procedure here, as a
            // testbench does through `dut.count = 0`: the alias is one too.
            let Some(local) = self.new_net(key.clone(), vty.clone(), var, span) else {
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
    ) -> Option<(Type, Vec<ir::Name>, ir::Name, bool)> {
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
        let var = net.kind != ir::NetKind::Wire;
        let instances = path[..path.len() - 1]
            .iter()
            .map(|n| ir::Name::new(n.as_str()))
            .collect();
        Some((ty, instances, ir::Name::new(last.as_str()), var))
    }

    /// The module instances in `items` and in its named generate blocks,
    /// each by its IR name (`prefix` and its own), interfaces left out: an
    /// interface instance is flattened into this module, so `si.v` names a
    /// net of its own here, not one below (aliasing it too declared `si.v`
    /// twice, an internal error).
    fn collect_instances(
        &mut self,
        items: &'ast [Item],
        prefix: &str,
        out: &mut Vec<(String, &'ast ast::Instantiation, Span)>,
    ) {
        for item in items {
            match &item.kind {
                ItemKind::Instance(inst) if !self.env.cx.table.is_interface(&inst.module.name) => {
                    for one in &inst.instances {
                        if let Some(name) = &one.name
                            && one.dims.is_empty()
                        {
                            out.push((format!("{prefix}{}", name.name), inst, name.span));
                        }
                    }
                }
                ItemKind::Generate(inner) => self.collect_instances(inner, prefix, out),
                ItemKind::GenBlock(b) => self.collect_block(b, prefix, out),
                ItemKind::GenIf(g) => {
                    self.collect_block(&g.then_block, prefix, out);
                    if let Some(e) = &g.else_block {
                        self.collect_block(e, prefix, out);
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_block(
        &mut self,
        b: &'ast ast::GenBlock,
        prefix: &str,
        out: &mut Vec<(String, &'ast ast::Instantiation, Span)>,
    ) {
        match &b.label {
            Some(l) => {
                let inner = format!("{prefix}{}.", l.name);
                self.collect_instances(&b.items, &inner, out);
            }
            None => self.collect_instances(&b.items, prefix, out),
        }
    }
}
