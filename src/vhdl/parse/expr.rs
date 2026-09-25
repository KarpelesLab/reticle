//! Expressions, by precedence level (IEEE 1076-2008 clause 9):
//!
//! ```text
//! expression        ::= ?? primary | logical_expression
//! logical           ::= relation { and|or|xor|xnor relation } | relation [ nand|nor relation ]
//! relation          ::= shift [ = /= < <= > >= ?= ?/= ?< ?<= ?> ?>= shift ]
//! shift             ::= simple [ sll srl sla sra rol ror simple ]
//! simple            ::= [ + | - ] term { + | - | & term }
//! term              ::= factor { * | / | mod | rem factor }
//! factor            ::= primary [ ** primary ] | abs primary | not primary | logical_op primary
//! primary           ::= name | literal | aggregate | qualified | allocator | ( expression )
//! ```

use super::{PResult, Parser, char_of, ident_of};
use crate::diag::Diagnostic;
use crate::source::Span;
use crate::vhdl::ast::*;
use crate::vhdl::token::TokenKind;

/// The binary operator a token denotes, if any, with its precedence class.
fn binary_op(k: TokenKind) -> Option<BinaryOp> {
    Some(match k {
        TokenKind::And => BinaryOp::And,
        TokenKind::Or => BinaryOp::Or,
        TokenKind::Nand => BinaryOp::Nand,
        TokenKind::Nor => BinaryOp::Nor,
        TokenKind::Xor => BinaryOp::Xor,
        TokenKind::Xnor => BinaryOp::Xnor,
        TokenKind::Eq => BinaryOp::Eq,
        TokenKind::Neq => BinaryOp::Neq,
        TokenKind::Lt => BinaryOp::Lt,
        TokenKind::Le => BinaryOp::Le,
        TokenKind::Gt => BinaryOp::Gt,
        TokenKind::Ge => BinaryOp::Ge,
        TokenKind::QEq => BinaryOp::MatchEq,
        TokenKind::QNeq => BinaryOp::MatchNeq,
        TokenKind::QLt => BinaryOp::MatchLt,
        TokenKind::QLe => BinaryOp::MatchLe,
        TokenKind::QGt => BinaryOp::MatchGt,
        TokenKind::QGe => BinaryOp::MatchGe,
        TokenKind::Sll => BinaryOp::Sll,
        TokenKind::Srl => BinaryOp::Srl,
        TokenKind::Sla => BinaryOp::Sla,
        TokenKind::Sra => BinaryOp::Sra,
        TokenKind::Rol => BinaryOp::Rol,
        TokenKind::Ror => BinaryOp::Ror,
        TokenKind::Plus => BinaryOp::Add,
        TokenKind::Minus => BinaryOp::Sub,
        TokenKind::Amp => BinaryOp::Concat,
        TokenKind::Star => BinaryOp::Mul,
        TokenKind::Slash => BinaryOp::Div,
        TokenKind::Mod => BinaryOp::Mod,
        TokenKind::Rem => BinaryOp::Rem,
        TokenKind::StarStar => BinaryOp::Pow,
        _ => return None,
    })
}

fn is_relational(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq
            | BinaryOp::Neq
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::MatchEq
            | BinaryOp::MatchNeq
            | BinaryOp::MatchLt
            | BinaryOp::MatchLe
            | BinaryOp::MatchGt
            | BinaryOp::MatchGe
    )
}

fn is_shift(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Sll
            | BinaryOp::Srl
            | BinaryOp::Sla
            | BinaryOp::Sra
            | BinaryOp::Rol
            | BinaryOp::Ror
    )
}

fn is_adding(op: BinaryOp) -> bool {
    matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Concat)
}

fn is_multiplying(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Rem
    )
}

/// The unary reduction operator a logical keyword denotes (VHDL-2008).
fn reduction_op(k: TokenKind) -> Option<UnaryOp> {
    Some(match k {
        TokenKind::And => UnaryOp::And,
        TokenKind::Or => UnaryOp::Or,
        TokenKind::Nand => UnaryOp::Nand,
        TokenKind::Nor => UnaryOp::Nor,
        TokenKind::Xor => UnaryOp::Xor,
        TokenKind::Xnor => UnaryOp::Xnor,
        _ => return None,
    })
}

