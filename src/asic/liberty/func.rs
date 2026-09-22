//! Liberty boolean expressions: the `function`, `three_state`,
//! `next_state`, `clocked_on` and `when` attributes.
//!
//! Syntax as the Liberty reference defines it:
//!
//! | Operator      | Meaning | Precedence   |
//! |---------------|---------|--------------|
//! | `'` (postfix) | NOT     | highest      |
//! | `!` (prefix)  | NOT     |              |
//! | `^`           | XOR     |              |
//! | `&`, `*`, juxtaposition (`A B`) | AND | |
//! | `+`, `\|`     | OR      | lowest       |
//!
//! XOR binds tighter than AND, which differs from most programming
//! languages; the tests pin this down. `0` and `1` are constants and
//! parentheses group. Pin names may contain `[`, `]`, `.` and `_`.
//!
//! [`BoolExpr::eval`] and [`BoolExpr::truth_table`] are what the
//! standard-cell mapper uses to match a cell's function against a cut.

use std::fmt;

/// A parsed Liberty boolean expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoolExpr {
    /// `0` or `1`.
    Const(bool),
    /// A pin name.
    Pin(String),
    /// Negation.
    Not(Box<BoolExpr>),
    /// Conjunction of two or more terms.
    And(Vec<BoolExpr>),
    /// Disjunction of two or more terms.
    Or(Vec<BoolExpr>),
    /// Exclusive or.
    Xor(Box<BoolExpr>, Box<BoolExpr>),
}

/// Why a function string did not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuncError {
    /// Byte offset of the problem inside the function string.
    pub offset: usize,
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for FuncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at offset {}", self.message, self.offset)
    }
}

impl std::error::Error for FuncError {}

impl BoolExpr {
    /// Parses a Liberty function string.
    pub fn parse(text: &str) -> Result<BoolExpr, FuncError> {
        let mut p = FuncParser {
            text,
            chars: text.char_indices().peekable(),
        };
        p.skip_ws();
        if p.peek().is_none() {
            return Err(FuncError {
                offset: 0,
                message: "empty function".into(),
            });
        }
        let e = p.or()?;
        p.skip_ws();
        match p.peek() {
            None => Ok(e),
            Some((i, c)) => Err(FuncError {
                offset: i,
                message: format!("unexpected `{c}`"),
            }),
        }
    }

    /// The pin names referenced, in order of first appearance, without
    /// duplicates.
    pub fn variables(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_vars(&mut out);
        out
    }

    fn collect_vars(&self, out: &mut Vec<String>) {
        match self {
            BoolExpr::Const(_) => {}
            BoolExpr::Pin(p) => {
                if !out.iter().any(|v| v == p) {
                    out.push(p.clone());
                }
            }
            BoolExpr::Not(e) => e.collect_vars(out),
            BoolExpr::And(v) | BoolExpr::Or(v) => v.iter().for_each(|e| e.collect_vars(out)),
            BoolExpr::Xor(a, b) => {
                a.collect_vars(out);
                b.collect_vars(out);
            }
        }
    }

    /// Evaluates the expression with `values[i]` as the value of pin
    /// `inputs[i]`. A pin missing from `inputs` reads as `false`.
    pub fn eval(&self, inputs: &[&str], values: &[bool]) -> bool {
        match self {
            BoolExpr::Const(b) => *b,
            BoolExpr::Pin(p) => inputs
                .iter()
                .position(|n| n == p)
                .and_then(|i| values.get(i).copied())
                .unwrap_or(false),
            BoolExpr::Not(e) => !e.eval(inputs, values),
            BoolExpr::And(v) => v.iter().all(|e| e.eval(inputs, values)),
            BoolExpr::Or(v) => v.iter().any(|e| e.eval(inputs, values)),
            BoolExpr::Xor(a, b) => a.eval(inputs, values) ^ b.eval(inputs, values),
        }
    }

    /// The truth table over the given input order, for up to six inputs:
    /// bit `i` of the result is the output for the input assignment whose
    /// bit `j` is the value of `inputs[j]`. `None` with more than six
    /// inputs.
    pub fn truth_table(&self, inputs: &[&str]) -> Option<u64> {
        if inputs.len() > 6 {
            return None;
        }
        let mut table = 0u64;
        let mut values = vec![false; inputs.len()];
        for row in 0..(1u64 << inputs.len()) {
            for (j, v) in values.iter_mut().enumerate() {
                *v = (row >> j) & 1 == 1;
            }
            if self.eval(inputs, &values) {
                table |= 1 << row;
            }
        }
        Some(table)
    }

    /// True when the expression is a single pin, optionally negated.
    pub fn is_literal(&self) -> bool {
        match self {
            BoolExpr::Pin(_) => true,
            BoolExpr::Not(e) => matches!(**e, BoolExpr::Pin(_)),
            _ => false,
        }
    }

