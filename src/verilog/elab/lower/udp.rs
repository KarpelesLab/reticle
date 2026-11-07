//! User-defined primitives (IEEE 1364-2005 §8).
//!
//! A primitive's table becomes one process. Its output is the first port,
//! its inputs the others, in order. Each row is a condition on the inputs
//! (and, for a sequential primitive, on the current output), the first
//! that holds gives the output, and none holding gives `x`:
//!
//! - a level symbol matches a value by `===`: `0`, `1` and `x` themselves,
//!   `b` either of 0 and 1, `?` anything;
//! - an edge, `(vw)`, `r`, `f`, `p`, `n` or `*`, matches when that input
//!   went from a value matching `v` to one matching `w` since the process
//!   last ran, which a sequential primitive keeps in a variable per input;
//! - `-` as the next state keeps the output.
//!
//! Rows on levels alone are tried before rows with an edge, which is the
//! precedence the standard gives them (§8.6). A combinational primitive's
//! process runs at time zero, as its inputs' first values reach it; a
//! sequential one's waits for a change, its output starting at its
//! `initial` value or `x`.

use crate::ir::{self, AssignKind, BinaryOp, ExprId, NetId, ProcessKind};
use crate::logic::Logic;
use crate::source::Span;
use crate::verilog::ast;

use super::super::codes;
use super::super::types::{Packed, VType};
use super::Lowerer;

/// One input field of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    /// A level symbol.
    Level(char),
    /// An edge, from and to.
    Edge(char, char),
    /// One of several edges: `p` (rising), `n` (falling) or `*` (any).
    Edges(char),
}

/// One parsed row: input fields, the current-state symbol (sequential),
/// and the output symbol.
struct Row {
    inputs: Vec<Field>,
    state: Option<char>,
    output: char,
    span: Span,
}