fn binary(op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
    let span = lhs.span().to(rhs.span());
    Expr::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
        span,
    }
}

impl<'t, 'src> Parser<'t, 'src> {
    /// An expression.
    pub(super) fn parse_expr(&mut self) -> PResult<Expr> {
        if self.at(TokenKind::QQ) {
            let start = self.bump().span;
            self.require_2008(start, "the `??` condition operator");
            let operand = self.parse_primary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Condition,
                operand: Box::new(operand),
                span: self.span_from(start),
            });
        }
        self.parse_logical()
    }

    /// A condition that resumes at `stop` when it fails to parse, so the
    /// enclosing statement survives with an [`Expr::Error`] placeholder.
    pub(super) fn parse_condition_until(&mut self, stop: TokenKind) -> PResult<Expr> {
        let start = self.span();
        match self.parse_expr() {
            Ok(e) => Ok(e),
            Err(r) => {
                if self.skip_to(&[stop]) {
                    Ok(Expr::Error(self.span_from(start)))
                } else {
                    Err(r)
                }
            }
        }
    }

    /// `relation { logical_op relation }`, rejecting mixed operators.
    fn parse_logical(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_relation()?;
        let mut first: Option<(BinaryOp, Span)> = None;
        while let Some(op) = binary_op(self.kind()).filter(|op| op.is_logical()) {
            let op_span = self.bump().span;
            match first {
                None => first = Some((op, op_span)),
                Some((prev, prev_span)) => {
                    if prev != op {
                        self.report(
                            Diagnostic::error(format!(
                                "`{}` and `{}` cannot be mixed without parentheses",
                                prev.as_str(),
                                op.as_str()
                            ))
                            .with_span(op_span)
                            .with_secondary(prev_span, "first operator here"),
                        );
                    } else if matches!(op, BinaryOp::Nand | BinaryOp::Nor) {
                        self.report(
                            Diagnostic::error(format!(
                                "`{}` is not associative; use parentheses",
                                op.as_str()
                            ))
                            .with_span(op_span),
                        );
                    }
                }
            }
            let rhs = self.parse_relation()?;
            lhs = binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    /// `shift [ relational_op shift ]`
    fn parse_relation(&mut self) -> PResult<Expr> {
        let lhs = self.parse_shift()?;
        if let Some(op) = binary_op(self.kind()).filter(|op| is_relational(*op)) {
            let t = self.bump();
            if matches!(
                t.kind,
                TokenKind::QEq
                    | TokenKind::QNeq
                    | TokenKind::QLt
                    | TokenKind::QLe
                    | TokenKind::QGt
                    | TokenKind::QGe
            ) {
                self.require_2008(t.span, "matching relational operators");
            }
            let rhs = self.parse_shift()?;
            return Ok(binary(op, lhs, rhs));
        }
        Ok(lhs)
    }

    /// `simple [ shift_op simple ]`
    fn parse_shift(&mut self) -> PResult<Expr> {
        let lhs = self.parse_simple_expression()?;
        if let Some(op) = binary_op(self.kind()).filter(|op| is_shift(*op)) {
            self.bump();
            let rhs = self.parse_simple_expression()?;
            return Ok(binary(op, lhs, rhs));
        }
        Ok(lhs)
    }

    /// `[sign] term { adding_op term }`
    pub(super) fn parse_simple_expression(&mut self) -> PResult<Expr> {
        let start = self.span();
        let sign = match self.kind() {
            TokenKind::Plus => Some(UnaryOp::Plus),
            TokenKind::Minus => Some(UnaryOp::Minus),
            _ => None,
        };
        let mut lhs = if let Some(op) = sign {
            self.bump();
            let operand = self.parse_term()?;
            Expr::Unary {
                op,
                operand: Box::new(operand),
                span: self.span_from(start),
            }
        } else {
            self.parse_term()?
        };
        while let Some(op) = binary_op(self.kind()).filter(|op| is_adding(*op)) {
            self.bump();
            let rhs = self.parse_term()?;
            lhs = binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    /// `factor { multiplying_op factor }`
    fn parse_term(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_factor()?;
        while let Some(op) = binary_op(self.kind()).filter(|op| is_multiplying(*op)) {
            self.bump();
            let rhs = self.parse_factor()?;
            lhs = binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    /// `primary [** primary] | abs primary | not primary | logical_op primary`
    fn parse_factor(&mut self) -> PResult<Expr> {
        let start = self.span();
        let unary = match self.kind() {
            TokenKind::Abs => Some(UnaryOp::Abs),
            TokenKind::Not => Some(UnaryOp::Not),
            k => reduction_op(k),
        };
        if let Some(op) = unary {
            self.bump();
            if !matches!(op, UnaryOp::Abs | UnaryOp::Not) {
                self.require_2008(start, "unary reduction operators");
            }
            let operand = self.parse_primary()?;
            return Ok(Expr::Unary {
                op,
                operand: Box::new(operand),
                span: self.span_from(start),
            });
        }
        let lhs = self.parse_primary()?;
        if self.at(TokenKind::StarStar) {
            self.bump();
            let rhs = self.parse_primary()?;
            if self.at(TokenKind::StarStar) {
                self.error_here("`**` is not associative; use parentheses");
            }
            return Ok(binary(BinaryOp::Pow, lhs, rhs));
        }
        Ok(lhs)
    }

    /// A primary: literal, name, aggregate, qualified expression, allocator
    /// or parenthesised expression.
    pub(super) fn parse_primary(&mut self) -> PResult<Expr> {
        let start = self.span();
        match self.kind() {
            TokenKind::Integer | TokenKind::Real => {
                let t = self.bump();
                let kind = if self.at(TokenKind::Ident) {
                    let unit = ident_of(self.bump());
                    LiteralKind::Physical {
                        value: t.text().to_owned(),
                        unit,
                    }
                } else if t.kind == TokenKind::Integer {
                    LiteralKind::Integer(t.text().to_owned())
                } else {
                    LiteralKind::Real(t.text().to_owned())
                };
                Ok(Expr::Literal(Literal {
                    kind,
                    span: self.span_from(start),
                }))
            }
            TokenKind::CharLit => {
                let t = self.bump();
                Ok(Expr::Literal(Literal {
                    kind: LiteralKind::Char(char_of(t)),
                    span: t.span,
                }))
            }
            TokenKind::StringLit => {
                if self.kind_at(1) == TokenKind::LParen {
                    return self.parse_name_expr();
                }
                let t = self.bump();
                Ok(Expr::Literal(Literal {
                    kind: LiteralKind::String(t.text().to_owned()),
                    span: t.span,
                }))
            }
            TokenKind::BitStringLit => {
                let t = self.bump();
                Ok(Expr::Literal(Literal {
                    kind: LiteralKind::BitString(t.text().to_owned()),
                    span: t.span,
                }))
            }
            TokenKind::Null => Ok(Expr::Literal(Literal {
                kind: LiteralKind::Null,
                span: self.bump().span,
            })),
            TokenKind::Open => Ok(Expr::Open(self.bump().span)),
            TokenKind::New => self.parse_allocator(),
            TokenKind::LParen => self.parse_paren_or_aggregate(),
            TokenKind::Ident | TokenKind::ExtendedIdent | TokenKind::LtLt => self.parse_name_expr(),
            _ => Err(self.expected("expression")),
        }
    }

    /// A name, or a qualified expression when `'(` follows it.
    fn parse_name_expr(&mut self) -> PResult<Expr> {
        let start = self.span();
        let name = self.parse_name()?;
        if self.at(TokenKind::Tick) && self.kind_at(1) == TokenKind::LParen {
            self.bump();
            let operand = self.parse_paren_or_aggregate()?;
            return Ok(Expr::Qualified {
                type_mark: name,
                operand: Box::new(operand),
                span: self.span_from(start),
            });
        }
        Ok(Expr::Name(name))
    }

    /// `new subtype_indication | new type_mark'(value)`
    fn parse_allocator(&mut self) -> PResult<Expr> {
        let start = self.span();
        self.expect(TokenKind::New)?;
        let sub_start = self.span();
        let type_mark = self.parse_type_mark()?;
        let kind = if self.at(TokenKind::Tick) && self.kind_at(1) == TokenKind::LParen {
            self.bump();
            let operand = self.parse_paren_or_aggregate()?;
            Allocator::Qualified { type_mark, operand }
        } else {
            let constraint = self.parse_optional_constraint()?;
            Allocator::Subtype(SubtypeIndication {
                resolution: None,
                type_mark,
                constraint,
                span: self.span_from(sub_start),
            })
        };
        Ok(Expr::Allocator {
            kind: Box::new(kind),
            span: self.span_from(start),
        })
    }

    /// `( expression )` or an aggregate.
    pub(super) fn parse_paren_or_aggregate(&mut self) -> PResult<Expr> {
        let start = self.span();
        self.expect(TokenKind::LParen)?;
        let mut elements = Vec::new();
        loop {
            let el_start = self.span();
            let (choices, value) = if self.at(TokenKind::Others) {
                let choices = self.parse_choices()?;
                self.expect(TokenKind::Arrow)?;
                (choices, self.parse_expr()?)
            } else {
                let e = self.parse_expr()?;
                if self.at_any(&[
                    TokenKind::To,
                    TokenKind::Downto,
                    TokenKind::Bar,
                    TokenKind::Arrow,
                    TokenKind::Range,
                ]) {
                    let first = self.choice_after_expr(e, el_start)?;
                    let mut choices = vec![first];
                    while self.eat(TokenKind::Bar).is_some() {
                        choices.push(self.parse_choice()?);
                    }
                    self.expect(TokenKind::Arrow)?;
                    (choices, self.parse_expr()?)
                } else {
                    (Vec::new(), e)
                }
            };
            elements.push(ElementAssociation {
                choices,
                value,
                span: self.span_from(el_start),
            });
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        let span = self.span_from(start);
        if elements.len() == 1 && elements[0].choices.is_empty() {
            let inner = elements.pop().map(|e| e.value).unwrap_or(Expr::Error(span));
            return Ok(Expr::Paren {
                inner: Box::new(inner),
                span,
            });
        }
        Ok(Expr::Aggregate(Aggregate { elements, span }))
    }

    /// An aggregate where the grammar requires one (assignment targets).
    pub(super) fn parse_aggregate(&mut self) -> PResult<Aggregate> {
        match self.parse_paren_or_aggregate()? {
            Expr::Aggregate(a) => Ok(a),
            Expr::Paren { inner, span } => Ok(Aggregate {
                elements: vec![ElementAssociation {
                    choices: Vec::new(),
                    span: inner.span(),
                    value: *inner,
                }],
                span,
            }),
            _ => unreachable!("parse_paren_or_aggregate returns Paren or Aggregate"),
        }
    }

    /// `choice { | choice }`
    pub(super) fn parse_choices(&mut self) -> PResult<Vec<Choice>> {
        let mut choices = vec![self.parse_choice()?];
        while self.eat(TokenKind::Bar).is_some() {
            choices.push(self.parse_choice()?);
        }
        Ok(choices)
    }

    /// `others`, a discrete range or a (simple) expression.
    fn parse_choice(&mut self) -> PResult<Choice> {
        let start = self.span();
        if self.at(TokenKind::Others) {
            return Ok(Choice::Others(self.bump().span));
        }
        let e = self.parse_expr()?;
        self.choice_after_expr(e, start)
    }

    /// Completes a choice whose expression has been parsed.
    ///
    /// A bare `x'range` is a choice too — `(v'range => '0')` is an
    /// aggregate over the whole of `v` — and the expression parser cannot
    /// tell it apart from a value attribute, so it is turned into a range
    /// here rather than left to fail as an expression.
    fn choice_after_expr(&mut self, e: Expr, start: Span) -> PResult<Choice> {
        if self.at_any(&[TokenKind::To, TokenKind::Downto, TokenKind::Range]) {
            let r = self.discrete_range_after_expr(e, start)?;
            return Ok(Choice::Range(r));
        }
        if let Expr::Name(n) = &e
            && super::types::is_range_attribute(n)
        {
            let Expr::Name(n) = e else { unreachable!() };
            return Ok(Choice::Range(DiscreteRange::Range(Range::Attribute(n))));
        }
        Ok(Choice::Expr(e))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::parse;
    use crate::vhdl::ast::*;
    use crate::vhdl::ast_dump::dump_expr;

    /// Parses `constant c : t := <text>;` inside a package and dumps the
    /// initial value.
    fn expr(text: &str) -> String {
        let (file, diags) = parse(&format!("package p is constant c : t := {text}; end;"));
        assert!(diags.is_empty(), "{diags}");
        let LibraryUnit::Package(p) = &file.units[0].unit else {
            panic!("expected a package");
        };
        let Declaration::Object(o) = &p.decls[0] else {
            panic!("expected a constant");
        };
        dump_expr(o.init.as_ref().unwrap())
    }

    #[test]
    fn precedence() {
        assert_eq!(expr("a + b * c"), "(+ a (* b c))");
        assert_eq!(expr("a * b + c"), "(+ (* a b) c)");
        assert_eq!(expr("-a * b"), "(- (* a b))");
        assert_eq!(expr("-a - b"), "(- (- a) b)");
        assert_eq!(expr("a ** b * c"), "(* (** a b) c)");
        assert_eq!(expr("not a and b"), "(and (not a) b)");
        assert_eq!(expr("a = b and c /= d"), "(and (= a b) (/= c d))");
        assert_eq!(expr("a & b = c"), "(= (& a b) c)");
        assert_eq!(expr("a sll 2 + 1"), "(sll a (+ 2 1))");
        assert_eq!(expr("abs a mod b"), "(mod (abs a) b)");
        assert_eq!(expr("a and b and c"), "(and (and a b) c)");
        assert_eq!(expr("a ?= b"), "(?= a b)");
        assert_eq!(expr("?? a"), "(?? a)");
        assert_eq!(expr("(a or b) and c"), "(and (paren (or a b)) c)");
    }

    #[test]
    fn mixed_logical_operators_are_rejected() {
        let (_, diags) = parse("package p is constant c : t := a and b or c; end;");
        assert!(diags.contains("`and` and `or` cannot be mixed"), "{diags}");
        let (_, diags) = parse("package p is constant c : t := a nand b nand c; end;");
        assert!(diags.contains("`nand` is not associative"), "{diags}");
        let (_, diags) = parse("package p is constant c : t := 2 ** -1; end;");
        assert!(diags.contains("expected expression, found `-`"), "{diags}");
    }

    #[test]
    fn primaries() {
        assert_eq!(expr("10 ns"), "10 ns");
        assert_eq!(expr("16#FF#"), "16#FF#");
        assert_eq!(expr("x\"F0\""), "x\"F0\"");
        assert_eq!(expr("'1'"), "'1'");
        assert_eq!(expr("\"abc\""), "\"abc\"");
        assert_eq!(expr("null"), "null");
        assert_eq!(expr("f(1, b => 2)"), "f(1, b => 2)");
        assert_eq!(expr("a.b.c"), "a.b.c");
        assert_eq!(expr("v(7 downto 0)"), "v(7 downto 0)");
        assert_eq!(expr("v(x'range)"), "v(x'range)");
        assert_eq!(expr("t'(1)"), "t'(1)");
        assert_eq!(expr("t'(a, b)"), "t'(agg a, b)");
        assert_eq!(expr("(others => '0')"), "(agg others => '0')");
        assert_eq!(
            expr("(1 to 3 => '1', 5 | 7 => '0')"),
            "(agg 1 to 3 => '1', 5 | 7 => '0')"
        );
        assert_eq!(expr("new t'(1)"), "(new t'(1))");
        assert_eq!(expr("new t(1 to 3)"), "(new t(1 to 3))");
        assert_eq!(expr("\"+\"(a, b)"), "\"+\"(a, b)");
        assert_eq!(expr("x'length(1)"), "x'length(1)");
        assert_eq!(
            expr("f[integer return bit]'path_name"),
            "f[integer return bit]'path_name"
        );
        assert_eq!(expr("<< signal .top.x : bit >>"), "<<signal .top.x : bit>>");
        assert_eq!(expr("p.all"), "p.all");
    }
}
