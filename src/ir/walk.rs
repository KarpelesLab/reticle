//! Traversals and structural edits that passes share.
//!
//! Three families of helpers live here:
//!
//! - **Visitors** over expressions and statements: [`Module::for_each_expr`],
//!   [`Module::for_each_root_expr`], [`Module::for_each_stmt`] and the
//!   block-level [`walk_block`].
//! - **Id rewriting**: [`Module::map_nets`] and [`Module::map_exprs`] visit
//!   every slot holding a [`NetId`] or [`ExprId`] so a pass that renumbers
//!   or merges objects can fix up all references in one call. Built on them
//!   are [`Module::rename_net`], [`Module::remove_unused_nets`] and
//!   [`Module::gc_exprs`].
//! - **Hierarchy queries** on [`Design`]: [`Design::instances_of`],
//!   [`Design::children`] and [`Design::topological_order`].
//!
//! Flattening, unique-ification and hierarchical path lookup build on
//! these primitives but live next door in [`super::hier`].

use super::Name;
use super::design::{Design, InstanceId, Module, ModuleId, ModuleRef, NetId};
use super::expr::{Expr, ExprId, ExprKind, operands};
use super::process::{Block, Lvalue, ProcessKind, Stmt, StmtKind, WaitKind};

/// Calls `f` for every statement in `block`, depth first, parents before
/// their nested blocks.
pub fn walk_block<'a>(block: &'a Block, f: &mut dyn FnMut(&'a Stmt)) {
    for stmt in block {
        f(stmt);
        for nested in stmt.blocks() {
            walk_block(nested, f);
        }
    }
}

/// Calls `f` for every statement in `block` with mutable access, parents
/// before their nested blocks.
pub fn walk_block_mut(block: &mut Block, f: &mut dyn FnMut(&mut Stmt)) {
    for stmt in block {
        f(stmt);
        for nested in stmt.blocks_mut() {
            walk_block_mut(nested, f);
        }
    }
}

/// The expression ids an lvalue holds directly (indices and addresses).
pub fn lvalue_exprs(lv: &Lvalue, f: &mut dyn FnMut(ExprId)) {
    match lv {
        Lvalue::Net(_) | Lvalue::Slice { .. } => {}
        Lvalue::Index { index, .. } => f(*index),
        Lvalue::Concat(parts) => parts.iter().for_each(|p| lvalue_exprs(p, f)),
        Lvalue::MemElem { addr, .. } => f(*addr),
    }
}

/// The expression ids a statement holds directly, not counting nested
/// statements.
pub fn stmt_exprs(stmt: &Stmt, f: &mut dyn FnMut(ExprId)) {
    match &stmt.kind {
        StmtKind::Assign { target, value, .. } => {
            lvalue_exprs(target, f);
            f(*value);
        }
        StmtKind::If { cond, .. } => f(*cond),
        StmtKind::Case { subject, arms, .. } => {
            f(*subject);
            for arm in arms {
                arm.values.iter().copied().for_each(&mut *f);
            }
        }
        StmtKind::For {
            init, cond, step, ..
        } => {
            for (lv, e) in init.iter().chain(step.iter()) {
                lvalue_exprs(lv, f);
                f(*e);
            }
            if let Some(c) = cond {
                f(*c);
            }
        }
        StmtKind::While { cond, .. } => f(*cond),
        StmtKind::Repeat { count, .. } => f(*count),
        StmtKind::Wait(WaitKind::Delay(e) | WaitKind::Until(e)) => f(*e),
        StmtKind::SysCall { args, .. } => args.iter().copied().for_each(f),
        StmtKind::MemFile {
            file, start, end, ..
        } => {
            f(*file);
            start.iter().chain(end.iter()).copied().for_each(f);
        }
        StmtKind::MemWrite {
            addr,
            value,
            enable,
            ..
        } => {
            f(*addr);
            f(*value);
            if let Some(en) = enable {
                f(*en);
            }
        }
        StmtKind::Assert { cond, message, .. } => {
            f(*cond);
            message.iter().copied().for_each(f);
        }
        StmtKind::Forever { .. }
        | StmtKind::Block { .. }
        | StmtKind::Wait(WaitKind::Event(_))
        | StmtKind::Finish
        | StmtKind::Stop
        | StmtKind::Break
        | StmtKind::Continue => {}
    }
}

