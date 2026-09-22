//! Structural hashing: common subexpression and duplicate cell merging.
//!
//! [`Merge`] visits every reachable expression node bottom-up and gives
//! structurally identical nodes (same kind, same type, same operand ids
//! after merging) one id. Commutative operators (`and`, `or`, `xor`,
//! `xnor`, `land`, `lor`, `add`, `mul`, `eq`, `ne`, `ceq`, `cne`) are
//! normalised by operand order first. Every reference is then redirected
//! to the representative, so later passes can test equality by id.
//!
//! Two whole-net assigns of the same (merged) value make the second net
//! an alias of the first. Cells whose kind, parameters, attributes and
//! (merged) inputs are identical compute the same values: the later ones are dropped and each
//! of their outputs is replaced by an `assign` from the survivor's output,
//! which [`super::ConstFold`] then propagates away. Flip-flops are left to
//! [`super::FfOpt`], which also compares initial values; write ports and
//! black boxes are never merged.

use std::collections::HashMap;

use crate::diag::Diagnostics;
use crate::ir::{
    Assign, BinaryOp, CellKind, ExprId, ExprKind, Lvalue, MemoryId, Module, Name, NetId, Type,
    UnaryOp,
};
use crate::synth::util::{is_kept, mk_net};
use crate::synth::{Pass, PassStats};

/// The structural-hashing pass; see the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct Merge;

impl Pass for Merge {
    fn name(&self) -> &'static str {
        "merge"
    }

    fn run(&self, m: &mut Module, _diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        stats.bump("expressions merged", merge_exprs(m));
        stats.bump("assigns merged", merge_assigns(m));
        stats.bump("cells merged", merge_cells(m));
        stats
    }
}

/// A hashable mirror of [`ExprKind`] (which does not implement `Hash`).
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum ExprKey {
    Const(crate::ir::Const),
    String(String),
    Net(NetId),
    Slice(ExprId, u32, u32),
    Index(ExprId, ExprId),
    IndexedSlice(ExprId, ExprId, u32, bool),
    Concat(Vec<ExprId>),
    Replicate(u32, ExprId),
    Unary(UnaryOp, ExprId),
    Binary(BinaryOp, ExprId, ExprId),
    Ternary(ExprId, ExprId, ExprId),
    Resize(ExprId, u32, bool),
    MemRead(MemoryId, ExprId),
    Call(Name, Vec<ExprId>),
}

impl ExprKey {
    /// The key of a node kind.
    pub(crate) fn of(kind: &ExprKind) -> ExprKey {
        match kind {
            ExprKind::Const(c) => ExprKey::Const(c.clone()),
            ExprKind::String(s) => ExprKey::String(s.clone()),
            ExprKind::Net(n) => ExprKey::Net(*n),
            ExprKind::Slice { base, hi, lo } => ExprKey::Slice(*base, *hi, *lo),
            ExprKind::Index { base, index } => ExprKey::Index(*base, *index),
            ExprKind::IndexedSlice {
                base,
                offset,
                width,
                up,
            } => ExprKey::IndexedSlice(*base, *offset, *width, *up),
            ExprKind::Concat(parts) => ExprKey::Concat(parts.clone()),
            ExprKind::Replicate { count, expr } => ExprKey::Replicate(*count, *expr),
            ExprKind::Unary { op, expr } => ExprKey::Unary(*op, *expr),
            ExprKind::Binary { op, lhs, rhs } => ExprKey::Binary(*op, *lhs, *rhs),
            ExprKind::Ternary { cond, then_, else_ } => ExprKey::Ternary(*cond, *then_, *else_),
            ExprKind::Resize {
                expr,
                width,
                signed,
            } => ExprKey::Resize(*expr, *width, *signed),
            ExprKind::MemRead { mem, addr } => ExprKey::MemRead(*mem, *addr),
            ExprKind::Call { name, args } => ExprKey::Call(name.clone(), args.clone()),
        }
    }
}

/// A hashable key for a cell's kind: its `Debug` rendering, which is
/// injective over the primitive set.
pub(crate) fn cell_kind_key(kind: &CellKind) -> String {
    format!("{kind:?}")
}

fn commutative(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::Xnor
            | BinaryOp::LogicAnd
            | BinaryOp::LogicOr
            | BinaryOp::Add
            | BinaryOp::Mul
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::CaseEq
            | BinaryOp::CaseNe
    )
}

/// Merges identical expression nodes; returns how many were redirected.
fn merge_exprs(m: &mut Module) -> u64 {
    let mut order = Vec::new();
    m.for_each_expr(|id, _| order.push(id));
    let mut canon: HashMap<ExprId, ExprId> = HashMap::new();
    let mut table: HashMap<(ExprKey, Type), ExprId> = HashMap::new();
    let mut merged = 0u64;
    for id in order {
        let mut kind = m.expr(id).kind.clone();
        super::replace_operands(&mut kind, &mut |slot| {
            if let Some(c) = canon.get(slot) {
                *slot = *c;
            }
        });
        if let ExprKind::Binary { op, lhs, rhs } = &mut kind
            && commutative(*op)
            && lhs > rhs
            && m.expr(*lhs).ty == m.expr(*rhs).ty
        {
            std::mem::swap(lhs, rhs);
        }
        let key = (ExprKey::of(&kind), m.expr(id).ty.clone());
        match table.get(&key) {
            Some(rep) if *rep != id => {
                canon.insert(id, *rep);
                merged += 1;
            }
            Some(_) => {}
            None => {
                table.insert(key, id);
            }
        }
    }
    if merged == 0 {
        return 0;
    }
    m.map_exprs(|id| canon.get(&id).copied().unwrap_or(id));
    merged
}

