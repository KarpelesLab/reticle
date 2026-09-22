//! Design files, context clauses and library units.

use super::{PResult, Parser, Recover, is_unit_keyword};
use crate::vhdl::ast::*;
use crate::vhdl::token::TokenKind;

impl<'t, 'src> Parser<'t, 'src> {
    /// Parses every design unit up to end of input.
    pub(super) fn design_file(&mut self) -> DesignFile {
        let mut units = Vec::new();
        while !self.at_eof() {
            let start = self.pos;
            match self.parse_design_unit() {
                Ok(u) => units.push(u),
                Err(Recover) => self.recover_design_unit(start),
            }
        }
        DesignFile { units }
    }

    /// Skips to the next plausible start of a design unit.
    fn recover_design_unit(&mut self, start: usize) {
        loop {
            if self.at_eof() {
                return;
            }
            if self.pos > start && self.at_unit_start() {
                return;
            }
            self.bump();
        }
    }

    /// True when the tokens at the cursor look like the start of a design
    /// unit or context clause, rather than a keyword used inside one.
    fn at_unit_start(&self) -> bool {
        let k = self.kind();
        if !is_unit_keyword(k) {
            return false;
        }
        let next = self.kind_at(1);
        let ident = matches!(next, TokenKind::Ident | TokenKind::ExtendedIdent);
        match k {
            TokenKind::Library | TokenKind::Use => ident,
            TokenKind::Entity | TokenKind::Context => ident && self.kind_at(2) == TokenKind::Is,
            TokenKind::Architecture | TokenKind::Configuration => {
                ident && self.kind_at(2) == TokenKind::Of
            }
            TokenKind::Package => {
                next == TokenKind::Body || (ident && self.kind_at(2) == TokenKind::Is)
            }
            _ => false,
        }
    }

    /// `{context_item} library_unit`
    fn parse_design_unit(&mut self) -> PResult<DesignUnit> {
        let start = self.span();
        let mut context = Vec::new();
        loop {
            match self.kind() {
                TokenKind::Library => {
                    context.push(ContextItem::Library(self.parse_library_clause()?))
                }
                TokenKind::Use => context.push(ContextItem::Use(self.parse_use_clause()?)),
                TokenKind::Context if self.kind_at(2) != TokenKind::Is => {
                    context.push(ContextItem::Context(self.parse_context_reference()?));
                }
                _ => break,
            }
        }
        let unit = match self.kind() {
            TokenKind::Entity => LibraryUnit::Entity(self.parse_entity()?),
            TokenKind::Architecture => LibraryUnit::Architecture(self.parse_architecture()?),
            TokenKind::Package if self.kind_at(1) == TokenKind::Body => {
                LibraryUnit::PackageBody(self.parse_package_body()?)
            }
            TokenKind::Package if self.kind_at(3) == TokenKind::New => {
                LibraryUnit::PackageInstantiation(self.parse_package_instantiation()?)
            }
            TokenKind::Package => LibraryUnit::Package(self.parse_package()?),
            TokenKind::Configuration => LibraryUnit::Configuration(self.parse_configuration()?),
            TokenKind::Context => LibraryUnit::Context(self.parse_context_decl()?),
            _ => return Err(self.expected(
                "a design unit (`entity`, `architecture`, `package`, `configuration` or `context`)",
            )),
        };
        Ok(DesignUnit {
            context,
            unit,
            span: self.span_from(start),
        })
    }

    /// `library ident {, ident};`
    pub(super) fn parse_library_clause(&mut self) -> PResult<LibraryClause> {
        let start = self.span();
        self.expect(TokenKind::Library)?;
        let names = self.parse_ident_list()?;
        self.expect_semi()?;
        Ok(LibraryClause {
            names,
            span: self.span_from(start),
        })
    }