    fn precedence(&self) -> u8 {
        match self {
            BoolExpr::Const(_) | BoolExpr::Pin(_) | BoolExpr::Not(_) => 4,
            BoolExpr::Xor(..) => 3,
            BoolExpr::And(_) => 2,
            BoolExpr::Or(_) => 1,
        }
    }

    fn write(&self, f: &mut fmt::Formatter<'_>, parent: u8) -> fmt::Result {
        let mine = self.precedence();
        let paren = mine < parent;
        if paren {
            f.write_str("(")?;
        }
        match self {
            BoolExpr::Const(b) => f.write_str(if *b { "1" } else { "0" })?,
            BoolExpr::Pin(p) => f.write_str(p)?,
            BoolExpr::Not(e) => {
                f.write_str("!")?;
                e.write(f, 4)?;
            }
            BoolExpr::And(v) => {
                for (i, e) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str("&")?;
                    }
                    e.write(f, mine)?;
                }
            }
            BoolExpr::Or(v) => {
                for (i, e) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str("|")?;
                    }
                    e.write(f, mine)?;
                }
            }
            BoolExpr::Xor(a, b) => {
                a.write(f, mine)?;
                f.write_str("^")?;
                // Right operand needs parentheses when it is itself an
                // XOR, to keep the tree shape on re-parse.
                b.write(f, mine + 1)?;
            }
        }
        if paren {
            f.write_str(")")?;
        }
        Ok(())
    }
}

impl fmt::Display for BoolExpr {
    /// Canonical form: `!` for NOT, `&` for AND, `|` for OR, `^` for XOR,
    /// no spaces, parentheses only where precedence requires them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, 0)
    }
}

struct FuncParser<'a> {
    text: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
}