/// The net ids a statement holds directly (assignment targets and event
/// waits), not counting nested statements or expressions.
pub fn stmt_nets(stmt: &Stmt, f: &mut dyn FnMut(NetId)) {
    match &stmt.kind {
        StmtKind::Assign { target, .. } => target.nets().into_iter().for_each(f),
        StmtKind::For { init, step, .. } => {
            for (lv, _) in init.iter().chain(step.iter()) {
                lv.nets().into_iter().for_each(&mut *f);
            }
        }
        StmtKind::Wait(WaitKind::Event(edges)) => edges.iter().for_each(|e| f(e.net)),
        _ => {}
    }
}

/// Calls `f` on every [`ExprId`] slot held directly by an lvalue.
fn lvalue_expr_slots(lv: &mut Lvalue, f: &mut dyn FnMut(&mut ExprId)) {
    match lv {
        Lvalue::Net(_) | Lvalue::Slice { .. } => {}
        Lvalue::Index { index, .. } => f(index),
        Lvalue::Concat(parts) => parts.iter_mut().for_each(|p| lvalue_expr_slots(p, f)),
        Lvalue::MemElem { addr, .. } => f(addr),
    }
}

/// Calls `f` on every [`NetId`] slot held directly by an lvalue.
fn lvalue_net_slots(lv: &mut Lvalue, f: &mut dyn FnMut(&mut NetId)) {
    match lv {
        Lvalue::Net(net) | Lvalue::Slice { net, .. } | Lvalue::Index { net, .. } => f(net),
        Lvalue::Concat(parts) => parts.iter_mut().for_each(|p| lvalue_net_slots(p, f)),
        Lvalue::MemElem { .. } => {}
    }
}

/// Calls `f` on every [`ExprId`] slot held directly by a statement.
fn stmt_expr_slots(stmt: &mut Stmt, f: &mut dyn FnMut(&mut ExprId)) {
    match &mut stmt.kind {
        StmtKind::Assign { target, value, .. } => {
            lvalue_expr_slots(target, f);
            f(value);
        }
        StmtKind::If { cond, .. } => f(cond),
        StmtKind::Case { subject, arms, .. } => {
            f(subject);
            for arm in arms {
                arm.values.iter_mut().for_each(&mut *f);
            }
        }
        StmtKind::For {
            init, cond, step, ..
        } => {
            for (lv, e) in init.iter_mut().chain(step.iter_mut()) {
                lvalue_expr_slots(lv, f);
                f(e);
            }
            if let Some(c) = cond {
                f(c);
            }
        }
        StmtKind::While { cond, .. } => f(cond),
        StmtKind::Repeat { count, .. } => f(count),
        StmtKind::Wait(WaitKind::Delay(e) | WaitKind::Until(e)) => f(e),
        StmtKind::SysCall { args, .. } => args.iter_mut().for_each(f),
        StmtKind::MemFile {
            file, start, end, ..
        } => {
            f(file);
            start.iter_mut().chain(end.iter_mut()).for_each(f);
        }
        StmtKind::MemWrite {
            addr,
            value,
            enable,
            ..
        } => {
            f(addr);
            f(value);
            if let Some(en) = enable {
                f(en);
            }
        }
        StmtKind::Assert { cond, message, .. } => {
            f(cond);
            message.iter_mut().for_each(f);
        }
        StmtKind::Forever { .. }
        | StmtKind::Block { .. }
        | StmtKind::Wait(WaitKind::Event(_))
        | StmtKind::Finish
        | StmtKind::Stop
        | StmtKind::Break
        | StmtKind::Continue => {}
    }
}

/// Calls `f` on every [`NetId`] slot held directly by a statement.
fn stmt_net_slots(stmt: &mut Stmt, f: &mut dyn FnMut(&mut NetId)) {
    match &mut stmt.kind {
        StmtKind::Assign { target, .. } => lvalue_net_slots(target, f),
        StmtKind::For { init, step, .. } => {
            for (lv, _) in init.iter_mut().chain(step.iter_mut()) {
                lvalue_net_slots(lv, f);
            }
        }
        StmtKind::Wait(WaitKind::Event(edges)) => {
            for edge in edges {
                f(&mut edge.net);
            }
        }
        _ => {}
    }
}

/// Calls `f` on every operand slot of an expression node.
fn operand_slots(kind: &mut ExprKind, f: &mut dyn FnMut(&mut ExprId)) {
    match kind {
        ExprKind::Const(_) | ExprKind::String(_) | ExprKind::Net(_) => {}
        ExprKind::Slice { base, .. } => f(base),
        ExprKind::Index { base, index } => {
            f(base);
            f(index);
        }
        ExprKind::IndexedSlice { base, offset, .. } => {
            f(base);
            f(offset);
        }
        ExprKind::Concat(parts) => parts.iter_mut().for_each(f),
        ExprKind::Replicate { expr, .. } | ExprKind::Unary { expr, .. } => f(expr),
        ExprKind::Binary { lhs, rhs, .. } => {
            f(lhs);
            f(rhs);
        }
        ExprKind::Ternary { cond, then_, else_ } => {
            f(cond);
            f(then_);
            f(else_);
        }
        ExprKind::Resize { expr, .. } => f(expr),
        ExprKind::MemRead { addr, .. } => f(addr),
        ExprKind::Call { args, .. } => args.iter_mut().for_each(f),
    }
}