    /// `use name {, name};`
    pub(super) fn parse_use_clause(&mut self) -> PResult<UseClause> {
        let start = self.span();
        self.expect(TokenKind::Use)?;
        let mut names = vec![self.parse_name()?];
        while self.eat(TokenKind::Comma).is_some() {
            names.push(self.parse_name()?);
        }
        self.expect_semi()?;
        Ok(UseClause {
            names,
            span: self.span_from(start),
        })
    }

    /// `context name {, name};`
    fn parse_context_reference(&mut self) -> PResult<ContextReference> {
        let start = self.span();
        self.expect(TokenKind::Context)?;
        let mut names = vec![self.parse_name()?];
        while self.eat(TokenKind::Comma).is_some() {
            names.push(self.parse_name()?);
        }
        self.expect_semi()?;
        Ok(ContextReference {
            names,
            span: self.span_from(start),
        })
    }

    /// `entity name is [generic] [port] decls [begin stmts] end;`
    fn parse_entity(&mut self) -> PResult<EntityDecl> {
        let start = self.span();
        self.expect(TokenKind::Entity)?;
        let name = self.parse_ident()?;
        self.expect_soft(TokenKind::Is);
        self.with_open(TokenKind::Entity, |p| {
            let generics = p.parse_optional_generic_clause()?;
            let ports = p.parse_optional_port_clause()?;
            let decls = p.parse_declarative_part();
            let statements = if p.eat(TokenKind::Begin).is_some() {
                p.parse_concurrent_statements()
            } else {
                Vec::new()
            };
            p.parse_end(&[TokenKind::Entity], true, Some(&name))?;
            Ok(EntityDecl {
                name,
                generics,
                ports,
                decls,
                statements,
                span: p.span_from(start),
            })
        })
    }

    /// `architecture name of entity is decls begin stmts end;`
    fn parse_architecture(&mut self) -> PResult<ArchitectureBody> {
        let start = self.span();
        self.expect(TokenKind::Architecture)?;
        let name = self.parse_ident()?;
        self.expect(TokenKind::Of)?;
        let entity = self.parse_name()?;
        self.expect_soft(TokenKind::Is);
        self.with_open(TokenKind::Architecture, |p| {
            let decls = p.parse_declarative_part();
            p.expect_or_skip_to(TokenKind::Begin)?;
            let statements = p.parse_concurrent_statements();
            p.parse_end(&[TokenKind::Architecture], true, Some(&name))?;
            Ok(ArchitectureBody {
                name,
                entity,
                decls,
                statements,
                span: p.span_from(start),
            })
        })
    }

    /// `package name is [generic (...); [generic map (...);]] decls end;`
    pub(super) fn parse_package(&mut self) -> PResult<PackageDecl> {
        let start = self.span();
        self.expect(TokenKind::Package)?;
        let name = self.parse_ident()?;
        self.expect_soft(TokenKind::Is);
        self.with_open(TokenKind::Package, |p| {
            let generics = p.parse_optional_generic_clause()?;
            let generic_map = if !generics.is_empty() && p.at(TokenKind::Generic) {
                let m = p.parse_generic_map_aspect()?;
                p.expect_semi()?;
                Some(m)
            } else {
                None
            };
            let decls = p.parse_declarative_part();
            p.parse_end(&[TokenKind::Package], true, Some(&name))?;
            Ok(PackageDecl {
                name,
                generics,
                generic_map,
                decls,
                span: p.span_from(start),
            })
        })
    }

    /// `package body name is decls end;`
    pub(super) fn parse_package_body(&mut self) -> PResult<PackageBody> {
        let start = self.span();
        self.expect(TokenKind::Package)?;
        self.expect(TokenKind::Body)?;
        let name = self.parse_ident()?;
        self.expect_soft(TokenKind::Is);
        self.with_open(TokenKind::Package, |p| {
            let decls = p.parse_declarative_part();
            p.parse_end(&[TokenKind::Package, TokenKind::Body], true, Some(&name))?;
            Ok(PackageBody {
                name,
                decls,
                span: p.span_from(start),
            })
        })
    }

