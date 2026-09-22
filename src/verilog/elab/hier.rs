//! The declaration table and top-module selection.
//!
//! Elaboration starts from a set of parsed files with no notion of which
//! file declares what. [`Table::collect`] indexes every module, interface,
//! program, primitive and package by name, keeps the items that sit
//! outside any of them (the compilation-unit scope), and reports names
//! declared twice.
//!
//! [`Table::roots`] then answers "what is the top": every module that no
//! other module instantiates. Instantiations are counted through generate
//! regions, so a module used only inside `generate if (0)` still counts as
//! used, which is the conservative choice — it is better to elaborate one
//! module too few as a root than to elaborate a leaf twice.
//!
//! When the caller names a top with `ElabOptions::top`, that module is the
//! top and the others are still elaborated if they are roots, so a file of
//! independent modules (a library) yields a design holding all of them.
//! An instantiated name that the table does not know becomes a black box:
//! the instance keeps a [`ModuleRef::Unresolved`] and a warning says so.
//!
//! [`ModuleRef::Unresolved`]: crate::ir::ModuleRef

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{Diagnostic, Diagnostics};
use crate::source::Span;
use crate::verilog::ast::{self, Item, ItemKind, ModuleKind};
use crate::verilog::token::Dialect;

use super::codes;

/// A module, interface, program or primitive declaration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ModuleDecl<'ast> {
    /// The declaration.
    pub def: &'ast ast::Module,
    /// The item, for its attributes.
    pub item: &'ast Item,
    /// The dialect the file was parsed in.
    pub dialect: Dialect,
    /// The `` `timescale `` in effect, if one was seen in the file.
    pub timescale: Option<(u64, crate::ir::TimeUnit, u64, crate::ir::TimeUnit)>,
}

impl ModuleDecl<'_> {
    /// True for `interface`.
    pub(crate) fn is_interface(&self) -> bool {
        self.def.kind == ModuleKind::Interface
    }

    /// Where the declaration starts.
    pub(crate) fn span(&self) -> Span {
        self.def.name.span
    }
}

/// A package declaration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PackageDecl<'ast> {
    /// The declaration.
    pub def: &'ast ast::Package,
    /// The dialect the file was parsed in.
    pub dialect: Dialect,
}

/// Every declaration of one elaboration run.
#[derive(Debug, Default)]
pub(crate) struct Table<'ast> {
    /// Modules, interfaces, programs and primitives by name.
    pub modules: BTreeMap<String, ModuleDecl<'ast>>,
    /// Module names in source order.
    pub order: Vec<String>,
    /// Packages by name.
    pub packages: BTreeMap<String, PackageDecl<'ast>>,
    /// Items outside any module or package, in source order.
    pub unit_items: Vec<(&'ast Item, Dialect)>,
}

