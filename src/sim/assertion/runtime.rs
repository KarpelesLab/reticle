//! Compiling a property onto the simulator's signals, and running its
//! attempts.
//!
//! Compilation does three things: it resolves every net name to a
//! [`SigId`], it interns every distinct boolean into a predicate table so a
//! boolean written twice is evaluated once per cycle, and it turns every
//! sequence into an [`Nfa`]. What is left is a [`Node`] tree with the
//! property-level operators (`not`, `and`, `or`, `|->`, `|=>`) that a
//! single automaton cannot express, because those need an obligation that
//! outlives the sequence that created it.
//!
//! At run time a [`Checker`] holds one [`Attempt`] per cycle it has
//! started. Every attempt is stepped with the same predicate valuation, so
//! `a |-> ##2 b` restarted every cycle tracks all its overlapping attempts
//! in lockstep. An attempt ends when its verdict is no longer
//! [`Verdict::Pending`]; attempts still pending when the run ends are
//! reported as incomplete rather than as failures.
//!
//! Values are *sampled*: a cycle reads the values the nets had just before
//! the time slot in which the clocking event occurred, which is the
//! preponed region of IEEE 1800 §4.4. That is what makes an assertion see
//! the state a flip-flop sampled rather than the state it is about to
//! take.

use crate::diag::Diagnostic;
use crate::logic::{Bit, Logic};
use crate::source::Span;

use super::automaton::{Nfa, PredId, Run};
use super::property::{
    BoolExpr, CmpOp, Directive, DirectiveKind, Operand, PropertyExpr, Select, Sequence,
};
use crate::ir::BinaryOp;
use crate::sim::elab::SigId;

/// The largest automaton a single property may compile to.
///
/// A range like `[*1:100000]` is one copy of the sequence per repetition,
/// so the bound turns a typo into a diagnostic instead of a hang.
const MAX_STATES: usize = 100_000;

/// The largest cycle count a delay or repetition range may name.
///
/// Every cycle of a range is at least one automaton state, so this is what
/// keeps `a[*1:1000000]` a diagnostic rather than a hang.
const MAX_CYCLES: u32 = 4096;

/// A net a property reads, resolved.
#[derive(Clone, Debug)]
pub(crate) struct BoundNet {
    /// The signal behind it.
    pub(crate) sig: SigId,
    /// The hierarchical name, for reports.
    pub(crate) name: String,
}

/// A resolved operand.
#[derive(Clone, Debug, PartialEq)]
enum BoundOperand {
    /// Index into [`Compiled::nets`], with an optional select.
    Net(usize, Option<Select>),
    /// A literal.
    Const(Logic),
}

/// A resolved boolean.
#[derive(Clone, Debug, PartialEq)]
enum BoundBool {
    Const(bool),
    Value(BoundOperand),
    Not(Box<BoundBool>),
    And(Box<BoundBool>, Box<BoundBool>),
    Or(Box<BoundBool>, Box<BoundBool>),
    Implies(Box<BoundBool>, Box<BoundBool>),
    Cmp {
        op: CmpOp,
        lhs: BoundOperand,
        rhs: BoundOperand,
    },
    Rose(usize),
    Fell(usize),
    Stable(usize),
}

/// The property-level operators, over compiled sequences.
#[derive(Clone, Debug)]
pub(crate) enum Node {
    /// A sequence, which holds when it matches.
    Seq(Nfa),
    /// `not p`.
    Not(Box<Node>),
    /// `p1 and p2`.
    And(Box<Node>, Box<Node>),
    /// `p1 or p2`.
    Or(Box<Node>, Box<Node>),
    /// `seq |-> p` / `seq |=> p`.
    Implies {
        /// The antecedent.
        ante: Nfa,
        /// True for `|->`.
        overlapped: bool,
        /// The consequent.
        conseq: Box<Node>,
    },
}

/// A property compiled against one simulation.
#[derive(Clone, Debug)]
pub(crate) struct Compiled {
    /// Predicates, indexed by [`PredId`].
    preds: Vec<BoundBool>,
    /// Nets read by the property, in order of first appearance.
    pub(crate) nets: Vec<BoundNet>,
    /// The property tree.
    pub(crate) node: Node,
    /// The `disable iff` predicate.
    pub(crate) disable: Option<PredId>,
    /// True when the property has an implication, so a pass without an
    /// antecedent match is vacuous.
    pub(crate) implicative: bool,
}

