//! Flip-flop optimisation.
//!
//! [`FfOpt`] rewrites `dff` cells one at a time until nothing applies:
//!
//! - A constant `en` of 1 is dropped; a constant `en` of 0 makes `d` the
//!   feedback `q`.
//! - A constant `rst` that is never active is dropped; one that is always
//!   active turns a synchronous reset into a constant `d`, and an
//!   asynchronous reset into a constant net.
//! - **Sync-reset extraction**: with no reset and no enable, `d =
//!   mux(c, K, rest)` (or `mux(c, rest, K)`) with constant `K` becomes a
//!   synchronous reset on `c`. It runs before enable extraction because
//!   `rst` has priority over `en` in the cell.
//! - **Enable extraction**: `d = mux(c, v, q)` becomes `en = c, d = v`
//!   (`mux(c, q, v)` becomes `en = not(c)`); an existing enable is
//!   ANDed in.
//! - A register whose `d` is its own `q` never changes: it is replaced by
//!   its initial value, its reset value when both agree, or `x`
//!   (don't-care) when it has neither.
//! - A register whose `d` is a constant `K` is replaced by `K` when its
//!   reset value (if any) and initial value (if any) are `K` too, or
//!   absent: before the first clock it holds `x` or `K` either way.
//! - A register whose `q` nothing reads is removed.
//! - Two registers with the same kind, inputs, attributes and initial
//!   value are merged; the second's `q` becomes an alias assign.

use std::collections::HashMap;

use crate::diag::Diagnostics;
use crate::ir::{
    Assign, AttrValue, Attrs, CellId, CellKind, Const, ExprId, ExprKind, Lvalue, Module, Name,
    NetId, Reset, UnaryOp,
};
use crate::synth::opt::{expr_net_refs, root_net_refs};
use crate::synth::util::{is_kept, is_port, mk_and, mk_const, mk_net, mk_not};
use crate::synth::{Pass, PassStats};

/// The flip-flop optimisation pass; see the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct FfOpt;

impl Pass for FfOpt {
    fn name(&self) -> &'static str {
        "ff_opt"
    }

    fn run(&self, m: &mut Module, _diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        let ids: Vec<CellId> = m
            .cells
            .iter()
            .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }) && !is_kept(&c.attrs))
            .map(|(id, _)| id)
            .collect();
        let mut constants: Vec<(CellId, Const)> = Vec::new();
        for id in ids {
            loop {
                match step(m, id, &mut stats) {
                    Step::Again => {}
                    Step::Done => break,
                    Step::Constant(v) => {
                        constants.push((id, v));
                        break;
                    }
                }
            }
        }
        let mut doomed = Vec::new();
        for (id, value) in constants {
            let cell = &m.cells[id];
            let Some(q) = cell.output("q") else {
                continue;
            };
            let span = cell.span;
            let signed = m.nets[q].ty.is_signed();
            let e = mk_const(m, value.with_signed(signed), span);
            m.assigns.push(Assign {
                target: Lvalue::Net(q),
                value: e,
                delay: None,
                attrs: Attrs::new(),
                span,
            });
            doomed.push(id);
            stats.bump("constant registers", 1);
        }
        m.cells.retain(|id, _| !doomed.contains(&id));
        stats.bump("unused registers removed", remove_unused(m));
        stats.bump("registers merged", merge_duplicates(m));
        stats
    }
}

enum Step {
    Again,
    Done,
    Constant(Const),
}

fn const_of(m: &Module, id: ExprId) -> Option<&Const> {
    m.expr(id).as_const()
}

fn const_bit(m: &Module, id: ExprId) -> Option<bool> {
    let c = const_of(m, id)?;
    (c.width() == 1).then(|| c.bit(0).to_bool()).flatten()
}

fn init_of(m: &Module, q: NetId) -> Option<Const> {
    match m.nets[q].attrs.get("init")? {
        AttrValue::Const(c) => Some(c.clone()),
        _ => None,
    }
}

fn set_input(m: &mut Module, id: CellId, port: &str, value: Option<ExprId>) {
    let cell = &mut m.cells[id];
    cell.inputs.retain(|(n, _)| n.as_str() != port);
    if let Some(v) = value {
        cell.inputs.push((Name::new(port), v));
    }
    // Keep the canonical port order.
    const ORDER: [&str; 4] = ["clk", "d", "en", "rst"];
    cell.inputs.sort_by_key(|(n, _)| {
        ORDER
            .iter()
            .position(|p| *p == n.as_str())
            .unwrap_or(usize::MAX)
    });
}

