//! Declarations: nets, variables, parameters, ports, typedefs, functions
//! and tasks, continuous assignments, and the smaller items (`defparam`,
//! `alias`, clocking blocks, property declarations, UDP tables).

use super::super::ast::{
    AssignPair, Clocking, ContAssign, Defparam, Direction, ItemKind, Lifetime, NetDecl, NetType,
    ParamDecl, ParamKind, Port, PortDecl, PropertyDecl, RawTokens, Strength, StrengthLevel,
    Subroutine, Typedef, VarDecl, Vectored,
};
use super::super::token::{Keyword, Punct, TokenKind};
use super::{PResult, Parser};

impl Parser<'_> {
    /// The net type a keyword denotes.
    pub(super) fn net_type_of(kind: &TokenKind) -> Option<NetType> {
        use Keyword as K;
        let TokenKind::Keyword(kw) = kind else {
            return None;
        };
        Some(match kw {
            K::Wire => NetType::Wire,
            K::Tri => NetType::Tri,
            K::Tri0 => NetType::Tri0,
            K::Tri1 => NetType::Tri1,
            K::Triand => NetType::Triand,
            K::Trior => NetType::Trior,
            K::Trireg => NetType::Trireg,
            K::Wand => NetType::Wand,
            K::Wor => NetType::Wor,
            K::Supply0 => NetType::Supply0,
            K::Supply1 => NetType::Supply1,
            K::Uwire => NetType::Uwire,
            K::Interconnect => NetType::Interconnect,
            _ => return None,
        })
    }

    /// The strength level a keyword denotes.
    fn strength_level_of(kind: &TokenKind) -> Option<StrengthLevel> {
        use Keyword as K;
        let TokenKind::Keyword(kw) = kind else {
            return None;
        };
        Some(match kw {
            K::Supply0 => StrengthLevel::Supply0,
            K::Strong0 => StrengthLevel::Strong0,
            K::Pull0 => StrengthLevel::Pull0,
            K::Weak0 => StrengthLevel::Weak0,
            K::Highz0 => StrengthLevel::Highz0,
            K::Supply1 => StrengthLevel::Supply1,
            K::Strong1 => StrengthLevel::Strong1,
            K::Pull1 => StrengthLevel::Pull1,
            K::Weak1 => StrengthLevel::Weak1,
            K::Highz1 => StrengthLevel::Highz1,
            K::Small => StrengthLevel::Small,
            K::Medium => StrengthLevel::Medium,
            K::Large => StrengthLevel::Large,
            _ => return None,
        })
    }

    /// `(strength [, strength])` when the cursor is on a `(` followed by a
    /// strength keyword; otherwise nothing is consumed.
    pub(super) fn parse_strength_opt(&mut self) -> PResult<Option<Strength>> {
        if !self.at_punct(Punct::LParen) || Self::strength_level_of(self.nth(1)).is_none() {
            return Ok(None);
        }
        let start = self.bump();
        let mut levels = Vec::new();
        loop {
            match Self::strength_level_of(self.kind()) {
                Some(level) => {
                    self.bump();
                    levels.push(level);
                }
                None => return Err(self.expected("strength")),
            }
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(Some(Strength {
            levels,
            span: self.span_from(start),
        }))
    }

    /// `automatic` or `static`, if present.
    pub(super) fn parse_lifetime(&mut self) -> Option<Lifetime> {
        if self.eat_kw(Keyword::Automatic).is_some() {
            Some(Lifetime::Automatic)
        } else if self.eat_kw(Keyword::Static).is_some() {
            Some(Lifetime::Static)
        } else {
            None
        }
    }

    /// `wire [(strength)] [vectored] [type] [#delay] decls;`, cursor on
    /// the net type.
    pub(super) fn parse_net_decl(&mut self) -> PResult<NetDecl> {
        let Some(net_type) = Self::net_type_of(self.kind()) else {
            return Err(self.expected("net type"));
        };
        self.bump();
        let strength = self.parse_strength_opt()?;
        let vectored = if self.eat_kw(Keyword::Vectored).is_some() {
            Some(Vectored::Vectored)
        } else if self.eat_kw(Keyword::Scalared).is_some() {
            Some(Vectored::Scalared)
        } else {
            None
        };
        let data_type = self.parse_data_type_or_implicit()?;
        let delay = self.parse_delay_opt()?;
        let decls = self.parse_declarators()?;
        self.expect_semi()?;
        Ok(NetDecl {
            net_type,
            strength,
            vectored,
            data_type,
            delay,
            decls,
        })
    }

    /// True on a keyword that begins a variable declaration.
    pub(super) fn at_var_decl_start(&self) -> bool {
        use Keyword as K;
        matches!(
            self.kind(),
            TokenKind::Keyword(K::Var | K::Const | K::Automatic | K::Static)
        ) || (self.at_data_type_keyword() && !self.at_kw(K::Void))
    }

    /// `[lifetime] [const] [var] type decls;` or, for a user-defined type,
    /// `name decls;`.
    pub(super) fn parse_var_decl(&mut self) -> PResult<VarDecl> {
        let lifetime = self.parse_lifetime();
        let constant = self.eat_kw(Keyword::Const).is_some();
        let var = self.eat_kw(Keyword::Var).is_some();
        let data_type = if var {
            self.parse_data_type_or_implicit()?
        } else {
            self.parse_data_type()?
        };
        let decls = self.parse_declarators()?;
        self.expect_semi()?;
        Ok(VarDecl {
            lifetime,
            constant,
            var,
            data_type,
            decls,
        })
    }

    /// `genvar a, b;`, cursor on `genvar`.
    pub(super) fn parse_genvar_decl(&mut self) -> PResult<ItemKind> {
        self.expect_kw(Keyword::Genvar)?;
        let mut names = vec![self.expect_ident()?];
        while self.eat_punct(Punct::Comma).is_some() {
            names.push(self.expect_ident()?);
        }
        self.expect_semi()?;
        Ok(ItemKind::Genvar(names))
    }

    /// `parameter [type|data_type] decls;`, cursor on the keyword.
    pub(super) fn parse_param_decl(&mut self) -> PResult<ParamDecl> {
        let mut decl = self.parse_param_decl_head()?;
        decl.decls = self.parse_declarators()?;
        self.expect_semi()?;
        Ok(decl)
    }

    /// The keyword and type of a parameter declaration, with no
    /// declarators yet; shared with the parameter port list.
    pub(super) fn parse_param_decl_head(&mut self) -> PResult<ParamDecl> {
        let kind = match self.kind() {
            TokenKind::Keyword(Keyword::Parameter) => ParamKind::Parameter,
            TokenKind::Keyword(Keyword::Localparam) => ParamKind::Localparam,
            TokenKind::Keyword(Keyword::Specparam) => ParamKind::Specparam,
            _ => return Err(self.expected("`parameter`")),
        };
        self.bump();
        let (is_type, data_type) = self.parse_param_type()?;
        Ok(ParamDecl {
            kind,
            is_type,
            data_type,
            decls: Vec::new(),
        })
    }

    /// `type` or a data type (possibly implicit) after a parameter
    /// keyword.
    pub(super) fn parse_param_type(&mut self) -> PResult<(bool, super::super::ast::DataType)> {
        if self.at_kw(Keyword::Type) && !self.nth_is_punct(1, Punct::LParen) {
            let span = self.bump();
            let empty = crate::source::Span::new(span.file, span.end, span.end);
            return Ok((true, super::super::ast::DataType::implicit(empty)));
        }
        Ok((false, self.parse_data_type_or_implicit()?))
    }

    /// `input [net_type|var] [type] decls;`, cursor on the direction.
    pub(super) fn parse_port_decl(&mut self) -> PResult<PortDecl> {
        let Some(direction) = self.parse_direction() else {
            return Err(self.expected("port direction"));
        };
        let net_type = Self::net_type_of(self.kind());
        if net_type.is_some() {
            self.bump();
        }
        let var = self.eat_kw(Keyword::Var).is_some();
        let data_type = self.parse_data_type_or_implicit()?;
        let decls = self.parse_declarators()?;
        self.expect_semi()?;
        Ok(PortDecl {
            direction,
            net_type,
            var,
            data_type,
            decls,
        })
    }

    /// `input`, `output`, `inout`, `ref` or `const ref`, if present.
    pub(super) fn parse_direction(&mut self) -> Option<Direction> {
        let dir = match self.kind() {
            TokenKind::Keyword(Keyword::Input) => Direction::Input,
            TokenKind::Keyword(Keyword::Output) => Direction::Output,
            TokenKind::Keyword(Keyword::Inout) => Direction::Inout,
            TokenKind::Keyword(Keyword::Ref) => Direction::Ref,
            TokenKind::Keyword(Keyword::Const) if self.nth_is_kw(1, Keyword::Ref) => {
                self.bump();
                Direction::ConstRef
            }
            _ => return None,
        };
        self.bump();
        Some(dir)
    }

    /// `typedef type name dims;` or a forward `typedef [kind] name;`,
    /// cursor on `typedef`.
    pub(super) fn parse_typedef(&mut self) -> PResult<Typedef> {
        self.expect_kw(Keyword::Typedef)?;
        // Forward declarations: `typedef enum name;`, `typedef name;`.
        let forward_kind = matches!(
            self.kind(),
            TokenKind::Keyword(Keyword::Enum | Keyword::Struct | Keyword::Union | Keyword::Class)
        ) && self.nth(1).is_ident()
            && self.nth_is_punct(2, Punct::Semi);
        if forward_kind || (self.at_ident() && self.nth_is_punct(1, Punct::Semi)) {
            if forward_kind {
                self.bump();
            }
            let name = self.expect_ident()?;
            self.expect_semi()?;
            return Ok(Typedef {
                name,
                data_type: None,
                dims: Vec::new(),
            });
        }
        let data_type = self.parse_data_type()?;
        let name = self.expect_ident()?;
        let dims = self.parse_dims()?;
        self.expect_semi()?;
        Ok(Typedef {
            name,
            data_type: Some(data_type),
            dims,
        })
    }

    /// `function [lifetime] [ret] name [(ports)]; body endfunction`.
    pub(super) fn parse_function(&mut self) -> PResult<Subroutine> {
        self.expect_kw(Keyword::Function)?;
        let lifetime = self.parse_lifetime();
        // The return type is present unless the name comes next, which is
        // the case when a name is followed by `(` or `;`.
        let name_next = self.at_ident()
            && (self.nth_is_punct(1, Punct::LParen) || self.nth_is_punct(1, Punct::Semi));
        let ret = if name_next {
            let at = self.span();
            super::super::ast::DataType::implicit(crate::source::Span::new(
                at.file, at.start, at.start,
            ))
        } else {
            self.parse_data_type_or_implicit()?
        };
        self.parse_subroutine_rest(lifetime, Some(ret), Keyword::Endfunction)
    }

    /// `task [lifetime] name [(ports)]; body endtask`.
    pub(super) fn parse_task(&mut self) -> PResult<Subroutine> {
        self.expect_kw(Keyword::Task)?;
        let lifetime = self.parse_lifetime();
        self.parse_subroutine_rest(lifetime, None, Keyword::Endtask)
    }

    /// The name, port list, body and end keyword shared by functions and
    /// tasks.
    fn parse_subroutine_rest(
        &mut self,
        lifetime: Option<Lifetime>,
        ret: Option<super::super::ast::DataType>,
        end: Keyword,
    ) -> PResult<Subroutine> {
        let name = self.expect_ident()?;
        let ports = if self.at_punct(Punct::LParen) {
            self.bump();
            let mut ports = Vec::new();
            if !self.at_punct(Punct::RParen) {
                loop {
                    ports.push(self.parse_tf_port()?);
                    if self.eat_punct(Punct::Comma).is_none() {
                        break;
                    }
                }
            }
            self.expect_punct(Punct::RParen)?;
            Some(ports)
        } else {
            None
        };
        self.expect_semi()?;
        let body = self.parse_stmts_until(&[end]);
        self.expect_end(end);
        Ok(Subroutine {
            lifetime,
            ret,
            name,
            ports,
            body,
        })
    }

    /// One task or function port: `[direction] [var] [type] name [dims]
    /// [= default]`.
    fn parse_tf_port(&mut self) -> PResult<Port> {
        let start = self.span();
        let attrs = self.parse_attrs()?;
        let direction = self.parse_direction();
        let var = self.eat_kw(Keyword::Var).is_some();
        let data_type = self.parse_data_type_or_implicit()?;
        let name = self.expect_ident()?;
        let dims = self.parse_dims()?;
        let default = if self.eat_punct(Punct::Eq).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Port {
            attrs,
            direction,
            net_type: None,
            var,
            data_type,
            name,
            dims,
            default,
            span: self.span_from(start),
        })
    }

    /// `defparam a.b = 1, c = 2;`, cursor on `defparam`.
    pub(super) fn parse_defparam(&mut self) -> PResult<Vec<Defparam>> {
        self.expect_kw(Keyword::Defparam)?;
        let mut list = Vec::new();
        loop {
            let target = self.parse_lvalue()?;
            self.expect_punct(Punct::Eq)?;
            let value = self.parse_expr()?;
            let span = target.span.to(value.span);
            list.push(Defparam {
                target,
                value,
                span,
            });
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        self.expect_semi()?;
        Ok(list)
    }

    /// `assign [(strength)] [#delay] a = b, c = d;`, cursor on `assign`.
    pub(super) fn parse_cont_assign(&mut self) -> PResult<ContAssign> {
        self.expect_kw(Keyword::Assign)?;
        let strength = self.parse_strength_opt()?;
        let delay = self.parse_delay_opt()?;
        let mut assigns = Vec::new();
        loop {
            let lhs = self.parse_lvalue()?;
            self.expect_punct(Punct::Eq)?;
            let rhs = self.parse_expr()?;
            let span = lhs.span.to(rhs.span);
            assigns.push(AssignPair { lhs, rhs, span });
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        self.expect_semi()?;
        Ok(ContAssign {
            strength,
            delay,
            assigns,
        })
    }

    /// `alias a = b = c;`, cursor on `alias`.
    pub(super) fn parse_alias(&mut self) -> PResult<ItemKind> {
        self.expect_kw(Keyword::Alias)?;
        let mut nets = vec![self.parse_lvalue()?];
        while self.eat_punct(Punct::Eq).is_some() {
            nets.push(self.parse_lvalue()?);
        }
        self.expect_semi()?;
        Ok(ItemKind::Alias(nets))
    }

    /// `[default|global] clocking [name] @(event); ... endclocking`,
    /// cursor on the first keyword.
    pub(super) fn parse_clocking(&mut self) -> PResult<Clocking> {
        let is_default = self.eat_kw(Keyword::Default).is_some();
        let is_global = self.eat_kw(Keyword::Global).is_some();
        self.expect_kw(Keyword::Clocking)?;
        let name = self.eat_ident();
        if name.is_some() && self.at_punct(Punct::Semi) {
            // `default clocking name;` refers to a block declared elsewhere.
            let semi = self.bump();
            return Ok(Clocking {
                is_default,
                is_global,
                name,
                event: None,
                body: RawTokens {
                    text: String::new(),
                    span: crate::source::Span::new(semi.file, semi.start, semi.start),
                },
            });
        }
        let event = Some(self.parse_event_control()?);
        self.expect_semi()?;
        let body = self.skip_to_kw(Keyword::Endclocking);
        self.eat_end_label();
        Ok(Clocking {
            is_default,
            is_global,
            name,
            event,
            body,
        })
    }

    /// `property name ... endproperty` or `sequence ... endsequence`,
    /// kept verbatim; cursor on the keyword.
    pub(super) fn parse_property_decl(&mut self) -> PResult<PropertyDecl> {
        let is_sequence = self.at_kw(Keyword::Sequence);
        self.bump();
        let name = self.expect_ident()?;
        let end = if is_sequence {
            Keyword::Endsequence
        } else {
            Keyword::Endproperty
        };
        let body = self.skip_to_kw(end);
        self.eat_end_label();
        Ok(PropertyDecl {
            is_sequence,
            name,
            body,
        })
    }

    /// `table rows endtable`, one raw row per `;`; cursor on `table`.
    pub(super) fn parse_table(&mut self) -> PResult<ItemKind> {
        self.expect_kw(Keyword::Table)?;
        let mut rows: Vec<RawTokens> = Vec::new();
        while !self.at_kw(Keyword::Endtable) && !self.at_eof() {
            let from = self.pos;
            while !self.at_punct(Punct::Semi) && !self.at_kw(Keyword::Endtable) && !self.at_eof() {
                self.bump();
            }
            rows.push(self.raw_tokens(from, self.pos));
            self.eat_punct(Punct::Semi);
        }
        self.expect_kw(Keyword::Endtable)?;
        Ok(ItemKind::Table(rows))
    }
}
