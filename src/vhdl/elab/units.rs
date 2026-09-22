//! The design-unit index: what may be elaborated, and what binds to what.
//!
//! Elaboration starts from an [`Analysis`], which holds every analysed
//! design unit in dependency order. This module turns that flat list into
//! the three questions elaboration asks:
//!
//! 1. **Which entities are there, and which architecture does each use?**
//!    [`Index::architecture_of`] returns the *last analysed* architecture of
//!    an entity, which is what IEEE 1076-2008 clause 13.3 prescribes for a
//!    default binding, unless a configuration names another one.
//! 2. **Which entity is the top?** [`Index::roots`] lists the entities that
//!    no other design unit instantiates, in analysis order; the first is
//!    used when `--top` is not given.
//! 3. **What does a component instantiation bind to?** [`Index::entity`]
//!    looks an entity up by library and name, which is what the default
//!    binding rule of clause 7.3.3 needs: a component with no binding
//!    indication binds to the entity of the same name in the working
//!    library.
//!
//! Nothing here touches the tree beyond the instantiation statements it has
//! to scan to answer question 2.

use std::collections::HashSet;

use crate::intern::Symbol;
use crate::vhdl::ast;
use crate::vhdl::sema::{Analysis, DeclKind, LibraryUnitKind, UnitId};

/// One entity of the design, with the architectures compiled for it.
#[derive(Clone, Debug)]
pub(crate) struct EntityInfo {
    /// The entity's design unit.
    pub unit: UnitId,
    /// Its architectures, in analysis order; the last one is the default.
    pub architectures: Vec<UnitId>,
}

/// The index of everything elaboration may need to look up.
#[derive(Debug, Default)]
pub(crate) struct Index {
    /// Entities in analysis order.
    pub entities: Vec<EntityInfo>,
    /// Configuration declarations in analysis order.
    pub configurations: Vec<UnitId>,
    /// Entities that some design unit instantiates.
    instantiated: HashSet<UnitId>,
}

impl Index {
    /// Builds the index from an analysis.
    pub(crate) fn build(a: &Analysis) -> Index {
        let mut index = Index::default();
        for (i, u) in a.units.iter().enumerate() {
            if !u.analyzed {
                continue;
            }
            match u.kind {
                LibraryUnitKind::Entity => index.entities.push(EntityInfo {
                    unit: UnitId::from_index(i),
                    architectures: Vec::new(),
                }),
                LibraryUnitKind::Configuration => {
                    index.configurations.push(UnitId::from_index(i));
                }
                _ => {}
            }
        }
        for (i, u) in a.units.iter().enumerate() {
            if u.kind != LibraryUnitKind::Architecture || !u.analyzed {
                continue;
            }
            let Some(primary) = u.primary else { continue };
            if let Some(e) = index.entities.iter_mut().find(|e| {
                a.units[e.unit.index()].name == primary
                    && a.units[e.unit.index()].library == u.library
            }) {
                e.architectures.push(UnitId::from_index(i));
            }
        }
        index.scan_instantiations(a);
        index
    }

    /// The entity named `name` in `library`.
    pub(crate) fn entity(&self, a: &Analysis, library: Symbol, name: Symbol) -> Option<UnitId> {
        self.entities
            .iter()
            .map(|e| e.unit)
            .find(|&u| a.units[u.index()].library == library && a.units[u.index()].name == name)
    }

    /// The entity's default (last analysed) architecture.
    pub(crate) fn architecture_of(&self, entity: UnitId) -> Option<UnitId> {
        self.entities
            .iter()
            .find(|e| e.unit == entity)
            .and_then(|e| e.architectures.last().copied())
    }

    /// The entity's architecture called `name`, if it has one.
    pub(crate) fn architecture_named(
        &self,
        a: &Analysis,
        entity: UnitId,
        name: Symbol,
    ) -> Option<UnitId> {
        self.entities
            .iter()
            .find(|e| e.unit == entity)?
            .architectures
            .iter()
            .copied()
            .find(|&u| a.units[u.index()].name == name)
    }

    /// The entities no design unit instantiates, in analysis order.
    pub(crate) fn roots(&self) -> Vec<UnitId> {
        self.entities
            .iter()
            .map(|e| e.unit)
            .filter(|u| !self.instantiated.contains(u))
            .collect()
    }

