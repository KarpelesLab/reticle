//! Dead code elimination.
//!
//! [`Dce`] keeps what an observer can see and drops the rest. Observable
//! are output and inout ports, instance connections, whatever remaining
//! processes read or write, black-box cells, and anything carrying a
//! `keep` attribute. From those roots, liveness flows backwards: a live
//! net makes its drivers (assigns, cell outputs) live, a live cell or
//! assign makes the nets its inputs read live, a live memory read port or
//! `MemRead` makes the memory and all its write ports live.
//!
//! Dead cells, assigns and memories are removed, then unreachable
//! expression nodes and unreferenced nets are garbage collected with the
//! helpers in [`crate::ir::walk`]. Memory ids are renumbered through a
//! local remap, since the walk helpers do not cover memories yet.

use std::collections::HashMap;

use crate::diag::Diagnostics;
use crate::ir::walk::{lvalue_exprs, stmt_exprs, walk_block};
use crate::ir::{
    CellKind, ExprId, ExprKind, Lvalue, MemoryId, Module, NetId, PortDir, expr::operands,
};
use crate::synth::util::is_kept;
use crate::synth::{Pass, PassStats};

/// The dead-code-elimination pass; see the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct Dce;

impl Pass for Dce {
    fn name(&self) -> &'static str {
        "dce"
    }

    fn run(&self, m: &mut Module, _diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        let live = Liveness::compute(m);

        let before = m.cells.len();
        m.cells.retain(|id, _| live.cells[id.index()]);
        stats.bump(
            "cells removed",
            u64::try_from(before - m.cells.len()).unwrap_or(u64::MAX),
        );

        let before = m.assigns.len();
        let mut i = 0;
        m.assigns.retain(|_| {
            let keep = live.assigns[i];
            i += 1;
            keep
        });
        stats.bump(
            "assigns removed",
            u64::try_from(before - m.assigns.len()).unwrap_or(u64::MAX),
        );

        // Collect the expressions first. A dead `MemRead` still names the
        // memory it read, and `map_memories` walks *every* expression, so
        // renumbering before the collection would ask the remap for a
        // memory that is going away — which is not a wrong answer, it is
        // no answer at all. Once the cells and assigns that reached those
        // expressions are gone the expressions are unreachable, so the
        // collection takes them and nothing dead is left holding an id.
        stats.bump(
            "expressions collected",
            u64::try_from(m.gc_exprs()).unwrap_or(u64::MAX),
        );

        let dead_mems = live.memories.iter().filter(|l| !**l).count();
        if dead_mems > 0 {
            let remap = m.memories.retain(|id, _| live.memories[id.index()]);
            map_memories(m, |old| remap[old.index()].expect("live memory kept"));
            stats.bump(
                "memories removed",
                u64::try_from(dead_mems).unwrap_or(u64::MAX),
            );
        }
        stats.bump(
            "nets removed",
            u64::try_from(m.remove_unused_nets()).unwrap_or(u64::MAX),
        );
        stats
    }
}

/// Liveness of every removable object.
struct Liveness {
    nets: Vec<bool>,
    cells: Vec<bool>,
    assigns: Vec<bool>,
    memories: Vec<bool>,
}

