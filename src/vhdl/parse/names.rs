//! Names, association lists and external names.

use super::{PResult, Parser, char_of, ident_of};
use crate::vhdl::ast::*;
use crate::vhdl::token::TokenKind;

impl<'t, 'src> Parser<'t, 'src> {
    /// A name with every postfix: selections, calls and indexes, slices,
    /// attributes. Stops before `'(`, which starts a qualified expression.
    pub(super) fn parse_name(&mut self) -> PResult<Name> {
        let prefix = self.parse_name_prefix()?;
        self.parse_name_postfix(prefix, true)
    }

    /// A name without a parenthesised postfix at the top level, for type
    /// marks and other places where `(` starts something else.
    pub(super) fn parse_name_no_call(&mut self) -> PResult<Name> {
        let prefix = self.parse_name_prefix()?;
        self.parse_name_postfix(prefix, false)
    }

    /// True at a token that can start a name.
    pub(super) fn at_name_start(&self) -> bool {
        matches!(
            self.kind(),
            TokenKind::Ident | TokenKind::ExtendedIdent | TokenKind::StringLit | TokenKind::LtLt
        )
    }

    /// The first element of a name.
    fn parse_name_prefix(&mut self) -> PResult<Name> {
        match self.kind() {
            TokenKind::Ident | TokenKind::ExtendedIdent => Ok(Name::Simple(ident_of(self.bump()))),
            TokenKind::StringLit => {
                let t = self.bump();
                Ok(Name::Operator {
                    symbol: t.text().to_owned(),
                    span: t.span,
                })
            }
            TokenKind::CharLit => {
                let t = self.bump();
                Ok(Name::Char {
                    ch: char_of(t),
                    span: t.span,
                })
            }
            TokenKind::LtLt => self.parse_external_name(),
            _ => Err(self.expected("a name")),
        }
    }

    /// Applies postfixes to `prefix` while they are present.
    pub(super) fn parse_name_postfix(
        &mut self,
        mut prefix: Name,
        allow_call: bool,
    ) -> PResult<Name> {
        let start = prefix.span();
        loop {
            match self.kind() {
                TokenKind::Dot => {
                    self.bump();
                    let suffix = match self.kind() {
                        TokenKind::All => Suffix::All(self.bump().span),
                        _ => Suffix::Designator(self.parse_designator()?),
                    };
                    prefix = Name::Selected {
                        prefix: Box::new(prefix),
                        suffix,
                        span: self.span_from(start),
                    };
                }
                TokenKind::LParen if allow_call => {
                    let args = self.parse_association_list()?;
                    let span = self.span_from(start);
                    prefix = match single_range(args) {
                        Ok(range) => Name::Slice {
                            prefix: Box::new(prefix),
                            range: Box::new(range),
                            span,
                        },
                        Err(args) => Name::Call {
                            prefix: Box::new(prefix),
                            args,
                            span,
                        },
                    };
                }
                TokenKind::LBracket if self.signature_precedes_tick() => {
                    let signature = self.parse_optional_signature()?.map(Box::new);
                    self.expect(TokenKind::Tick)?;
                    let attribute = self.parse_attribute_designator()?;
                    prefix = Name::Attribute {
                        prefix: Box::new(prefix),
                        signature,
                        attribute,
                        span: self.span_from(start),
                    };
                }
                TokenKind::Tick => {
                    if self.kind_at(1) == TokenKind::LParen {
                        break;
                    }
                    self.bump();
                    let attribute = self.parse_attribute_designator()?;
                    prefix = Name::Attribute {
                        prefix: Box::new(prefix),
                        signature: None,
                        attribute,
                        span: self.span_from(start),
                    };
                }
                _ => break,
            }
        }
        Ok(prefix)
    }

    /// True when the `[` at the cursor opens a signature that is followed
    /// by an attribute tick (`f[bit return bit]'path_name`), as opposed to
    /// the trailing signature of an alias or subprogram instantiation.
    fn signature_precedes_tick(&self) -> bool {
        let mut i = self.pos;
        loop {
            match self.tokens.get(i).map(|t| t.kind) {
                Some(TokenKind::RBracket) => {
                    return self
                        .tokens
                        .get(i + 1)
                        .is_some_and(|t| t.kind == TokenKind::Tick);
                }
                Some(TokenKind::Semi | TokenKind::Eof) | None => return false,
                _ => i += 1,
            }
        }
    }

    /// The identifier after an attribute tick; `range` is a reserved word
    /// and is accepted too.
    fn parse_attribute_designator(&mut self) -> PResult<Ident> {
        match self.kind() {
            TokenKind::Ident | TokenKind::ExtendedIdent => self.parse_ident(),
            TokenKind::Range => {
                let t = self.bump();
                Ok(Ident {
                    name: t.text().to_owned(),
                    span: t.span,
                    extended: false,
                })
            }
            _ => Err(self.expected("an attribute designator")),
        }
    }

    // --- association lists -------------------------------------------------------