    /// Records every entity an instantiation statement reaches, so
    /// [`Index::roots`] can subtract them.
    fn scan_instantiations(&mut self, a: &Analysis) {
        let units: Vec<UnitId> = (0..a.units.len()).map(UnitId::from_index).collect();
        for u in units {
            let unit = &a.units[u.index()];
            if !unit.analyzed {
                continue;
            }
            let Some(file) = a.files.get(unit.file) else {
                continue;
            };
            let Some(du) = file.ast.units.get(unit.index) else {
                continue;
            };
            let library = unit.library;
            match &du.unit {
                ast::LibraryUnit::Architecture(arch) => {
                    self.scan_statements(a, library, &arch.statements);
                }
                ast::LibraryUnit::Entity(e) => {
                    self.scan_statements(a, library, &e.statements);
                }
                ast::LibraryUnit::Configuration(c) => {
                    // `configuration c of e` uses `e` as its top, but a
                    // configuration that nothing else names does not make
                    // `e` a non-root; only component bindings inside it do.
                    self.scan_block_configuration(a, library, &c.block);
                }
                _ => {}
            }
        }
    }

    fn scan_block_configuration(
        &mut self,
        a: &Analysis,
        library: Symbol,
        block: &ast::BlockConfiguration,
    ) {
        for item in &block.items {
            match item {
                ast::ConfigurationItem::Block(b) => {
                    self.scan_block_configuration(a, library, b);
                }
                ast::ConfigurationItem::Component(c) => {
                    if let Some(b) = &c.binding
                        && let Some(ast::EntityAspect::Entity { name, .. }) = &b.entity_aspect
                    {
                        self.note_entity_name(a, library, name);
                    }
                    if let Some(b) = &c.block {
                        self.scan_block_configuration(a, library, b);
                    }
                }
            }
        }
    }

    fn scan_statements(
        &mut self,
        a: &Analysis,
        library: Symbol,
        stmts: &[ast::ConcurrentStatement],
    ) {
        for s in stmts {
            match &s.kind {
                ast::ConcurrentKind::Instantiation(i) => match &i.unit {
                    ast::InstantiatedUnit::Component(n)
                    | ast::InstantiatedUnit::Entity { name: n, .. } => {
                        self.note_entity_name(a, library, n);
                    }
                    ast::InstantiatedUnit::Configuration(_) => {}
                },
                ast::ConcurrentKind::Block(b) => self.scan_statements(a, library, &b.statements),
                ast::ConcurrentKind::ForGenerate(g) => {
                    self.scan_statements(a, library, &g.body.statements);
                }
                ast::ConcurrentKind::IfGenerate(g) => {
                    for arm in &g.arms {
                        self.scan_statements(a, library, &arm.body.statements);
                    }
                    if let Some(e) = &g.else_arm {
                        self.scan_statements(a, library, &e.statements);
                    }
                }
                ast::ConcurrentKind::CaseGenerate(g) => {
                    for arm in &g.arms {
                        self.scan_statements(a, library, &arm.body.statements);
                    }
                }
                _ => {}
            }
        }
    }

    /// Marks the entity a component or entity name denotes as instantiated.
    ///
    /// The name may resolve to the entity directly (a selected name or a
    /// direct entity instantiation), or to a component declaration, in
    /// which case the default binding rule looks for an entity of the same
    /// name in the same library.
    fn note_entity_name(&mut self, a: &Analysis, library: Symbol, name: &ast::Name) {
        if let Some(d) = a.decl_of(name.span())
            && let DeclKind::Unit { unit, .. } = a.decl(d).kind
        {
            self.instantiated.insert(unit);
            return;
        }
        let Some((lib, entity)) = super::conc::split_entity_name(name) else {
            return;
        };
        let library = match a.interner.get_ci(&lib) {
            Some(l) if !lib.eq_ignore_ascii_case("work") => l,
            _ => library,
        };
        if let Some(sym) = a.interner.get_ci(&entity)
            && let Some(u) = self.entity(a, library, sym)
        {
            self.instantiated.insert(u);
        }
    }
}