impl Module {
    /// Calls `f` on every expression id referenced from outside the
    /// expression arena: assignment values and targets, cell inputs,
    /// instance connections and statements, in that order. Operands are not
    /// visited; see [`Module::for_each_expr`] for that.
    pub fn for_each_root_expr(&self, mut f: impl FnMut(ExprId)) {
        for assign in &self.assigns {
            lvalue_exprs(&assign.target, &mut f);
            f(assign.value);
        }
        for (_, cell) in self.cells.iter() {
            cell.inputs.iter().for_each(|(_, e)| f(*e));
        }
        for (_, inst) in self.instances.iter() {
            inst.connections.iter().for_each(|(_, e)| f(*e));
        }
        for (_, process) in self.processes.iter() {
            walk_block(&process.body, &mut |stmt| stmt_exprs(stmt, &mut f));
        }
    }

    /// Calls `f` on every expression node reachable from a root, each node
    /// once, operands before the nodes that use them.
    pub fn for_each_expr(&self, mut f: impl FnMut(ExprId, &Expr)) {
        let mut seen = vec![false; self.exprs.len()];
        let mut roots = Vec::new();
        self.for_each_root_expr(|id| roots.push(id));
        for root in roots {
            self.visit_expr(root, &mut seen, &mut f);
        }
    }

    fn visit_expr(&self, id: ExprId, seen: &mut [bool], f: &mut impl FnMut(ExprId, &Expr)) {
        let Some(expr) = self.exprs.get(id) else {
            return;
        };
        if seen[id.index()] {
            return;
        }
        seen[id.index()] = true;
        for operand in operands(&expr.kind) {
            self.visit_expr(operand, seen, f);
        }
        f(id, expr);
    }

    /// Calls `f` on every net id held outside the expression arena: ports,
    /// assignment targets, cell outputs, process triggers and statements.
    pub fn for_each_root_net(&self, mut f: impl FnMut(NetId)) {
        self.ports.iter().for_each(|p| f(p.net));
        for assign in &self.assigns {
            assign.target.nets().into_iter().for_each(&mut f);
        }
        for (_, cell) in self.cells.iter() {
            cell.outputs.iter().for_each(|(_, n)| f(*n));
        }
        for (_, process) in self.processes.iter() {
            match &process.kind {
                ProcessKind::Sequential { clocks, resets } => {
                    clocks.iter().chain(resets).for_each(|e| f(e.net));
                }
                ProcessKind::Sensitive(nets) => nets.iter().copied().for_each(&mut f),
                ProcessKind::Comb | ProcessKind::Initial | ProcessKind::Free => {}
            }
            walk_block(&process.body, &mut |stmt| stmt_nets(stmt, &mut f));
        }
    }

    /// Calls `f` on every statement of every process, depth first.
    pub fn for_each_stmt<'a>(&'a self, mut f: impl FnMut(&'a Stmt)) {
        for (_, process) in self.processes.iter() {
            walk_block(&process.body, &mut f);
        }
    }

    /// Calls `f` on every statement of every process with mutable access.
    pub fn for_each_stmt_mut(&mut self, mut f: impl FnMut(&mut Stmt)) {
        for (_, process) in self.processes.iter_mut() {
            walk_block_mut(&mut process.body, &mut f);
        }
    }

    /// Rewrites every net reference in the module through `f`: ports,
    /// expressions, lvalues, cell outputs, process triggers and waits.
    pub fn map_nets(&mut self, mut f: impl FnMut(NetId) -> NetId) {
        let mut g = |slot: &mut NetId| *slot = f(*slot);
        for port in &mut self.ports {
            g(&mut port.net);
        }
        for assign in &mut self.assigns {
            lvalue_net_slots(&mut assign.target, &mut g);
        }
        for (_, cell) in self.cells.iter_mut() {
            cell.outputs.iter_mut().for_each(|(_, n)| g(n));
        }
        for (_, process) in self.processes.iter_mut() {
            match &mut process.kind {
                ProcessKind::Sequential { clocks, resets } => {
                    for edge in clocks.iter_mut().chain(resets.iter_mut()) {
                        g(&mut edge.net);
                    }
                }
                ProcessKind::Sensitive(nets) => nets.iter_mut().for_each(&mut g),
                ProcessKind::Comb | ProcessKind::Initial | ProcessKind::Free => {}
            }
            walk_block_mut(&mut process.body, &mut |stmt| stmt_net_slots(stmt, &mut g));
        }
        for (_, expr) in self.exprs.iter_mut() {
            if let ExprKind::Net(net) = &mut expr.kind {
                g(net);
            }
        }
    }