impl<'ast> Table<'ast> {
    /// Indexes `files`, reporting duplicate declarations into `diags`.
    pub(crate) fn collect(
        files: &[(&'ast ast::SourceFile, Dialect)],
        diags: &mut Diagnostics,
    ) -> Self {
        let mut table = Table::default();
        for (file, dialect) in files {
            let timescale = timescale_of(&file.items);
            for item in &file.items {
                match &item.kind {
                    ItemKind::Module(m) => {
                        let decl = ModuleDecl {
                            def: m,
                            item,
                            dialect: *dialect,
                            timescale,
                        };
                        if let Some(first) = table.modules.insert(m.name.name.clone(), decl) {
                            diags.push(
                                Diagnostic::error(format!(
                                    "`{}` is declared more than once",
                                    m.name.name
                                ))
                                .with_code(codes::DUPLICATE)
                                .with_label(m.name.span, "second declaration")
                                .with_secondary(first.span(), "first declaration"),
                            );
                        } else {
                            table.order.push(m.name.name.clone());
                        }
                    }
                    ItemKind::Package(p) => {
                        let decl = PackageDecl {
                            def: p,
                            dialect: *dialect,
                        };
                        if let Some(first) = table.packages.insert(p.name.name.clone(), decl) {
                            diags.push(
                                Diagnostic::error(format!(
                                    "package `{}` is declared more than once",
                                    p.name.name
                                ))
                                .with_code(codes::DUPLICATE)
                                .with_label(p.name.span, "second declaration")
                                .with_secondary(first.def.name.span, "first declaration"),
                            );
                        }
                    }
                    _ => table.unit_items.push((item, *dialect)),
                }
            }
        }
        table
    }

    /// True when `name` is an interface declaration.
    pub(crate) fn is_interface(&self, name: &str) -> bool {
        self.modules.get(name).is_some_and(ModuleDecl::is_interface)
    }

    /// The module names that no module instantiates, in source order.
    ///
    /// Interfaces, programs and primitives are never roots: they exist to
    /// be used by something else.
    pub(crate) fn roots(&self) -> Vec<String> {
        let mut used: BTreeSet<&str> = BTreeSet::new();
        for decl in self.modules.values() {
            collect_instantiated(&decl.def.items, &mut used);
        }
        self.order
            .iter()
            .filter(|name| {
                let decl = &self.modules[*name];
                decl.def.kind == ModuleKind::Module || decl.def.kind == ModuleKind::Macromodule
            })
            .filter(|name| !used.contains(name.as_str()))
            .cloned()
            .collect()
    }
}

/// Records every module name instantiated inside `items`, following
/// generate regions.
fn collect_instantiated<'a>(items: &'a [Item], out: &mut BTreeSet<&'a str>) {
    for item in items {
        match &item.kind {
            ItemKind::Instance(inst) => {
                out.insert(inst.module.name.as_str());
            }
            ItemKind::Generate(items) => collect_instantiated(items, out),
            ItemKind::GenIf(g) => {
                collect_instantiated(&g.then_block.items, out);
                if let Some(e) = &g.else_block {
                    collect_instantiated(&e.items, out);
                }
            }
            ItemKind::GenCase(g) => {
                for arm in &g.items {
                    collect_instantiated(&arm.block.items, out);
                }
            }
            ItemKind::GenFor(g) => collect_instantiated(&g.body.items, out),
            ItemKind::GenBlock(b) => collect_instantiated(&b.items, out),
            ItemKind::Bind(b) => {
                out.insert(b.inst.module.name.as_str());
            }
            _ => {}
        }
    }
}

/// The `` `timescale `` of a file: the first directive item that reached
/// the parser, as `(unit value, unit, precision value, precision)`.
fn timescale_of(items: &[Item]) -> Option<(u64, crate::ir::TimeUnit, u64, crate::ir::TimeUnit)> {
    for item in items {
        if let ItemKind::Directive(d) = &item.kind
            && d.name == "timescale"
            && let Some(ts) = parse_timescale(&d.args)
        {
            return Some(ts);
        }
    }
    None
}

/// Parses `1 ns / 1 ps` into its two delays.
fn parse_timescale(args: &str) -> Option<(u64, crate::ir::TimeUnit, u64, crate::ir::TimeUnit)> {
    let (unit, precision) = args.split_once('/')?;
    let (uv, uu) = parse_time(unit)?;
    let (pv, pu) = parse_time(precision)?;
    Some((uv, uu, pv, pu))
}

/// Parses `1 ns`, `10ps` or `100 us` into a value and a unit.
fn parse_time(text: &str) -> Option<(u64, crate::ir::TimeUnit)> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let split = text.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = text.split_at(split);
    let value: u64 = num.parse().ok()?;
    let unit = crate::ir::TimeUnit::from_name(unit)?;
    Some((value, unit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;
    use crate::verilog::{NoIncludes, parse_source};

    fn parse(text: &str) -> (SourceMap, ast::SourceFile) {
        let mut map = SourceMap::new();
        let id = map.add("t.sv", text).unwrap();
        let mut diags = Diagnostics::new();
        let file = parse_source(
            &mut map,
            id,
            Dialect::SystemVerilog,
            &mut NoIncludes,
            &mut diags,
        );
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        (map, file)
    }

    #[test]
    fn finds_roots_and_duplicates() {
        let (map, file) = parse(
            "module leaf(input a); endmodule\n\
             module mid(input a); leaf u0(.a(a)); endmodule\n\
             module top(input a); mid u1(.a(a)); endmodule\n\
             interface i_if; logic x; endinterface\n\
             package p; localparam int K = 1; endpackage\n",
        );
        let mut diags = Diagnostics::new();
        let table = Table::collect(&[(&file, Dialect::SystemVerilog)], &mut diags);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        assert_eq!(table.roots(), ["top"]);
        assert!(table.is_interface("i_if"));
        assert!(!table.is_interface("top"));
        assert!(table.packages.contains_key("p"));
        assert_eq!(table.order.len(), 4);
    }

    #[test]
    fn generate_instances_count_as_uses() {
        let (_, file) = parse(
            "module leaf(input a); endmodule\n\
             module top(input a);\n\
               generate if (1) begin : g leaf u0(.a(a)); end endgenerate\n\
             endmodule\n",
        );
        let mut diags = Diagnostics::new();
        let table = Table::collect(&[(&file, Dialect::SystemVerilog)], &mut diags);
        assert_eq!(table.roots(), ["top"]);
    }

    #[test]
    fn duplicate_modules_are_reported() {
        let (_, file) = parse("module m; endmodule\nmodule m; endmodule\n");
        let mut diags = Diagnostics::new();
        let table = Table::collect(&[(&file, Dialect::SystemVerilog)], &mut diags);
        assert_eq!(diags.error_count(), 1);
        assert_eq!(table.order, ["m"]);
    }

    #[test]
    fn timescales_parse() {
        assert_eq!(
            parse_timescale("1 ns / 1 ps"),
            Some((1, crate::ir::TimeUnit::Ns, 1, crate::ir::TimeUnit::Ps))
        );
        assert_eq!(
            parse_timescale("10ps/1ps"),
            Some((10, crate::ir::TimeUnit::Ps, 1, crate::ir::TimeUnit::Ps))
        );
        assert_eq!(parse_timescale("1ns"), None);
        assert_eq!(parse_time("1xs"), None);
    }
}
