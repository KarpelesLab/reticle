//! Expressions.
//!
//! Binary operators are parsed by precedence climbing over the table in
//! [`binary_op`], with the conditional operator and `inside` handled in the
//! same loop at their own levels. Everything binds left to right except
//! `?:`, `->` and `<->`, which the standard makes right-associative.
//!
//! Left-hand sides of assignments go through [`Parser::parse_lvalue`],
//! which stops before any binary operator; that is what keeps `a <= b` a
//! non-blocking assignment rather than a comparison.
//!
//! A data type may appear where an expression is expected (`$bits(int)`,
//! `.T(logic [3:0])`, streaming slice sizes) and is returned as
//! [`ExprKind::Type`]; a type followed by `'(` is a cast.

use super::super::ast::{
    Arg, AssignOp, BinaryOp, CastTarget, DataType, DataTypeKind, Delay, Expr, ExprKind, Ident,
    Literal, PatternItem, RangeKind, Signing, UnaryOp,
};
use super::super::token::{Keyword, Punct, TokenKind};
use super::{PResult, Parser};

/// Precedence of the conditional operator.
const PREC_TERNARY: u8 = 2;
/// Precedence of `inside`, shared with the relational operators.
const PREC_RELATIONAL: u8 = 9;

/// The binary operator a token denotes, with its precedence (higher binds
/// tighter).
fn binary_op(kind: &TokenKind) -> Option<(BinaryOp, u8)> {
    let TokenKind::Punct(p) = kind else {
        return None;
    };
    Some(match p {
        Punct::Arrow => (BinaryOp::Implies, 1),
        Punct::Equiv => (BinaryOp::Equiv, 1),
        Punct::OrOr => (BinaryOp::LogicOr, 3),
        Punct::AndAnd => (BinaryOp::LogicAnd, 4),
        Punct::Pipe => (BinaryOp::BitOr, 5),
        Punct::Caret => (BinaryOp::BitXor, 6),
        Punct::TildeCaret | Punct::CaretTilde => (BinaryOp::BitXnor, 6),
        Punct::Amp => (BinaryOp::BitAnd, 7),
        Punct::EqEq => (BinaryOp::Eq, 8),
        Punct::BangEq => (BinaryOp::Ne, 8),
        Punct::CaseEq => (BinaryOp::CaseEq, 8),
        Punct::CaseNe => (BinaryOp::CaseNe, 8),
        Punct::WildEq => (BinaryOp::WildEq, 8),
        Punct::WildNe => (BinaryOp::WildNe, 8),
        Punct::Lt => (BinaryOp::Lt, PREC_RELATIONAL),
        Punct::Le => (BinaryOp::Le, PREC_RELATIONAL),
        Punct::Gt => (BinaryOp::Gt, PREC_RELATIONAL),
        Punct::Ge => (BinaryOp::Ge, PREC_RELATIONAL),
        Punct::Shl => (BinaryOp::Shl, 10),
        Punct::Shr => (BinaryOp::Shr, 10),
        Punct::Ashl => (BinaryOp::Ashl, 10),
        Punct::Ashr => (BinaryOp::Ashr, 10),
        Punct::Plus => (BinaryOp::Add, 11),
        Punct::Minus => (BinaryOp::Sub, 11),
        Punct::Star => (BinaryOp::Mul, 12),
        Punct::Slash => (BinaryOp::Div, 12),
        Punct::Percent => (BinaryOp::Mod, 12),
        Punct::StarStar => (BinaryOp::Pow, 13),
        _ => return None,
    })
}

/// The prefix operator a token denotes.
fn unary_op(kind: &TokenKind) -> Option<UnaryOp> {
    let TokenKind::Punct(p) = kind else {
        return None;
    };
    Some(match p {
        Punct::Plus => UnaryOp::Plus,
        Punct::Minus => UnaryOp::Minus,
        Punct::Bang => UnaryOp::LogicNot,
        Punct::Tilde => UnaryOp::BitNot,
        Punct::Amp => UnaryOp::ReduceAnd,
        Punct::TildeAmp => UnaryOp::ReduceNand,
        Punct::Pipe => UnaryOp::ReduceOr,
        Punct::TildePipe => UnaryOp::ReduceNor,
        Punct::Caret => UnaryOp::ReduceXor,
        Punct::TildeCaret | Punct::CaretTilde => UnaryOp::ReduceXnor,
        _ => return None,
    })
}