    /// `( association_element {, association_element} )`
    pub(super) fn parse_association_list(&mut self) -> PResult<Vec<AssociationElement>> {
        self.expect(TokenKind::LParen)?;
        let mut items = Vec::new();
        if !self.at(TokenKind::RParen) {
            items.push(self.parse_association_element()?);
            while self.eat(TokenKind::Comma).is_some() {
                items.push(self.parse_association_element()?);
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(items)
    }

    /// `[formal =>] actual`
    fn parse_association_element(&mut self) -> PResult<AssociationElement> {
        let start = self.span();
        if self.at_any(&[TokenKind::Open, TokenKind::Inertial]) {
            let actual = self.parse_actual()?;
            return Ok(AssociationElement {
                formal: None,
                actual,
                span: self.span_from(start),
            });
        }
        let e = self.parse_expr()?;
        if self.eat(TokenKind::Arrow).is_some() {
            let actual = self.parse_actual()?;
            return Ok(AssociationElement {
                formal: Some(e),
                actual,
                span: self.span_from(start),
            });
        }
        let actual = self.actual_after_expr(e, start)?;
        Ok(AssociationElement {
            formal: None,
            actual,
            span: self.span_from(start),
        })
    }

    /// `open`, `inertial expr`, an expression or a discrete range.
    fn parse_actual(&mut self) -> PResult<Actual> {
        let start = self.span();
        match self.kind() {
            TokenKind::Open => Ok(Actual::Open(self.bump().span)),
            TokenKind::Inertial => {
                self.bump();
                self.require_2008(start, "`inertial` actuals");
                Ok(Actual::Inertial(self.parse_expr()?))
            }
            _ => {
                let e = self.parse_expr()?;
                self.actual_after_expr(e, start)
            }
        }
    }

    /// Turns an already parsed expression into an actual, continuing it
    /// into a range when `to`, `downto` or `range` follows.
    fn actual_after_expr(&mut self, e: Expr, start: crate::source::Span) -> PResult<Actual> {
        if self.at_any(&[TokenKind::To, TokenKind::Downto, TokenKind::Range]) {
            let r = self.discrete_range_after_expr(e, start)?;
            Ok(Actual::Range(r))
        } else {
            Ok(Actual::Expr(e))
        }
    }

    // --- external names ----------------------------------------------------------

    /// `<< class pathname : subtype_indication >>`
    fn parse_external_name(&mut self) -> PResult<Name> {
        let start = self.span();
        self.expect(TokenKind::LtLt)?;
        self.require_2008(start, "external names");
        let class = match self.kind() {
            TokenKind::Constant => ExternalClass::Constant,
            TokenKind::Signal => ExternalClass::Signal,
            TokenKind::Variable => ExternalClass::Variable,
            _ => return Err(self.expected("`constant`, `signal` or `variable`")),
        };
        self.bump();
        let path = self.parse_external_path()?;
        self.expect(TokenKind::Colon)?;
        let subtype = self.parse_subtype_indication()?;
        self.expect(TokenKind::GtGt)?;
        Ok(Name::External(Box::new(ExternalName {
            class,
            path,
            subtype,
            span: self.span_from(start),
        })))
    }

    /// `@lib.pkg.obj`, `.top.a.b` or `[^.]* a.b`
    fn parse_external_path(&mut self) -> PResult<ExternalPath> {
        let start = self.span();
        let kind = match self.kind() {
            TokenKind::At => {
                self.bump();
                ExternalPathKind::Package
            }
            TokenKind::Dot => {
                self.bump();
                ExternalPathKind::Absolute
            }
            _ => {
                let mut ups = 0u32;
                while self.at(TokenKind::Caret) {
                    self.bump();
                    self.expect(TokenKind::Dot)?;
                    ups += 1;
                }
                ExternalPathKind::Relative(ups)
            }
        };
        let mut elements = vec![self.parse_path_element()?];
        while self.eat(TokenKind::Dot).is_some() {
            elements.push(self.parse_path_element()?);
        }
        Ok(ExternalPath {
            kind,
            elements,
            span: self.span_from(start),
        })
    }

    /// `name [(index)]`
    fn parse_path_element(&mut self) -> PResult<PathElement> {
        let start = self.span();
        let name = self.parse_ident()?;
        let index = if self.eat(TokenKind::LParen).is_some() {
            let e = self.parse_expr()?;
            self.expect(TokenKind::RParen)?;
            Some(e)
        } else {
            None
        };
        Ok(PathElement {
            name,
            index,
            span: self.span_from(start),
        })
    }
}

/// When `args` is exactly one positional range, returns it (the name is a
/// slice); otherwise hands the arguments back.
fn single_range(
    mut args: Vec<AssociationElement>,
) -> Result<DiscreteRange, Vec<AssociationElement>> {
    if args.len() == 1 && args[0].formal.is_none() && matches!(args[0].actual, Actual::Range(_)) {
        match args.pop().map(|a| a.actual) {
            Some(Actual::Range(r)) => Ok(r),
            _ => unreachable!(),
        }
    } else {
        Err(args)
    }
}