    /// `package name is new uninstantiated [generic map (...)];`
    pub(super) fn parse_package_instantiation(&mut self) -> PResult<PackageInstantiation> {
        let start = self.span();
        self.expect(TokenKind::Package)?;
        let name = self.parse_ident()?;
        self.expect(TokenKind::Is)?;
        self.expect(TokenKind::New)?;
        self.require_2008(start, "package instantiation");
        let uninstantiated = self.parse_name()?;
        let generic_map = if self.at(TokenKind::Generic) {
            self.parse_generic_map_aspect()?
        } else {
            Vec::new()
        };
        self.expect_semi()?;
        Ok(PackageInstantiation {
            name,
            uninstantiated,
            generic_map,
            span: self.span_from(start),
        })
    }

    /// `context name is items end;`
    fn parse_context_decl(&mut self) -> PResult<ContextDecl> {
        let start = self.span();
        self.expect(TokenKind::Context)?;
        let name = self.parse_ident()?;
        self.expect_soft(TokenKind::Is);
        self.with_open(TokenKind::Context, |p| {
            let mut items = Vec::new();
            loop {
                let item_start = p.pos;
                let r = match p.kind() {
                    TokenKind::Library => p.parse_library_clause().map(ContextItem::Library),
                    TokenKind::Use => p.parse_use_clause().map(ContextItem::Use),
                    TokenKind::Context => p.parse_context_reference().map(ContextItem::Context),
                    _ => break,
                };
                match r {
                    Ok(i) => items.push(i),
                    Err(Recover) => p.recover(item_start, |k| {
                        matches!(
                            k,
                            TokenKind::Library
                                | TokenKind::Use
                                | TokenKind::Context
                                | TokenKind::End
                        )
                    }),
                }
            }
            p.parse_end(&[TokenKind::Context], true, Some(&name))?;
            Ok(ContextDecl {
                name,
                items,
                span: p.span_from(start),
            })
        })
    }

    /// `configuration name of entity is decls block_configuration end;`
    fn parse_configuration(&mut self) -> PResult<ConfigurationDecl> {
        let start = self.span();
        self.expect(TokenKind::Configuration)?;
        let name = self.parse_ident()?;
        self.expect(TokenKind::Of)?;
        let entity = self.parse_name()?;
        self.expect_soft(TokenKind::Is);
        self.with_open(TokenKind::Configuration, |p| {
            let mut decls = Vec::new();
            while p.at_any(&[TokenKind::Use, TokenKind::Attribute, TokenKind::Group]) {
                let item_start = p.pos;
                match p.parse_declaration() {
                    Ok(d) => decls.push(d),
                    Err(Recover) => p.recover(item_start, |k| {
                        matches!(
                            k,
                            TokenKind::Use
                                | TokenKind::Attribute
                                | TokenKind::Group
                                | TokenKind::For
                                | TokenKind::End
                        )
                    }),
                }
            }
            let block = p.parse_block_configuration()?;
            p.parse_end(&[TokenKind::Configuration], true, Some(&name))?;
            Ok(ConfigurationDecl {
                name,
                entity,
                decls,
                block,
                span: p.span_from(start),
            })
        })
    }

    /// `for block_spec {use} {item} end for;`
    fn parse_block_configuration(&mut self) -> PResult<BlockConfiguration> {
        let start = self.span();
        self.expect(TokenKind::For)?;
        let spec = self.parse_name()?;
        self.with_open(TokenKind::For, |p| {
            let mut uses = Vec::new();
            while p.at(TokenKind::Use) {
                uses.push(p.parse_use_clause()?);
            }
            let mut items = Vec::new();
            while p.at(TokenKind::For) {
                let item_start = p.pos;
                match p.parse_configuration_item() {
                    Ok(i) => items.push(i),
                    Err(Recover) => {
                        p.recover(item_start, |k| matches!(k, TokenKind::For | TokenKind::End));
                    }
                }
            }
            p.parse_end(&[TokenKind::For], false, None)?;
            Ok(BlockConfiguration {
                spec,
                uses,
                items,
                span: p.span_from(start),
            })
        })
    }