/// Turns repeated whole-net assigns of one value into aliases of the
/// first; returns how many were rewritten.
fn merge_assigns(m: &mut Module) -> u64 {
    let mut first: HashMap<ExprId, crate::ir::NetId> = HashMap::new();
    let mut rewrites: Vec<(usize, crate::ir::NetId)> = Vec::new();
    for (i, a) in m.assigns.iter().enumerate() {
        let Lvalue::Net(target) = a.target else {
            continue;
        };
        if a.delay.is_some() || is_kept(&a.attrs) {
            continue;
        }
        if matches!(m.expr(a.value).kind, ExprKind::Net(_) | ExprKind::Const(_)) {
            continue;
        }
        match first.get(&a.value) {
            Some(rep) if m.nets[*rep].ty == m.nets[target].ty => rewrites.push((i, *rep)),
            Some(_) => {}
            None => {
                first.insert(a.value, target);
            }
        }
    }
    for (i, rep) in &rewrites {
        let span = m.assigns[*i].span;
        let alias = mk_net(m, *rep, span);
        m.assigns[*i].value = alias;
    }
    u64::try_from(rewrites.len()).unwrap_or(u64::MAX)
}

/// Merges identical cells; returns how many were dropped.
fn merge_cells(m: &mut Module) -> u64 {
    type Key = (
        String,
        Vec<(Name, ExprId)>,
        Vec<(Name, crate::ir::AttrValue)>,
    );
    let mut table: HashMap<Key, crate::ir::CellId> = HashMap::new();
    let mut doomed = Vec::new();
    let mut aliases: Vec<(crate::ir::NetId, crate::ir::NetId, crate::source::Span)> = Vec::new();
    for (id, cell) in m.cells.iter() {
        if is_kept(&cell.attrs)
            || matches!(
                cell.kind,
                CellKind::MemWrPort { .. } | CellKind::Blackbox(_) | CellKind::Dff { .. }
            )
        {
            continue;
        }
        let attrs: Vec<(Name, crate::ir::AttrValue)> = cell
            .attrs
            .iter()
            .chain(cell.params.iter())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let key = (cell_kind_key(&cell.kind), cell.inputs.clone(), attrs);
        match table.get(&key) {
            Some(rep) => {
                let rep_cell = &m.cells[*rep];
                let same_outputs = rep_cell.outputs.len() == cell.outputs.len()
                    && rep_cell
                        .outputs
                        .iter()
                        .zip(&cell.outputs)
                        .all(|((a, na), (b, nb))| a == b && m.nets[*na].ty == m.nets[*nb].ty);
                if !same_outputs {
                    continue;
                }
                for ((_, dup), (_, keep)) in cell.outputs.iter().zip(&rep_cell.outputs) {
                    aliases.push((*dup, *keep, cell.span));
                }
                doomed.push(id);
            }
            None => {
                table.insert(key, id);
            }
        }
    }
    if doomed.is_empty() {
        return 0;
    }
    for (dup, keep, span) in aliases {
        let value = mk_net(m, keep, span);
        m.assigns.push(Assign {
            target: Lvalue::Net(dup),
            value,
            delay: None,
            attrs: crate::ir::Attrs::new(),
            span,
        });
    }
    m.cells.retain(|id, _| !doomed.contains(&id));
    u64::try_from(doomed.len()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Type;
    use crate::ir::builder::ModuleBuilder;
    use crate::synth::opt::testutil::{run, span};

    #[test]
    fn merges_expressions_and_cells() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let z = b.output("z", Type::bits(4));
        let t1 = b.add_net("t1", Type::bits(4));
        let t2 = b.add_net("t2", Type::bits(4));
        let (an, cn) = (b.net(a), b.net(c));
        let an2 = b.net(a);
        let cn2 = b.net(c);
        let s1 = b.add(an, cn);
        let s2 = b.add(cn2, an2);
        b.assign(y, s1);
        b.assign(z, s2);
        b.cell2("x1", CellKind::Xor, an, cn, t1);
        b.cell2("x2", CellKind::Xor, an2, cn2, t2);
        let mut m = b.finish();
        let stats = run(&Merge, &mut m);
        assert_eq!(stats.get("cells merged"), 1);
        assert!(stats.get("expressions merged") >= 3);
        assert_eq!(stats.get("assigns merged"), 1);
        assert_eq!(m.cells.len(), 1);
        assert!(m.to_text().contains("assign %z = %y"));
        assert!(m.to_text().contains("assign %t2 = %t1"));
        let again = run(&Merge, &mut m);
        assert!(!again.changed);
    }

    #[test]
    fn does_not_merge_different_types_or_kept_cells() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let s = b.input("s", Type::sbits(4));
        let y = b.output("y", Type::bits(4));
        let z = b.output("z", Type::bits(4));
        let t1 = b.add_net("t1", Type::bits(4));
        let t2 = b.add_net("t2", Type::bits(4));
        let (an, sn) = (b.net(a), b.net(s));
        let x1 = b.xor(an, an);
        let x2 = b.xor(sn, sn);
        b.assign(y, x1);
        let r = b.resize(x2, 4, false);
        b.assign(z, r);
        let c1 = b.cell2("k1", CellKind::And, an, an, t1);
        b.cell2("k2", CellKind::And, an, an, t2);
        b.module_mut().cells[c1].attrs.set("keep", 1);
        let mut m = b.finish();
        let stats = run(&Merge, &mut m);
        assert_eq!(stats.get("cells merged"), 0);
        assert_ne!(m.assigns[0].value, m.assigns[1].value);
    }
}