/// The assignment operator a token denotes.
pub(super) fn assign_op(kind: &TokenKind) -> Option<AssignOp> {
    let TokenKind::Punct(p) = kind else {
        return None;
    };
    Some(match p {
        Punct::Eq => AssignOp::Blocking,
        Punct::Le => AssignOp::NonBlocking,
        Punct::PlusEq => AssignOp::Add,
        Punct::MinusEq => AssignOp::Sub,
        Punct::StarEq => AssignOp::Mul,
        Punct::SlashEq => AssignOp::Div,
        Punct::PercentEq => AssignOp::Mod,
        Punct::AmpEq => AssignOp::And,
        Punct::PipeEq => AssignOp::Or,
        Punct::CaretEq => AssignOp::Xor,
        Punct::ShlEq => AssignOp::Shl,
        Punct::ShrEq => AssignOp::Shr,
        Punct::AshlEq => AssignOp::Ashl,
        Punct::AshrEq => AssignOp::Ashr,
        _ => return None,
    })
}

impl Parser<'_> {
    /// A full expression.
    pub fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_expr_prec(0)
    }

    /// Expressions with operators of precedence at least `min_prec`.
    fn parse_expr_prec(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            if self.at_punct(Punct::Question) && min_prec <= PREC_TERNARY {
                self.bump();
                self.skip_expr_attrs()?;
                let then_expr = self.parse_expr()?;
                self.expect_punct(Punct::Colon)?;
                let else_expr = self.parse_expr_prec(PREC_TERNARY)?;
                let span = lhs.span.to(else_expr.span);
                lhs = Expr::new(
                    ExprKind::Ternary {
                        cond: Box::new(lhs),
                        then_expr: Box::new(then_expr),
                        else_expr: Box::new(else_expr),
                    },
                    span,
                );
                continue;
            }
            if self.at_kw(Keyword::Inside) && min_prec <= PREC_RELATIONAL {
                self.bump();
                self.expect_punct(Punct::LBrace)?;
                let set = self.parse_expr_list(Punct::RBrace)?;
                let end = self.expect_punct(Punct::RBrace)?;
                let span = lhs.span.to(end);
                lhs = Expr::new(
                    ExprKind::Inside {
                        expr: Box::new(lhs),
                        set,
                    },
                    span,
                );
                continue;
            }
            let Some((op, prec)) = binary_op(self.kind()) else {
                break;
            };
            if prec < min_prec {
                break;
            }
            self.bump();
            self.skip_expr_attrs()?;
            // Left-associative operators need the right side to bind
            // strictly tighter; the implication operators are right
            // associative and take their own level.
            let next_min = if prec == 1 { prec } else { prec + 1 };
            let rhs = self.parse_expr_prec(next_min)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr::new(
                ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            );
        }
        Ok(lhs)
    }

    /// Attributes are allowed after an operator (`a + (* attr *) b`);
    /// they carry no meaning the parser keeps.
    fn skip_expr_attrs(&mut self) -> PResult<()> {
        if self.at_punct(Punct::AttrOpen) {
            self.parse_attrs()?;
        }
        Ok(())
    }

    /// A prefix operator, `++`/`--`, or a postfix expression.
    fn parse_unary(&mut self) -> PResult<Expr> {
        let start = self.span();
        if let Some(op) = unary_op(self.kind()) {
            self.bump();
            self.skip_expr_attrs()?;
            let operand = self.parse_unary()?;
            let span = start.to(operand.span);
            return Ok(Expr::new(
                ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
                span,
            ));
        }
        if self.at_punct(Punct::PlusPlus) || self.at_punct(Punct::MinusMinus) {
            let increment = self.at_punct(Punct::PlusPlus);
            self.bump();
            let target = self.parse_unary()?;
            let span = start.to(target.span);
            return Ok(Expr::new(
                ExprKind::IncDec {
                    increment,
                    prefix: true,
                    target: Box::new(target),
                },
                span,
            ));
        }
        self.parse_postfix()
    }

    /// A primary followed by selects, member accesses, calls, casts and
    /// postfix `++`/`--`. This is also the shape of an assignment target.
    pub(super) fn parse_lvalue(&mut self) -> PResult<Expr> {
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            let start = expr.span;
            match self.kind() {
                TokenKind::Punct(Punct::LBracket) => {
                    self.bump();
                    let first = self.parse_expr()?;
                    let kind = if self.eat_punct(Punct::Colon).is_some() {
                        Some(RangeKind::Fixed)
                    } else if self.eat_punct(Punct::PlusColon).is_some() {
                        Some(RangeKind::IndexedUp)
                    } else if self.eat_punct(Punct::MinusColon).is_some() {
                        Some(RangeKind::IndexedDown)
                    } else {
                        None
                    };
                    let kind = match kind {
                        Some(kind) => {
                            let right = self.parse_expr()?;
                            ExprKind::Range {
                                base: Box::new(expr),
                                kind,
                                left: Box::new(first),
                                right: Box::new(right),
                            }
                        }
                        None => ExprKind::Index {
                            base: Box::new(expr),
                            index: Box::new(first),
                        },
                    };
                    let end = self.expect_punct(Punct::RBracket)?;
                    expr = Expr::new(kind, start.to(end));
                }
                TokenKind::Punct(Punct::Dot) if self.nth(1).is_ident() => {
                    self.bump();
                    let name = self.expect_ident()?;
                    let span = start.to(name.span);
                    expr = Expr::new(
                        ExprKind::Member {
                            base: Box::new(expr),
                            name,
                        },
                        span,
                    );
                }
                TokenKind::Punct(Punct::ColonColon) if self.nth(1).is_ident() => {
                    self.bump();
                    let name = self.expect_ident()?;
                    let span = start.to(name.span);
                    expr = Expr::new(
                        ExprKind::Scoped {
                            scope: Box::new(expr),
                            name,
                        },
                        span,
                    );
                }
                TokenKind::Punct(Punct::LParen) if is_callable(&expr) => {
                    let args = self.parse_args()?;
                    let span = self.span_from(start);
                    expr = Expr::new(
                        ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    );
                }
                TokenKind::Punct(Punct::Apostrophe) if self.nth_is_punct(1, Punct::LParen) => {
                    self.bump();
                    self.bump();
                    let inner = self.parse_expr()?;
                    let end = self.expect_punct(Punct::RParen)?;
                    let target = cast_target_of(expr);
                    expr = Expr::new(
                        ExprKind::Cast {
                            target,
                            expr: Box::new(inner),
                        },
                        start.to(end),
                    );
                }
                TokenKind::Punct(Punct::ApostropheBrace) => {
                    let pattern = self.parse_pattern()?;
                    let span = start.to(pattern.span);
                    let target = cast_target_of(expr);
                    expr = Expr::new(
                        ExprKind::Cast {
                            target,
                            expr: Box::new(pattern),
                        },
                        span,
                    );
                }
                TokenKind::Punct(Punct::PlusPlus | Punct::MinusMinus) => {
                    let increment = self.at_punct(Punct::PlusPlus);
                    let end = self.bump();
                    expr = Expr::new(
                        ExprKind::IncDec {
                            increment,
                            prefix: false,
                            target: Box::new(expr),
                        },
                        start.to(end),
                    );
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    /// The atoms: literals, names, parenthesised expressions,
    /// concatenations, patterns, casts and types.
    fn parse_primary(&mut self) -> PResult<Expr> {
        use Keyword as K;
        let start = self.span();
        let kind = match self.kind() {
            TokenKind::Number { text } => {
                let text = text.clone();
                let span = self.bump();
                ExprKind::Literal(Literal::Number { text, span })
            }
            TokenKind::Str { value } => {
                let value = value.clone();
                let span = self.bump();
                ExprKind::Literal(Literal::Str { value, span })
            }
            TokenKind::Ident { .. } | TokenKind::EscapedIdent { .. } => {
                let id = self.expect_ident()?;
                ExprKind::Ident(id)
            }
            TokenKind::SystemIdent { name } => {
                let name = name.clone();
                let span = self.bump();
                ExprKind::SystemIdent(Ident { name, span })
            }
            TokenKind::Punct(Punct::Dollar) => {
                let span = self.bump();
                ExprKind::Literal(Literal::Unbounded(span))
            }
            TokenKind::Punct(Punct::LParen) => {
                self.bump();
                // `(a = b)` is an assignment expression in SystemVerilog.
                let first = self.parse_expr_or_assign()?;
                let inner = if self.eat_punct(Punct::Colon).is_some() {
                    let typ = self.parse_expr()?;
                    self.expect_punct(Punct::Colon)?;
                    let max = self.parse_expr()?;
                    let span = first.span.to(max.span);
                    Expr::new(
                        ExprKind::MinTypMax {
                            min: Box::new(first),
                            typ: Box::new(typ),
                            max: Box::new(max),
                        },
                        span,
                    )
                } else {
                    first
                };
                let end = self.expect_punct(Punct::RParen)?;
                return Ok(Expr::new(inner.kind, start.to(end)));
            }
            TokenKind::Punct(Punct::LBrace) => return self.parse_brace(),
            TokenKind::Punct(Punct::ApostropheBrace) => return self.parse_pattern(),
            TokenKind::Punct(Punct::LBracket) => {
                self.bump();
                let low = self.parse_expr()?;
                self.expect_punct(Punct::Colon)?;
                let high = self.parse_expr()?;
                self.expect_punct(Punct::RBracket)?;
                ExprKind::ValueRange {
                    low: Box::new(low),
                    high: Box::new(high),
                }
            }
            TokenKind::Punct(Punct::AttrOpen) => {
                self.parse_attrs()?;
                return self.parse_primary();
            }
            TokenKind::Keyword(K::Null) => {
                let span = self.bump();
                ExprKind::Literal(Literal::Null(span))
            }
            TokenKind::Keyword(K::Default) => {
                self.bump();
                ExprKind::Default
            }
            TokenKind::Keyword(K::New) => {
                self.bump();
                let args = if self.at_punct(Punct::LParen) {
                    self.bump();
                    let args = self.parse_expr_list(Punct::RParen)?;
                    self.expect_punct(Punct::RParen)?;
                    args
                } else if self.at_punct(Punct::LBracket) {
                    self.bump();
                    let size = self.parse_expr()?;
                    self.expect_punct(Punct::RBracket)?;
                    let mut args = vec![size];
                    // `new[size](init)` copies an existing array.
                    if self.eat_punct(Punct::LParen).is_some() {
                        args.push(self.parse_expr()?);
                        self.expect_punct(Punct::RParen)?;
                    }
                    args
                } else {
                    Vec::new()
                };
                ExprKind::New(args)
            }
            TokenKind::Keyword(K::Signed | K::Unsigned | K::Const) => {
                let target = match self.kind() {
                    TokenKind::Keyword(K::Signed) => CastTarget::Signing(Signing::Signed),
                    TokenKind::Keyword(K::Unsigned) => CastTarget::Signing(Signing::Unsigned),
                    _ => CastTarget::Const,
                };
                self.bump();
                self.expect_punct(Punct::Apostrophe)?;
                self.expect_punct(Punct::LParen)?;
                let inner = self.parse_expr()?;
                self.expect_punct(Punct::RParen)?;
                ExprKind::Cast {
                    target,
                    expr: Box::new(inner),
                }
            }
            _ if self.at_data_type_keyword() => {
                let ty = self.parse_data_type()?;
                return self.finish_type_primary(ty, start);
            }
            _ => return Err(self.expected("expression")),
        };
        Ok(Expr::new(kind, self.span_from(start)))
    }

    /// A type just parsed in expression position: a cast if `'(` or `'{`
    /// follows, else the type itself.
    fn finish_type_primary(&mut self, ty: DataType, start: crate::source::Span) -> PResult<Expr> {
        if self.at_punct(Punct::Apostrophe) && self.nth_is_punct(1, Punct::LParen) {
            self.bump();
            self.bump();
            let inner = self.parse_expr()?;
            let end = self.expect_punct(Punct::RParen)?;
            return Ok(Expr::new(
                ExprKind::Cast {
                    target: CastTarget::Type(Box::new(ty)),
                    expr: Box::new(inner),
                },
                start.to(end),
            ));
        }
        if self.at_punct(Punct::ApostropheBrace) {
            let pattern = self.parse_pattern()?;
            let span = start.to(pattern.span);
            return Ok(Expr::new(
                ExprKind::Cast {
                    target: CastTarget::Type(Box::new(ty)),
                    expr: Box::new(pattern),
                },
                span,
            ));
        }
        let span = ty.span;
        Ok(Expr::new(ExprKind::Type(Box::new(ty)), span))
    }

    /// `{a, b}`, `{n{a}}`, `{<< [slice] {a}}`, cursor on `{`.
    fn parse_brace(&mut self) -> PResult<Expr> {
        let start = self.expect_punct(Punct::LBrace)?;
        if self.at_punct(Punct::Shl) || self.at_punct(Punct::Shr) {
            let right_to_left = self.at_punct(Punct::Shl);
            self.bump();
            let slice = if self.at_punct(Punct::LBrace) {
                None
            } else {
                Some(Box::new(self.parse_expr()?))
            };
            self.expect_punct(Punct::LBrace)?;
            let elems = self.parse_expr_list(Punct::RBrace)?;
            self.expect_punct(Punct::RBrace)?;
            let end = self.expect_punct(Punct::RBrace)?;
            return Ok(Expr::new(
                ExprKind::Streaming {
                    right_to_left,
                    slice,
                    elems,
                },
                start.to(end),
            ));
        }
        if self.at_punct(Punct::RBrace) {
            return Err(self.expected("expression"));
        }
        let first = self.parse_expr()?;
        if self.at_punct(Punct::LBrace) {
            self.bump();
            let elems = self.parse_expr_list(Punct::RBrace)?;
            self.expect_punct(Punct::RBrace)?;
            let end = self.expect_punct(Punct::RBrace)?;
            return Ok(Expr::new(
                ExprKind::Replicate {
                    count: Box::new(first),
                    elems,
                },
                start.to(end),
            ));
        }
        let mut elems = vec![first];
        while self.eat_punct(Punct::Comma).is_some() {
            elems.push(self.parse_expr()?);
        }
        let end = self.expect_punct(Punct::RBrace)?;
        Ok(Expr::new(ExprKind::Concat(elems), start.to(end)))
    }

    /// `'{ ... }`, cursor on `'{`.
    fn parse_pattern(&mut self) -> PResult<Expr> {
        let start = self.expect_punct(Punct::ApostropheBrace)?;
        let mut items = Vec::new();
        if !self.at_punct(Punct::RBrace) {
            loop {
                let item_start = self.span();
                let first = self.parse_expr()?;
                let (key, value) = if self.eat_punct(Punct::Colon).is_some() {
                    (Some(first), self.parse_expr()?)
                } else if self.at_punct(Punct::LBrace) {
                    // `'{n{...}}`: a replication inside a pattern.
                    self.bump();
                    let elems = self.parse_expr_list(Punct::RBrace)?;
                    let end = self.expect_punct(Punct::RBrace)?;
                    let span = first.span.to(end);
                    (
                        None,
                        Expr::new(
                            ExprKind::Replicate {
                                count: Box::new(first),
                                elems,
                            },
                            span,
                        ),
                    )
                } else {
                    (None, first)
                };
                let span = self.span_from(item_start);
                items.push(PatternItem { key, value, span });
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
        }
        let end = self.expect_punct(Punct::RBrace)?;
        Ok(Expr::new(ExprKind::Pattern(items), start.to(end)))
    }

    /// Comma-separated expressions up to (not including) `close`; empty
    /// when `close` is next.
    pub(super) fn parse_expr_list(&mut self, close: Punct) -> PResult<Vec<Expr>> {
        let mut list = Vec::new();
        if self.at_punct(close) {
            return Ok(list);
        }
        loop {
            list.push(self.parse_expr()?);
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        Ok(list)
    }

    /// A parenthesised argument list with positional, named and empty
    /// arguments, cursor on `(`.
    pub(super) fn parse_args(&mut self) -> PResult<Vec<Arg>> {
        self.expect_punct(Punct::LParen)?;
        let mut args = Vec::new();
        if self.at_punct(Punct::RParen) {
            self.bump();
            return Ok(args);
        }
        loop {
            let start = self.span();
            let arg = if self.at_punct(Punct::Dot) && self.nth(1).is_ident() {
                self.bump();
                let name = self.expect_ident()?;
                self.expect_punct(Punct::LParen)?;
                let value = if self.at_punct(Punct::RParen) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.expect_punct(Punct::RParen)?;
                Arg {
                    name: Some(name),
                    value,
                    span: self.span_from(start),
                }
            } else if self.at_punct(Punct::Comma) || self.at_punct(Punct::RParen) {
                Arg {
                    name: None,
                    value: None,
                    span: crate::source::Span::new(start.file, start.start, start.start),
                }
            } else {
                let value = self.parse_expr()?;
                let span = value.span;
                Arg {
                    name: None,
                    value: Some(value),
                    span,
                }
            };
            args.push(arg);
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(args)
    }

    /// An expression optionally followed by an assignment operator and a
    /// value, as in a `for` step: `i = i + 1`, `i += 2`, `i++`, `f(i)`.
    pub(super) fn parse_expr_or_assign(&mut self) -> PResult<Expr> {
        let lhs = self.parse_expr()?;
        if let Some(op) = assign_op(self.kind()) {
            self.bump();
            let rhs = self.parse_expr()?;
            let span = lhs.span.to(rhs.span);
            return Ok(Expr::new(
                ExprKind::Assign {
                    lhs: Box::new(lhs),
                    op,
                    rhs: Box::new(rhs),
                },
                span,
            ));
        }
        Ok(lhs)
    }

    /// `#value`, `#(a)`, `#(a, b, c)` with `min:typ:max` values, cursor
    /// on `#`.
    pub(super) fn parse_delay(&mut self) -> PResult<Delay> {
        let start = self.expect_punct(Punct::Hash)?;
        let mut values = Vec::new();
        if self.eat_punct(Punct::LParen).is_some() {
            loop {
                values.push(self.parse_mintypmax()?);
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
            self.expect_punct(Punct::RParen)?;
        } else {
            match self.kind() {
                TokenKind::Number { .. } => values.push(self.parse_primary()?),
                TokenKind::Ident { .. } | TokenKind::EscapedIdent { .. } => {
                    // A parameter name, possibly scoped or hierarchical;
                    // no selects or calls, which would need parentheses.
                    let mut e = self.parse_primary()?;
                    while (self.at_punct(Punct::Dot) || self.at_punct(Punct::ColonColon))
                        && self.nth(1).is_ident()
                    {
                        let scoped = self.at_punct(Punct::ColonColon);
                        self.bump();
                        let name = self.expect_ident()?;
                        let span = e.span.to(name.span);
                        let kind = if scoped {
                            ExprKind::Scoped {
                                scope: Box::new(e),
                                name,
                            }
                        } else {
                            ExprKind::Member {
                                base: Box::new(e),
                                name,
                            }
                        };
                        e = Expr::new(kind, span);
                    }
                    values.push(e);
                }
                _ => return Err(self.expected("delay value")),
            }
        }
        Ok(Delay {
            values,
            span: self.span_from(start),
        })
    }

    /// `expr` or `min:typ:max`.
    pub(super) fn parse_mintypmax(&mut self) -> PResult<Expr> {
        let min = self.parse_expr()?;
        if self.eat_punct(Punct::Colon).is_none() {
            return Ok(min);
        }
        let typ = self.parse_expr()?;
        self.expect_punct(Punct::Colon)?;
        let max = self.parse_expr()?;
        let span = min.span.to(max.span);
        Ok(Expr::new(
            ExprKind::MinTypMax {
                min: Box::new(min),
                typ: Box::new(typ),
                max: Box::new(max),
            },
            span,
        ))
    }
}

/// True for expressions that a `(` may turn into a call.
fn is_callable(expr: &Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::Ident(_)
            | ExprKind::SystemIdent(_)
            | ExprKind::Member { .. }
            | ExprKind::Scoped { .. }
    )
}

/// What an expression before `'(` casts to: a name is a type, a number is
/// a width, anything else is a width expression.
fn cast_target_of(expr: Expr) -> CastTarget {
    let named = |package: Option<Ident>, name: Ident| {
        CastTarget::Type(Box::new(DataType {
            kind: DataTypeKind::Named {
                package,
                name,
                member: None,
            },
            signing: None,
            packed: Vec::new(),
            span: expr.span,
        }))
    };
    match expr.kind {
        ExprKind::Ident(name) => named(None, name),
        ExprKind::Scoped { scope, name } if matches!(scope.kind, ExprKind::Ident(_)) => {
            let ExprKind::Ident(package) = scope.kind else {
                unreachable!("matched above");
            };
            named(Some(package), name)
        }
        kind => CastTarget::Size(Box::new(Expr::new(kind, expr.span))),
    }
}

#[cfg(test)]
mod tests {
    use super::Parser;
    use crate::diag::Diagnostics;
    use crate::source::SourceMap;
    use crate::verilog::ast_dump::expr_to_string;
    use crate::verilog::lex::lex;
    use crate::verilog::token::Dialect;

    /// Parses `src` as one expression and renders it fully parenthesised.
    fn parse(src: &str) -> String {
        let mut map = SourceMap::new();
        let id = map.add("e.sv", src).unwrap();
        let mut diags = Diagnostics::new();
        let tokens = lex(src, id, Dialect::SystemVerilog, &mut diags).tokens;
        assert!(diags.is_empty(), "lex errors in {src:?}");
        let mut parser = Parser::new(&tokens, Dialect::SystemVerilog);
        let expr = parser.parse_expr().expect("parses");
        assert!(
            parser.diagnostics().is_empty(),
            "parse errors in {src:?}: {:?}",
            parser.diagnostics()
        );
        assert!(parser.at_eof(), "trailing tokens in {src:?}");
        expr_to_string(&expr)
    }

    #[test]
    fn precedence_levels_low_to_high() {
        assert_eq!(parse("a -> b ? c : d"), "(a -> (b ? c : d))");
        assert_eq!(parse("a ? b : c || d"), "(a ? b : (c || d))");
        assert_eq!(parse("a || b && c"), "(a || (b && c))");
        assert_eq!(parse("a && b | c"), "(a && (b | c))");
        assert_eq!(parse("a | b ^ c"), "(a | (b ^ c))");
        assert_eq!(parse("a ^ b & c"), "(a ^ (b & c))");
        assert_eq!(parse("a & b == c"), "(a & (b == c))");
        assert_eq!(parse("a == b < c"), "(a == (b < c))");
        assert_eq!(parse("a < b << c"), "(a < (b << c))");
        assert_eq!(parse("a << b + c"), "(a << (b + c))");
        assert_eq!(parse("a + b * c"), "(a + (b * c))");
        assert_eq!(parse("a * b ** c"), "(a * (b ** c))");
        assert_eq!(parse("a inside {b} == c"), "((a inside {b}) == c)");
        assert_eq!(parse("a + b inside {c}"), "((a + b) inside {c})");
    }

    #[test]
    fn associativity() {
        assert_eq!(parse("a - b - c"), "((a - b) - c)");
        assert_eq!(parse("a / b * c"), "((a / b) * c)");
        assert_eq!(parse("a ** b ** c"), "((a ** b) ** c)");
        assert_eq!(parse("a << b >> c"), "((a << b) >> c)");
        assert_eq!(parse("a == b != c"), "((a == b) != c)");
        assert_eq!(parse("a ? b : c ? d : e"), "(a ? b : (c ? d : e))");
        assert_eq!(parse("a -> b -> c"), "(a -> (b -> c))");
        assert_eq!(parse("a <-> b -> c"), "(a <-> (b -> c))");
    }

    #[test]
    fn unary_binds_tighter_than_binary() {
        assert_eq!(parse("-a + b"), "((-a) + b)");
        assert_eq!(parse("-a ** b"), "((-a) ** b)");
        assert_eq!(parse("!a && !b"), "((!a) && (!b))");
        assert_eq!(parse("~&a | &b"), "((~&a) | (&b))");
        assert_eq!(parse("- -a"), "(-(-a))");
        assert_eq!(parse("^~a"), "(~^a)");
        assert_eq!(parse("++i + i++"), "(++i + i++)");
    }

    #[test]
    fn parentheses_override() {
        assert_eq!(parse("(a + b) * c"), "((a + b) * c)");
        assert_eq!(parse("a * (b + c)"), "(a * (b + c))");
        assert_eq!(parse("((a))"), "a");
        assert_eq!(parse("(a ? b : c) ? d : e"), "((a ? b : c) ? d : e)");
    }

    #[test]
    fn postfix_forms() {
        assert_eq!(parse("a[3:0][1]"), "a[3:0][1]");
        assert_eq!(parse("a.b[2].c"), "a.b[2].c");
        assert_eq!(parse("p::q.r"), "p::q.r");
        assert_eq!(parse("f(a, .n(b), )"), "f(a, .n(b), )");
        assert_eq!(parse("a[i+:8] + b[j-:8]"), "(a[i+:8] + b[j-:8])");
        assert_eq!(parse("$bits(logic [3:0])"), "$bits(logic [3:0])");
        assert_eq!(parse("q[$-1]"), "q[($ - 1)]");
    }

    #[test]
    fn casts_and_literals() {
        assert_eq!(parse("int'(a + b)"), "int'((a + b))");
        assert_eq!(parse("8'(a)"), "8'(a)");
        assert_eq!(parse("(W+1)'(a)"), "(W + 1)'(a)");
        assert_eq!(parse("signed'(a) * 2"), "(signed'(a) * 2)");
        assert_eq!(parse("T'{a: 1}"), "T'{a: 1}");
        assert_eq!(parse("'0 + 'x"), "('0 + 'x)");
        assert_eq!(parse("8'hff"), "8'hff");
        assert_eq!(parse("\"s\""), "\"s\"");
    }

    #[test]
    fn braces() {
        assert_eq!(parse("{a, b, c}"), "{a, b, c}");
        assert_eq!(parse("{3{a}}"), "{3{a}}");
        assert_eq!(parse("{{2{a}}, b}"), "{{2{a}}, b}");
        assert_eq!(parse("{<< 8 {a, b}}"), "{<< 8 {a, b}}");
        assert_eq!(parse("'{default: 0, 1: x}"), "'{default: 0, 1: x}");
    }

    #[test]
    fn one_diagnostic_per_mistake() {
        let mut map = SourceMap::new();
        let src = "a + * b";
        let id = map.add("e.sv", src).unwrap();
        let mut diags = Diagnostics::new();
        let tokens = lex(src, id, Dialect::SystemVerilog, &mut diags).tokens;
        let mut parser = Parser::new(&tokens, Dialect::SystemVerilog);
        assert!(parser.parse_expr().is_err());
        let diags = parser.take_diagnostics();
        assert_eq!(diags.len(), 1);
        let msg = &diags.iter().next().unwrap().message;
        assert_eq!(msg, "expected expression, found `*`");
    }
}
