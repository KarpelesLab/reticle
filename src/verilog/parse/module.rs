//! Module-level constructs: the headers of modules, interfaces, programs
//! and primitives (parameter port lists, ANSI and non-ANSI port lists),
//! module instantiations, gate instantiations, `bind` and modports.

use super::super::ast::{
    Direction, GateDecl, GateInstance, GateKind, Instance, Instantiation, Modport, ModportItem,
    ModportItemKind, Module, ModuleKind, NamedConn, NonAnsiPort, ParamDecl, ParamKind,
    ParamOverride, Port, PortConn, PortConnKind, Ports,
};
use super::super::token::{Keyword, Punct, TokenKind};
use super::{PResult, ParseError, Parser};

/// The gate a keyword denotes.
fn gate_kind_of(kind: &TokenKind) -> Option<GateKind> {
    use Keyword as K;
    let TokenKind::Keyword(kw) = kind else {
        return None;
    };
    Some(match kw {
        K::And => GateKind::And,
        K::Nand => GateKind::Nand,
        K::Or => GateKind::Or,
        K::Nor => GateKind::Nor,
        K::Xor => GateKind::Xor,
        K::Xnor => GateKind::Xnor,
        K::Buf => GateKind::Buf,
        K::Not => GateKind::Not,
        K::Bufif0 => GateKind::Bufif0,
        K::Bufif1 => GateKind::Bufif1,
        K::Notif0 => GateKind::Notif0,
        K::Notif1 => GateKind::Notif1,
        K::Nmos => GateKind::Nmos,
        K::Pmos => GateKind::Pmos,
        K::Cmos => GateKind::Cmos,
        K::Rnmos => GateKind::Rnmos,
        K::Rpmos => GateKind::Rpmos,
        K::Rcmos => GateKind::Rcmos,
        K::Tran => GateKind::Tran,
        K::Rtran => GateKind::Rtran,
        K::Tranif0 => GateKind::Tranif0,
        K::Tranif1 => GateKind::Tranif1,
        K::Rtranif0 => GateKind::Rtranif0,
        K::Rtranif1 => GateKind::Rtranif1,
        K::Pullup => GateKind::Pullup,
        K::Pulldown => GateKind::Pulldown,
        _ => return None,
    })
}