fn step(m: &mut Module, id: CellId, stats: &mut PassStats) -> Step {
    let cell = m.cells[id].clone();
    let CellKind::Dff {
        clk_pos,
        has_enable,
        reset,
    } = cell.kind.clone()
    else {
        return Step::Done;
    };
    let (Some(q), Some(d)) = (cell.output("q"), cell.input("d")) else {
        return Step::Done;
    };
    let span = cell.span;
    let width = m.nets[q].ty.width().unwrap_or(0);
    let init = init_of(m, q);

    // Constant enable.
    if has_enable && let Some(en) = cell.input("en") {
        match const_bit(m, en) {
            Some(true) => {
                set_input(m, id, "en", None);
                m.cells[id].kind = CellKind::Dff {
                    clk_pos,
                    has_enable: false,
                    reset,
                };
                stats.bump("constant enables removed", 1);
                return Step::Again;
            }
            Some(false) => {
                let fb = mk_net(m, q, span);
                set_input(m, id, "d", Some(fb));
                set_input(m, id, "en", None);
                m.cells[id].kind = CellKind::Dff {
                    clk_pos,
                    has_enable: false,
                    reset,
                };
                stats.bump("constant enables removed", 1);
                return Step::Again;
            }
            None => {}
        }
    }

    // Constant reset.
    if let Some(r) = &reset
        && let Some(rst) = cell.input("rst")
        && let Some(level) = const_bit(m, rst)
    {
        let active = level == r.active_high;
        if !active {
            set_input(m, id, "rst", None);
            m.cells[id].kind = CellKind::Dff {
                clk_pos,
                has_enable,
                reset: None,
            };
            stats.bump("constant resets removed", 1);
            return Step::Again;
        }
        if r.asynchronous {
            return Step::Constant(r.value.clone());
        }
        let k = mk_const(m, r.value.clone(), span);
        set_input(m, id, "d", Some(k));
        set_input(m, id, "rst", None);
        set_input(m, id, "en", None);
        m.cells[id].kind = CellKind::Dff {
            clk_pos,
            has_enable: false,
            reset: None,
        };
        stats.bump("constant resets removed", 1);
        return Step::Again;
    }

    // Sync-reset extraction.
    if reset.is_none()
        && !has_enable
        && let ExprKind::Ternary { cond, then_, else_ } = m.expr(d).kind
    {
        let pick = if const_of(m, then_).is_some_and(|c| c.width() == width && c.is_fully_known()) {
            Some((then_, else_, true))
        } else if const_of(m, else_).is_some_and(|c| c.width() == width && c.is_fully_known()) {
            Some((else_, then_, false))
        } else {
            None
        };
        if let Some((k, rest, active_high)) = pick {
            let value = const_of(m, k).cloned().expect("constant branch");
            let (rst, active_high) = match m.expr(cond).kind {
                ExprKind::Unary {
                    op: UnaryOp::Not | UnaryOp::LogicNot,
                    expr,
                } if m.expr(expr).ty.is_bit() => (expr, !active_high),
                _ => (cond, active_high),
            };
            set_input(m, id, "d", Some(rest));
            set_input(m, id, "rst", Some(rst));
            m.cells[id].kind = CellKind::Dff {
                clk_pos,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high,
                    value,
                }),
            };
            stats.bump("sync resets extracted", 1);
            return Step::Again;
        }
    }

    // Enable extraction.
    if let ExprKind::Ternary { cond, then_, else_ } = m.expr(d).kind {
        let feedback = |e: ExprId| m.expr(e).kind == ExprKind::Net(q);
        let pick = if feedback(else_) {
            Some((cond, then_))
        } else if feedback(then_) {
            Some((mk_not(m, cond, span), else_))
        } else {
            None
        };
        if let Some((c, v)) = pick {
            let en = match (has_enable, cell.input("en")) {
                (true, Some(e)) => mk_and(m, e, c, span),
                _ => c,
            };
            set_input(m, id, "d", Some(v));
            set_input(m, id, "en", Some(en));
            m.cells[id].kind = CellKind::Dff {
                clk_pos,
                has_enable: true,
                reset,
            };
            stats.bump("enables extracted", 1);
            return Step::Again;
        }
    }

    // Feedback only: the register never changes.
    if m.expr(d).kind == ExprKind::Net(q) {
        let reset_value = reset.as_ref().map(|r| r.value.clone());
        return match (init, reset_value) {
            (Some(i), Some(r)) if i == r => Step::Constant(i),
            (Some(_), Some(_)) => Step::Done,
            (Some(i), None) => Step::Constant(i),
            (None, Some(r)) => Step::Constant(r),
            (None, None) => Step::Constant(Const::x(width)),
        };
    }

    // Constant data.
    if let Some(k) = const_of(m, d).cloned() {
        let reset_ok = reset.as_ref().is_none_or(|r| r.value == k);
        let init_ok = init.as_ref().is_none_or(|i| *i == k || !i.is_fully_known());
        if reset_ok && init_ok {
            return Step::Constant(k);
        }
    }
    Step::Done
}