impl Liveness {
    fn compute(m: &Module) -> Liveness {
        let mut l = Liveness {
            nets: vec![false; m.nets.len()],
            cells: vec![false; m.cells.len()],
            assigns: vec![false; m.assigns.len()],
            memories: vec![false; m.memories.len()],
        };
        // Drivers of each net.
        let mut net_assigns: HashMap<NetId, Vec<usize>> = HashMap::new();
        for (i, a) in m.assigns.iter().enumerate() {
            for n in a.target.nets() {
                net_assigns.entry(n).or_default().push(i);
            }
        }
        let mut net_cells: HashMap<NetId, Vec<usize>> = HashMap::new();
        for (id, c) in m.cells.iter() {
            for (_, n) in &c.outputs {
                net_cells.entry(*n).or_default().push(id.index());
            }
        }
        let mut mem_writers: HashMap<MemoryId, Vec<usize>> = HashMap::new();
        for (id, c) in m.cells.iter() {
            if let CellKind::MemWrPort { mem, .. } = c.kind {
                mem_writers.entry(mem).or_default().push(id.index());
            }
        }

        let mut net_work: Vec<NetId> = Vec::new();
        let mut expr_work: Vec<ExprId> = Vec::new();
        let mut mem_work: Vec<MemoryId> = Vec::new();

        // Roots.
        for p in &m.ports {
            if p.dir != PortDir::In {
                net_work.push(p.net);
            }
        }
        for (_, inst) in m.instances.iter() {
            expr_work.extend(inst.connections.iter().map(|(_, e)| *e));
        }
        for (_, p) in m.processes.iter() {
            walk_block(&p.body, &mut |s| {
                stmt_exprs(s, &mut |e| expr_work.push(e));
                crate::ir::walk::stmt_nets(s, &mut |n| net_work.push(n));
                if let crate::ir::StmtKind::MemWrite { mem, .. }
                | crate::ir::StmtKind::MemFile { mem, .. } = s.kind
                {
                    mem_work.push(mem);
                }
                if let crate::ir::StmtKind::Assign {
                    target: Lvalue::MemElem { mem, .. },
                    ..
                } = s.kind
                {
                    mem_work.push(mem);
                }
            });
            if let crate::ir::ProcessKind::Sequential { clocks, resets } = &p.kind {
                net_work.extend(clocks.iter().chain(resets).map(|e| e.net));
            }
        }
        for (id, n) in m.nets.iter() {
            if is_kept(&n.attrs) {
                net_work.push(id);
            }
        }
        for (id, c) in m.cells.iter() {
            if is_kept(&c.attrs) || matches!(c.kind, CellKind::Blackbox(_)) {
                l.mark_cell(m, id.index(), &mut expr_work, &mut mem_work);
            }
        }
        for (i, a) in m.assigns.iter().enumerate() {
            if is_kept(&a.attrs) {
                l.mark_assign(m, i, &mut expr_work);
            }
        }
        for (id, mem) in m.memories.iter() {
            if is_kept(&mem.attrs) {
                mem_work.push(id);
            }
        }

        let mut seen_exprs = vec![false; m.exprs.len()];
        loop {
            if let Some(e) = expr_work.pop() {
                if seen_exprs[e.index()] {
                    continue;
                }
                seen_exprs[e.index()] = true;
                match &m.expr(e).kind {
                    ExprKind::Net(n) => net_work.push(*n),
                    ExprKind::MemRead { mem, .. } => mem_work.push(*mem),
                    _ => {}
                }
                expr_work.extend(operands(&m.expr(e).kind));
                continue;
            }
            if let Some(n) = net_work.pop() {
                if l.nets[n.index()] {
                    continue;
                }
                l.nets[n.index()] = true;
                for i in net_assigns.get(&n).into_iter().flatten() {
                    l.mark_assign(m, *i, &mut expr_work);
                }
                for i in net_cells.get(&n).into_iter().flatten() {
                    l.mark_cell(m, *i, &mut expr_work, &mut mem_work);
                }
                continue;
            }
            if let Some(mem) = mem_work.pop() {
                if l.memories[mem.index()] {
                    continue;
                }
                l.memories[mem.index()] = true;
                for i in mem_writers.get(&mem).into_iter().flatten() {
                    l.mark_cell(m, *i, &mut expr_work, &mut mem_work);
                }
                continue;
            }
            break;
        }
        l
    }

    fn mark_cell(
        &mut self,
        m: &Module,
        index: usize,
        expr_work: &mut Vec<ExprId>,
        mem_work: &mut Vec<MemoryId>,
    ) {
        if self.cells[index] {
            return;
        }
        self.cells[index] = true;
        let cell = m.cells.values().nth(index).expect("cell index");
        expr_work.extend(cell.inputs.iter().map(|(_, e)| *e));
        match cell.kind {
            CellKind::MemRdPort { mem, .. } | CellKind::MemWrPort { mem, .. } => {
                mem_work.push(mem);
            }
            _ => {}
        }
    }

    fn mark_assign(&mut self, m: &Module, index: usize, expr_work: &mut Vec<ExprId>) {
        if self.assigns[index] {
            return;
        }
        self.assigns[index] = true;
        let a = &m.assigns[index];
        expr_work.push(a.value);
        lvalue_exprs(&a.target, &mut |e| expr_work.push(e));
    }
}