/// Parses one row's text, `( 01 ) 0 : ? : 0`.
fn parse_row(text: &str) -> Option<(Vec<Field>, Option<char>, char)> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let parts: Vec<&str> = compact.split(':').collect();
    let (inputs, state, output) = match parts.as_slice() {
        [i, o] => (*i, None, *o),
        [i, s, o] => (*i, Some(*s), *o),
        _ => return None,
    };
    let mut fields = Vec::new();
    let mut chars = inputs.chars().map(|c| c.to_ascii_lowercase());
    while let Some(c) = chars.next() {
        fields.push(match c {
            '(' => {
                let a = chars.next()?;
                let b = chars.next()?;
                if chars.next()? != ')' {
                    return None;
                }
                Field::Edge(a, b)
            }
            'r' => Field::Edge('0', '1'),
            'f' => Field::Edge('1', '0'),
            'p' | 'n' | '*' => Field::Edges(c),
            '0' | '1' | 'x' | 'b' | '?' => Field::Level(c),
            _ => return None,
        });
    }
    let one = |s: &str| -> Option<char> {
        let mut it = s.chars();
        let c = it.next()?.to_ascii_lowercase();
        it.next().is_none().then_some(c)
    };
    let state = match state {
        Some(s) => Some(one(s)?),
        None => None,
    };
    Some((fields, state, one(output)?))
}

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    /// The table of the primitive being lowered.
    pub(super) fn udp_table(&mut self, rows: &'ast [ast::RawTokens], item: &'ast ast::Item) {
        let ports: Vec<Option<NetId>> = self.ports.iter().map(|p| p.net).collect();
        let (Some(&Some(out)), inputs) = (ports.first(), &ports[1.min(ports.len())..]) else {
            self.env
                .error(codes::TYPE, item.span, "a primitive needs an output port");
            return;
        };
        let Some(inputs) = inputs.iter().copied().collect::<Option<Vec<NetId>>>() else {
            self.env.error(
                codes::TYPE,
                item.span,
                "a primitive's ports must all be declared",
            );
            return;
        };
        let mut parsed = Vec::with_capacity(rows.len());
        for raw in rows {
            match parse_row(&raw.text) {
                Some((fields, state, output)) if fields.len() == inputs.len() => {
                    parsed.push(Row {
                        inputs: fields,
                        state,
                        output,
                        span: raw.span,
                    });
                }
                _ => {
                    self.env.error(
                        codes::TYPE,
                        raw.span,
                        format!(
                            "this table row is not {} input symbol(s), a state and an output",
                            inputs.len()
                        ),
                    );
                    return;
                }
            }
        }
        let sequential = parsed.iter().any(|r| r.state.is_some());
        // Rows on levels first: they take precedence over edges.
        parsed.sort_by_key(|r| r.inputs.iter().any(|f| !matches!(f, Field::Level(_))));
        // The values the inputs had when the process last ran.
        let mut prev = Vec::new();
        if sequential {
            for (i, _) in inputs.iter().enumerate() {
                let name = self.fresh(&format!("udp_prev{i}"));
                let Some(net) = self.new_var(name, VType::Packed(Packed::bits(1)), item.span)
                else {
                    return;
                };
                prev.push(net);
            }
        }
        let kind = if sequential {
            ProcessKind::Sensitive(inputs.clone())
        } else {
            ProcessKind::Comb
        };
        self.b.span = item.span;
        let mut p = self.b.process(None, kind);
        p.span = item.span;
        if sequential {
            p.attrs.set(ir::WAITS_FIRST_ATTR, ir::AttrValue::Int(1));
        }
        // An `if` chain over the rows, `x` when none matches.
        let x = self.b.constant(Logic::x(1));
        let mut rest = self.b.block();
        rest.span = item.span;
        rest.assign(out, x, AssignKind::Blocking);
        let mut chain = rest.finish();
        for row in parsed.iter().rev() {
            self.b.span = row.span;
            let mut cond = self.b.const_bit(true);
            for (i, field) in row.inputs.iter().enumerate() {
                let now = self.b.net(inputs[i]);
                let hit = match *field {
                    Field::Level(c) => self.level_match(now, c),
                    Field::Edge(a, b) => {
                        let Some(&before) = prev.get(i) else {
                            self.env.error(
                                codes::TYPE,
                                row.span,
                                "an edge in a combinational primitive's table",
                            );
                            return;
                        };
                        let was = self.b.net(before);
                        let from = self.level_match(was, a);
                        let to = self.level_match(now, b);
                        let changed = self.b.binary(BinaryOp::CaseNe, was, now);
                        let both = self.b.land(from, to);
                        self.b.land(both, changed)
                    }
                    Field::Edges(c) => {
                        let Some(&before) = prev.get(i) else {
                            self.env.error(
                                codes::TYPE,
                                row.span,
                                "an edge in a combinational primitive's table",
                            );
                            return;
                        };
                        let pairs: &[(char, char)] = match c {
                            'p' => &[('0', '1'), ('0', 'x'), ('x', '1')],
                            'n' => &[('1', '0'), ('1', 'x'), ('x', '0')],
                            _ => &[('?', '?')],
                        };
                        let was = self.b.net(before);
                        let changed = self.b.binary(BinaryOp::CaseNe, was, now);
                        let mut any = self.b.const_bit(false);
                        for (a, b) in pairs {
                            let from = self.level_match(was, *a);
                            let to = self.level_match(now, *b);
                            let both = self.b.land(from, to);
                            any = self.b.lor(any, both);
                        }
                        self.b.land(any, changed)
                    }
                };
                cond = self.b.land(cond, hit);
            }
            if let Some(s) = row.state {
                let q = self.b.net(out);
                let hit = self.level_match(q, s);
                cond = self.b.land(cond, hit);
            }
            let mut then = self.b.block();
            then.span = row.span;
            match row.output {
                '-' => {}
                c => {
                    let v = match c {
                        '0' => Logic::zero(1),
                        '1' => Logic::ones(1),
                        _ => Logic::x(1),
                    };
                    let v = self.b.constant(v);
                    then.assign(out, v, AssignKind::Blocking);
                }
            }
            let mut b = self.b.block();
            b.span = row.span;
            b.if_(cond, then.finish(), chain);
            chain = b.finish();
        }
        for stmt in chain {
            p.span = stmt.span;
            p.push(stmt.kind);
        }
        for (i, before) in prev.iter().enumerate() {
            let now = self.b.net(inputs[i]);
            p.span = item.span;
            p.assign(*before, now, AssignKind::Blocking);
        }
        self.b.end_process(p);
        let _ = self.record_udp_driver(out, item.span);
    }

    /// Whether `value` (one bit) matches the level symbol `c`.
    fn level_match(&mut self, value: ExprId, c: char) -> ExprId {
        match c {
            // An input at `z` is read as `x` (IEEE 1364-2005 §8.1.6).
            'x' => {
                let x = self.b.constant(Logic::x(1));
                let z = self.b.constant(Logic::z(1));
                let a = self.b.binary(BinaryOp::CaseEq, value, x);
                let b = self.b.binary(BinaryOp::CaseEq, value, z);
                self.b.lor(a, b)
            }
            '0' | '1' => {
                let v = match c {
                    '0' => Logic::zero(1),
                    '1' => Logic::ones(1),
                    _ => Logic::x(1),
                };
                let v = self.b.constant(v);
                self.b.binary(BinaryOp::CaseEq, value, v)
            }
            'b' => {
                let zero = self.b.constant(Logic::zero(1));
                let one = self.b.constant(Logic::ones(1));
                let a = self.b.binary(BinaryOp::CaseEq, value, zero);
                let b = self.b.binary(BinaryOp::CaseEq, value, one);
                self.b.lor(a, b)
            }
            _ => self.b.const_bit(true),
        }
    }

    /// The process is the output's driver.
    fn record_udp_driver(&mut self, out: NetId, span: Span) -> Option<()> {
        let index = self.b.module().processes.len().checked_sub(1)?;
        self.record_driver(out, super::DriverKey::Process(index), true, span);
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_parse() {
        assert_eq!(
            parse_row("0 0 ? : 0"),
            Some((
                vec![Field::Level('0'), Field::Level('0'), Field::Level('?')],
                None,
                '0'
            ))
        );
        assert_eq!(
            parse_row("( 01 ) 0 : ? : -"),
            Some((
                vec![Field::Edge('0', '1'), Field::Level('0')],
                Some('?'),
                '-'
            ))
        );
        assert_eq!(
            parse_row("r * : ? : 1"),
            Some((
                vec![Field::Edge('0', '1'), Field::Edges('*')],
                Some('?'),
                '1'
            ))
        );
        assert_eq!(parse_row("0 0"), None);
        assert_eq!(parse_row("(0 1 : 0"), None);
    }
}
