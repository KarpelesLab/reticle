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
/// the span of the whole; `None` for anything else. An element of a
/// generate loop or an instance array, `g[2]`, is one part of it, as the IR
/// names that scope.
fn dotted(e: &Expr) -> Option<Vec<String>> {
    match &e.kind {
        ExprKind::Ident(id) => Some(vec![id.name.clone()]),
        ExprKind::Member { base, name } => {
            let mut path = dotted(base)?;
            path.push(name.name.clone());
            Some(path)
        }
        ExprKind::Index { base, index } => {
            let mut path = dotted(base)?;
            let i = literal_index(index)?;
            let last = path.last_mut()?;
            last.push_str(&format!("[{i}]"));
            Some(path)
        }
        _ => None,
    }
}

/// The value of an index written as a decimal number, `2` in `g[2]`.
pub(crate) fn literal_index(e: &Expr) -> Option<i64> {
    match &e.kind {
        ExprKind::Literal(ast::Literal::Number { text, .. })
            if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit() || b == b'_') =>
        {
            text.replace('_', "").parse().ok()
        }
        _ => None,
    }
}

/// An instance a hierarchical name may start at: its name as the IR has it,
/// its instantiation, and the generate loop variables it was made with.
struct Found<'ast> {
    name: String,
    inst: &'ast ast::Instantiation,
    span: Span,
    genvars: Vec<(String, i64)>,
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
        let mut instances: Vec<Found<'ast>> = Vec::new();
        self.collect_instances(items, "", &[], &mut instances);
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
                    .find(|f| f.name == head)
                    .map(|f| (k, f.name.clone(), f.inst, f.span, f.genvars.clone()))
            });
            let Some((k, head, inst, inst_span, genvars)) = found else {
                continue;
            };
            let mut rest = vec![head];
            rest.extend(path[k..].iter().cloned());
            // An instance in a generate loop is elaborated with the loop's
            // variables at that iteration's values, as the loop did.
            let scope = self.env.enter_new(String::new());
            for (var, value) in &genvars {
                self.bind_genvar(scope, var, *value, inst_span);
            }
            let target = self.hier_target(inst, inst_span, &rest);
            self.env.leave();
            let Some((ty, instance_path, net_name, var)) = target else {
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
        genvars: &[(String, i64)],
        out: &mut Vec<Found<'ast>>,
    ) {
        for item in items {
            match &item.kind {
                ItemKind::Instance(inst) if !self.env.cx.table.is_interface(&inst.module.name) => {
                    for one in &inst.instances {
                        let Some(name) = &one.name else { continue };
                        let mut found = |n: String| {
                            out.push(Found {
                                name: n,
                                inst,
                                span: name.span,
                                genvars: genvars.to_vec(),
                            });
                        };
                        match one.dims.as_slice() {
                            [] => found(format!("{prefix}{}", name.name)),
                            // An instance array's elements are `ua[0]`...
                            [dim] => {
                                for i in self.array_indices(dim).unwrap_or_default() {
                                    found(format!("{prefix}{}[{i}]", name.name));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                ItemKind::Generate(inner) => self.collect_instances(inner, prefix, genvars, out),
                ItemKind::GenBlock(b) => self.collect_block(b, prefix, genvars, out),
                ItemKind::GenIf(g) => {
                    self.collect_block(&g.then_block, prefix, genvars, out);
                    if let Some(e) = &g.else_block {
                        self.collect_block(e, prefix, genvars, out);
                    }
                }
                ItemKind::GenFor(g) => self.collect_loop(g, prefix, genvars, out),
                _ => {}
            }
        }
    }

    /// The instances of each iteration of a generate loop, `label[i].u`,
    /// with the loop variable's value there.
    fn collect_loop(
        &mut self,
        g: &'ast ast::GenFor,
        prefix: &str,
        genvars: &[(String, i64)],
        out: &mut Vec<Found<'ast>>,
    ) {
        let Some(label) = g.body.label.as_ref().map(|l| l.name.clone()) else {
            return;
        };
        let scope = self.env.enter_new(String::new());
        for (var, value) in genvars {
            self.bind_genvar(scope, var, *value, g.var.span);
        }
        let start = self.env.probe(|env| {
            super::super::constant::Evaluator::new(env)
                .eval_i64(&g.init)
                .ok()
        });
        let mut value = start;
        let mut count = 0;
        while let Some(v) = value {
            self.bind_genvar(scope, &g.var.name, v, g.var.span);
            let go = self
                .env
                .probe(|env| {
                    super::super::constant::Evaluator::new(env)
                        .eval_self(&g.cond)
                        .ok()
                })
                .is_some_and(|c| c.truth() == crate::logic::Bit::One);
            count += 1;
            if !go || count > 4096 {
                break;
            }
            let mut inner = genvars.to_vec();
            inner.push((g.var.name.clone(), v));
            let p = format!("{prefix}{label}[{v}].");
            self.collect_instances(&g.body.items, &p, &inner, out);
            let next = self.step_genvar(g, v);
            if next == Some(v) {
                break;
            }
            value = next;
        }
        self.env.leave();
    }

    fn collect_block(
        &mut self,
        b: &'ast ast::GenBlock,
        prefix: &str,
        genvars: &[(String, i64)],
        out: &mut Vec<Found<'ast>>,
    ) {
        match &b.label {
            Some(l) => {
                let inner = format!("{prefix}{}.", l.name);
                self.collect_instances(&b.items, &inner, genvars, out);
            }
            None => self.collect_instances(&b.items, prefix, genvars, out),
        }
    }
}