    /// Rewrites every expression reference held outside the arena and every
    /// operand inside it through `f`.
    pub fn map_exprs(&mut self, mut f: impl FnMut(ExprId) -> ExprId) {
        let mut g = |slot: &mut ExprId| *slot = f(*slot);
        for assign in &mut self.assigns {
            lvalue_expr_slots(&mut assign.target, &mut g);
            g(&mut assign.value);
        }
        for (_, cell) in self.cells.iter_mut() {
            cell.inputs.iter_mut().for_each(|(_, e)| g(e));
        }
        for (_, inst) in self.instances.iter_mut() {
            inst.connections.iter_mut().for_each(|(_, e)| g(e));
        }
        for (_, process) in self.processes.iter_mut() {
            walk_block_mut(&mut process.body, &mut |stmt| stmt_expr_slots(stmt, &mut g));
        }
        for (_, expr) in self.exprs.iter_mut() {
            operand_slots(&mut expr.kind, &mut g);
        }
    }

    /// Renames a net. The caller keeps names unique.
    pub fn rename_net(&mut self, net: NetId, name: impl Into<Name>) {
        self.nets[net].name = name.into();
    }

    /// True for each net that a port exposes or that anything references.
    pub fn used_nets(&self) -> Vec<bool> {
        let mut used = vec![false; self.nets.len()];
        let mut mark = |id: NetId| {
            if let Some(slot) = used.get_mut(id.index()) {
                *slot = true;
            }
        };
        self.for_each_root_net(&mut mark);
        self.for_each_expr(|_, expr| {
            if let ExprKind::Net(net) = expr.kind {
                mark(net);
            }
        });
        used
    }

    /// Removes every net that no port exposes and nothing references, then
    /// renumbers the rest. Returns the number of nets removed.
    pub fn remove_unused_nets(&mut self) -> usize {
        let used = self.used_nets();
        let before = self.nets.len();
        let remap = self.nets.retain(|id, _| used[id.index()]);
        // Only unreferenced nets were removed, so every remaining slot maps
        // to a kept net; unreachable expression nodes may still name a
        // removed net, and those are rewritten to the first net (they are
        // garbage and `gc_exprs` drops them).
        let fallback = NetId(0);
        self.map_nets(|old| remap[old.index()].unwrap_or(fallback));
        before - self.nets.len()
    }

    /// Drops expression nodes that no root reaches and renumbers the rest.
    /// Returns the number of nodes removed.
    pub fn gc_exprs(&mut self) -> usize {
        let mut live = vec![false; self.exprs.len()];
        self.for_each_expr(|id, _| live[id.index()] = true);
        let before = self.exprs.len();
        let remap = self.exprs.retain(|id, _| live[id.index()]);
        self.map_exprs(|old| remap[old.index()].expect("dead expression still referenced"));
        before - self.exprs.len()
    }
}

impl Design {
    /// Every instance of `module` in the design, as `(parent, instance)`
    /// pairs in parent order.
    pub fn instances_of(&self, module: ModuleId) -> Vec<(ModuleId, InstanceId)> {
        let mut result = Vec::new();
        for (parent, m) in self.modules.iter() {
            for (id, inst) in m.instances.iter() {
                if inst.module == ModuleRef::Resolved(module) {
                    result.push((parent, id));
                }
            }
        }
        result
    }

    /// The modules `module` instantiates directly, deduplicated, in first
    /// use order. Unresolved references are skipped.
    pub fn children(&self, module: ModuleId) -> Vec<ModuleId> {
        let mut children = Vec::new();
        for (_, inst) in self.modules[module].instances.iter() {
            if let Some(id) = inst.module.id()
                && !children.contains(&id)
            {
                children.push(id);
            }
        }
        children
    }