/// Removes registers whose output nothing reads.
fn remove_unused(m: &mut Module) -> u64 {
    let expr_refs = expr_net_refs(m);
    let root_refs = root_net_refs(m);
    let mut doomed = Vec::new();
    for (id, cell) in m.cells.iter() {
        if !matches!(cell.kind, CellKind::Dff { .. }) || is_kept(&cell.attrs) {
            continue;
        }
        let Some(q) = cell.output("q") else {
            continue;
        };
        if is_port(m, q) || is_kept(&m.nets[q].attrs) {
            continue;
        }
        // The only root reference is this cell's own output.
        if expr_refs[q.index()] == 0 && root_refs[q.index()] == 1 {
            doomed.push(id);
        }
    }
    m.cells.retain(|id, _| !doomed.contains(&id));
    u64::try_from(doomed.len()).unwrap_or(u64::MAX)
}

/// Merges registers with identical kind, inputs, attributes and init.
fn merge_duplicates(m: &mut Module) -> u64 {
    type Key = (
        String,
        Vec<(Name, ExprId)>,
        Vec<(Name, AttrValue)>,
        Option<Const>,
    );
    let mut table: HashMap<Key, CellId> = HashMap::new();
    let mut doomed = Vec::new();
    let mut aliases = Vec::new();
    for (id, cell) in m.cells.iter() {
        if !matches!(cell.kind, CellKind::Dff { .. }) || is_kept(&cell.attrs) {
            continue;
        }
        let Some(q) = cell.output("q") else {
            continue;
        };
        let attrs: Vec<(Name, AttrValue)> = cell
            .attrs
            .iter()
            .chain(cell.params.iter())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let key = (
            crate::synth::opt::merge::cell_kind_key(&cell.kind),
            cell.inputs.clone(),
            attrs,
            init_of(m, q),
        );
        match table.get(&key) {
            Some(rep) => {
                let keep = m.cells[*rep].output("q").expect("dff output");
                if m.nets[keep].ty == m.nets[q].ty {
                    aliases.push((q, keep, cell.span));
                    doomed.push(id);
                }
            }
            None => {
                table.insert(key, id);
            }
        }
    }
    for (dup, keep, span) in aliases {
        let value = mk_net(m, keep, span);
        m.assigns.push(Assign {
            target: Lvalue::Net(dup),
            value,
            delay: None,
            attrs: Attrs::new(),
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

    fn dff(
        b: &mut ModuleBuilder,
        name: &str,
        clk: ExprId,
        d: ExprId,
        q: NetId,
        en: Option<ExprId>,
        reset: Option<(Reset, ExprId)>,
    ) -> CellId {
        let mut inputs = vec![(Name::new("clk"), clk), (Name::new("d"), d)];
        if let Some(e) = en {
            inputs.push((Name::new("en"), e));
        }
        let reset = reset.map(|(r, rst)| {
            inputs.push((Name::new("rst"), rst));
            r
        });
        b.cell(
            name,
            CellKind::Dff {
                clk_pos: true,
                has_enable: en.is_some(),
                reset,
            },
            inputs,
            vec![(Name::new("q"), q)],
        )
    }

    #[test]
    fn extracts_reset_and_enable() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let en = b.input("en", Type::bit());
        let d = b.input("d", Type::bits(4));
        let q = b.output("q", Type::bits(4));
        let (clkn, rstn, enn, dn, qn) = (b.net(clk), b.net(rst), b.net(en), b.net(d), b.net(q));
        let zero = b.const_u64(4, 0);
        let inner = b.mux(enn, dn, qn);
        let nrst = b.lnot(rstn);
        let outer = b.mux(nrst, inner, zero);
        dff(&mut b, "ff", clkn, outer, q, None, None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("sync resets extracted"), 1);
        assert_eq!(stats.get("enables extracted"), 1);
        assert!(
            m.to_text().contains(
                "cell ff dff pos en srst pos 4'd0 (clk=%clk, d=%d, en=%en, rst=%rst) -> (q=%q)"
            ),
            "{}",
            m.to_text()
        );
    }

    #[test]
    fn constant_registers_become_assigns() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let q1 = b.output("q1", Type::bits(4));
        let q2 = b.output("q2", Type::bits(4));
        let q3 = b.output("q3", Type::bits(4));
        let q4 = b.output("q4", Type::bits(4));
        let (clkn, rstn) = (b.net(clk), b.net(rst));
        let five = b.const_u64(4, 5);
        let r = Reset {
            asynchronous: false,
            active_high: true,
            value: Const::from_u64(5, 4),
        };
        dff(&mut b, "f1", clkn, five, q1, None, Some((r.clone(), rstn)));
        let q2n = b.net(q2);
        dff(&mut b, "f2", clkn, q2n, q2, None, Some((r, rstn)));
        let q3n = b.net(q3);
        b.net_attr(q3, "init", Const::from_u64(9, 4));
        dff(&mut b, "f3", clkn, q3n, q3, None, None);
        let one = b.const_bit(true);
        let ra = Reset {
            asynchronous: true,
            active_high: true,
            value: Const::from_u64(3, 4),
        };
        let q4n = b.net(q4);
        dff(&mut b, "f4", clkn, q4n, q4, None, Some((ra, one)));
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("constant registers"), 4);
        let t = m.to_text();
        assert!(t.contains("assign %q1 = 4'd5"), "{t}");
        assert!(t.contains("assign %q2 = 4'd5"), "{t}");
        assert!(t.contains("assign %q3 = 4'd9"), "{t}");
        assert!(t.contains("assign %q4 = 4'd3"), "{t}");
        assert!(m.cells.is_empty());
    }

    #[test]
    fn constant_controls_unused_and_duplicates() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bits(4));
        let q1 = b.output("q1", Type::bits(4));
        let q2 = b.output("q2", Type::bits(4));
        let dead = b.add_net("dead", Type::bits(4));
        let (clkn, dn) = (b.net(clk), b.net(d));
        let one = b.const_bit(true);
        let zero = b.const_bit(false);
        let r = Reset {
            asynchronous: false,
            active_high: true,
            value: Const::from_u64(0, 4),
        };
        dff(
            &mut b,
            "f1",
            clkn,
            dn,
            q1,
            Some(one),
            Some((r.clone(), zero)),
        );
        dff(&mut b, "f2", clkn, dn, q2, None, None);
        dff(&mut b, "f3", clkn, dn, dead, Some(zero), None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("constant enables removed"), 2);
        assert_eq!(stats.get("constant resets removed"), 1);
        assert_eq!(stats.get("registers merged"), 1);
        assert_eq!(stats.get("constant registers"), 1);
        let t = m.to_text();
        assert!(
            t.contains("cell f1 dff pos (clk=%clk, d=%d) -> (q=%q1)"),
            "{t}"
        );
        assert!(t.contains("assign %q2 = %q1"), "{t}");
        assert!(t.contains("assign %dead = 4'bxxxx"), "{t}");
    }

    #[test]
    fn removes_unused_registers() {
        let mut b = ModuleBuilder::new("m", span());
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bits(4));
        let dead = b.add_net("dead", Type::bits(4));
        let (clkn, dn) = (b.net(clk), b.net(d));
        dff(&mut b, "f", clkn, dn, dead, None, None);
        let mut m = b.finish();
        let stats = run(&FfOpt, &mut m);
        assert_eq!(stats.get("unused registers removed"), 1);
        assert!(m.cells.is_empty());
    }
}
