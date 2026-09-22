//! Finite state machine extraction and re-encoding.
//!
//! [`Fsm`] looks for *state registers*: `dff` cells whose next-state logic
//! is a tree of `mux(...)` ternaries and `pmux` cells with only constant
//! leaves and the register's own `q` (hold), and whose `q` is read
//! nowhere else than in `eq` / `ne` comparisons against constants. The
//! constants (plus the reset and initial values) are the states.
//!
//! A detected machine is re-encoded when asked to, by the net's
//! `fsm_encoding` attribute (`"one-hot"`, `"binary"`, `"gray"`; `"auto"`
//! means one-hot, `"none"` opts out) or, for registers without the
//! attribute, by [`SynthOptions::fsm_encoding`](super::SynthOptions)
//! when it is not `Auto`/`None`. States are numbered in increasing order
//! of their original code. A new register net named `<q>$fsm` replaces
//! `q`; every constant leaf and reset / initial value is translated;
//! `eq(q, K)` becomes a bit test of the one-hot register or a comparison
//! with the new code; the old logic is left for [`super::opt::Dce`]. The
//! new net carries `fsm_encoded = "<encoding>"` so it is not extracted
//! again.
//!
//! Limits: the next-state tree must be private to the register (no shared
//! subtrees), the state width must be at most 64 bits, and comparisons
//! must be plain `eq` / `ne` against fully known constants; anything else
//! leaves the register untouched.

use std::collections::{HashMap, HashSet};

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{
    AttrValue, BinaryOp, CellId, CellKind, Const, ExprId, ExprKind, Module, Name, NetId, Type,
    UnaryOp, expr::operands,
};
use crate::synth::opt::root_net_refs;
use crate::synth::util::{
    add_cell, add_wire, is_kept, is_port, mk, mk_binary, mk_const, mk_mux, mk_net, mk_slice,
    mk_unary,
};
use crate::synth::{FsmEncoding, Pass, PassStats};

/// The FSM pass; see the module docs.
#[derive(Debug, Clone, Copy)]
pub struct Fsm {
    encoding: FsmEncoding,
}

impl Fsm {
    /// A pass applying `encoding` to registers without an `fsm_encoding`
    /// attribute (`Auto` and `None` leave them alone).
    pub fn new(encoding: FsmEncoding) -> Self {
        Fsm { encoding }
    }
}

impl Pass for Fsm {
    fn name(&self) -> &'static str {
        "fsm"
    }

    fn run(&self, m: &mut Module, diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        let cells: Vec<CellId> = m
            .cells
            .iter()
            .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
            .map(|(id, _)| id)
            .collect();
        for id in cells {
            let Some(plan) = analyse(m, id, self.encoding) else {
                continue;
            };
            let name = m.nets[plan.q].name.clone();
            let n = plan.states.len();
            let enc = plan.encoding;
            let span = m.cells[id].span;
            apply(m, plan);
            diags.push(
                Diagnostic::note(format!(
                    "state machine `{name}` with {n} states re-encoded as {}",
                    encoding_name(enc)
                ))
                .with_code("S0020")
                .with_span(span),
            );
            stats.bump("state machines re-encoded", 1);
        }
        stats
    }
}

fn encoding_name(e: FsmEncoding) -> &'static str {
    match e {
        FsmEncoding::OneHot | FsmEncoding::Auto => "one-hot",
        FsmEncoding::Binary => "binary",
        FsmEncoding::Gray => "gray",
        FsmEncoding::None => "none",
    }
}

/// A detected state machine and how to re-encode it.
struct Plan {
    cell: CellId,
    q: NetId,
    states: Vec<Const>,
    encoding: FsmEncoding,
    /// Compare nodes `eq`/`ne` of `q` against a constant: `(node, is_ne,
    /// state)`.
    compares: Vec<(ExprId, bool, Const)>,
    /// Pmux cells inside the next-state tree, by output net.
    pmuxes: HashMap<NetId, CellId>,
}

/// The encoding requested for a net, if any.
fn requested_encoding(m: &Module, q: NetId, default: FsmEncoding) -> Option<FsmEncoding> {
    match m.nets[q].attrs.get("fsm_encoding") {
        Some(AttrValue::String(s)) => match FsmEncoding::from_attr(s)? {
            FsmEncoding::None => None,
            FsmEncoding::Auto => Some(FsmEncoding::OneHot),
            e => Some(e),
        },
        Some(_) => None,
        None => match default {
            FsmEncoding::Auto | FsmEncoding::None => None,
            e => Some(e),
        },
    }
}

