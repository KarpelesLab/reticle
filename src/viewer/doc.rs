//! The documentation page: what a module is, from the IR and the comments.
//!
//! This is the "documentation generation from source comments and port
//! lists" half of the viewer. Everything on the page comes either from the
//! IR — which knows the ports, their directions and widths, the resolved
//! parameters, the memories, the inferred storage and the instance tree —
//! or from the comment side table the frontends' lexers keep, which is
//! where the prose lives:
//!
//! - the run of whole-line comments above a module is its description;
//! - the comment after a port on its own line is that port's note;
//! - the same holds for parameters and memories.
//!
//! Instances link to the page of the module they name, so the tree is
//! walkable, and every source location links to its line in the listing
//! [`super::source`] renders.

use std::collections::BTreeMap;

use super::html::{Crumb, Html, page};
use super::{Links, ViewerOptions, graph};
use crate::ir::{AttrValue, CellKind, Design, Module, ModuleId, ModuleRef, PortDir, Type};
use crate::source::{SourceMap, Span};

/// How deep the instance tree is walked before it is cut off.
const MAX_DEPTH: usize = 12;

/// The module's own description, so a per-object lookup that walks up to
/// the same comment block does not repeat it in every row.
#[derive(Clone, Copy)]
struct Described<'a> {
    module: Option<&'a str>,
}

/// Renders the documentation page of one module.
pub fn render(
    design: &Design,
    map: &SourceMap,
    id: ModuleId,
    links: &Links,
    options: &ViewerOptions,
) -> String {
    let module = design.module(id);
    let mut h = Html::new();

    h.open("h1", &[]);
    h.text(module.name.as_str());
    if module.blackbox {
        h.element("span", &[("class", "kind")], " black box");
    }
    h.close();

    h.open("p", &[("class", "loc")]);
    location(&mut h, map, links, module.span);
    if let Some(path) = links.schematic(id) {
        h.text(" \u{b7} ");
        h.element("a", &[("href", path)], "schematic");
    }
    h.text(" \u{b7} ");
    h.element("a", &[("href", "index.html")], "index");
    h.close();

    let description = options.comments.leading(map, module.span);
    match &description {
        Some(text) => h.element("p", &[("class", "desc")], text),
        None => h.element(
            "p",
            &[("class", "desc none")],
            "No description: the source has no comment above this module.",
        ),
    }
    let described = Described {
        module: description.as_deref(),
    };

    summary(&mut h, module);
    ports(&mut h, map, module, options, described);
    params(&mut h, map, module, options, described);
    memories(&mut h, map, links, module, options, described);
    storage(&mut h, map, links, module);
    instances(&mut h, design, links, id);

    let crumbs: Vec<Crumb> = vec![
        ("index.html".to_owned(), "index".to_owned()),
        (String::new(), module.name.to_string()),
    ];
    page(
        &format!("{} \u{2014} module reference", module.name),
        &crumbs,
        &h.finish(),
        false,
        false,
    )
}

/// The one-line shape of the module.
fn summary(h: &mut Html, module: &Module) {
    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    for cell in module.cells.values() {
        *kinds.entry(cell.kind.keyword()).or_insert(0) += 1;
    }
    h.element("h2", &[], "At a glance");
    h.open("table", &[]);
    row(h, "ports", &module.ports.len().to_string());
    row(h, "nets", &module.nets.len().to_string());
    row(h, "cells", &module.cells.values().len().to_string());
    row(
        h,
        "logic depth",
        &graph::combinational_depth(module).to_string(),
    );
    row(h, "continuous assigns", &module.assigns.len().to_string());
    row(h, "processes", &module.processes.values().len().to_string());
    row(h, "instances", &module.instances.values().len().to_string());
    if let Some(timescale) = &module.timescale {
        row(
            h,
            "timescale",
            &format!(
                "{} {} / {} {}",
                timescale.unit.value,
                timescale.unit.unit.name(),
                timescale.precision.value,
                timescale.precision.unit.name()
            ),
        );
    }
    if !kinds.is_empty() {
        let text: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
        row(h, "cell kinds", &text.join(", "));
    }
    for (name, value) in module.attrs.iter() {
        row(h, &format!("attribute {name}"), &value.to_string());
    }
    h.close();
}

/// One `label / value` row of a two-column table.
fn row(h: &mut Html, label: &str, value: &str) {
    h.open("tr", &[]);
    h.element("th", &[], label);
    h.element("td", &[("class", "mono")], value);
    h.close();
}

