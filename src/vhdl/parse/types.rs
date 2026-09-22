//! Type declarations, subtype indications, constraints, ranges and
//! signatures.

use super::{PResult, Parser, Recover};
use crate::vhdl::ast::*;
use crate::vhdl::token::TokenKind;

impl<'t, 'src> Parser<'t, 'src> {
    /// `type name [is type_definition];`
    pub(super) fn parse_type_decl(&mut self) -> PResult<TypeDecl> {
        let start = self.span();
        self.expect(TokenKind::Type)?;
        let name = self.parse_ident()?;
        if self.eat(TokenKind::Semi).is_some() {
            return Ok(TypeDecl {
                name,
                def: None,
                span: self.span_from(start),
            });
        }
        self.expect(TokenKind::Is)?;
        let def = self.parse_type_definition(&name)?;
        self.expect_semi()?;
        Ok(TypeDecl {
            name,
            def: Some(def),
            span: self.span_from(start),
        })
    }

    /// The definition after `type name is`.
    fn parse_type_definition(&mut self, name: &Ident) -> PResult<TypeDef> {
        let start = self.span();
        match self.kind() {
            TokenKind::LParen => {
                let lits = self.parse_paren_list(|p| match p.kind() {
                    TokenKind::Ident | TokenKind::ExtendedIdent | TokenKind::CharLit => {
                        p.parse_designator()
                    }
                    _ => Err(p.expected("an enumeration literal")),
                })?;
                Ok(TypeDef::Enumeration(lits))
            }
            TokenKind::Range => {
                self.bump();
                let range = self.parse_range()?;
                if self.at(TokenKind::Units) {
                    self.bump();
                    let primary_unit = self.parse_ident()?;
                    self.expect_semi()?;
                    let mut secondary_units = Vec::new();
                    while !self.at_any(&[TokenKind::End, TokenKind::Eof]) {
                        let item_start = self.pos;
                        match self.parse_secondary_unit() {
                            Ok(u) => secondary_units.push(u),
                            Err(Recover) => self.recover(item_start, |k| k == TokenKind::End),
                        }
                    }
                    self.with_open(TokenKind::Units, |p| {
                        p.parse_end_no_semi(&[TokenKind::Units], Some(name))
                    })?;
                    return Ok(TypeDef::Physical(PhysicalTypeDef {
                        range,
                        primary_unit,
                        secondary_units,
                        span: self.span_from(start),
                    }));
                }
                Ok(TypeDef::Range(range))
            }
            TokenKind::Array => {
                self.bump();
                let indices = self.parse_paren_list(|p| p.parse_array_index())?;
                self.expect(TokenKind::Of)?;
                let element = self.parse_subtype_indication()?;
                Ok(TypeDef::Array(ArrayTypeDef {
                    indices,
                    element,
                    span: self.span_from(start),
                }))
            }
            TokenKind::Record => {
                self.bump();
                let mut elements = Vec::new();
                self.with_open(TokenKind::Record, |p| {
                    while !p.at_any(&[TokenKind::End, TokenKind::Eof]) {
                        let item_start = p.pos;
                        match p.parse_record_element() {
                            Ok(e) => elements.push(e),
                            Err(Recover) => p.recover(item_start, |k| k == TokenKind::End),
                        }
                    }
                    p.parse_end_no_semi(&[TokenKind::Record], Some(name))
                })?;
                Ok(TypeDef::Record(RecordTypeDef {
                    elements,
                    span: self.span_from(start),
                }))
            }
            TokenKind::Access => {
                self.bump();
                Ok(TypeDef::Access(self.parse_subtype_indication()?))
            }
            TokenKind::File => {
                self.bump();
                self.expect(TokenKind::Of)?;
                Ok(TypeDef::File(self.parse_type_mark()?))
            }
            TokenKind::Protected => {
                self.bump();
                let body = self.eat(TokenKind::Body).is_some();
                let decls = self.with_open(TokenKind::Protected, |p| {
                    let decls = p.parse_declarative_part();
                    let kws: &[TokenKind] = if body {
                        &[TokenKind::Protected, TokenKind::Body]
                    } else {
                        &[TokenKind::Protected]
                    };
                    p.parse_end_no_semi(kws, Some(name))?;
                    Ok(decls)
                })?;
                let span = self.span_from(start);
                Ok(if body {
                    TypeDef::ProtectedBody(ProtectedTypeBody { decls, span })
                } else {
                    TypeDef::Protected(ProtectedTypeDecl { decls, span })
                })
            }
            _ => Err(self.expected("a type definition")),
        }
    }

