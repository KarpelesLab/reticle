//! Declarative parts, declarations and interface lists.

use super::{PResult, Parser, Recover, is_declaration_keyword};
use crate::vhdl::ast::*;
use crate::vhdl::token::TokenKind;

impl<'t, 'src> Parser<'t, 'src> {
    /// Parses declarations up to `begin`, `end` or end of input, recovering
    /// after each failed one.
    pub(super) fn parse_declarative_part(&mut self) -> Vec<Declaration> {
        let mut decls = Vec::new();
        loop {
            if self.at(TokenKind::End) && self.consume_stray_end() {
                continue;
            }
            if self.at_any(&[TokenKind::Begin, TokenKind::End, TokenKind::Eof]) {
                break;
            }
            let start = self.pos;
            match self.parse_declaration() {
                Ok(d) => decls.push(d),
                Err(Recover) => self.recover(start, |k| {
                    is_declaration_keyword(k) || matches!(k, TokenKind::Begin | TokenKind::End)
                }),
            }
        }
        decls
    }

    /// True at a token that begins a declaration.
    pub(super) fn at_declaration_start(&self) -> bool {
        is_declaration_keyword(self.kind())
    }

    /// One declarative item.
    pub(super) fn parse_declaration(&mut self) -> PResult<Declaration> {
        Ok(match self.kind() {
            TokenKind::Constant | TokenKind::Signal | TokenKind::Variable | TokenKind::Shared => {
                Declaration::Object(self.parse_object_decl()?)
            }
            TokenKind::File => Declaration::File(self.parse_file_decl()?),
            TokenKind::Type => Declaration::Type(self.parse_type_decl()?),
            TokenKind::Subtype => Declaration::Subtype(self.parse_subtype_decl()?),
            TokenKind::Alias => Declaration::Alias(self.parse_alias_decl()?),
            TokenKind::Attribute => self.parse_attribute_decl_or_spec()?,
            TokenKind::Component => Declaration::Component(self.parse_component_decl()?),
            TokenKind::Function | TokenKind::Procedure | TokenKind::Pure | TokenKind::Impure => {
                self.parse_subprogram()?
            }
            TokenKind::Use => Declaration::Use(self.parse_use_clause()?),
            TokenKind::For => Declaration::ConfigurationSpec(self.parse_configuration_spec()?),
            TokenKind::Group => self.parse_group()?,
            TokenKind::Disconnect => Declaration::Disconnection(self.parse_disconnection()?),
            TokenKind::Package if self.kind_at(1) == TokenKind::Body => {
                Declaration::PackageBody(self.parse_package_body()?)
            }
            TokenKind::Package if self.kind_at(3) == TokenKind::New => {
                Declaration::PackageInstantiation(self.parse_package_instantiation()?)
            }
            TokenKind::Package => Declaration::Package(self.parse_package()?),
            _ => return Err(self.expected("a declaration")),
        })
    }

    /// `constant|signal|[shared] variable names : subtype [register|bus] [:= expr];`
    fn parse_object_decl(&mut self) -> PResult<ObjectDecl> {
        let start = self.span();
        let kind = match self.bump().kind {
            TokenKind::Constant => ObjectKind::Constant,
            TokenKind::Signal => ObjectKind::Signal,
            TokenKind::Variable => ObjectKind::Variable,
            _ => {
                self.expect(TokenKind::Variable)?;
                ObjectKind::SharedVariable
            }
        };
        let names = self.parse_ident_list()?;
        self.expect(TokenKind::Colon)?;
        let subtype = self.parse_subtype_indication()?;
        let signal_kind = match self.kind() {
            TokenKind::Register => {
                self.bump();
                Some(SignalKind::Register)
            }
            TokenKind::Bus => {
                self.bump();
                Some(SignalKind::Bus)
            }
            _ => None,
        };
        let init = if self.eat(TokenKind::ColonEq).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect_semi()?;
        Ok(ObjectDecl {
            kind,
            names,
            subtype,
            signal_kind,
            init,
            span: self.span_from(start),
        })
    }