/// The port list: the interface, with each port's own comment.
fn ports(
    h: &mut Html,
    map: &SourceMap,
    module: &Module,
    options: &ViewerOptions,
    described: Described<'_>,
) {
    h.element("h2", &[], "Ports");
    if module.ports.is_empty() {
        h.element("p", &[("class", "none")], "This module has no ports.");
        return;
    }
    h.open("table", &[]);
    head(
        h,
        &["name", "direction", "width", "type", "net", "description"],
    );
    h.open("tbody", &[]);
    for port in &module.ports {
        let net = &module.nets[port.net];
        h.open("tr", &[]);
        h.element("td", &[("class", "mono")], port.name.as_str());
        h.element("td", &[], port.dir.keyword());
        h.element("td", &[("class", "num")], &width_text(&net.ty));
        h.element("td", &[("class", "mono")], &net.ty.to_string());
        h.element("td", &[("class", "mono")], net.name.as_str());
        note(h, map, options, port.span, described);
        h.close();
    }
    h.close();
    h.close();
}

/// The resolved parameters, with the range their type allows.
fn params(
    h: &mut Html,
    map: &SourceMap,
    module: &Module,
    options: &ViewerOptions,
    described: Described<'_>,
) {
    h.element("h2", &[], "Parameters");
    if module.params.is_empty() {
        h.element(
            "p",
            &[("class", "none")],
            "This module has no parameters, or elaboration has already folded them away.",
        );
        return;
    }
    h.open("table", &[]);
    head(h, &["name", "default", "type", "range", "description"]);
    h.open("tbody", &[]);
    for param in &module.params {
        h.open("tr", &[]);
        h.element("td", &[("class", "mono")], param.name.as_str());
        h.element("td", &[("class", "mono")], &value_text(&param.value));
        h.element("td", &[("class", "mono")], value_type(&param.value));
        h.element("td", &[("class", "mono")], &value_range(&param.value));
        // Elaboration folds parameters away and leaves them the module's
        // own span, so a comment lookup there would find the module's
        // description; only a span of its own can carry a note.
        if param.span == module.span {
            h.element("td", &[("class", "none")], "\u{2014}");
        } else {
            note(h, map, options, param.span, described);
        }
        h.close();
    }
    h.close();
    h.close();
}

/// The memories the module declares.
fn memories(
    h: &mut Html,
    map: &SourceMap,
    links: &Links,
    module: &Module,
    options: &ViewerOptions,
    described: Described<'_>,
) {
    if module.memories.values().len() == 0 {
        return;
    }
    h.element("h2", &[], "Memories");
    h.open("table", &[]);
    head(
        h,
        &[
            "name",
            "element",
            "depth",
            "bits",
            "initialised",
            "where",
            "description",
        ],
    );
    h.open("tbody", &[]);
    for memory in module.memories.values() {
        let width = memory.elem.width().unwrap_or(0);
        h.open("tr", &[]);
        h.element("td", &[("class", "mono")], memory.name.as_str());
        h.element("td", &[("class", "mono")], &memory.elem.to_string());
        h.element("td", &[("class", "num")], &memory.size.to_string());
        h.element(
            "td",
            &[("class", "num")],
            &(memory.size * u64::from(width)).to_string(),
        );
        h.element(
            "td",
            &[],
            match &memory.init {
                Some(values) if !values.is_empty() => "yes",
                _ => "no",
            },
        );
        h.open("td", &[("class", "loc")]);
        location(h, map, links, memory.span);
        h.close();
        note(h, map, options, memory.span, described);
        h.close();
    }
    h.close();
    h.close();
}

/// One inferred storage element, as the table shows it.
struct StorageRow {
    /// What was inferred: `flip-flop`, `latch`, a memory port.
    kind: &'static str,
    /// The cell's name.
    cell: String,
    /// The net driven, or the memory accessed.
    target: String,
    /// Width in bits of the stored value.
    width: u32,
    /// Features in words: `enable`, `sync reset ...`, `clocked`.
    features: Vec<String>,
    /// Where the source construct was.
    span: Span,
}