    /// `name = physical_literal;`
    fn parse_secondary_unit(&mut self) -> PResult<SecondaryUnit> {
        let start = self.span();
        let name = self.parse_ident()?;
        self.expect(TokenKind::Eq)?;
        let value = self.parse_primary()?;
        self.expect_semi()?;
        Ok(SecondaryUnit {
            name,
            value,
            span: self.span_from(start),
        })
    }

    /// `names : subtype;`
    fn parse_record_element(&mut self) -> PResult<RecordElement> {
        let start = self.span();
        let names = self.parse_ident_list()?;
        self.expect(TokenKind::Colon)?;
        let subtype = self.parse_subtype_indication()?;
        self.expect_semi()?;
        Ok(RecordElement {
            names,
            subtype,
            span: self.span_from(start),
        })
    }

    /// One dimension of an array type: `type_mark range <>` or a discrete
    /// range.
    fn parse_array_index(&mut self) -> PResult<ArrayIndex> {
        let start = self.span();
        let e = self.parse_expr()?;
        if self.at(TokenKind::Range) && self.kind_at(1) == TokenKind::Box {
            self.bump();
            self.bump();
            let name = self.expr_to_name(e)?;
            return Ok(ArrayIndex::Unbounded(name));
        }
        self.discrete_range_after_expr(e, start)
            .map(ArrayIndex::Constrained)
    }

    // --- subtype indications ----------------------------------------------------

    /// `[resolution] type_mark [constraint]`
    pub(super) fn parse_subtype_indication(&mut self) -> PResult<SubtypeIndication> {
        let start = self.span();
        let mut resolution = if self.at(TokenKind::LParen) {
            Some(self.parse_element_resolution()?)
        } else {
            None
        };
        let mut type_mark = self.parse_type_mark()?;
        if resolution.is_none() && self.at_any(&[TokenKind::Ident, TokenKind::ExtendedIdent]) {
            resolution = Some(ResolutionIndication::Function(type_mark));
            type_mark = self.parse_type_mark()?;
        }
        let constraint = self.parse_optional_constraint()?;
        Ok(SubtypeIndication {
            resolution,
            type_mark,
            constraint,
            span: self.span_from(start),
        })
    }