impl Compiled {
    /// The value of every predicate this cycle, indexed by [`PredId`].
    pub(crate) fn evaluate(&self, now: &[Logic], prev: &[Logic]) -> Vec<bool> {
        self.preds
            .iter()
            .map(|p| eval_bool(p, now, prev) == Bit::One)
            .collect()
    }

    /// The number of predicates, for tests.
    #[cfg(test)]
    pub(crate) fn pred_count(&self) -> usize {
        self.preds.len()
    }
}

/// Resolves a net name to a signal, for [`compile`].
pub(crate) type Resolver<'a> = dyn Fn(&str) -> Option<(SigId, String)> + 'a;

/// Compiles `directive` against the nets `resolve` knows.
pub(crate) fn compile(
    directive: &Directive,
    resolve: &Resolver<'_>,
) -> Result<Compiled, Diagnostic> {
    let mut b = Binder {
        preds: Vec::new(),
        nets: Vec::new(),
        resolve,
    };
    let node = b.node(&directive.property)?;
    let disable = match &directive.disable {
        Some(e) => Some(b.pred(e)?),
        None => None,
    };
    let implicative = has_implication(&node);
    Ok(Compiled {
        preds: b.preds,
        nets: b.nets,
        node,
        disable,
        implicative,
    })
}

/// True when `node` contains an implication anywhere.
fn has_implication(node: &Node) -> bool {
    match node {
        Node::Seq(_) => false,
        Node::Not(i) => has_implication(i),
        Node::And(a, b) | Node::Or(a, b) => has_implication(a) || has_implication(b),
        Node::Implies { .. } => true,
    }
}

/// The state carried while compiling one directive.
struct Binder<'a> {
    preds: Vec<BoundBool>,
    nets: Vec<BoundNet>,
    resolve: &'a Resolver<'a>,
}