fn analyse(m: &Module, cell: CellId, default: FsmEncoding) -> Option<Plan> {
    let c = &m.cells[cell];
    let CellKind::Dff { reset, .. } = &c.kind else {
        return None;
    };
    let q = c.output("q")?;
    let d = c.input("d")?;
    let width = m.nets[q].ty.width()?;
    if !(2..=64).contains(&width) || is_port(m, q) || is_kept(&m.nets[q].attrs) {
        return None;
    }
    if m.nets[q].attrs.contains("fsm_encoded") {
        return None;
    }
    let encoding = requested_encoding(m, q, default)?;
    if root_net_refs(m)[q.index()] != 1 {
        return None;
    }

    // Parents and root-slot uses of every reachable node.
    let mut parents: HashMap<ExprId, Vec<ExprId>> = HashMap::new();
    let mut root_uses: HashMap<ExprId, u32> = HashMap::new();
    m.for_each_root_expr(|id| *root_uses.entry(id).or_insert(0) += 1);
    m.for_each_expr(|id, e| {
        for op in operands(&e.kind) {
            parents.entry(op).or_default().push(id);
        }
    });
    let drivers: HashMap<NetId, CellId> = m
        .cells
        .iter()
        .flat_map(|(id, c)| c.outputs.iter().map(move |(_, n)| (*n, id)))
        .collect();

    // Walk the next-state tree.
    let mut states: Vec<Const> = Vec::new();
    let add_state = |c: &Const, states: &mut Vec<Const>| -> Option<()> {
        if c.width() != width || !c.is_fully_known() {
            return None;
        }
        if !states.contains(c) {
            states.push(c.clone());
        }
        Some(())
    };
    let mut tree: HashSet<ExprId> = HashSet::new();
    let mut pmuxes: HashMap<NetId, CellId> = HashMap::new();
    let mut pmux_inputs: HashSet<ExprId> = HashSet::new();
    let mut leaves: HashSet<ExprId> = HashSet::new();
    let mut stack = vec![d];
    while let Some(id) = stack.pop() {
        match &m.expr(id).kind {
            ExprKind::Const(_) | ExprKind::Net(_) if !leaves.insert(id) => continue,
            ExprKind::Const(_) | ExprKind::Net(_) => {}
            _ if !tree.insert(id) => continue,
            _ => {}
        }
        match &m.expr(id).kind {
            ExprKind::Const(c) => add_state(c, &mut states)?,
            ExprKind::Net(n) if *n == q => {}
            ExprKind::Net(n) => {
                tree.insert(id);
                leaves.remove(&id);
                let pm = *drivers.get(n)?;
                let pc = &m.cells[pm];
                if !matches!(pc.kind, CellKind::Pmux) || is_kept(&pc.attrs) {
                    return None;
                }
                if root_net_refs(m)[n.index()] != 1 || is_port(m, *n) {
                    return None;
                }
                let a = pc.input("a")?;
                let b = pc.input("b")?;
                pmux_inputs.insert(a);
                pmux_inputs.insert(b);
                stack.push(a);
                if matches!(m.expr(b).kind, ExprKind::Concat(_)) {
                    tree.insert(b);
                }
                for arm in pmux_arms(m, b, width)? {
                    match arm {
                        Arm::Const(c) => add_state(&c, &mut states)?,
                        Arm::Expr(e) => stack.push(e),
                    }
                }
                pmuxes.insert(*n, pm);
            }
            ExprKind::Ternary { then_, else_, .. } => {
                stack.push(*then_);
                stack.push(*else_);
            }
            _ => return None,
        }
    }
    // The tree must be private: every inner node's parents are in the
    // tree and its only root use is the register's `d` or a pmux input.
    // Constants and reads of `q` are leaves that may be shared.
    for id in &tree {
        let expected_roots = u32::from(*id == d || pmux_inputs.contains(id));
        if root_uses.get(id).copied().unwrap_or(0) != expected_roots {
            return None;
        }
        if let Some(ps) = parents.get(id)
            && ps.iter().any(|p| !tree.contains(p))
        {
            return None;
        }
    }
    if m.expr(d).kind == ExprKind::Net(q) {
        return None;
    }
    if leaves.contains(&d) && root_uses.get(&d).copied().unwrap_or(0) != 1 {
        return None;
    }

    // Every other read of `q` is a comparison against a constant.
    let mut compares = Vec::new();
    let mut q_nodes = Vec::new();
    m.for_each_expr(|id, e| {
        if e.kind == ExprKind::Net(q) {
            q_nodes.push(id);
        }
    });
    for node in q_nodes {
        if root_uses.get(&node).copied().unwrap_or(0) != 0 {
            return None;
        }
        for p in parents.get(&node).cloned().unwrap_or_default() {
            if tree.contains(&p) {
                continue;
            }
            let ExprKind::Binary { op, lhs, rhs } = &m.expr(p).kind else {
                return None;
            };
            let is_ne = match op {
                BinaryOp::Eq => false,
                BinaryOp::Ne => true,
                _ => return None,
            };
            let other = if *lhs == node { *rhs } else { *lhs };
            let k = m.expr(other).as_const()?;
            add_state(k, &mut states)?;
            compares.push((p, is_ne, k.clone()));
        }
    }
    if let Some(r) = reset {
        add_state(&r.value, &mut states)?;
    }
    if let Some(AttrValue::Const(init)) = m.nets[q].attrs.get("init") {
        add_state(init, &mut states)?;
    }
    if states.len() < 2 {
        return None;
    }
    states.sort_by_key(|c| c.to_u64().unwrap_or(u64::MAX));
    compares.sort_by_key(|(id, ne, _)| (*id, *ne));
    compares.dedup_by_key(|(id, ne, _)| (*id, *ne));
    Some(Plan {
        cell,
        q,
        states,
        encoding,
        compares,
        pmuxes,
    })
}