impl FuncParser<'_> {
    fn peek(&mut self) -> Option<(usize, char)> {
        self.chars.peek().copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some((_, c)) if c.is_whitespace()) {
            self.chars.next();
        }
    }

    fn offset(&mut self) -> usize {
        self.peek().map_or(self.text.len(), |(i, _)| i)
    }

    fn error(&mut self, message: impl Into<String>) -> FuncError {
        FuncError {
            offset: self.offset(),
            message: message.into(),
        }
    }

    fn or(&mut self) -> Result<BoolExpr, FuncError> {
        let mut terms = vec![self.and()?];
        loop {
            self.skip_ws();
            match self.peek() {
                Some((_, '+' | '|')) => {
                    self.chars.next();
                    terms.push(self.and()?);
                }
                _ => break,
            }
        }
        Ok(if terms.len() == 1 {
            terms.pop().expect("one term")
        } else {
            BoolExpr::Or(terms)
        })
    }

    fn and(&mut self) -> Result<BoolExpr, FuncError> {
        let mut terms = vec![self.xor()?];
        loop {
            self.skip_ws();
            match self.peek() {
                Some((_, '&' | '*')) => {
                    self.chars.next();
                    terms.push(self.xor()?);
                }
                // Juxtaposition: `A B` or `A (B+C)` or `A !B`.
                Some((_, c)) if c == '(' || c == '!' || is_name_char(c) => {
                    terms.push(self.xor()?);
                }
                _ => break,
            }
        }
        Ok(if terms.len() == 1 {
            terms.pop().expect("one term")
        } else {
            BoolExpr::And(terms)
        })
    }

    fn xor(&mut self) -> Result<BoolExpr, FuncError> {
        let mut lhs = self.unary()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some((_, '^')) => {
                    self.chars.next();
                    let rhs = self.unary()?;
                    lhs = BoolExpr::Xor(Box::new(lhs), Box::new(rhs));
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<BoolExpr, FuncError> {
        self.skip_ws();
        let mut e = match self.peek() {
            Some((_, '!')) => {
                self.chars.next();
                BoolExpr::Not(Box::new(self.unary()?))
            }
            Some((_, '(')) => {
                self.chars.next();
                let inner = self.or()?;
                self.skip_ws();
                match self.peek() {
                    Some((_, ')')) => {
                        self.chars.next();
                    }
                    _ => return Err(self.error("expected `)`")),
                }
                inner
            }
            Some((_, '0')) if !self.name_continues() => {
                self.chars.next();
                BoolExpr::Const(false)
            }
            Some((_, '1')) if !self.name_continues() => {
                self.chars.next();
                BoolExpr::Const(true)
            }
            Some((start, c)) if is_name_char(c) => {
                let mut end = start;
                while let Some((i, c)) = self.peek() {
                    if is_name_char(c) {
                        end = i + c.len_utf8();
                        self.chars.next();
                    } else {
                        break;
                    }
                }
                BoolExpr::Pin(self.text[start..end].to_string())
            }
            Some((_, c)) => return Err(self.error(format!("unexpected `{c}`"))),
            None => return Err(self.error("unexpected end of expression")),
        };
        // Postfix `'`, possibly repeated.
        loop {
            self.skip_ws();
            if matches!(self.peek(), Some((_, '\''))) {
                self.chars.next();
                e = BoolExpr::Not(Box::new(e));
            } else {
                break;
            }
        }
        Ok(e)
    }

    /// True when the character after the current one continues a name
    /// (so `0` in `A0` is not a constant).
    fn name_continues(&mut self) -> bool {
        let mut look = self.chars.clone();
        look.next();
        look.peek().is_some_and(|(_, c)| is_name_char(*c))
    }
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '[' | ']' | '.' | '$')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> BoolExpr {
        BoolExpr::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn precedence_not_xor_and_or() {
        assert_eq!(p("A&B|C").to_string(), "A&B|C");
        assert_eq!(p("A|B&C").to_string(), "A|B&C");
        assert_eq!(p("(A|B)&C").to_string(), "(A|B)&C");
        // XOR binds tighter than AND in Liberty.
        assert_eq!(p("A&B^C").to_string(), "A&B^C");
        assert_eq!(
            p("A&B^C"),
            BoolExpr::And(vec![
                BoolExpr::Pin("A".into()),
                BoolExpr::Xor(
                    Box::new(BoolExpr::Pin("B".into())),
                    Box::new(BoolExpr::Pin("C".into()))
                ),
            ])
        );
        assert_eq!(p("(A&B)^C").to_string(), "(A&B)^C");
        // NOT binds tightest, in both spellings.
        assert_eq!(p("!A&B").to_string(), "!A&B");
        assert_eq!(p("A'&B").to_string(), "!A&B");
        assert_eq!(p("!(A&B)").to_string(), "!(A&B)");
        assert_eq!(p("(A&B)'").to_string(), "!(A&B)");
        assert_eq!(
            p("A''"),
            BoolExpr::Not(Box::new(BoolExpr::Not(Box::new(BoolExpr::Pin("A".into())))))
        );
    }

    #[test]
    fn alternative_spellings_and_juxtaposition() {
        assert_eq!(p("A*B+C").to_string(), "A&B|C");
        assert_eq!(p("A B"), p("A&B"));
        assert_eq!(p("A !B"), p("A&!B"));
        assert_eq!(p("A (B+C)"), p("A&(B|C)"));
        assert_eq!(p("(A1&A2)|B1").to_string(), "A1&A2|B1");
        assert_eq!(p(" ( A1 & A2 ) | B1 ").to_string(), "A1&A2|B1");
    }

    #[test]
    fn constants_and_names() {
        assert_eq!(p("0"), BoolExpr::Const(false));
        assert_eq!(p("1"), BoolExpr::Const(true));
        assert_eq!(p("A0"), BoolExpr::Pin("A0".into()));
        assert_eq!(p("D[3]&IQ_N"), p("D[3] & IQ_N"));
        assert_eq!(p("1&A").to_string(), "1&A");
    }

    #[test]
    fn parse_errors_carry_offsets() {
        let e = BoolExpr::parse("A&").unwrap_err();
        assert_eq!(e.offset, 2);
        let e = BoolExpr::parse("(A").unwrap_err();
        assert_eq!(e.offset, 2);
        let e = BoolExpr::parse("A)").unwrap_err();
        assert_eq!(e.offset, 1);
        assert!(BoolExpr::parse("").is_err());
        assert!(BoolExpr::parse("A # B").is_err());
    }

    #[test]
    fn eval_and_truth_tables() {
        let nand = p("!(A&B)");
        assert_eq!(nand.variables(), vec!["A", "B"]);
        assert_eq!(nand.truth_table(&["A", "B"]), Some(0b0111));
        let and = p("A&B");
        assert_eq!(and.truth_table(&["A", "B"]), Some(0b1000));
        let xor = p("A^B");
        assert_eq!(xor.truth_table(&["A", "B"]), Some(0b0110));
        let mux = p("(A&!S)|(B&S)");
        assert_eq!(mux.variables(), vec!["A", "S", "B"]);
        // Inputs A=bit0, B=bit1, S=bit2.
        let t = mux.truth_table(&["A", "B", "S"]).unwrap();
        for row in 0..8u64 {
            let a = row & 1 == 1;
            let b = row >> 1 & 1 == 1;
            let s = row >> 2 & 1 == 1;
            assert_eq!((t >> row) & 1 == 1, if s { b } else { a }, "row {row}");
        }
        assert!(p("A").eval(&["A"], &[true]));
        assert!(!p("Z").eval(&["A"], &[true]), "unknown pins read as false");
        let seven = ["a", "b", "c", "d", "e", "f", "g"];
        assert_eq!(p("a").truth_table(&seven), None);
        assert_eq!(p("a").truth_table(&seven[..6]), Some(0xAAAA_AAAA_AAAA_AAAA));
    }

    #[test]
    fn display_round_trips() {
        for s in [
            "A&B|C",
            "!(A&B)",
            "A^B^C",
            "A^(B^C)",
            "(A^B)&C",
            "!A&!B|C&D",
            "A&(B|C)&D",
            "1",
            "!A",
        ] {
            let e = p(s);
            let printed = e.to_string();
            assert_eq!(p(&printed), e, "{s} -> {printed}");
        }
    }
}