/// Rewrites every memory reference in the module through `f`.
pub(crate) fn map_memories(m: &mut Module, f: impl Fn(MemoryId) -> MemoryId) {
    for (_, e) in m.exprs.iter_mut() {
        if let ExprKind::MemRead { mem, .. } = &mut e.kind {
            *mem = f(*mem);
        }
    }
    for (_, c) in m.cells.iter_mut() {
        match &mut c.kind {
            CellKind::MemRdPort { mem, .. } | CellKind::MemWrPort { mem, .. } => *mem = f(*mem),
            _ => {}
        }
    }
    fn lvalue(lv: &mut Lvalue, f: &dyn Fn(MemoryId) -> MemoryId) {
        match lv {
            Lvalue::MemElem { mem, .. } => *mem = f(*mem),
            Lvalue::Concat(parts) => parts.iter_mut().for_each(|p| lvalue(p, f)),
            _ => {}
        }
    }
    for a in &mut m.assigns {
        lvalue(&mut a.target, &f);
    }
    m.for_each_stmt_mut(|s| match &mut s.kind {
        crate::ir::StmtKind::MemWrite { mem, .. } | crate::ir::StmtKind::MemFile { mem, .. } => {
            *mem = f(*mem);
        }
        crate::ir::StmtKind::Assign { target, .. } => lvalue(target, &f),
        crate::ir::StmtKind::For { init, step, .. } => {
            for (lv, _) in init.iter_mut().chain(step.iter_mut()) {
                lvalue(lv, &f);
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{ModuleRef, Name, Type};
    use crate::synth::opt::testutil::{run, span};

    #[test]
    fn removes_unobservable_logic() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let dead = b.add_net("dead", Type::bits(4));
        let dead2 = b.add_net("dead2", Type::bits(4));
        let kept = b.add_net("kept", Type::bits(4));
        b.net_attr(kept, "keep", 1);
        let an = b.net(a);
        let one = b.const_u64(4, 1);
        let sum = b.add(an, one);
        b.assign(y, sum);
        let mul = b.mul(an, one);
        b.assign(dead, mul);
        let clkn = b.net(clk);
        b.cell(
            "ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: None,
            },
            vec![(Name::new("clk"), clkn), (Name::new("d"), an)],
            vec![(Name::new("q"), dead2)],
        );
        b.assign(kept, an);
        let mem = b.memory("m1", Type::bits(4), 4);
        let unused_mem = b.memory("m2", Type::bits(4), 4);
        let addr = b.const_u64(2, 1);
        b.cell(
            "wr",
            CellKind::MemWrPort {
                mem,
                clocked: false,
            },
            vec![
                (Name::new("addr"), addr),
                (Name::new("data"), an),
                (Name::new("en"), clkn),
            ],
            vec![],
        );
        b.cell(
            "wr2",
            CellKind::MemWrPort {
                mem: unused_mem,
                clocked: false,
            },
            vec![
                (Name::new("addr"), addr),
                (Name::new("data"), an),
                (Name::new("en"), clkn),
            ],
            vec![],
        );
        let rd = b.mem_read(mem, addr);
        let z = b.output("z", Type::bits(4));
        b.assign(z, rd);
        let mut m = b.finish();
        let stats = run(&Dce, &mut m);
        assert_eq!(stats.get("cells removed"), 2);
        assert_eq!(stats.get("assigns removed"), 1);
        assert_eq!(stats.get("memories removed"), 1);
        assert_eq!(stats.get("nets removed"), 2);
        assert!(m.net_by_name("kept").is_some());
        assert!(m.net_by_name("dead").is_none());
        assert!(m.memory_by_name("m2").is_none());
        assert_eq!(m.cells.len(), 1);
        let again = run(&Dce, &mut m);
        assert!(!again.changed);
    }

    #[test]
    fn instances_and_processes_are_roots() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let t = b.add_net("t", Type::bits(4));
        let u = b.add_net("u", Type::bits(4));
        let an = b.net(a);
        let tn = b.net(t);
        let un = b.net(u);
        b.assign(t, an);
        b.assign(u, an);
        b.instance(
            "i0",
            ModuleRef::Unresolved(Name::new("leaf")),
            vec![(Name::new("x"), tn)],
        );
        let v = b.add_reg("v", Type::bits(4));
        let mut p = b.process(None, crate::ir::ProcessKind::Comb);
        p.blocking(v, un);
        b.end_process(p);
        let mut m = b.finish();
        let stats = run(&Dce, &mut m);
        assert_eq!(stats.get("assigns removed"), 0);
        assert_eq!(m.assigns.len(), 2);
    }
}