/// One arm of a `pmux`, most significant first.
enum Arm {
    /// A constant next state.
    Const(Const),
    /// A subtree.
    Expr(ExprId),
}

/// Splits a `pmux`'s `b` input into arms of `width` bits: a concatenation
/// of parts, where a constant part may hold several arms (constant folding
/// merges adjacent constants) and any other part is exactly one arm.
fn pmux_arms(m: &Module, b: ExprId, width: u32) -> Option<Vec<Arm>> {
    let parts: Vec<ExprId> = match &m.expr(b).kind {
        ExprKind::Concat(parts) => parts.clone(),
        _ => vec![b],
    };
    let mut arms = Vec::new();
    for p in parts {
        let pw = m.expr(p).ty.width()?;
        match &m.expr(p).kind {
            ExprKind::Const(c) if pw % width == 0 => {
                for i in (0..pw / width).rev() {
                    arms.push(Arm::Const(c.slice((i + 1) * width - 1, i * width)));
                }
            }
            _ if pw == width => arms.push(Arm::Expr(p)),
            _ => return None,
        }
    }
    Some(arms)
}

/// The code of state `i` under `encoding`, `n` states wide.
fn code(encoding: FsmEncoding, i: usize, n: usize) -> Const {
    let i = u64::try_from(i).unwrap_or(u64::MAX);
    match encoding {
        FsmEncoding::OneHot | FsmEncoding::Auto | FsmEncoding::None => {
            let mut c = Const::zero(u32::try_from(n).unwrap_or(u32::MAX));
            c.set_bit(u32::try_from(i).unwrap_or(u32::MAX), crate::logic::Bit::One);
            c
        }
        FsmEncoding::Binary => Const::from_u64(i, binary_width(n)),
        FsmEncoding::Gray => Const::from_u64(i ^ (i >> 1), binary_width(n)),
    }
}

fn binary_width(n: usize) -> u32 {
    let mut w = 1;
    while (1usize << w) < n {
        w += 1;
    }
    w
}