/// The storage synthesis inferred, read back out of the cell form.
fn storage(h: &mut Html, map: &SourceMap, links: &Links, module: &Module) {
    let mut rows: Vec<StorageRow> = Vec::new();
    for cell in module.cells.values() {
        let target_of = |port: &str| {
            cell.output(port)
                .map_or(String::new(), |n| module.nets[n].name.to_string())
        };
        let width_of = |port: &str| {
            cell.output(port)
                .and_then(|n| module.nets[n].ty.width())
                .unwrap_or(0)
        };
        match &cell.kind {
            CellKind::Dff {
                clk_pos,
                has_enable,
                reset,
            } => {
                let mut features = vec![if *clk_pos { "posedge" } else { "negedge" }.to_owned()];
                if *has_enable {
                    features.push("enable".to_owned());
                }
                if let Some(r) = reset {
                    features.push(format!(
                        "{} reset {}, value {}",
                        if r.asynchronous { "async" } else { "sync" },
                        if r.active_high {
                            "active-high"
                        } else {
                            "active-low"
                        },
                        graph::const_text(&r.value)
                    ));
                }
                rows.push(StorageRow {
                    kind: "flip-flop",
                    cell: cell.name.to_string(),
                    target: target_of("q"),
                    width: width_of("q"),
                    features,
                    span: cell.span,
                });
            }
            CellKind::Dlatch => rows.push(StorageRow {
                kind: "latch",
                cell: cell.name.to_string(),
                target: target_of("q"),
                width: width_of("q"),
                features: Vec::new(),
                span: cell.span,
            }),
            CellKind::MemRdPort { mem, clocked } | CellKind::MemWrPort { mem, clocked } => {
                let read = matches!(cell.kind, CellKind::MemRdPort { .. });
                rows.push(StorageRow {
                    kind: if read {
                        "memory read port"
                    } else {
                        "memory write port"
                    },
                    cell: cell.name.to_string(),
                    target: module.memories[*mem].name.to_string(),
                    width: module.memories[*mem].elem.width().unwrap_or(0),
                    features: vec![if *clocked { "clocked" } else { "asynchronous" }.to_owned()],
                    span: cell.span,
                });
            }
            _ => {}
        }
    }
    if rows.is_empty() {
        return;
    }
    h.element("h2", &[], "Inferred storage");
    h.open("table", &[]);
    head(h, &["kind", "cell", "target", "width", "features", "where"]);
    h.open("tbody", &[]);
    for row in rows {
        h.open("tr", &[]);
        h.element("td", &[], row.kind);
        h.element("td", &[("class", "mono")], &row.cell);
        h.element("td", &[("class", "mono")], &row.target);
        h.element("td", &[("class", "num")], &row.width.to_string());
        h.element("td", &[], &row.features.join(", "));
        h.open("td", &[("class", "loc")]);
        location(h, map, links, row.span);
        h.close();
        h.close();
    }
    h.close();
    h.close();
}

/// The instance tree below this module.
fn instances(h: &mut Html, design: &Design, links: &Links, id: ModuleId) {
    let module = design.module(id);
    h.element("h2", &[], "Instance tree");
    if module.instances.values().len() == 0 {
        h.element(
            "p",
            &[("class", "none")],
            "This module is a leaf: it instantiates nothing.",
        );
        return;
    }
    let mut path = vec![id];
    subtree(h, design, links, id, &mut path);
}

/// One level of the tree, with `path` holding the modules already entered
/// so a recursive instantiation stops instead of recursing.
fn subtree(h: &mut Html, design: &Design, links: &Links, id: ModuleId, path: &mut Vec<ModuleId>) {
    h.open("ul", &[("class", "tree")]);
    for instance in design.module(id).instances.values() {
        h.open("li", &[]);
        h.text(instance.name.as_str());
        h.text(" : ");
        match &instance.module {
            ModuleRef::Resolved(target) => {
                let name = design.module(*target).name.to_string();
                match links.doc(*target) {
                    Some(href) => h.element("a", &[("href", href)], &name),
                    None => h.text(&name),
                }
                if path.contains(target) {
                    h.element("span", &[("class", "loc")], " (already above)");
                } else if path.len() >= MAX_DEPTH {
                    h.element("span", &[("class", "loc")], " (tree cut off here)");
                } else {
                    path.push(*target);
                    if design.module(*target).instances.values().len() > 0 {
                        subtree(h, design, links, *target, path);
                    }
                    path.pop();
                }
            }
            ModuleRef::Unresolved(name) => {
                h.text(name.as_str());
                h.element("span", &[("class", "loc")], " (black box)");
            }
        }
        h.close();
    }
    h.close();
}

/// A table head row.
fn head(h: &mut Html, columns: &[&str]) {
    h.open("thead", &[]);
    h.open("tr", &[]);
    for column in columns {
        if matches!(*column, "width" | "depth" | "bits") {
            h.element("th", &[("class", "num")], column);
        } else {
            h.element("th", &[], column);
        }
    }
    h.close();
    h.close();
}

/// The description cell of a row: the comment written after the object,
/// or the block above it when that is not the module's own description.
fn note(
    h: &mut Html,
    map: &SourceMap,
    options: &ViewerOptions,
    span: Span,
    described: Described<'_>,
) {
    let text = options.comments.trailing(map, span).or_else(|| {
        options
            .comments
            .leading(map, span)
            .filter(|text| Some(text.as_str()) != described.module)
    });
    match text {
        Some(text) => h.element("td", &[], &text),
        None => h.element("td", &[("class", "none")], "\u{2014}"),
    }
}

