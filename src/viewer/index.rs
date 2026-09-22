//! The index page: every module at a glance, linking to both views.
//!
//! One row per module with the numbers a reader triages on — cells, logic
//! depth, ports, nets, instances — and a link to the schematic and to the
//! reference page. A module whose schematic was skipped for being too
//! large says so in place of the link, rather than linking to a page that
//! does not exist.

use std::collections::BTreeMap;

use super::html::{Crumb, Html, page};
use super::{Graph, Links, ViewerOptions, graph};
use crate::ir::{Design, ModuleId};
use crate::source::SourceMap;

/// Renders the index of a design.
pub fn render(
    design: &Design,
    map: &SourceMap,
    modules: &[ModuleId],
    graphs: &BTreeMap<ModuleId, Graph>,
    links: &Links,
    options: &ViewerOptions,
) -> String {
    let mut h = Html::new();
    h.element("h1", &[], &options.title);

    if modules.is_empty() {
        h.element(
            "p",
            &[("class", "none")],
            "No modules. Either the design is empty or every module was filtered out.",
        );
        return page(&options.title, &[], &h.finish(), false, false);
    }

    let total_cells: usize = modules
        .iter()
        .map(|&id| design.module(id).cells.values().len())
        .sum();
    h.element(
        "p",
        &[("class", "desc")],
        &format!(
            "{} module{}, {total_cells} cell{} in all.{}",
            modules.len(),
            if modules.len() == 1 { "" } else { "s" },
            if total_cells == 1 { "" } else { "s" },
            match design.top {
                Some(top) if modules.contains(&top) =>
                    format!(" The top is {}.", design.module(top).name),
                _ => String::new(),
            }
        ),
    );

    h.open("table", &[]);
    h.open("thead", &[]);
    h.open("tr", &[]);
    for column in [
        "module",
        "ports",
        "nets",
        "cells",
        "depth",
        "instances",
        "views",
    ] {
        if column == "module" || column == "views" {
            h.element("th", &[], column);
        } else {
            h.element("th", &[("class", "num")], column);
        }
    }
    h.close();
    h.close();
    h.open("tbody", &[]);
    for &id in modules {
        let module = design.module(id);
        h.open("tr", &[]);
        h.open("td", &[("class", "mono")]);
        match links.doc(id) {
            Some(href) => h.element("a", &[("href", href)], module.name.as_str()),
            None => h.text(module.name.as_str()),
        }
        if module.blackbox {
            h.element("span", &[("class", "loc")], " black box");
        }
        h.close();
        h.element("td", &[("class", "num")], &module.ports.len().to_string());
        h.element("td", &[("class", "num")], &module.nets.len().to_string());
        h.element(
            "td",
            &[("class", "num")],
            &module.cells.values().len().to_string(),
        );
        h.element(
            "td",
            &[("class", "num")],
            &graph::combinational_depth(module).to_string(),
        );
        h.element(
            "td",
            &[("class", "num")],
            &module.instances.values().len().to_string(),
        );
        h.open("td", &[]);
        match links.schematic(id) {
            Some(href) => h.element("a", &[("href", href)], "schematic"),
            None if !options.schematics => h.element("span", &[("class", "none")], "\u{2014}"),
            None => {
                let nodes = graphs.get(&id).map_or(0, |g| g.nodes.len());
                h.element(
                    "span",
                    &[("class", "none")],
                    &format!("schematic too large ({nodes} boxes)"),
                );
            }
        }
        if let Some(href) = links.doc(id) {
            h.text(" \u{b7} ");
            h.element("a", &[("href", href)], "reference");
        }
        h.close();
        h.close();
    }
    h.close();
    h.close();

    let files: Vec<(crate::source::SourceId, &str)> = links.sources().collect();
    if !files.is_empty() {
        h.element("h2", &[], "Sources");
        h.open("ul", &[("class", "tree")]);
        for (file, href) in files {
            h.open("li", &[]);
            h.element("a", &[("href", href)], map.file(file).name());
            h.close();
        }
        h.close();
    }

    let crumbs: Vec<Crumb> = vec![(String::new(), "index".to_owned())];
    page(&options.title, &crumbs, &h.finish(), false, false)
}