impl Binder<'_> {
    /// The index of `name` in [`Binder::nets`], resolving it on first use.
    fn net(&mut self, name: &str, span: Span) -> Result<usize, Diagnostic> {
        let Some((sig, full)) = (self.resolve)(name) else {
            return Err(
                Diagnostic::error(format!("unknown net `{name}` in a property")).with_span(span),
            );
        };
        if let Some(i) = self.nets.iter().position(|n| n.sig == sig) {
            return Ok(i);
        }
        self.nets.push(BoundNet { sig, name: full });
        Ok(self.nets.len() - 1)
    }

    fn operand(&mut self, op: &Operand) -> Result<BoundOperand, Diagnostic> {
        match op {
            Operand::Const(l) => Ok(BoundOperand::Const(l.clone())),
            Operand::Net(r) => {
                let i = self.net(&r.name, r.span)?;
                Ok(BoundOperand::Net(i, r.select))
            }
        }
    }

    fn bool_expr(&mut self, e: &BoolExpr) -> Result<BoundBool, Diagnostic> {
        Ok(match e {
            BoolExpr::Const(v) => BoundBool::Const(*v),
            BoolExpr::Value(o) => BoundBool::Value(self.operand(o)?),
            BoolExpr::Not(i) => BoundBool::Not(Box::new(self.bool_expr(i)?)),
            BoolExpr::And(a, b) => {
                BoundBool::And(Box::new(self.bool_expr(a)?), Box::new(self.bool_expr(b)?))
            }
            BoolExpr::Or(a, b) => {
                BoundBool::Or(Box::new(self.bool_expr(a)?), Box::new(self.bool_expr(b)?))
            }
            BoolExpr::Implies(a, b) => {
                BoundBool::Implies(Box::new(self.bool_expr(a)?), Box::new(self.bool_expr(b)?))
            }
            BoolExpr::Cmp { op, lhs, rhs } => BoundBool::Cmp {
                op: *op,
                lhs: self.operand(lhs)?,
                rhs: self.operand(rhs)?,
            },
            BoolExpr::Rose(r) => BoundBool::Rose(self.net(&r.name, r.span)?),
            BoolExpr::Fell(r) => BoundBool::Fell(self.net(&r.name, r.span)?),
            BoolExpr::Stable(r) => BoundBool::Stable(self.net(&r.name, r.span)?),
        })
    }

    /// Interns a boolean, returning its predicate id.
    fn pred(&mut self, e: &BoolExpr) -> Result<PredId, Diagnostic> {
        let bound = self.bool_expr(e)?;
        let index = match self.preds.iter().position(|p| *p == bound) {
            Some(i) => i,
            None => {
                self.preds.push(bound);
                self.preds.len() - 1
            }
        };
        Ok(PredId(
            u32::try_from(index).expect("property has fewer than 4 billion predicates"),
        ))
    }

    /// Compiles a sequence, checking the automaton stays a sane size.
    fn seq(&mut self, s: &Sequence) -> Result<Nfa, Diagnostic> {
        let nfa = self.seq_inner(s)?;
        if nfa.state_count() > MAX_STATES {
            return Err(Diagnostic::error(format!(
                "the property compiles to more than {MAX_STATES} automaton states"
            ))
            .with_note("a repetition or delay range this wide is almost always a typo"));
        }
        Ok(nfa)
    }

    /// Rejects a range wide enough to blow the automaton up.
    fn check_range(range: super::property::Range) -> Result<(), Diagnostic> {
        if range.min > MAX_CYCLES || range.max.is_some_and(|m| m > MAX_CYCLES) {
            return Err(Diagnostic::error(format!(
                "a cycle range beyond {MAX_CYCLES} is not supported"
            ))
            .with_note("every cycle of a range is an automaton state"));
        }
        Ok(())
    }

    fn seq_inner(&mut self, s: &Sequence) -> Result<Nfa, Diagnostic> {
        match s {
            Sequence::Delay { range, .. }
            | Sequence::Lead { range, .. }
            | Sequence::Repeat { range, .. }
            | Sequence::Goto { range, .. } => Self::check_range(*range)?,
            _ => {}
        }
        Ok(match s {
            Sequence::Bool(b) => Nfa::predicate(self.pred(b)?),
            Sequence::Delay { lhs, range, rhs } => {
                let l = self.seq(lhs)?;
                let r = self.seq(rhs)?;
                Nfa::concat(&l, &r, range.min, range.max)
            }
            Sequence::Lead { range, seq } => {
                let s = self.seq(seq)?;
                Nfa::lead(&s, range.min, range.max)
            }
            Sequence::Repeat { seq, range } => {
                if range.min == 0 {
                    return Err(Diagnostic::error(
                        "a repetition of zero would match no cycles, which is not supported",
                    ));
                }
                let s = self.seq(seq)?;
                Nfa::repeat(&s, range.min, range.max)
            }
            Sequence::Goto { expr, range } => {
                if range.min == 0 {
                    return Err(Diagnostic::error(
                        "goto repetition needs at least one occurrence",
                    ));
                }
                Nfa::goto(self.pred(expr)?, range.min, range.max)
            }
            Sequence::And(a, b) => {
                let (a, b) = (self.seq(a)?, self.seq(b)?);
                Nfa::and(&a, &b)
            }
            Sequence::Intersect(a, b) => {
                let (a, b) = (self.seq(a)?, self.seq(b)?);
                Nfa::intersect(&a, &b)
            }
            Sequence::Or(a, b) => {
                let (a, b) = (self.seq(a)?, self.seq(b)?);
                Nfa::or(&a, &b)
            }
            Sequence::Throughout { expr, seq } => {
                let p = self.pred(expr)?;
                let s = self.seq(seq)?;
                Nfa::throughout(p, &s)
            }
            Sequence::Within(a, b) => {
                let (a, b) = (self.seq(a)?, self.seq(b)?);
                Nfa::within(&a, &b)
            }
        })
    }

    fn node(&mut self, p: &PropertyExpr) -> Result<Node, Diagnostic> {
        Ok(match p {
            PropertyExpr::Seq(s) => Node::Seq(self.seq(s)?),
            PropertyExpr::Not(i) => Node::Not(Box::new(self.node(i)?)),
            PropertyExpr::And(a, b) => Node::And(Box::new(self.node(a)?), Box::new(self.node(b)?)),
            PropertyExpr::Or(a, b) => Node::Or(Box::new(self.node(a)?), Box::new(self.node(b)?)),
            PropertyExpr::Implies {
                ante,
                overlapped,
                conseq,
            } => Node::Implies {
                ante: self.seq(ante)?,
                overlapped: *overlapped,
                conseq: Box::new(self.node(conseq)?),
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Boolean evaluation
// ---------------------------------------------------------------------------

/// The value of a resolved operand this cycle.
fn operand_value(op: &BoundOperand, now: &[Logic]) -> Logic {
    match op {
        BoundOperand::Const(l) => l.clone(),
        BoundOperand::Net(i, select) => {
            let value = now.get(*i).cloned().unwrap_or_else(|| Logic::x(1));
            select_bits(&value, *select)
        }
    }
}

/// Applies a bit or part select; an out-of-range select reads as `x`.
fn select_bits(value: &Logic, select: Option<Select>) -> Logic {
    match select {
        None => value.clone(),
        Some(Select::Bit(i)) => match value.get(i) {
            Some(b) => Logic::from_bit(b),
            None => Logic::x(1),
        },
        Some(Select::Part { hi, lo }) => {
            if hi < value.width() && lo <= hi {
                value.slice(hi, lo)
            } else {
                Logic::x(hi.saturating_sub(lo) + 1)
            }
        }
    }
}

/// The truth of a resolved boolean, given this cycle's and the previous
/// cycle's sampled values.
fn eval_bool(e: &BoundBool, now: &[Logic], prev: &[Logic]) -> Bit {
    match e {
        BoundBool::Const(v) => Bit::from_bool(*v),
        BoundBool::Value(o) => operand_value(o, now).truth(),
        BoundBool::Not(i) => match eval_bool(i, now, prev) {
            Bit::One => Bit::Zero,
            Bit::Zero => Bit::One,
            _ => Bit::X,
        },
        BoundBool::And(a, b) => eval_bool(a, now, prev).and(eval_bool(b, now, prev)),
        BoundBool::Or(a, b) => eval_bool(a, now, prev).or(eval_bool(b, now, prev)),
        BoundBool::Implies(a, b) => {
            let lhs = match eval_bool(a, now, prev) {
                Bit::One => Bit::Zero,
                Bit::Zero => Bit::One,
                _ => Bit::X,
            };
            lhs.or(eval_bool(b, now, prev))
        }
        BoundBool::Cmp { op, lhs, rhs } => {
            let a = operand_value(lhs, now);
            let b = operand_value(rhs, now);
            let binop = match op {
                CmpOp::Eq => BinaryOp::Eq,
                CmpOp::Ne => BinaryOp::Ne,
                CmpOp::CaseEq => BinaryOp::CaseEq,
                CmpOp::CaseNe => BinaryOp::CaseNe,
                CmpOp::Lt => BinaryOp::Lt,
                CmpOp::Le => BinaryOp::Le,
                CmpOp::Gt => BinaryOp::Gt,
                CmpOp::Ge => BinaryOp::Ge,
            };
            crate::sim::eval::bits_binary(binop, a, b).truth()
        }
        BoundBool::Rose(i) => edge_bit(now, prev, *i, Bit::One),
        BoundBool::Fell(i) => edge_bit(now, prev, *i, Bit::Zero),
        BoundBool::Stable(i) => {
            Bit::from_bool(now.get(*i).is_some() && now.get(*i) == prev.get(*i))
        }
    }
}

/// `$rose` (`to` of [`Bit::One`]) and `$fell` (`to` of [`Bit::Zero`]): the
/// least significant bit reached `to` since the previous clocking event.
fn edge_bit(now: &[Logic], prev: &[Logic], i: usize, to: Bit) -> Bit {
    let new = now.get(i).map_or(Bit::X, |l| l.get(0).unwrap_or(Bit::X));
    let old = prev.get(i).map_or(Bit::X, |l| l.get(0).unwrap_or(Bit::X));
    Bit::from_bool(new == to && old != to)
}

// ---------------------------------------------------------------------------
// Attempts
// ---------------------------------------------------------------------------

/// How an attempt stands after a cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Still running.
    Pending,
    /// The property held.
    Holds,
    /// The property failed.
    Fails,
}

/// One side of an `and` / `or`, which may have settled before the other.
#[derive(Clone, Debug)]
enum Side {
    Pending(Box<NodeState>),
    Settled(bool),
}

/// The run-time state mirroring a [`Node`].
#[derive(Clone, Debug)]
enum NodeState {
    Seq(Run),
    Not(Box<NodeState>),
    Pair {
        left: Side,
        right: Side,
    },
    Implies {
        ante: Run,
        /// Obligations being checked this cycle.
        live: Vec<NodeState>,
        /// Obligations from a `|=>` match, starting next cycle.
        deferred: Vec<NodeState>,
    },
}

/// One evaluation attempt: a property started at one clocking event.
#[derive(Clone, Debug)]
pub(crate) struct Attempt {
    /// Simulation time the attempt started at.
    pub(crate) start_time: u64,
    /// Clock cycle the attempt started at.
    pub(crate) start_cycle: u64,
    /// True once an antecedent matched, so a pass is not vacuous.
    pub(crate) matched: bool,
    state: NodeState,
}

impl Attempt {
    /// A fresh attempt of `node`.
    pub(crate) fn new(node: &Node, start_time: u64, start_cycle: u64) -> Attempt {
        Attempt {
            start_time,
            start_cycle,
            matched: false,
            state: new_state(node),
        }
    }

    /// Advances the attempt by one clock cycle.
    pub(crate) fn step(&mut self, node: &Node, values: &[bool]) -> Verdict {
        let mut matched = self.matched;
        let verdict = step(node, &mut self.state, values, &mut matched);
        self.matched = matched;
        verdict
    }
}

/// The initial state of a node.
fn new_state(node: &Node) -> NodeState {
    match node {
        Node::Seq(nfa) => NodeState::Seq(nfa.start_run()),
        Node::Not(i) => NodeState::Not(Box::new(new_state(i))),
        Node::And(a, b) | Node::Or(a, b) => NodeState::Pair {
            left: Side::Pending(Box::new(new_state(a))),
            right: Side::Pending(Box::new(new_state(b))),
        },
        Node::Implies { ante, .. } => NodeState::Implies {
            ante: ante.start_run(),
            live: Vec::new(),
            deferred: Vec::new(),
        },
    }
}

/// Advances one side of an `and` / `or`; `None` is still pending.
fn step_side(node: &Node, side: &mut Side, values: &[bool], matched: &mut bool) -> Option<bool> {
    match side {
        Side::Settled(v) => Some(*v),
        Side::Pending(state) => match step(node, state, values, matched) {
            Verdict::Pending => None,
            Verdict::Holds => {
                *side = Side::Settled(true);
                Some(true)
            }
            Verdict::Fails => {
                *side = Side::Settled(false);
                Some(false)
            }
        },
    }
}

/// Advances a node by one cycle.
fn step(node: &Node, state: &mut NodeState, values: &[bool], matched: &mut bool) -> Verdict {
    match (node, state) {
        (Node::Seq(nfa), NodeState::Seq(run)) => {
            *run = nfa.step(run, values);
            if nfa.accepts(run) {
                Verdict::Holds
            } else if nfa.is_dead(run) {
                Verdict::Fails
            } else {
                Verdict::Pending
            }
        }
        (Node::Not(inner), NodeState::Not(s)) => match step(inner, s, values, matched) {
            Verdict::Holds => Verdict::Fails,
            Verdict::Fails => Verdict::Holds,
            Verdict::Pending => Verdict::Pending,
        },
        (Node::And(a, b), NodeState::Pair { left, right }) => {
            let l = step_side(a, left, values, matched);
            let r = step_side(b, right, values, matched);
            if l == Some(false) || r == Some(false) {
                Verdict::Fails
            } else if l == Some(true) && r == Some(true) {
                Verdict::Holds
            } else {
                Verdict::Pending
            }
        }
        (Node::Or(a, b), NodeState::Pair { left, right }) => {
            let l = step_side(a, left, values, matched);
            let r = step_side(b, right, values, matched);
            if l == Some(true) || r == Some(true) {
                Verdict::Holds
            } else if l == Some(false) && r == Some(false) {
                Verdict::Fails
            } else {
                Verdict::Pending
            }
        }
        (
            Node::Implies {
                ante,
                overlapped,
                conseq,
            },
            NodeState::Implies {
                ante: run,
                live,
                deferred,
            },
        ) => {
            live.append(deferred);
            let mut failed = false;
            let mut keep = Vec::new();
            for mut obligation in live.drain(..) {
                match step(conseq, &mut obligation, values, matched) {
                    Verdict::Fails => failed = true,
                    Verdict::Holds => {}
                    Verdict::Pending => keep.push(obligation),
                }
            }
            *live = keep;
            if failed {
                return Verdict::Fails;
            }
            *run = ante.step(run, values);
            if ante.accepts(run) {
                *matched = true;
                let mut obligation = new_state(conseq);
                if *overlapped {
                    match step(conseq, &mut obligation, values, matched) {
                        Verdict::Fails => return Verdict::Fails,
                        Verdict::Holds => {}
                        Verdict::Pending => live.push(obligation),
                    }
                } else {
                    deferred.push(obligation);
                }
            }
            if ante.is_dead(run) && live.is_empty() && deferred.is_empty() {
                Verdict::Holds
            } else {
                Verdict::Pending
            }
        }
        _ => unreachable!("attempt state does not match its property node"),
    }
}

/// Counts kept for one directive over a whole run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Attempts started.
    pub(crate) attempts: u64,
    /// Attempts that held with an antecedent match (a real pass, or a
    /// cover match).
    pub(crate) passes: u64,
    /// Attempts that held without any antecedent matching.
    pub(crate) vacuous: u64,
    /// Attempts that failed.
    pub(crate) failures: u64,
    /// Attempts abandoned by `disable iff`.
    pub(crate) disabled: u64,
}

/// One concurrent assertion being checked during a run.
pub(crate) struct Checker {
    /// The name used in reports.
    pub(crate) name: String,
    /// Which directive.
    pub(crate) kind: DirectiveKind,
    /// The source span reported against.
    pub(crate) span: Span,
    /// The clock signal.
    pub(crate) clock: SigId,
    /// Which edge of it starts a cycle.
    pub(crate) polarity: crate::ir::Polarity,
    /// The compiled property.
    pub(crate) compiled: Compiled,
    /// Live attempts, oldest first.
    pub(crate) attempts: Vec<Attempt>,
    /// Sampled values of [`Compiled::nets`] at the previous clocking
    /// event.
    pub(crate) previous: Vec<Logic>,
    /// Clocking events seen so far.
    pub(crate) cycle: u64,
    /// True when the clock edge occurred in the current time slot.
    pub(crate) ticked: bool,
    /// Counts over the run.
    pub(crate) stats: Stats,
}

/// What one clocking event produced, for the caller to report.
pub(crate) struct TickReport {
    /// Failing attempts: start time, start cycle.
    pub(crate) failures: Vec<(u64, u64)>,
}

impl Checker {
    /// Runs one clocking event: samples, starts an attempt and advances
    /// every live one.
    ///
    /// `now` holds this cycle's sampled value of every net in
    /// [`Compiled::nets`], in the same order.
    pub(crate) fn tick(&mut self, time: u64, now: &[Logic]) -> TickReport {
        let values = self.compiled.evaluate(now, &self.previous);
        let mut failures = Vec::new();
        if let Some(d) = self.compiled.disable
            && values.get(d.idx()).copied().unwrap_or(false)
        {
            let count = u64::try_from(self.attempts.len()).unwrap_or(u64::MAX);
            self.stats.disabled = self.stats.disabled.saturating_add(count);
            self.attempts.clear();
            self.previous = now.to_vec();
            self.cycle = self.cycle.saturating_add(1);
            return TickReport { failures };
        }
        self.attempts
            .push(Attempt::new(&self.compiled.node, time, self.cycle));
        self.stats.attempts = self.stats.attempts.saturating_add(1);
        let mut keep = Vec::with_capacity(self.attempts.len());
        let implicative = self.compiled.implicative;
        let mut taken = std::mem::take(&mut self.attempts);
        for attempt in &mut taken {
            match attempt.step(&self.compiled.node, &values) {
                Verdict::Pending => keep.push(attempt.clone()),
                Verdict::Holds => {
                    if attempt.matched || !implicative {
                        self.stats.passes = self.stats.passes.saturating_add(1);
                    } else {
                        self.stats.vacuous = self.stats.vacuous.saturating_add(1);
                    }
                }
                Verdict::Fails => {
                    self.stats.failures = self.stats.failures.saturating_add(1);
                    failures.push((attempt.start_time, attempt.start_cycle));
                }
            }
        }
        self.attempts = keep;
        self.previous = now.to_vec();
        self.cycle = self.cycle.saturating_add(1);
        TickReport { failures }
    }

    /// Attempts still pending, which the run ending makes inconclusive.
    pub(crate) fn incomplete(&self) -> u64 {
        u64::try_from(self.attempts.len()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::assertion::property::parse_directive;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("property", "").unwrap();
        Span::new(id, 0, 200)
    }

    /// Compiles against three nets called `a`, `b` and `c`, mapped to
    /// signals 0, 1 and 2.
    fn build(text: &str) -> Compiled {
        let d = parse_directive(text, span()).expect("parses");
        compile(&d, &|name| match name {
            "a" => Some((SigId(0), "top.a".into())),
            "b" => Some((SigId(1), "top.b".into())),
            "c" => Some((SigId(2), "top.c".into())),
            _ => None,
        })
        .expect("compiles")
    }

    /// One cycle of net values from a character: `a`, `b`, `c` set that
    /// net, `.` clears all, upper case sets all but that one.
    fn cycle(c: char) -> Vec<Logic> {
        let bits = match c {
            '.' => [false, false, false],
            '*' => [true, true, true],
            'a' => [true, false, false],
            'b' => [false, true, false],
            'c' => [false, false, true],
            'A' => [false, true, true],
            'B' => [true, false, true],
            'C' => [true, true, false],
            other => panic!("bad trace character `{other}`"),
        };
        bits.iter().map(|v| Logic::from_bool(*v)).collect()
    }

    /// Runs a directive over a trace, one attempt started per cycle, and
    /// returns `(start cycle, end cycle, verdict)` for every attempt that
    /// settled.
    fn run(text: &str, trace: &str) -> (Stats, Vec<(u64, u64)>) {
        let compiled = build(text);
        let mut checker = Checker {
            name: "p".into(),
            kind: DirectiveKind::Assert,
            span: span(),
            clock: SigId(0),
            polarity: crate::ir::Polarity::Pos,
            previous: vec![Logic::x(1); compiled.nets.len()],
            compiled,
            attempts: Vec::new(),
            cycle: 0,
            ticked: false,
            stats: Stats::default(),
        };
        let mut failures = Vec::new();
        for (i, c) in trace.chars().enumerate() {
            let values = cycle(c);
            let picked: Vec<Logic> = checker
                .compiled
                .nets
                .iter()
                .map(|n| values[n.sig.idx()].clone())
                .collect();
            let time = u64::try_from(i).expect("small") * 10;
            let report = checker.tick(time, &picked);
            failures.extend(report.failures);
        }
        assert_eq!(
            checker.incomplete(),
            u64::try_from(checker.attempts.len()).unwrap()
        );
        (checker.stats, failures)
    }

    #[test]
    fn overlapping_implication_attempts() {
        // `a |-> ##2 b`: every cycle starts an attempt, and the ones whose
        // antecedent matched carry an obligation two cycles on.
        let (stats, failures) = run("a |-> ##2 b", "a.b.");
        assert_eq!(failures, Vec::new());
        // Cycle 0 matched `a` and saw `b` at cycle 2.
        assert_eq!(stats.passes, 1);
        // The other three attempts had no antecedent match.
        assert_eq!(stats.vacuous, 3);
        assert_eq!(stats.failures, 0);
        // Two overlapping antecedents, the second one unsatisfied.
        let (stats, failures) = run("a |-> ##2 b", "aab..");
        assert_eq!(stats.passes, 1);
        assert_eq!(stats.failures, 1);
        // The attempt that started at cycle 1 fails at cycle 3.
        assert_eq!(failures, vec![(10, 1)]);
    }

    #[test]
    fn non_overlapped_implication() {
        // `a |=> b`: the consequent starts the cycle after the match.
        let (stats, _) = run("a |=> b", "ab..");
        assert_eq!(stats.failures, 0);
        assert_eq!(stats.passes, 1);
        let (stats, failures) = run("a |=> b", "a...");
        assert_eq!(stats.failures, 1);
        assert_eq!(failures, vec![(0, 0)]);
    }

    #[test]
    fn overlapped_implication_same_cycle() {
        // `a |-> b` needs both in the same cycle.
        let (stats, _) = run("a |-> b", "C...");
        assert_eq!(stats.passes, 1);
        assert_eq!(stats.failures, 0);
        let (stats, _) = run("a |-> b", "ab..");
        assert_eq!(stats.failures, 1);
    }

    #[test]
    fn cover_counts_matches() {
        // Three cycles in which `a ##1 b` matches.
        let (stats, _) = run("a ##1 b", "ab.ab.ab");
        assert_eq!(stats.passes, 3);
    }

    #[test]
    fn disable_iff_abandons_attempts() {
        let (stats, failures) = run("disable iff (c) a |=> b", "ac..");
        assert_eq!(failures, Vec::new());
        assert_eq!(stats.failures, 0);
        assert!(stats.disabled >= 1);
    }

    #[test]
    fn property_operators() {
        // `not` inverts a sequence verdict.
        let (stats, _) = run("not (a ##1 b)", "ab..");
        assert_eq!(stats.failures, 1);
        let (stats, _) = run("not (a ##1 b)", "a...");
        assert_eq!(stats.failures, 0);
        // A property-level `and`.
        let (stats, _) = run("(a |-> b) and (b |-> a)", "C...");
        assert_eq!(stats.failures, 0);
        let (stats, _) = run("(a |-> b) and (b |-> a)", "ab..");
        assert!(stats.failures > 0);
        // A property-level `or` holds as soon as one side does.
        let (stats, _) = run("(a |-> b) or (a |-> c)", "B...");
        assert_eq!(stats.failures, 0);
    }

    #[test]
    fn sampled_value_functions() {
        // `$rose(a) |-> b`
        let (stats, _) = run("$rose(a) |-> b", ".C..");
        assert_eq!(stats.failures, 0);
        assert_eq!(stats.passes, 1);
        let (stats, _) = run("$rose(a) |-> b", ".a..");
        assert_eq!(stats.failures, 1);
        let (stats, _) = run("$fell(a) |-> b", "ab..");
        assert_eq!(stats.failures, 0);
        let (stats, _) = run("$stable(a) |-> b", "..b");
        assert_eq!(stats.failures, 1);
    }

    #[test]
    fn predicates_are_interned() {
        // `a` appears three times but is evaluated once per cycle.
        let compiled = build("a |-> (a ##1 a)");
        assert_eq!(compiled.pred_count(), 1);
        let compiled = build("a |-> b");
        assert_eq!(compiled.pred_count(), 2);
    }

    #[test]
    fn unknown_nets_and_oversized_ranges_are_diagnostics() {
        let d = parse_directive("a |-> zz", span()).expect("parses");
        let err = compile(&d, &|name| {
            (name == "a").then(|| (SigId(0), "top.a".into()))
        })
        .expect_err("unknown net");
        assert!(err.message.contains("unknown net `zz`"));
        let d = parse_directive("a[*1:1000000]", span()).expect("parses");
        let err = compile(&d, &|_| Some((SigId(0), "top.a".into()))).expect_err("too large");
        assert!(err.message.contains("cycle range"), "{}", err.message);
    }

    #[test]
    fn selects_and_comparisons() {
        let compiled = build("a[0] == 1'b1");
        assert_eq!(compiled.nets.len(), 1);
        let now = [Logic::from_u64(1, 4)];
        assert_eq!(compiled.evaluate(&now, &now), vec![true]);
        let now = [Logic::from_u64(2, 4)];
        assert_eq!(compiled.evaluate(&now, &now), vec![false]);
        // An out-of-range select is unknown, which is not a match.
        let compiled = build("a[9]");
        assert_eq!(compiled.evaluate(&now, &now), vec![false]);
        let compiled = build("a[3:2] == 2'b00");
        assert_eq!(compiled.evaluate(&now, &now), vec![true]);
        let compiled = build("a > 1");
        assert_eq!(compiled.evaluate(&now, &now), vec![true]);
        let compiled = build("!a");
        assert_eq!(compiled.evaluate(&now, &now), vec![false]);
        let compiled = build("a -> b");
        let now = [Logic::from_u64(1, 4), Logic::from_u64(0, 4)];
        assert_eq!(compiled.evaluate(&now, &now), vec![false]);
    }
}