fn apply(m: &mut Module, plan: Plan) {
    let n = plan.states.len();
    let new_width = match plan.encoding {
        FsmEncoding::Binary | FsmEncoding::Gray => binary_width(n),
        _ => u32::try_from(n).unwrap_or(u32::MAX),
    };
    let span = m.nets[plan.q].span;
    let name = format!("{}$fsm", m.nets[plan.q].name);
    let nq = add_wire(m, &name, Type::bits(new_width), span);
    m.nets[nq]
        .attrs
        .set("fsm_encoded", encoding_name(plan.encoding));
    let encode = |c: &Const| -> Const {
        let i = plan.states.iter().position(|s| s == c).expect("state");
        code(plan.encoding, i, n)
    };
    if let Some(AttrValue::Const(init)) = m.nets[plan.q].attrs.get("init").cloned() {
        m.nets[nq]
            .attrs
            .set("init", AttrValue::Const(encode(&init)));
    }

    // Comparisons.
    let mut replacement: HashMap<ExprId, ExprId> = HashMap::new();
    for (node, is_ne, k) in &plan.compares {
        let cspan = m.expr(*node).span;
        let i = plan.states.iter().position(|s| s == k).expect("state");
        let qn = mk_net(m, nq, cspan);
        let test = match plan.encoding {
            FsmEncoding::Binary | FsmEncoding::Gray => {
                let kc = mk_const(m, code(plan.encoding, i, n), cspan);
                mk_binary(m, BinaryOp::Eq, qn, kc, cspan)
            }
            _ => {
                let i = u32::try_from(i).unwrap_or(u32::MAX);
                mk_slice(m, qn, i, i, cspan)
            }
        };
        let test = if *is_ne {
            mk_unary(m, UnaryOp::Not, test, cspan)
        } else {
            test
        };
        replacement.insert(*node, test);
    }

    // Next-state tree.
    let d = m.cells[plan.cell].input("d").expect("dff d");
    let new_d = rebuild(m, d, plan.q, nq, &plan.pmuxes, &encode, span);
    let cell = &mut m.cells[plan.cell];
    for (port, e) in &mut cell.inputs {
        if port.as_str() == "d" {
            *e = new_d;
        }
    }
    for (port, net) in &mut cell.outputs {
        if port.as_str() == "q" {
            *net = nq;
        }
    }
    if let CellKind::Dff { reset: Some(r), .. } = &mut cell.kind {
        r.value = encode(&r.value);
    }
    m.map_exprs(|id| replacement.get(&id).copied().unwrap_or(id));
}

/// Rebuilds the next-state tree over the new register.
fn rebuild(
    m: &mut Module,
    id: ExprId,
    q: NetId,
    nq: NetId,
    pmuxes: &HashMap<NetId, CellId>,
    encode: &dyn Fn(&Const) -> Const,
    span: crate::source::Span,
) -> ExprId {
    match m.expr(id).kind.clone() {
        ExprKind::Const(c) => {
            let v = encode(&c);
            mk_const(m, v, span)
        }
        ExprKind::Net(n) if n == q => mk_net(m, nq, span),
        ExprKind::Net(n) => {
            let pm = pmuxes[&n];
            let (a, b, s) = {
                let c = &m.cells[pm];
                (
                    c.input("a").expect("pmux a"),
                    c.input("b").expect("pmux b"),
                    c.input("s").expect("pmux s"),
                )
            };
            let na = rebuild(m, a, q, nq, pmuxes, encode, span);
            let old_width = m.nets[q].ty.width().expect("state width");
            let arms = pmux_arms(m, b, old_width).expect("analysed arms");
            let new_parts: Vec<ExprId> = arms
                .into_iter()
                .map(|arm| match arm {
                    Arm::Const(c) => {
                        let v = encode(&c);
                        mk_const(m, v, span)
                    }
                    Arm::Expr(e) => rebuild(m, e, q, nq, pmuxes, encode, span),
                })
                .collect();
            let nb = mk(m, ExprKind::Concat(new_parts), span);
            let base = format!("{}$fsm", m.nets[n].name);
            let ty = m.nets[nq].ty.clone();
            let y = add_wire(m, &base, ty, span);
            add_cell(
                m,
                &base,
                CellKind::Pmux,
                vec![("a", na), ("b", nb), ("s", s)],
                vec![("y", y)],
                span,
            );
            mk_net(m, y, span)
        }
        ExprKind::Ternary { cond, then_, else_ } => {
            let t = rebuild(m, then_, q, nq, pmuxes, encode, span);
            let e = rebuild(m, else_, q, nq, pmuxes, encode, span);
            mk_mux(m, cond, t, e, span)
        }
        _ => unreachable!("node outside the analysed tree"),
    }
}

