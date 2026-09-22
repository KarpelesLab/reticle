//! Data types and dimensions.
//!
//! Three entry points cover every place a type can appear:
//!
//! - [`Parser::parse_data_type`] requires a type: a keyword type, an
//!   `enum` / `struct`, `type(...)`, or a name (`T`, `pkg::T`,
//!   `bus_if.master`);
//! - [`Parser::parse_data_type_or_implicit`] also accepts nothing at all,
//!   or just `signed` / packed dimensions, which is what `wire [7:0]` or
//!   `parameter P` need. A name is only taken as a type when the lookahead
//!   [`Parser::named_type_ahead`] says another name follows it, since
//!   `parameter foo = 1` names a parameter, not a type;
//! - [`Parser::parse_dims`] reads a run of `[...]` dimensions in all their
//!   packed, unpacked, queue and associative forms.

use super::super::ast::{
    DataType, DataTypeKind, Declarator, Dim, DimKind, EnumType, EnumVariant, IntegerType, RealType,
    Signing, StructMember, StructType,
};
use super::super::token::{Keyword, Punct, TokenKind};
use super::{PResult, Parser};

impl Parser<'_> {
    /// True when the cursor is on a keyword that starts a data type
    /// (`logic`, `int`, `enum`, `string`, `type`, ...).
    pub(super) fn at_data_type_keyword(&self) -> bool {
        use Keyword as K;
        matches!(
            self.kind(),
            TokenKind::Keyword(
                K::Bit
                    | K::Logic
                    | K::Reg
                    | K::Byte
                    | K::Shortint
                    | K::Int
                    | K::Longint
                    | K::Integer
                    | K::Time
                    | K::Real
                    | K::Shortreal
                    | K::Realtime
                    | K::String
                    | K::Chandle
                    | K::Event
                    | K::Void
                    | K::Enum
                    | K::Struct
                    | K::Union
                    | K::Type
            )
        )
    }

    /// True when a name at the cursor is followed, after an optional
    /// `::name`, `.name` and any number of `[...]`, by another name: the
    /// shape of `foo_t x`, `pkg::t [3:0] x` or `bus_if.master p`, which
    /// makes the first name a type.
    pub(super) fn named_type_ahead(&self) -> bool {
        if !self.at_ident() {
            return false;
        }
        let mut i = self.pos + 1;
        while self.kind_at(i).is_punct(Punct::ColonColon) && self.kind_at(i + 1).is_ident() {
            i += 2;
        }
        if self.kind_at(i).is_punct(Punct::Dot) && self.kind_at(i + 1).is_ident() {
            i += 2;
        }
        while self.kind_at(i).is_punct(Punct::LBracket) {
            i = self.after_balanced(i);
        }
        self.kind_at(i).is_ident()
    }

    /// `signed` or `unsigned`, if present.
    pub(super) fn parse_signing(&mut self) -> Option<Signing> {
        if self.eat_kw(Keyword::Signed).is_some() {
            Some(Signing::Signed)
        } else if self.eat_kw(Keyword::Unsigned).is_some() {
            Some(Signing::Unsigned)
        } else {
            None
        }
    }

    /// A type that may be absent or reduced to `signed [7:0]`.
    pub(super) fn parse_data_type_or_implicit(&mut self) -> PResult<DataType> {
        if self.at_data_type_keyword() || self.named_type_ahead() {
            return self.parse_data_type();
        }
        let start = self.span();
        let signing = self.parse_signing();
        let packed = self.parse_dims()?;
        let span = if signing.is_some() || !packed.is_empty() {
            self.span_from(start)
        } else {
            crate::source::Span::new(start.file, start.start, start.start)
        };
        Ok(DataType {
            kind: DataTypeKind::Implicit,
            signing,
            packed,
            span,
        })
    }

    /// A type that must be present.
    pub(super) fn parse_data_type(&mut self) -> PResult<DataType> {
        use Keyword as K;
        let start = self.span();
        let (kind, signing) = match self.kind() {
            TokenKind::Keyword(kw) => match *kw {
                K::Bit | K::Logic | K::Reg => {
                    let int = match *kw {
                        K::Bit => IntegerType::Bit,
                        K::Logic => IntegerType::Logic,
                        _ => IntegerType::Reg,
                    };
                    self.bump();
                    (DataTypeKind::Integer(int), self.parse_signing())
                }
                K::Byte | K::Shortint | K::Int | K::Longint | K::Integer | K::Time => {
                    let int = match *kw {
                        K::Byte => IntegerType::Byte,
                        K::Shortint => IntegerType::Shortint,
                        K::Int => IntegerType::Int,
                        K::Longint => IntegerType::Longint,
                        K::Integer => IntegerType::Integer,
                        _ => IntegerType::Time,
                    };
                    self.bump();
                    (DataTypeKind::Integer(int), self.parse_signing())
                }
                K::Real | K::Shortreal | K::Realtime => {
                    let real = match *kw {
                        K::Real => RealType::Real,
                        K::Shortreal => RealType::Shortreal,
                        _ => RealType::Realtime,
                    };
                    self.bump();
                    (DataTypeKind::Real(real), None)
                }
                K::String => {
                    self.bump();
                    (DataTypeKind::String, None)
                }
                K::Chandle => {
                    self.bump();
                    (DataTypeKind::Chandle, None)
                }
                K::Event => {
                    self.bump();
                    (DataTypeKind::Event, None)
                }
                K::Void => {
                    self.bump();
                    (DataTypeKind::Void, None)
                }
                K::Enum => (DataTypeKind::Enum(self.parse_enum_type()?), None),
                K::Struct | K::Union => {
                    let (st, signing) = self.parse_struct_type()?;
                    (DataTypeKind::Struct(st), signing)
                }
                K::Type => {
                    self.bump();
                    self.expect_punct(Punct::LParen)?;
                    let inner = self.parse_expr()?;
                    self.expect_punct(Punct::RParen)?;
                    (DataTypeKind::TypeOf(Box::new(inner)), None)
                }
                K::Interface => {
                    self.bump();
                    let modport = if self.eat_punct(Punct::Dot).is_some() {
                        Some(self.expect_ident()?)
                    } else {
                        None
                    };
                    (DataTypeKind::Interface { modport }, None)
                }
                _ => return Err(self.expected("data type")),
            },
            TokenKind::Ident { .. } | TokenKind::EscapedIdent { .. } => {
                let first = self.expect_ident()?;
                let (package, name) = if self.at_punct(Punct::ColonColon) {
                    self.bump();
                    (Some(first), self.expect_ident()?)
                } else {
                    (None, first)
                };
                let member = if self.at_punct(Punct::Dot) && self.nth(1).is_ident() {
                    self.bump();
                    Some(self.expect_ident()?)
                } else {
                    None
                };
                (
                    DataTypeKind::Named {
                        package,
                        name,
                        member,
                    },
                    None,
                )
            }
            _ => return Err(self.expected("data type")),
        };
        let packed = self.parse_dims()?;
        Ok(DataType {
            kind,
            signing,
            packed,
            span: self.span_from(start),
        })
    }

    /// `enum [base] { A, B = 1, C[4], D[1:2] = 5 }`, cursor on `enum`.
    fn parse_enum_type(&mut self) -> PResult<EnumType> {
        self.expect_kw(Keyword::Enum)?;
        let base = if self.at_punct(Punct::LBrace) {
            None
        } else {
            Some(Box::new(self.parse_data_type_or_implicit_named()?))
        };
        self.expect_punct(Punct::LBrace)?;
        let mut variants = Vec::new();
        loop {
            let name = self.expect_ident()?;
            let range = if self.at_punct(Punct::LBracket) {
                Some(self.parse_dim()?)
            } else {
                None
            };
            let value = if self.eat_punct(Punct::Eq).is_some() {
                Some(self.parse_expr()?)
            } else {
                None
            };
            let span = self.span_from(name.span);
            variants.push(EnumVariant {
                name,
                range,
                value,
                span,
            });
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
            if self.at_punct(Punct::RBrace) {
                // Trailing comma, tolerated.
                break;
            }
        }
        self.expect_punct(Punct::RBrace)?;
        Ok(EnumType { base, variants })
    }

    /// Like [`Self::parse_data_type_or_implicit`] but a lone name is a type:
    /// used for `enum my_base_t { ... }`, where nothing else can follow.
    fn parse_data_type_or_implicit_named(&mut self) -> PResult<DataType> {
        if self.at_ident() {
            self.parse_data_type()
        } else {
            self.parse_data_type_or_implicit()
        }
    }

    /// `struct [packed] [signing] { members }` or `union [tagged] [packed]
    /// [signing] { members }`, cursor on the keyword.
    fn parse_struct_type(&mut self) -> PResult<(StructType, Option<Signing>)> {
        let is_union = self.at_kw(Keyword::Union);
        self.bump();
        let tagged = self.eat_kw(Keyword::Tagged).is_some();
        let packed = self.eat_kw(Keyword::Packed).is_some();
        let signing = self.parse_signing();
        self.expect_punct(Punct::LBrace)?;
        let mut members = Vec::new();
        while !self.at_punct(Punct::RBrace) && !self.at_eof() {
            let start = self.span();
            // `rand` / `randc` qualifiers are accepted and dropped.
            while self.eat_kw(Keyword::Rand).is_some() || self.eat_kw(Keyword::Randc).is_some() {}
            let data_type = self.parse_data_type()?;
            let decls = self.parse_declarators()?;
            self.expect_semi()?;
            members.push(StructMember {
                data_type,
                decls,
                span: self.span_from(start),
            });
        }
        self.expect_punct(Punct::RBrace)?;
        Ok((
            StructType {
                is_union,
                packed,
                tagged,
                members,
            },
            signing,
        ))
    }

    /// `name [dims] [= init] {, ...}`.
    pub(super) fn parse_declarators(&mut self) -> PResult<Vec<Declarator>> {
        let mut decls = Vec::new();
        loop {
            let name = self.expect_ident()?;
            let dims = self.parse_dims()?;
            // Parameter and specparam values may be a bare `min:typ:max`.
            let init = if self.eat_punct(Punct::Eq).is_some() {
                Some(self.parse_mintypmax()?)
            } else {
                None
            };
            let span = self.span_from(name.span);
            decls.push(Declarator {
                name,
                dims,
                init,
                span,
            });
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        Ok(decls)
    }

    /// Any number of `[...]` dimensions.
    pub(super) fn parse_dims(&mut self) -> PResult<Vec<Dim>> {
        let mut dims = Vec::new();
        while self.at_punct(Punct::LBracket) {
            dims.push(self.parse_dim()?);
        }
        Ok(dims)
    }

    /// One `[...]`: `[msb:lsb]`, `[N]`, `[]`, `[$]`, `[$:N]`, `[*]` or
    /// `[type]`.
    pub(super) fn parse_dim(&mut self) -> PResult<Dim> {
        let start = self.expect_punct(Punct::LBracket)?;
        let kind = if self.at_punct(Punct::RBracket) {
            DimKind::Unsized
        } else if self.at_punct(Punct::Dollar) {
            self.bump();
            let bound = if self.eat_punct(Punct::Colon).is_some() {
                Some(self.parse_expr()?)
            } else {
                None
            };
            DimKind::Queue(bound)
        } else if self.at_punct(Punct::Star) && self.nth_is_punct(1, Punct::RBracket) {
            self.bump();
            DimKind::Assoc(None)
        } else if self.at_data_type_keyword() {
            DimKind::Assoc(Some(self.parse_data_type()?))
        } else {
            let first = self.parse_expr()?;
            if self.eat_punct(Punct::Colon).is_some() {
                let second = self.parse_expr()?;
                DimKind::Range(first, second)
            } else {
                DimKind::Size(first)
            }
        };
        self.expect_punct(Punct::RBracket)?;
        Ok(Dim {
            kind,
            span: self.span_from(start),
        })
    }
}