/// A source location, linked to its line when the listing exists.
fn location(h: &mut Html, map: &SourceMap, links: &Links, span: Span) {
    let (text, href) = links.location(map, span);
    match href {
        Some(href) => h.element("a", &[("href", &href)], &text),
        None => h.text(&text),
    }
}

/// A type's width, as text.
fn width_text(ty: &Type) -> String {
    match ty.width() {
        Some(width) => width.to_string(),
        None => "\u{2014}".to_owned(),
    }
}

/// A parameter's value, with a bit vector written the way the `.rtl` text
/// format writes it.
fn value_text(value: &AttrValue) -> String {
    match value {
        AttrValue::Const(c) => graph::const_text(c),
        other => other.to_string(),
    }
}

/// The name of an attribute value's type, for the parameter table.
fn value_type(value: &AttrValue) -> &'static str {
    match value {
        AttrValue::Const(_) => "bit vector",
        AttrValue::String(_) => "text",
        AttrValue::Int(_) => "integer",
    }
}

/// The range a resolved parameter's type allows.
///
/// The IR keeps parameters *after* elaboration, so there is no declared
/// range to print; what a value can hold is what its type allows, which is
/// what a reader needs in order to know whether an override will fit.
fn value_range(value: &AttrValue) -> String {
    match value {
        AttrValue::Int(_) => format!("{} .. {}", i64::MIN, i64::MAX),
        AttrValue::String(_) => "any text".to_owned(),
        AttrValue::Const(c) => {
            let width = c.width();
            if width == 0 {
                return "empty".to_owned();
            }
            if width > 63 {
                return format!(
                    "{} bits{}",
                    width,
                    if c.is_signed() { ", signed" } else { "" }
                );
            }
            if c.is_signed() {
                let span = 1i64 << (width - 1);
                format!("{} .. {}", -span, span - 1)
            } else {
                format!("0 .. {}", (1u64 << width) - 1)
            }
        }
    }
}

/// True when the port direction reads values in.
#[allow(dead_code)]
fn is_input(dir: PortDir) -> bool {
    matches!(dir, PortDir::In | PortDir::InOut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer::{Comments, Links, ViewerOptions};

    const NETLIST: &str = include_str!("../../testdata/ir/netlist.rtl");

    fn rendered(text: &str, comments: Comments) -> String {
        let mut map = SourceMap::new();
        let file = map.add("t.rtl", text).unwrap();
        let design = Design::parse_text(text, file).unwrap();
        let id = design.modules.ids().next().unwrap();
        let options = ViewerOptions {
            comments,
            ..ViewerOptions::default()
        };
        render(&design, &map, id, &Links::default(), &options)
    }

    #[test]
    fn lists_the_interface_and_the_storage() {
        let html = rendered(NETLIST, Comments::new());
        assert!(html.contains("<h2>Ports</h2>"));
        assert!(html.contains("<h2>Inferred storage</h2>"));
        assert!(html.contains("flip-flop"));
        assert!(html.contains("memory write port"));
        assert!(html.contains("<h2>Memories</h2>"));
        assert!(html.contains("sync reset active-high, value 4&#39;d0"));
        // No comments were supplied, so the description says so.
        assert!(html.contains("No description"));
    }

    #[test]
    fn a_leading_comment_becomes_the_description() {
        let text = "// A small counter.\nmodule m\n  net %a u1 wire\n  port a in %a\nend\n";
        let mut map = SourceMap::new();
        let file = map.add("t.rtl", text).unwrap();
        let design = Design::parse_text(text, file).unwrap();
        let id = design.modules.ids().next().unwrap();
        let mut comments = Comments::new();
        comments.add(Span::new(file, 0, 19));
        let options = ViewerOptions {
            comments,
            ..ViewerOptions::default()
        };
        let html = render(&design, &map, id, &Links::default(), &options);
        assert!(html.contains("A small counter."), "{html}");
    }

    #[test]
    fn ranges_follow_the_resolved_type() {
        use crate::logic::Logic;
        assert_eq!(
            value_range(&AttrValue::Const(Logic::from_u64(3, 8))),
            "0 .. 255"
        );
        assert_eq!(
            value_range(&AttrValue::Const(Logic::from_i64(-1, 8))),
            "-128 .. 127"
        );
        assert_eq!(value_range(&AttrValue::String("x".into())), "any text");
        assert!(value_range(&AttrValue::Int(1)).contains(".."));
        assert_eq!(value_type(&AttrValue::Int(1)), "integer");
    }
}