impl Parser<'_> {
    /// `module name [imports] [#(params)] [(ports)]; items endmodule`, and
    /// likewise for the other design-unit keywords.
    pub(super) fn parse_module(&mut self) -> PResult<Module> {
        let (kind, end) = match self.kind() {
            TokenKind::Keyword(Keyword::Module) => (ModuleKind::Module, Keyword::Endmodule),
            TokenKind::Keyword(Keyword::Macromodule) => {
                (ModuleKind::Macromodule, Keyword::Endmodule)
            }
            TokenKind::Keyword(Keyword::Interface) => {
                (ModuleKind::Interface, Keyword::Endinterface)
            }
            TokenKind::Keyword(Keyword::Program) => (ModuleKind::Program, Keyword::Endprogram),
            TokenKind::Keyword(Keyword::Primitive) => {
                (ModuleKind::Primitive, Keyword::Endprimitive)
            }
            _ => return Err(self.expected("`module`")),
        };
        self.bump();
        let lifetime = self.parse_lifetime();
        let name = self.expect_ident()?;
        let mut imports = Vec::new();
        while self.at_kw(Keyword::Import) {
            self.bump();
            imports.extend(self.parse_package_refs()?);
            self.expect_semi()?;
        }
        // A broken header must not lose the whole module: skip to the
        // `;` and parse the body anyway.
        let mut header_ok = true;
        let mut params = None;
        if self.at_punct(Punct::Hash) && self.nth_is_punct(1, Punct::LParen) {
            match self.parse_param_port_list() {
                Ok(list) => params = Some(list),
                Err(ParseError) => header_ok = false,
            }
        }
        let mut ports = Ports::None;
        if header_ok && self.at_punct(Punct::LParen) {
            match self.parse_port_list() {
                Ok(list) => ports = list,
                Err(ParseError) => header_ok = false,
            }
        }
        if header_ok {
            if self.expect_semi().is_err() {
                self.recover_header();
            }
        } else {
            self.recover_header();
        }
        let end_family = [
            Keyword::Endmodule,
            Keyword::Endinterface,
            Keyword::Endprogram,
            Keyword::Endprimitive,
        ];
        let items = self.parse_items_until(&end_family);
        if !self.expect_end(end) && end_family.iter().any(|&k| self.at_kw(k)) {
            // The wrong end keyword still closes this unit.
            self.bump();
            self.eat_end_label();
        }
        Ok(Module {
            kind,
            lifetime,
            name,
            imports,
            params,
            ports,
            items,
        })
    }

    /// `#( [parameter|localparam] [type] name = value, ... )`.
    ///
    /// Entries without a keyword or type continue the previous
    /// declaration, as `list_of_param_assignments` does in the grammar.
    fn parse_param_port_list(&mut self) -> PResult<Vec<ParamDecl>> {
        self.expect_punct(Punct::Hash)?;
        self.expect_punct(Punct::LParen)?;
        let mut decls: Vec<ParamDecl> = Vec::new();
        if !self.at_punct(Punct::RParen) {
            loop {
                let has_keyword = self.at_kw(Keyword::Parameter) || self.at_kw(Keyword::Localparam);
                let has_type = self.at_kw(Keyword::Type)
                    || self.at_data_type_keyword()
                    || self.named_type_ahead()
                    || self.at_punct(Punct::LBracket)
                    || self.at_kw(Keyword::Signed)
                    || self.at_kw(Keyword::Unsigned);
                if has_keyword {
                    decls.push(self.parse_param_decl_head()?);
                } else if has_type || decls.is_empty() {
                    let kind = decls.last().map_or(ParamKind::Parameter, |d| d.kind);
                    let (is_type, data_type) = self.parse_param_type()?;
                    decls.push(ParamDecl {
                        kind,
                        is_type,
                        data_type,
                        decls: Vec::new(),
                    });
                }
                let name = self.expect_ident()?;
                let dims = self.parse_dims()?;
                let init = if self.eat_punct(Punct::Eq).is_some() {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                let span = self.span_from(name.span);
                decls
                    .last_mut()
                    .expect("a declaration was pushed above")
                    .decls
                    .push(super::super::ast::Declarator {
                        name,
                        dims,
                        init,
                        span,
                    });
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(decls)
    }

    /// The parenthesised port list, deciding between ANSI and non-ANSI
    /// forms from the first entry.
    fn parse_port_list(&mut self) -> PResult<Ports> {
        self.expect_punct(Punct::LParen)?;
        if self.eat_punct(Punct::RParen).is_some() {
            return Ok(Ports::Ansi(Vec::new()));
        }
        let ansi = self.first_port_is_ansi();
        let ports = if ansi {
            let mut ports = Vec::new();
            loop {
                ports.push(self.parse_ansi_port()?);
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
            Ports::Ansi(ports)
        } else {
            let mut ports = Vec::new();
            loop {
                ports.push(self.parse_non_ansi_port()?);
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
            Ports::NonAnsi(ports)
        };
        self.expect_punct(Punct::RParen)?;
        Ok(ports)
    }

    /// Lookahead past attributes: a direction, net type, `var`, type
    /// keyword, `interface`, or `name name` opens an ANSI port.
    fn first_port_is_ansi(&self) -> bool {
        let mut i = self.pos;
        while self.kind_at(i).is_punct(Punct::AttrOpen) {
            // `(*` is one token; find the `*)`.
            while !self.kind_at(i).is_punct(Punct::AttrClose) && i < self.limit {
                i += 1;
            }
            i += 1;
        }
        let kind = self.kind_at(i);
        let ansi_keyword = matches!(
            kind,
            TokenKind::Keyword(
                Keyword::Input
                    | Keyword::Output
                    | Keyword::Inout
                    | Keyword::Ref
                    | Keyword::Var
                    | Keyword::Interface
                    | Keyword::Signed
                    | Keyword::Unsigned
            )
        ) || Self::net_type_of(kind).is_some()
            || self.data_type_keyword_at(i);
        if ansi_keyword {
            return true;
        }
        if kind.is_ident() {
            // `name name`, `pkg::t name`, `if.mp name`, `t [..] name`.
            let mut j = i + 1;
            while self.kind_at(j).is_punct(Punct::ColonColon) && self.kind_at(j + 1).is_ident() {
                j += 2;
            }
            if self.kind_at(j).is_punct(Punct::Dot) && self.kind_at(j + 1).is_ident() {
                j += 2;
            }
            while self.kind_at(j).is_punct(Punct::LBracket) {
                j = self.after_balanced(j);
            }
            return self.kind_at(j).is_ident();
        }
        false
    }

    /// [`Parser::at_data_type_keyword`] at an absolute index.
    fn data_type_keyword_at(&self, i: usize) -> bool {
        use Keyword as K;
        matches!(
            self.kind_at(i),
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
                    | K::Enum
                    | K::Struct
                    | K::Union
                    | K::Type
            )
        )
    }

    /// `[attrs] [direction] [net_type|var] [type] name [dims] [= default]`.
    fn parse_ansi_port(&mut self) -> PResult<Port> {
        let start = self.span();
        let attrs = self.parse_attrs()?;
        let direction = self.parse_direction();
        let net_type = Self::net_type_of(self.kind());
        if net_type.is_some() {
            self.bump();
        }
        let var = self.eat_kw(Keyword::Var).is_some();
        let data_type = if self.at_kw(Keyword::Interface) {
            self.parse_data_type()?
        } else {
            self.parse_data_type_or_implicit()?
        };
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
            net_type,
            var,
            data_type,
            name,
            dims,
            default,
            span: self.span_from(start),
        })
    }

    /// `name`, `name[3:0]`, `{a, b}`, `.name(expr)`, `.name()` or empty.
    fn parse_non_ansi_port(&mut self) -> PResult<NonAnsiPort> {
        let start = self.span();
        if self.at_punct(Punct::Comma) || self.at_punct(Punct::RParen) {
            return Ok(NonAnsiPort {
                name: None,
                expr: None,
                span: crate::source::Span::new(start.file, start.start, start.start),
            });
        }
        if self.at_punct(Punct::Dot) {
            self.bump();
            let name = self.expect_ident()?;
            self.expect_punct(Punct::LParen)?;
            let expr = if self.at_punct(Punct::RParen) {
                None
            } else {
                Some(self.parse_expr()?)
            };
            self.expect_punct(Punct::RParen)?;
            return Ok(NonAnsiPort {
                name: Some(name),
                expr,
                span: self.span_from(start),
            });
        }
        let expr = self.parse_expr()?;
        Ok(NonAnsiPort {
            name: None,
            expr: Some(expr),
            span: self.span_from(start),
        })
    }

    /// At a name: true for `name #`, `name name (`, `name name [..] (`
    /// and `name (`.
    pub(super) fn looks_like_instantiation(&self) -> bool {
        if !self.at_ident() {
            return false;
        }
        match self.nth(1) {
            TokenKind::Punct(Punct::Hash) | TokenKind::Punct(Punct::LParen) => true,
            k if k.is_ident() => {
                let after = self.after_balanced_run(self.pos + 2);
                self.kind_at(after).is_punct(Punct::LParen)
            }
            _ => false,
        }
    }

    /// Index past any run of `[...]` groups starting at `i`.
    fn after_balanced_run(&self, mut i: usize) -> usize {
        while self.kind_at(i).is_punct(Punct::LBracket) {
            i = self.after_balanced(i);
        }
        i
    }

    /// `module [#(params)] inst [dims] (conns), ...;`, cursor on the
    /// module name.
    pub(super) fn parse_instantiation(&mut self) -> PResult<Instantiation> {
        let module = self.expect_ident()?;
        let params = if self.at_punct(Punct::Hash) {
            self.parse_param_overrides()?
        } else {
            Vec::new()
        };
        let mut instances = Vec::new();
        loop {
            let start = self.span();
            let name = self.eat_ident();
            let dims = self.parse_dims()?;
            let conns = self.parse_port_conns()?;
            instances.push(Instance {
                name,
                dims,
                conns,
                span: self.span_from(start),
            });
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        self.expect_semi()?;
        Ok(Instantiation {
            module,
            params,
            instances,
        })
    }

    /// `#(value, .name(value), .name())`, cursor on `#`.
    fn parse_param_overrides(&mut self) -> PResult<Vec<ParamOverride>> {
        self.expect_punct(Punct::Hash)?;
        self.expect_punct(Punct::LParen)?;
        let mut overrides = Vec::new();
        if !self.at_punct(Punct::RParen) {
            loop {
                let start = self.span();
                if self.at_punct(Punct::Dot) {
                    self.bump();
                    let name = self.expect_ident()?;
                    self.expect_punct(Punct::LParen)?;
                    let value = if self.at_punct(Punct::RParen) {
                        None
                    } else {
                        Some(self.parse_expr()?)
                    };
                    self.expect_punct(Punct::RParen)?;
                    overrides.push(ParamOverride {
                        name: Some(name),
                        value,
                        span: self.span_from(start),
                    });
                } else if self.at_punct(Punct::Comma) || self.at_punct(Punct::RParen) {
                    overrides.push(ParamOverride {
                        name: None,
                        value: None,
                        span: crate::source::Span::new(start.file, start.start, start.start),
                    });
                } else {
                    let value = self.parse_expr()?;
                    let span = value.span;
                    overrides.push(ParamOverride {
                        name: None,
                        value: Some(value),
                        span,
                    });
                }
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(overrides)
    }

    /// `(expr, .name(expr), .name, .name(), .*, )`, cursor on `(`.
    fn parse_port_conns(&mut self) -> PResult<Vec<PortConn>> {
        self.expect_punct(Punct::LParen)?;
        let mut conns = Vec::new();
        if self.eat_punct(Punct::RParen).is_some() {
            return Ok(conns);
        }
        loop {
            let start = self.span();
            let kind = if self.eat_punct(Punct::DotStar).is_some() {
                PortConnKind::Wildcard
            } else if self.at_punct(Punct::Dot) {
                self.bump();
                let name = self.expect_ident()?;
                let conn = if self.eat_punct(Punct::LParen).is_some() {
                    let conn = if self.at_punct(Punct::RParen) {
                        NamedConn::Open
                    } else {
                        NamedConn::Expr(self.parse_expr()?)
                    };
                    self.expect_punct(Punct::RParen)?;
                    conn
                } else {
                    NamedConn::Implicit
                };
                PortConnKind::Named { name, conn }
            } else if self.at_punct(Punct::Comma) || self.at_punct(Punct::RParen) {
                PortConnKind::Positional(None)
            } else {
                PortConnKind::Positional(Some(self.parse_expr()?))
            };
            let span = if matches!(kind, PortConnKind::Positional(None)) {
                crate::source::Span::new(start.file, start.start, start.start)
            } else {
                self.span_from(start)
            };
            conns.push(PortConn { kind, span });
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(conns)
    }

    /// True on a gate or switch keyword.
    pub(super) fn at_gate_keyword(&self) -> bool {
        gate_kind_of(self.kind()).is_some()
    }

    /// `and [(strength)] [#delay] [name [dims]] (terminals), ...;`.
    pub(super) fn parse_gate_decl(&mut self) -> PResult<GateDecl> {
        let Some(kind) = gate_kind_of(self.kind()) else {
            return Err(self.expected("gate"));
        };
        self.bump();
        let strength = self.parse_strength_opt()?;
        let delay = self.parse_delay_opt()?;
        let mut instances = Vec::new();
        loop {
            let start = self.span();
            let name = self.eat_ident();
            let dims = self.parse_dims()?;
            self.expect_punct(Punct::LParen)?;
            let conns = self.parse_expr_list(Punct::RParen)?;
            self.expect_punct(Punct::RParen)?;
            instances.push(GateInstance {
                name,
                dims,
                conns,
                span: self.span_from(start),
            });
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        self.expect_semi()?;
        Ok(GateDecl {
            kind,
            strength,
            delay,
            instances,
        })
    }

    /// `bind target [: inst, ...] module inst (...);`, cursor on `bind`.
    pub(super) fn parse_bind(&mut self) -> PResult<super::super::ast::Bind> {
        self.expect_kw(Keyword::Bind)?;
        let target = self.parse_lvalue()?;
        let mut instances = Vec::new();
        if self.eat_punct(Punct::Colon).is_some() {
            loop {
                instances.push(self.parse_lvalue()?);
                if self.eat_punct(Punct::Comma).is_none() {
                    break;
                }
            }
        }
        let inst = self.parse_instantiation()?;
        Ok(super::super::ast::Bind {
            target,
            instances,
            inst,
        })
    }

    /// `modport name (entries), name (entries);`, cursor on `modport`.
    pub(super) fn parse_modports(&mut self) -> PResult<Vec<Modport>> {
        self.expect_kw(Keyword::Modport)?;
        let mut modports = Vec::new();
        loop {
            let start = self.span();
            let name = self.expect_ident()?;
            self.expect_punct(Punct::LParen)?;
            let mut items = Vec::new();
            let mut direction: Option<Direction> = None;
            if !self.at_punct(Punct::RParen) {
                loop {
                    let start = self.span();
                    self.parse_attrs()?;
                    if let Some(dir) = self.parse_direction() {
                        direction = Some(dir);
                    }
                    let kind =
                        if self.eat_kw(Keyword::Import).is_some() || self.at_kw(Keyword::Export) {
                            let export = self.eat_kw(Keyword::Export).is_some();
                            let id = self.parse_modport_tf()?;
                            if export {
                                ModportItemKind::Export(id)
                            } else {
                                ModportItemKind::Import(id)
                            }
                        } else if self.eat_kw(Keyword::Clocking).is_some() {
                            ModportItemKind::Clocking(self.expect_ident()?)
                        } else {
                            let Some(direction) = direction else {
                                return Err(self.expected("port direction"));
                            };
                            if self.at_punct(Punct::Dot) {
                                self.bump();
                                let name = self.expect_ident()?;
                                self.expect_punct(Punct::LParen)?;
                                let expr = self.parse_expr()?;
                                self.expect_punct(Punct::RParen)?;
                                ModportItemKind::Port {
                                    direction,
                                    name,
                                    expr: Some(expr),
                                }
                            } else {
                                ModportItemKind::Port {
                                    direction,
                                    name: self.expect_ident()?,
                                    expr: None,
                                }
                            }
                        };
                    items.push(ModportItem {
                        kind,
                        span: self.span_from(start),
                    });
                    if self.eat_punct(Punct::Comma).is_none() {
                        break;
                    }
                }
            }
            self.expect_punct(Punct::RParen)?;
            modports.push(Modport {
                name,
                items,
                span: self.span_from(start),
            });
            if self.eat_punct(Punct::Comma).is_none() {
                break;
            }
        }
        self.expect_semi()?;
        Ok(modports)
    }

    /// The name of an imported or exported task/function in a modport:
    /// `name`, or a `task`/`function` prototype whose name is kept and
    /// whose argument list is skipped.
    fn parse_modport_tf(&mut self) -> PResult<super::super::ast::Ident> {
        if self.eat_kw(Keyword::Task).is_some() || self.eat_kw(Keyword::Function).is_some() {
            // Skip a return type: everything up to the name before `(`.
            while !self.at_eof() && !(self.at_ident() && self.nth_is_punct(1, Punct::LParen)) {
                if self.at_punct(Punct::Comma) || self.at_punct(Punct::RParen) {
                    return Err(self.expected("prototype"));
                }
                self.bump();
            }
            let name = self.expect_ident()?;
            self.skip_balanced();
            return Ok(name);
        }
        self.expect_ident()
    }
}