#[allow(dead_code)]
fn name_of(m: &Module, n: NetId) -> &Name {
    &m.nets[n].name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Reset;
    use crate::ir::builder::ModuleBuilder;
    use crate::synth::opt::testutil::{run, span};
    use crate::synth::opt::{ConstFold, Dce};

    /// A three-state machine: idle -> run -> done -> idle, `busy` while
    /// not idle.
    fn machine(attr: Option<&str>) -> Module {
        let mut b = ModuleBuilder::new("fsm", span());
        let clk = b.input("clk", Type::bit());
        let rst = b.input("rst", Type::bit());
        let go = b.input("go", Type::bit());
        let busy = b.output("busy", Type::bit());
        let state = b.add_net("state", Type::bits(2));
        if let Some(a) = attr {
            b.net_attr(state, "fsm_encoding", a);
        }
        let (clkn, rstn, gon, sn) = (b.net(clk), b.net(rst), b.net(go), b.net(state));
        let (idle, run, done) = (b.const_u64(2, 0), b.const_u64(2, 1), b.const_u64(2, 2));
        let is_idle = b.eq(sn, idle);
        let is_run = b.eq(sn, run);
        let next_from_idle = b.mux(gon, run, sn);
        let next_from_run = b.mux(is_run, done, idle);
        let next = b.mux(is_idle, next_from_idle, next_from_run);
        let not_idle = b.ne(sn, idle);
        b.assign(busy, not_idle);
        b.cell(
            "state$ff",
            CellKind::Dff {
                clk_pos: true,
                has_enable: false,
                reset: Some(Reset {
                    asynchronous: false,
                    active_high: true,
                    value: Const::from_u64(0, 2),
                }),
            },
            vec![
                (Name::new("clk"), clkn),
                (Name::new("d"), next),
                (Name::new("rst"), rstn),
            ],
            vec![(Name::new("q"), state)],
        );
        b.finish()
    }

    #[test]
    fn one_hot_by_attribute() {
        let mut m = machine(Some("one-hot"));
        let stats = run(&Fsm::new(FsmEncoding::Auto), &mut m);
        assert_eq!(stats.get("state machines re-encoded"), 1);
        run(&ConstFold, &mut m);
        run(&Dce, &mut m);
        let t = m.to_text();
        assert!(
            t.contains("attr fsm_encoded = \"one-hot\"\n  net %state$fsm u3 wire"),
            "{t}"
        );
        assert!(t.contains("assign %busy = not(%state$fsm[0:0])"), "{t}");
        assert!(t.contains("cell state$ff dff pos srst pos 3'd1 (clk=%clk, d=mux(%state$fsm[0:0], mux(%go, 3'd2, %state$fsm), mux(%state$fsm[1:1], 3'd4, 3'd1)), rst=%rst) -> (q=%state$fsm)"), "{t}");
        assert!(m.net_by_name("state").is_none());
        // Not extracted twice.
        let again = run(&Fsm::new(FsmEncoding::OneHot), &mut m);
        assert!(!again.changed);
    }

    #[test]
    fn gray_and_binary_by_option() {
        let mut m = machine(None);
        assert!(!run(&Fsm::new(FsmEncoding::Auto), &mut m).changed);
        assert!(!run(&Fsm::new(FsmEncoding::None), &mut m).changed);
        let stats = run(&Fsm::new(FsmEncoding::Gray), &mut m);
        assert_eq!(stats.get("state machines re-encoded"), 1);
        let t = m.to_text();
        assert!(
            t.contains("attr fsm_encoded = \"gray\"\n  net %state$fsm u2 wire"),
            "{t}"
        );
        assert!(t.contains("mux(eq(%state$fsm, 2'd1), 2'd3, 2'd0)"), "{t}");
        let mut m2 = machine(Some("binary"));
        run(&Fsm::new(FsmEncoding::None), &mut m2);
        assert!(m2.to_text().contains("attr fsm_encoded = \"binary\""));
        let mut m3 = machine(Some("none"));
        assert!(!run(&Fsm::new(FsmEncoding::OneHot), &mut m3).changed);
        assert_eq!(name_of(&m3, m3.net_by_name("state").unwrap()), "state");
    }

    #[test]
    fn shared_or_exposed_state_is_left_alone() {
        let mut m = machine(Some("one-hot"));
        // Expose the state on a port: the encoding is observable.
        let state = m.net_by_name("state").unwrap();
        m.ports.push(crate::ir::Port {
            name: Name::new("state"),
            dir: crate::ir::PortDir::Out,
            net: state,
            span: span(),
        });
        assert!(!run(&Fsm::new(FsmEncoding::Auto), &mut m).changed);
        assert_eq!(code(FsmEncoding::Gray, 3, 4), Const::from_u64(2, 2));
        assert_eq!(binary_width(1), 1);
        assert_eq!(binary_width(5), 3);
    }
}
