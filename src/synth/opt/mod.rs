//! Technology-independent optimisation passes.
//!
//! Each pass is a [`Pass`](super::Pass) over one module and is safe to
//! run in any order and any number of times; [`super::run`] iterates them
//! to a fixpoint. They work on both forms the IR can hold after process
//! lowering: expression trees on assigns and cell inputs, and cells.
//!
//! | Pass          | What it does                                                        |
//! |---------------|---------------------------------------------------------------------|
//! | [`ConstFold`] | Evaluates constant subtrees, applies identities, folds cells with constant inputs to assigns, propagates constant and alias assigns |
//! | [`Dce`]       | Removes cells, assigns, nets, memories and expressions nothing observable depends on |
//! | [`Merge`]     | Shares structurally identical expressions and cells (CSE / structural hashing) |
//! | [`WReduce`]   | Narrows arithmetic and logic whose upper bits are unused or provably zero |
//! | [`FfOpt`]     | Simplifies flip-flops: constant / feedback D, enable and sync-reset extraction, unused and duplicate registers |

pub(crate) mod const_fold;
mod dce;
mod ff_opt;
pub(crate) mod merge;
mod wreduce;

pub use const_fold::ConstFold;
pub use dce::Dce;
pub use ff_opt::FfOpt;
pub use merge::Merge;
pub use wreduce::WReduce;

use std::collections::HashMap;

use crate::ir::{ExprId, Module};

/// Rewrites every reachable expression bottom-up through `simplify`, which
/// receives a node whose operands are already rewritten and returns its
/// replacement (possibly itself). Root slots are updated afterwards; the
/// number of root slots that changed is returned.
///
/// `simplify` must return a node of the same type as the one it is given,
/// since parents were typed against the original.
pub(crate) fn rewrite_exprs(
    m: &mut Module,
    mut simplify: impl FnMut(&mut Module, ExprId) -> ExprId,
) -> usize {
    let mut order = Vec::new();
    m.for_each_expr(|id, _| order.push(id));
    let mut memo: HashMap<ExprId, ExprId> = HashMap::new();
    for id in order {
        // Rebuild the node over its rewritten operands when any changed.
        let kind = m.expr(id).kind.clone();
        let ops = crate::ir::expr::operands(&kind);
        let new_ops: Vec<ExprId> = ops
            .iter()
            .map(|o| memo.get(o).copied().unwrap_or(*o))
            .collect();
        let base = if new_ops == ops {
            id
        } else {
            let mut kind = kind;
            let mut i = 0;
            replace_operands(&mut kind, &mut |slot| {
                *slot = new_ops[i];
                i += 1;
            });
            let (ty, span) = (m.expr(id).ty.clone(), m.expr(id).span);
            m.add_expr(crate::ir::Expr::new(kind, ty, span))
        };
        let out = simplify(m, base);
        debug_assert_eq!(
            m.expr(out).ty,
            m.expr(id).ty,
            "simplify changed the type of {id}"
        );
        if out != id {
            memo.insert(id, out);
        }
    }
    let mut changed = 0;
    m.map_exprs(|id| match memo.get(&id) {
        Some(new) => {
            changed += 1;
            *new
        }
        None => id,
    });
    changed
}

/// Calls `f` on every operand slot of `kind`, in
/// [`crate::ir::expr::operands`] order.
pub(crate) fn replace_operands(kind: &mut crate::ir::ExprKind, f: &mut dyn FnMut(&mut ExprId)) {
    use crate::ir::ExprKind;
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

/// Number of times each net is referenced from reachable expressions.
pub(crate) fn expr_net_refs(m: &Module) -> Vec<u32> {
    let mut refs = vec![0u32; m.nets.len()];
    let mut parents: HashMap<ExprId, u32> = HashMap::new();
    m.for_each_root_expr(|id| *parents.entry(id).or_insert(0) += 1);
    m.for_each_expr(|_, e| {
        for op in crate::ir::expr::operands(&e.kind) {
            *parents.entry(op).or_insert(0) += 1;
        }
    });
    m.for_each_expr(|id, e| {
        if let crate::ir::ExprKind::Net(n) = e.kind {
            refs[n.index()] += parents.get(&id).copied().unwrap_or(0);
        }
    });
    refs
}

/// Number of times each net appears in a root net slot (ports, assignment
/// targets, cell outputs, process triggers and statements).
pub(crate) fn root_net_refs(m: &Module) -> Vec<u32> {
    let mut refs = vec![0u32; m.nets.len()];
    m.for_each_root_net(|n| refs[n.index()] += 1);
    refs
}

#[cfg(test)]
pub(crate) mod testutil {
    use crate::diag::Diagnostics;
    use crate::ir::Module;
    use crate::ir::validate::validate_module;
    use crate::source::{SourceMap, Span};
    use crate::synth::{Pass, PassStats};

    /// A span into an empty scratch file.
    pub(crate) fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    /// Runs a pass and asserts the module is still valid.
    pub(crate) fn run(pass: &dyn Pass, m: &mut Module) -> PassStats {
        let mut diags = Diagnostics::new();
        let stats = pass.run(m, &mut diags);
        let problems = validate_module(m);
        assert!(
            !problems.has_errors(),
            "{} left the module invalid:\n{}\n{}",
            pass.name(),
            problems
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            m.to_text()
        );
        stats
    }

    /// Runs a pass until it reports no change (at most eight rounds).
    pub(crate) fn run_fix(pass: &dyn Pass, m: &mut Module) -> PassStats {
        let mut total = PassStats::default();
        for _ in 0..8 {
            let s = run(pass, m);
            let changed = s.changed;
            total.merge(&s);
            if !changed {
                break;
            }
        }
        total
    }
}