    /// Orders modules so every module comes after everything it
    /// instantiates (leaves first). Modules that nothing reaches are
    /// included too. Fails with a module on the cycle when the hierarchy is
    /// recursive.
    pub fn topological_order(&self) -> Result<Vec<ModuleId>, ModuleId> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            None,
            Active,
            Done,
        }
        let mut marks = vec![Mark::None; self.modules.len()];
        let mut order = Vec::with_capacity(self.modules.len());
        // Iterative depth-first search with an explicit stack, so a deep
        // hierarchy cannot overflow the call stack.
        for start in self.modules.ids() {
            if marks[start.index()] != Mark::None {
                continue;
            }
            let mut stack: Vec<(ModuleId, Vec<ModuleId>, usize)> =
                vec![(start, self.children(start), 0)];
            marks[start.index()] = Mark::Active;
            while let Some((node, children, next)) = stack.last_mut() {
                if let Some(child) = children.get(*next).copied() {
                    *next += 1;
                    match marks[child.index()] {
                        Mark::None => {
                            marks[child.index()] = Mark::Active;
                            stack.push((child, self.children(child), 0));
                        }
                        Mark::Active => return Err(child),
                        Mark::Done => {}
                    }
                } else {
                    marks[node.index()] = Mark::Done;
                    order.push(*node);
                    stack.pop();
                }
            }
        }
        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Attrs;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::design::{Instance, PortDir};
    use crate::ir::process::{AssignKind, Edge};
    use crate::ir::types::Type;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    #[test]
    fn walks_and_removes_unused_nets() {
        let span = span();
        let mut b = ModuleBuilder::new("m", span);
        let a = b.input("a", Type::bits(4));
        let _unused = b.add_net("unused", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let clk = b.input("clk", Type::bit());
        let tmp = b.add_net("tmp", Type::bits(4));
        let an = b.net(a);
        let tn = b.net(tmp);
        let one = b.const_u64(4, 1);
        let sum = b.add(an, one);
        b.assign(tmp, sum);
        let mut p = b.process(Some("p"), ProcessKind::posedge(clk));
        p.assign(y, tn, AssignKind::NonBlocking);
        b.end_process(p);
        // A dangling expression nobody references.
        b.const_u64(8, 3);
        let mut m = b.finish();

        let mut count = 0;
        m.for_each_stmt(|_| count += 1);
        assert_eq!(count, 1);
        let mut nodes = Vec::new();
        m.for_each_expr(|id, _| nodes.push(id));
        assert_eq!(nodes.len(), 4);
        let mut roots = Vec::new();
        m.for_each_root_expr(|id| roots.push(id));
        assert_eq!(roots, [sum, tn]);

        assert_eq!(m.remove_unused_nets(), 1);
        assert_eq!(m.nets.len(), 4);
        assert_eq!(m.net_by_name("unused"), None);
        assert_eq!(m.gc_exprs(), 1);
        assert_eq!(m.exprs.len(), 4);
        m.rename_net(m.net_by_name("tmp").unwrap(), "t2");
        assert!(m.net_by_name("t2").is_some());
        assert_eq!(m.ports.len(), 3);
        assert_eq!(m.port("clk").map(|p| p.dir), Some(PortDir::In));
        // The process trigger still points at the (renumbered) clock.
        let ProcessKind::Sequential { clocks, .. } = &m.processes.values().next().unwrap().kind
        else {
            panic!("kind")
        };
        assert_eq!(clocks, &[Edge::pos(m.net_by_name("clk").unwrap())]);
        assert!(crate::ir::validate::validate_module(&m).is_empty());
        let mut visited = 0;
        m.for_each_stmt_mut(|_| visited += 1);
        assert_eq!(visited, 1);
    }

    #[test]
    fn hierarchy_queries() {
        let span = span();
        let mut design = Design::new();
        let leaf = design.add_module(Module::new("leaf", span));
        let mid = design.add_module(Module::new("mid", span));
        let top = design.add_module(Module::new("top", span));
        let inst = |m: ModuleRef| Instance {
            name: Name::new("u"),
            module: m,
            connections: Vec::new(),
            params: Attrs::new(),
            attrs: Attrs::new(),
            span,
        };
        design.modules[top]
            .instances
            .push(inst(ModuleRef::Resolved(mid)));
        design.modules[top]
            .instances
            .push(inst(ModuleRef::Resolved(leaf)));
        design.modules[mid]
            .instances
            .push(inst(ModuleRef::Resolved(leaf)));
        design.modules[mid]
            .instances
            .push(inst(ModuleRef::Unresolved(Name::new("bb"))));
        assert_eq!(
            design.instances_of(leaf),
            [(mid, InstanceId(0)), (top, InstanceId(1))]
        );
        assert_eq!(design.children(top), [mid, leaf]);
        assert_eq!(design.topological_order(), Ok(vec![leaf, mid, top]));
        design.modules[leaf]
            .instances
            .push(inst(ModuleRef::Resolved(top)));
        assert!(design.topological_order().is_err());
    }
}