    /// A block configuration or a component configuration, told apart by
    /// what follows `for`: an instantiation list ends with `:`.
    fn parse_configuration_item(&mut self) -> PResult<ConfigurationItem> {
        let is_component = match self.kind_at(1) {
            TokenKind::Others | TokenKind::All => true,
            TokenKind::Ident | TokenKind::ExtendedIdent => {
                matches!(self.kind_at(2), TokenKind::Colon | TokenKind::Comma)
            }
            _ => false,
        };
        if is_component {
            self.parse_component_configuration()
                .map(|c| ConfigurationItem::Component(Box::new(c)))
        } else {
            self.parse_block_configuration()
                .map(ConfigurationItem::Block)
        }
    }

    /// `for component_spec [binding;] [block_configuration] end for;`
    fn parse_component_configuration(&mut self) -> PResult<ComponentConfiguration> {
        let start = self.span();
        self.expect(TokenKind::For)?;
        let spec = self.parse_component_specification()?;
        self.with_open(TokenKind::For, |p| {
            let binding = if p.at_any(&[TokenKind::Use, TokenKind::Generic, TokenKind::Port]) {
                let b = p.parse_binding_indication()?;
                p.expect_semi()?;
                Some(b)
            } else {
                None
            };
            let block = if p.at(TokenKind::For) {
                Some(p.parse_block_configuration()?)
            } else {
                None
            };
            p.parse_end(&[TokenKind::For], false, None)?;
            Ok(ComponentConfiguration {
                spec,
                binding,
                block,
                span: p.span_from(start),
            })
        })
    }

    /// `instantiation_list : component_name`
    pub(super) fn parse_component_specification(&mut self) -> PResult<ComponentSpecification> {
        let start = self.span();
        let instances = match self.kind() {
            TokenKind::Others => InstantiationList::Others(self.bump().span),
            TokenKind::All => InstantiationList::All(self.bump().span),
            _ => InstantiationList::Labels(self.parse_ident_list()?),
        };
        self.expect(TokenKind::Colon)?;
        let component = self.parse_name()?;
        Ok(ComponentSpecification {
            instances,
            component,
            span: self.span_from(start),
        })
    }

    /// `[use entity_aspect] [generic map] [port map]`
    pub(super) fn parse_binding_indication(&mut self) -> PResult<BindingIndication> {
        let start = self.span();
        let entity_aspect = if self.eat(TokenKind::Use).is_some() {
            Some(self.parse_entity_aspect()?)
        } else {
            None
        };
        let generic_map = if self.at(TokenKind::Generic) {
            Some(self.parse_generic_map_aspect()?)
        } else {
            None
        };
        let port_map = if self.at(TokenKind::Port) {
            Some(self.parse_port_map_aspect()?)
        } else {
            None
        };
        Ok(BindingIndication {
            entity_aspect,
            generic_map,
            port_map,
            span: self.span_from(start),
        })
    }

    /// `entity name [(arch)] | configuration name | open`
    fn parse_entity_aspect(&mut self) -> PResult<EntityAspect> {
        let start = self.span();
        match self.kind() {
            TokenKind::Entity => {
                self.bump();
                let name = self.parse_name_no_call()?;
                let architecture = if self.eat(TokenKind::LParen).is_some() {
                    let a = self.parse_ident()?;
                    self.expect(TokenKind::RParen)?;
                    Some(a)
                } else {
                    None
                };
                Ok(EntityAspect::Entity {
                    name,
                    architecture,
                    span: self.span_from(start),
                })
            }
            TokenKind::Configuration => {
                self.bump();
                Ok(EntityAspect::Configuration(self.parse_name()?))
            }
            TokenKind::Open => Ok(EntityAspect::Open(self.bump().span)),
            _ => Err(self.expected("`entity`, `configuration` or `open`")),
        }
    }
}