    /// `file names : subtype [[open expr] is [mode] expr];`
    fn parse_file_decl(&mut self) -> PResult<FileDecl> {
        let start = self.span();
        self.expect(TokenKind::File)?;
        let names = self.parse_ident_list()?;
        self.expect(TokenKind::Colon)?;
        let subtype = self.parse_subtype_indication()?;
        let open_kind = if self.eat(TokenKind::Open).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        let mut mode87 = None;
        let logical_name = if self.eat(TokenKind::Is).is_some() {
            mode87 = match self.kind() {
                TokenKind::In => {
                    self.bump();
                    Some(Mode::In)
                }
                TokenKind::Out => {
                    self.bump();
                    Some(Mode::Out)
                }
                _ => None,
            };
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect_semi()?;
        Ok(FileDecl {
            names,
            subtype,
            open_kind,
            logical_name,
            mode87,
            span: self.span_from(start),
        })
    }

    /// `subtype name is subtype_indication;`
    fn parse_subtype_decl(&mut self) -> PResult<SubtypeDecl> {
        let start = self.span();
        self.expect(TokenKind::Subtype)?;
        let name = self.parse_ident()?;
        self.expect(TokenKind::Is)?;
        let subtype = self.parse_subtype_indication()?;
        self.expect_semi()?;
        Ok(SubtypeDecl {
            name,
            subtype,
            span: self.span_from(start),
        })
    }

    /// `alias designator [: subtype] is name [signature];`
    fn parse_alias_decl(&mut self) -> PResult<AliasDecl> {
        let start = self.span();
        self.expect(TokenKind::Alias)?;
        let designator = self.parse_designator()?;
        let subtype = if self.eat(TokenKind::Colon).is_some() {
            Some(self.parse_subtype_indication()?)
        } else {
            None
        };
        self.expect(TokenKind::Is)?;
        let target = self.parse_name()?;
        let signature = self.parse_optional_signature()?;
        self.expect_semi()?;
        Ok(AliasDecl {
            designator,
            subtype,
            target,
            signature,
            span: self.span_from(start),
        })
    }

    /// `attribute name : type_mark;` or `attribute name of ... : class is expr;`
    fn parse_attribute_decl_or_spec(&mut self) -> PResult<Declaration> {
        let start = self.span();
        self.expect(TokenKind::Attribute)?;
        let name = self.parse_ident()?;
        if self.eat(TokenKind::Colon).is_some() {
            let type_mark = self.parse_type_mark()?;
            self.expect_semi()?;
            return Ok(Declaration::Attribute(AttributeDecl {
                name,
                type_mark,
                span: self.span_from(start),
            }));
        }
        self.expect(TokenKind::Of)?;
        let entities = match self.kind() {
            TokenKind::Others => EntityNameList::Others(self.bump().span),
            TokenKind::All => EntityNameList::All(self.bump().span),
            _ => {
                let mut list = vec![self.parse_entity_designator()?];
                while self.eat(TokenKind::Comma).is_some() {
                    list.push(self.parse_entity_designator()?);
                }
                EntityNameList::Names(list)
            }
        };
        self.expect(TokenKind::Colon)?;
        let class = self.parse_entity_class()?;
        self.expect(TokenKind::Is)?;
        let value = self.parse_expr()?;
        self.expect_semi()?;
        Ok(Declaration::AttributeSpec(AttributeSpec {
            attribute: name,
            entities,
            class,
            value,
            span: self.span_from(start),
        }))
    }

    /// `designator [signature]`
    fn parse_entity_designator(&mut self) -> PResult<EntityDesignator> {
        let start = self.span();
        let designator = self.parse_designator()?;
        let signature = self.parse_optional_signature()?;
        Ok(EntityDesignator {
            designator,
            signature,
            span: self.span_from(start),
        })
    }

    /// One of the entity class reserved words.
    fn parse_entity_class(&mut self) -> PResult<EntityClass> {
        let class = match self.kind() {
            TokenKind::Entity => EntityClass::Entity,
            TokenKind::Architecture => EntityClass::Architecture,
            TokenKind::Configuration => EntityClass::Configuration,
            TokenKind::Procedure => EntityClass::Procedure,
            TokenKind::Function => EntityClass::Function,
            TokenKind::Package => EntityClass::Package,
            TokenKind::Type => EntityClass::Type,
            TokenKind::Subtype => EntityClass::Subtype,
            TokenKind::Constant => EntityClass::Constant,
            TokenKind::Signal => EntityClass::Signal,
            TokenKind::Variable => EntityClass::Variable,
            TokenKind::Component => EntityClass::Component,
            TokenKind::Label => EntityClass::Label,
            TokenKind::Literal => EntityClass::Literal,
            TokenKind::Units => EntityClass::Units,
            TokenKind::Group => EntityClass::Group,
            TokenKind::File => EntityClass::File,
            TokenKind::Property => EntityClass::Property,
            TokenKind::Sequence => EntityClass::Sequence,
            _ => return Err(self.expected("an entity class")),
        };
        self.bump();
        Ok(class)
    }

    /// `component name [is] [generic] [port] end component [name];`
    fn parse_component_decl(&mut self) -> PResult<ComponentDecl> {
        let start = self.span();
        self.expect(TokenKind::Component)?;
        let name = self.parse_ident()?;
        self.eat(TokenKind::Is);
        self.with_open(TokenKind::Component, |p| {
            let generics = p.parse_optional_generic_clause()?;
            let ports = p.parse_optional_port_clause()?;
            p.parse_end(&[TokenKind::Component], false, Some(&name))?;
            Ok(ComponentDecl {
                name,
                generics,
                ports,
                span: p.span_from(start),
            })
        })
    }

    /// A subprogram declaration, body or instantiation.
    fn parse_subprogram(&mut self) -> PResult<Declaration> {
        let start = self.span();
        if self.at_any(&[TokenKind::Function, TokenKind::Procedure])
            && matches!(
                self.kind_at(1),
                TokenKind::Ident | TokenKind::ExtendedIdent | TokenKind::StringLit
            )
            && self.kind_at(2) == TokenKind::Is
            && self.kind_at(3) == TokenKind::New
        {
            return self
                .parse_subprogram_instantiation()
                .map(Declaration::SubprogramInstantiation);
        }
        let spec = self.parse_subprogram_spec()?;
        if self.eat(TokenKind::Semi).is_some() {
            return Ok(Declaration::Subprogram(SubprogramDecl {
                spec,
                span: self.span_from(start),
            }));
        }
        self.expect(TokenKind::Is)?;
        let kw = match spec.kind {
            SubprogramKind::Function => TokenKind::Function,
            SubprogramKind::Procedure => TokenKind::Procedure,
        };
        self.with_open(kw, |p| {
            let decls = p.parse_declarative_part();
            p.expect_or_skip_to(TokenKind::Begin)?;
            let statements = p.parse_sequential_statements();
            p.parse_end_designator(&[kw], &spec.designator)?;
            Ok(Declaration::SubprogramBody(SubprogramBody {
                spec,
                decls,
                statements,
                span: p.span_from(start),
            }))
        })
    }

    /// `[pure|impure] function|procedure designator [generic] [generic map]
    /// [[parameter] (params)] [return type_mark]`
    pub(super) fn parse_subprogram_spec(&mut self) -> PResult<SubprogramSpec> {
        let start = self.span();
        let pure = match self.kind() {
            TokenKind::Pure => {
                self.bump();
                Some(true)
            }
            TokenKind::Impure => {
                self.bump();
                Some(false)
            }
            _ => None,
        };
        let kind = match self.kind() {
            TokenKind::Function => SubprogramKind::Function,
            TokenKind::Procedure => SubprogramKind::Procedure,
            _ => return Err(self.expected("`function` or `procedure`")),
        };
        self.bump();
        let designator = self.parse_designator()?;
        let generics = self.parse_optional_generic_clause_bare()?;
        let generic_map = if !generics.is_empty() && self.at(TokenKind::Generic) {
            Some(self.parse_generic_map_aspect()?)
        } else {
            None
        };
        if let Some(t) = self.eat(TokenKind::Parameter) {
            self.require_2008(t.span, "the `parameter` keyword");
        }
        let params = if self.at(TokenKind::LParen) {
            self.parse_interface_list()?
        } else {
            Vec::new()
        };
        let return_type = if kind == SubprogramKind::Function {
            self.expect(TokenKind::Return)?;
            Some(self.parse_type_mark()?)
        } else if self.at(TokenKind::Return) {
            self.error_here("a procedure cannot return a value");
            self.bump();
            let _ = self.parse_type_mark()?;
            None
        } else {
            None
        };
        Ok(SubprogramSpec {
            kind,
            pure,
            designator,
            generics,
            generic_map,
            params,
            return_type,
            span: self.span_from(start),
        })
    }

    /// `function|procedure designator is new name [signature] [generic map];`
    fn parse_subprogram_instantiation(&mut self) -> PResult<SubprogramInstantiation> {
        let start = self.span();
        let kind = if self.eat(TokenKind::Function).is_some() {
            SubprogramKind::Function
        } else {
            self.expect(TokenKind::Procedure)?;
            SubprogramKind::Procedure
        };
        let designator = self.parse_designator()?;
        self.expect(TokenKind::Is)?;
        self.expect(TokenKind::New)?;
        self.require_2008(start, "subprogram instantiation");
        let uninstantiated = self.parse_name()?;
        let signature = self.parse_optional_signature()?;
        let generic_map = if self.at(TokenKind::Generic) {
            self.parse_generic_map_aspect()?
        } else {
            Vec::new()
        };
        self.expect_semi()?;
        Ok(SubprogramInstantiation {
            kind,
            designator,
            uninstantiated,
            signature,
            generic_map,
            span: self.span_from(start),
        })
    }

    /// `group name is (class [<>], ...);` or `group name : template (...);`
    fn parse_group(&mut self) -> PResult<Declaration> {
        let start = self.span();
        self.expect(TokenKind::Group)?;
        let name = self.parse_ident()?;
        if self.eat(TokenKind::Is).is_some() {
            let entries = self.parse_paren_list(|p| {
                let s = p.span();
                let class = p.parse_entity_class()?;
                let unbounded = p.eat(TokenKind::Box).is_some();
                Ok(GroupTemplateEntry {
                    class,
                    unbounded,
                    span: p.span_from(s),
                })
            })?;
            self.expect_semi()?;
            return Ok(Declaration::GroupTemplate(GroupTemplateDecl {
                name,
                entries,
                span: self.span_from(start),
            }));
        }
        self.expect(TokenKind::Colon)?;
        let template = self.parse_name_no_call()?;
        let constituents = self.parse_paren_list(|p| p.parse_expr())?;
        self.expect_semi()?;
        Ok(Declaration::Group(GroupDecl {
            name,
            template,
            constituents,
            span: self.span_from(start),
        }))
    }

    /// `disconnect signals : type_mark after expr;`
    fn parse_disconnection(&mut self) -> PResult<DisconnectionSpec> {
        let start = self.span();
        self.expect(TokenKind::Disconnect)?;
        let signals = match self.kind() {
            TokenKind::Others => SignalList::Others(self.bump().span),
            TokenKind::All => SignalList::All(self.bump().span),
            _ => {
                let mut names = vec![self.parse_name()?];
                while self.eat(TokenKind::Comma).is_some() {
                    names.push(self.parse_name()?);
                }
                SignalList::Names(names)
            }
        };
        self.expect(TokenKind::Colon)?;
        let type_mark = self.parse_type_mark()?;
        self.expect(TokenKind::After)?;
        let time = self.parse_expr()?;
        self.expect_semi()?;
        Ok(DisconnectionSpec {
            signals,
            type_mark,
            time,
            span: self.span_from(start),
        })
    }

    /// `for component_spec binding_indication; [end for;]`
    fn parse_configuration_spec(&mut self) -> PResult<ConfigurationSpec> {
        let start = self.span();
        self.expect(TokenKind::For)?;
        let spec = self.parse_component_specification()?;
        let binding = self.parse_binding_indication()?;
        self.expect_semi()?;
        if self.at(TokenKind::End) && self.kind_at(1) == TokenKind::For {
            self.bump();
            self.bump();
            self.expect_semi()?;
        }
        Ok(ConfigurationSpec {
            spec,
            binding,
            span: self.span_from(start),
        })
    }

    // --- interface lists ----------------------------------------------------

    /// `[generic (interface_list);]`
    pub(super) fn parse_optional_generic_clause(&mut self) -> PResult<Vec<InterfaceDecl>> {
        let list = self.parse_optional_generic_clause_bare()?;
        if !list.is_empty() {
            self.expect_semi()?;
        }
        Ok(list)
    }

    /// `[generic (interface_list)]` without the trailing `;` (subprogram
    /// headers).
    fn parse_optional_generic_clause_bare(&mut self) -> PResult<Vec<InterfaceDecl>> {
        if self.at(TokenKind::Generic) && self.kind_at(1) == TokenKind::LParen {
            self.bump();
            self.parse_interface_list()
        } else {
            Ok(Vec::new())
        }
    }

    /// `[port (interface_list);]`
    pub(super) fn parse_optional_port_clause(&mut self) -> PResult<Vec<InterfaceDecl>> {
        if self.at(TokenKind::Port) && self.kind_at(1) == TokenKind::LParen {
            self.bump();
            let list = self.parse_interface_list()?;
            self.expect_semi()?;
            Ok(list)
        } else {
            Ok(Vec::new())
        }
    }

    /// `( interface_element {; interface_element} )`
    pub(super) fn parse_interface_list(&mut self) -> PResult<Vec<InterfaceDecl>> {
        self.expect(TokenKind::LParen)?;
        let mut items = Vec::new();
        loop {
            let start = self.pos;
            match self.parse_interface_element() {
                Ok(i) => items.push(i),
                Err(Recover) => {
                    // Resynchronise inside the list: to the next `;` or the
                    // closing `)`.
                    loop {
                        match self.kind() {
                            TokenKind::Semi | TokenKind::RParen | TokenKind::Eof => break,
                            TokenKind::LParen => {
                                if let Some(end) = self.matching_paren(self.pos) {
                                    self.pos = end + 1;
                                } else {
                                    self.bump();
                                }
                            }
                            _ => {
                                self.bump();
                            }
                        }
                    }
                    if self.pos == start {
                        return Err(Recover);
                    }
                }
            }
            if self.eat(TokenKind::Semi).is_none() {
                break;
            }
            if self.at(TokenKind::RParen) {
                self.error_here("trailing `;` before `)`");
                break;
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(items)
    }

    /// One element of an interface list.
    fn parse_interface_element(&mut self) -> PResult<InterfaceDecl> {
        let start = self.span();
        match self.kind() {
            TokenKind::Type => {
                self.bump();
                let name = self.parse_ident()?;
                self.require_2008(start, "generic types");
                Ok(InterfaceDecl::Type(InterfaceType {
                    name,
                    span: self.span_from(start),
                }))
            }
            TokenKind::Function | TokenKind::Procedure | TokenKind::Pure | TokenKind::Impure => {
                let spec = self.parse_subprogram_spec()?;
                self.require_2008(start, "generic subprograms");
                let default = if self.eat(TokenKind::Is).is_some() {
                    if let Some(t) = self.eat(TokenKind::Box) {
                        Some(SubprogramDefault::Box(t.span))
                    } else {
                        Some(SubprogramDefault::Name(self.parse_name()?))
                    }
                } else {
                    None
                };
                Ok(InterfaceDecl::Subprogram(InterfaceSubprogram {
                    spec,
                    default,
                    span: self.span_from(start),
                }))
            }
            TokenKind::Package => {
                self.bump();
                let name = self.parse_ident()?;
                self.expect(TokenKind::Is)?;
                self.expect(TokenKind::New)?;
                self.require_2008(start, "generic packages");
                let uninstantiated = self.parse_name()?;
                self.expect(TokenKind::Generic)?;
                self.expect(TokenKind::Map)?;
                let generic_map = match (self.kind(), self.kind_at(1), self.kind_at(2)) {
                    (TokenKind::LParen, TokenKind::Box, TokenKind::RParen) => {
                        let s = self.bump().span;
                        self.bump();
                        let e = self.bump().span;
                        InterfacePackageMap::Box(s.to(e))
                    }
                    (TokenKind::LParen, TokenKind::Default, TokenKind::RParen) => {
                        let s = self.bump().span;
                        self.bump();
                        let e = self.bump().span;
                        InterfacePackageMap::Default(s.to(e))
                    }
                    _ => InterfacePackageMap::Map(self.parse_association_list()?),
                };
                Ok(InterfaceDecl::Package(InterfacePackage {
                    name,
                    uninstantiated,
                    generic_map,
                    span: self.span_from(start),
                }))
            }
            _ => self
                .parse_interface_object()
                .map(|o| InterfaceDecl::Object(Box::new(o))),
        }
    }

    /// `[class] names : [mode] subtype [bus] [:= default]`
    fn parse_interface_object(&mut self) -> PResult<InterfaceObject> {
        let start = self.span();
        let class = match self.kind() {
            TokenKind::Constant => Some(ObjectClass::Constant),
            TokenKind::Signal => Some(ObjectClass::Signal),
            TokenKind::Variable => Some(ObjectClass::Variable),
            TokenKind::File => Some(ObjectClass::File),
            _ => None,
        };
        if class.is_some() {
            self.bump();
        }
        let names = self.parse_ident_list()?;
        self.expect(TokenKind::Colon)?;
        let mode = match self.kind() {
            TokenKind::In => Some(Mode::In),
            TokenKind::Out => Some(Mode::Out),
            TokenKind::Inout => Some(Mode::Inout),
            TokenKind::Buffer => Some(Mode::Buffer),
            TokenKind::Linkage => Some(Mode::Linkage),
            _ => None,
        };
        if mode.is_some() {
            self.bump();
        }
        let subtype = self.parse_subtype_indication()?;
        let bus = self.eat(TokenKind::Bus).is_some();
        let default = if self.eat(TokenKind::ColonEq).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(InterfaceObject {
            class,
            names,
            mode,
            subtype,
            bus,
            default,
            span: self.span_from(start),
        })
    }

    /// `generic map (association_list)`
    pub(super) fn parse_generic_map_aspect(&mut self) -> PResult<Vec<AssociationElement>> {
        self.expect(TokenKind::Generic)?;
        self.expect(TokenKind::Map)?;
        self.parse_association_list()
    }

    /// `port map (association_list)`
    pub(super) fn parse_port_map_aspect(&mut self) -> PResult<Vec<AssociationElement>> {
        self.expect(TokenKind::Port)?;
        self.expect(TokenKind::Map)?;
        self.parse_association_list()
    }
}