    /// `( resolution )` or `( name resolution, ... )` (VHDL-2008).
    fn parse_element_resolution(&mut self) -> PResult<ResolutionIndication> {
        let start = self.span();
        self.expect(TokenKind::LParen)?;
        self.require_2008(start, "element resolution");
        let is_record = matches!(self.kind(), TokenKind::Ident | TokenKind::ExtendedIdent)
            && matches!(
                self.kind_at(1),
                TokenKind::Ident | TokenKind::ExtendedIdent | TokenKind::LParen
            );
        let r = if is_record {
            let mut entries = Vec::new();
            loop {
                let s = self.span();
                let name = self.parse_ident()?;
                let resolution = self.parse_resolution_indication()?;
                entries.push(RecordResolution {
                    name,
                    resolution,
                    span: self.span_from(s),
                });
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
            self.expect(TokenKind::RParen)?;
            ResolutionIndication::Record(entries, self.span_from(start))
        } else {
            let inner = self.parse_resolution_indication()?;
            self.expect(TokenKind::RParen)?;
            ResolutionIndication::Array(Box::new(inner), self.span_from(start))
        };
        Ok(r)
    }

    /// A resolution function name or a parenthesised element resolution.
    fn parse_resolution_indication(&mut self) -> PResult<ResolutionIndication> {
        if self.at(TokenKind::LParen) {
            self.parse_element_resolution()
        } else {
            Ok(ResolutionIndication::Function(self.parse_type_mark()?))
        }
    }

    /// A type mark: a name without a trailing parenthesised part, so that
    /// `t(7 downto 0)` leaves the constraint to the caller.
    pub(super) fn parse_type_mark(&mut self) -> PResult<Name> {
        self.parse_name_no_call()
    }

    /// `[range r | (index_constraint) [element] | (record_constraint)]`
    pub(super) fn parse_optional_constraint(&mut self) -> PResult<Option<Constraint>> {
        match self.kind() {
            TokenKind::Range => {
                self.bump();
                Ok(Some(Constraint::Range(self.parse_range()?)))
            }
            TokenKind::LParen => Ok(Some(self.parse_paren_constraint()?)),
            _ => Ok(None),
        }
    }

    /// An array or record constraint in parentheses, with any element
    /// constraint that follows.
    fn parse_paren_constraint(&mut self) -> PResult<Constraint> {
        let start = self.span();
        self.expect(TokenKind::LParen)?;
        if self.at(TokenKind::Open) && self.kind_at(1) == TokenKind::RParen {
            self.bump();
            self.bump();
            self.require_2008(start, "`(open)` constraints");
            let element = self.parse_element_constraint()?;
            return Ok(Constraint::Array {
                indices: Vec::new(),
                element,
                span: self.span_from(start),
            });
        }
        if self.at_record_constraint() {
            let mut entries = Vec::new();
            loop {
                let s = self.span();
                let name = self.parse_ident()?;
                let constraint = self.parse_paren_constraint()?;
                entries.push(RecordConstraint {
                    name,
                    constraint,
                    span: self.span_from(s),
                });
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
            self.expect(TokenKind::RParen)?;
            self.require_2008(start, "record constraints");
            return Ok(Constraint::Record(entries, self.span_from(start)));
        }
        let mut indices = vec![self.parse_discrete_range()?];
        while self.eat(TokenKind::Comma).is_some() {
            indices.push(self.parse_discrete_range()?);
        }
        self.expect(TokenKind::RParen)?;
        let element = self.parse_element_constraint()?;
        Ok(Constraint::Array {
            indices,
            element,
            span: self.span_from(start),
        })
    }

    /// True when the parenthesised list at the cursor is a record
    /// constraint: `name ( ... ) ,` or `name ( ... ) )`.
    fn at_record_constraint(&self) -> bool {
        if !matches!(self.kind(), TokenKind::Ident | TokenKind::ExtendedIdent)
            || self.kind_at(1) != TokenKind::LParen
        {
            return false;
        }
        match self.matching_paren(self.pos + 1) {
            Some(end) => matches!(
                self.tokens.get(end + 1).map(|t| t.kind),
                Some(TokenKind::Comma | TokenKind::RParen)
            ),
            None => false,
        }
    }

    /// An element constraint following an array constraint (VHDL-2008).
    fn parse_element_constraint(&mut self) -> PResult<Option<Box<Constraint>>> {
        if self.at(TokenKind::LParen) {
            let start = self.span();
            let c = self.parse_paren_constraint()?;
            self.require_2008(start, "element constraints");
            Ok(Some(Box::new(c)))
        } else {
            Ok(None)
        }
    }

    // --- ranges ------------------------------------------------------------------

    /// `expr to|downto expr` or a range attribute name.
    pub(super) fn parse_range(&mut self) -> PResult<Range> {
        let start = self.span();
        let left = self.parse_simple_expression()?;
        self.range_after_expr(left, start)
    }

    /// Completes a range whose left bound (or attribute name) has been
    /// parsed.
    fn range_after_expr(&mut self, left: Expr, start: crate::source::Span) -> PResult<Range> {
        let direction = match self.kind() {
            TokenKind::To => Direction::To,
            TokenKind::Downto => Direction::Downto,
            _ => {
                return match left {
                    Expr::Name(n) if is_attribute_name(&n) => Ok(Range::Attribute(n)),
                    _ => Err(self.expected("`to`, `downto` or a range attribute")),
                };
            }
        };
        self.bump();
        let right = self.parse_simple_expression()?;
        Ok(Range::Bounds {
            left,
            direction,
            right,
            span: self.span_from(start),
        })
    }

    /// A discrete range: a range, or a subtype indication.
    pub(super) fn parse_discrete_range(&mut self) -> PResult<DiscreteRange> {
        let start = self.span();
        let e = self.parse_expr()?;
        self.discrete_range_after_expr(e, start)
    }

    /// Completes a discrete range whose first expression has been parsed:
    /// `e to x`, `e range r`, `e` as a range attribute, or `e` as a subtype
    /// name.
    pub(super) fn discrete_range_after_expr(
        &mut self,
        e: Expr,
        start: crate::source::Span,
    ) -> PResult<DiscreteRange> {
        match self.kind() {
            TokenKind::To | TokenKind::Downto => {
                self.range_after_expr(e, start).map(DiscreteRange::Range)
            }
            TokenKind::Range => {
                self.bump();
                let type_mark = self.expr_to_name(e)?;
                let range = self.parse_range()?;
                Ok(DiscreteRange::Subtype(Box::new(SubtypeIndication {
                    resolution: None,
                    type_mark,
                    constraint: Some(Constraint::Range(range)),
                    span: self.span_from(start),
                })))
            }
            _ => match e {
                Expr::Name(n) if is_attribute_name(&n) => {
                    Ok(DiscreteRange::Range(Range::Attribute(n)))
                }
                Expr::Name(type_mark) => Ok(DiscreteRange::Subtype(Box::new(SubtypeIndication {
                    resolution: None,
                    type_mark,
                    constraint: None,
                    span: self.span_from(start),
                }))),
                _ => Err(self.expected("`to`, `downto` or `range`")),
            },
        }
    }

    /// Reinterprets an expression as a name, or reports an error.
    pub(super) fn expr_to_name(&mut self, e: Expr) -> PResult<Name> {
        match e {
            Expr::Name(n) => Ok(n),
            other => {
                let span = other.span();
                self.report(crate::diag::Diagnostic::error("expected a name").with_span(span));
                Err(Recover)
            }
        }
    }

    // --- signatures ---------------------------------------------------------------

    /// `[ [type_mark {, type_mark}] [return type_mark] ]` when the cursor
    /// is at `[`.
    pub(super) fn parse_optional_signature(&mut self) -> PResult<Option<Signature>> {
        if !self.at(TokenKind::LBracket) {
            return Ok(None);
        }
        let start = self.span();
        self.bump();
        let mut params = Vec::new();
        if !self.at_any(&[TokenKind::Return, TokenKind::RBracket]) {
            params.push(self.parse_type_mark()?);
            while self.eat(TokenKind::Comma).is_some() {
                params.push(self.parse_type_mark()?);
            }
        }
        let return_type = if self.eat(TokenKind::Return).is_some() {
            Some(self.parse_type_mark()?)
        } else {
            None
        };
        self.expect(TokenKind::RBracket)?;
        Ok(Some(Signature {
            params,
            return_type,
            span: self.span_from(start),
        }))
    }
}

/// True for an attribute name, possibly with an argument (`x'range`,
/// `x'reverse_range(1)`), which may denote a range.
pub(super) fn is_attribute_name(n: &Name) -> bool {
    match n {
        Name::Attribute { .. } => true,
        Name::Call { prefix, .. } => matches!(**prefix, Name::Attribute { .. }),
        _ => false,
    }
}
