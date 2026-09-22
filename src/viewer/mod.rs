//! The schematic and documentation viewer: a design rendered as a small
//! static web site.
//!
//! [`render`] takes a [`Design`], the [`SourceMap`] it was built from and a
//! [`ViewerOptions`], and returns a [`Site`]: a list of `(path, contents)`
//! pairs. Nothing here touches the filesystem, so the library stays
//! sans-I/O and the caller — `reticle viewer`, a build script, a test —
//! decides where the bytes go.
//!
//! # What comes out
//!
//! | Page                      | Contents                                                     |
//! |---------------------------|--------------------------------------------------------------|
//! | `index.html`              | every module, with cell count and logic depth, linking to both views |
//! | `<module>.schematic.html` | the netlist as inline SVG, with pan, zoom, highlighting and search |
//! | `<module>.doc.html`       | description, ports, parameters, memories, storage, instance tree |
//! | `source/<n>-<file>.html`  | the source text, one anchor per line, for every location linked to |
//!
//! Every page is self-contained: the style sheet and the script are
//! inlined, there is no font import and no external reference of any kind,
//! so a page opens from `file://` and can be attached to a bug report.
//! `tests/viewer_offline.rs` is what keeps that true.
//!
//! # The two views
//!
//! The **schematic** ([`schematic`]) lays the cell form out as a layered
//! graph: [`graph`] turns the module into nodes and wires, [`layout`]
//! ranks them by longest path, orders each rank with the barycentre
//! heuristic and routes the wires as orthogonal polylines through channels
//! between the columns. Flip-flops, latches and memory ports are drawn
//! apart from combinational cells, constants as triangles, ports on the
//! edges of the canvas, and a bus carries its width.
//!
//! The **documentation** ([`doc`]) is built from the IR plus the comment
//! side table the frontends' lexers keep ([`Comments`]): the comment above
//! a module becomes its description, the comment after a port its note.
//! Instances link to the pages of the modules they name, and every source
//! location links to its line.
//!
//! Both views share one renderer ([`html`]), so the chrome, the escaping
//! and the palette are defined once.
//!
//! # Determinism
//!
//! Geometry is computed in integer units, iteration follows arena order,
//! and every map in the pipeline is a `BTreeMap`. The same design renders
//! to the same bytes, which is what makes the golden tests in
//! `tests/viewer_golden.rs` worth reading in review.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::{Design, ModuleId};
use crate::source::{SourceId, SourceMap, Span};

pub mod comments;
pub mod doc;
pub mod graph;
pub mod html;
pub mod index;
pub mod layout;
pub mod schematic;
pub mod source;

pub use comments::Comments;
pub use graph::{Graph, Node, NodeKind, Shape};
pub use layout::{Layout, LayoutOptions, Placed, Point, Wire};

/// What to render, and how.
#[derive(Clone, Debug)]
pub struct ViewerOptions {
    /// The heading of the index page.
    pub title: String,
    /// The comments the documentation view reads; an empty table simply
    /// leaves the prose out.
    pub comments: Comments,
    /// Render a schematic page per module.
    pub schematics: bool,
    /// Render a documentation page per module.
    pub docs: bool,
    /// Render a source listing per file a location points into.
    pub sources: bool,
    /// Render only these modules, by name; empty means all of them.
    pub only: Vec<String>,
    /// Skip the schematic of a module whose graph has more nodes than
    /// this. A netlist of ten thousand cells makes a page no browser
    /// enjoys and no reader can follow; its documentation page is still
    /// written, and the index says why the schematic is missing.
    pub max_nodes: usize,
    /// The sizes the layout is built from.
    pub layout: LayoutOptions,
}

impl Default for ViewerOptions {
    fn default() -> Self {
        ViewerOptions {
            title: "Design".to_owned(),
            comments: Comments::new(),
            schematics: true,
            docs: true,
            sources: true,
            only: Vec::new(),
            max_nodes: 1500,
            layout: LayoutOptions::default(),
        }
    }
}

impl ViewerOptions {
    /// True when `name` is to be rendered.
    pub fn selects(&self, name: &str) -> bool {
        self.only.is_empty() || self.only.iter().any(|m| m == name)
    }
}

/// The rendered pages, as `(path, contents)` pairs sorted by path.
///
/// Paths are relative and use `/` as the separator; a caller writing them
/// out creates the `source` directory itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Site {
    /// Every file, sorted by path.
    pub files: Vec<(String, String)>,
}

impl Site {
    /// The contents of one file.
    pub fn get(&self, path: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, text)| text.as_str())
    }

    /// Every path, in order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|(path, _)| path.as_str())
    }

    /// Number of files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True when nothing was rendered.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Where each page lives, so one view can link to another.
///
/// Module file names come from the module names, made safe for a file
/// system and made unique by a numeric suffix when two names differ only
/// in a character that had to be replaced.
#[derive(Clone, Debug, Default)]
pub struct Links {
    schematics: BTreeMap<ModuleId, String>,
    docs: BTreeMap<ModuleId, String>,
    sources: BTreeMap<SourceId, String>,
}

impl Links {
    /// The schematic page of a module, when one was rendered.
    pub fn schematic(&self, module: ModuleId) -> Option<&str> {
        self.schematics.get(&module).map(String::as_str)
    }

    /// The documentation page of a module, when one was rendered.
    pub fn doc(&self, module: ModuleId) -> Option<&str> {
        self.docs.get(&module).map(String::as_str)
    }

    /// The source listing of a file, when one was rendered.
    pub fn source(&self, file: SourceId) -> Option<&str> {
        self.sources.get(&file).map(String::as_str)
    }

    /// Every source listing, in file order.
    pub fn sources(&self) -> impl Iterator<Item = (SourceId, &str)> {
        self.sources.iter().map(|(id, path)| (*id, path.as_str()))
    }

    /// A span as `file:line:col`, and the link to that line when the
    /// file's listing was rendered.
    ///
    /// The link is relative to the root of the site, which is where every
    /// page that shows a location lives; only the listings themselves sit
    /// in a sub-directory, and they show no locations of their own.
    pub fn location(&self, map: &SourceMap, span: Span) -> (String, Option<String>) {
        let (name, loc) = map.locate(span);
        let text = format!("{name}:{loc}");
        let href = self
            .source(span.file)
            .map(|path| format!("{path}#L{}", loc.line));
        (text, href)
    }
}

/// Renders `design` as a static site.
///
/// The site is always non-empty: even a design with no modules gets an
/// index saying so.
pub fn render(design: &Design, map: &SourceMap, options: &ViewerOptions) -> Site {
    let modules: Vec<ModuleId> = design
        .modules
        .iter()
        .filter(|(_, m)| options.selects(m.name.as_str()))
        .map(|(id, _)| id)
        .collect();

    let mut links = Links::default();
    let mut used = BTreeMap::<String, usize>::new();
    let mut stem = BTreeMap::<ModuleId, String>::new();
    for &id in &modules {
        let base = slug(design.module(id).name.as_str(), "module");
        let count = used.entry(base.clone()).or_insert(0);
        *count += 1;
        let name = if *count == 1 {
            base
        } else {
            format!("{base}-{count}")
        };
        stem.insert(id, name);
    }

    // Which modules get which pages is decided first, so a page can link
    // to another that has not been rendered yet.
    let mut with_schematic: BTreeSet<ModuleId> = BTreeSet::new();
    let mut graphs: BTreeMap<ModuleId, Graph> = BTreeMap::new();
    for &id in &modules {
        let graph = Graph::of_module(design, id);
        if options.schematics && graph.nodes.len() <= options.max_nodes {
            with_schematic.insert(id);
            links
                .schematics
                .insert(id, format!("{}.schematic.html", stem[&id]));
        }
        graphs.insert(id, graph);
        if options.docs {
            links.docs.insert(id, format!("{}.doc.html", stem[&id]));
        }
    }
    if options.sources {
        for file in referenced_sources(design, &modules) {
            let name = slug(map.file(file).name(), "source");
            links
                .sources
                .insert(file, format!("source/{}-{name}.html", file.index()));
        }
    }

    let mut files: Vec<(String, String)> = Vec::new();
    for &id in &modules {
        if with_schematic.contains(&id) {
            let path = links.schematics[&id].clone();
            let page = schematic::render(design, map, id, &graphs[&id], &links, options);
            files.push((path, page));
        }
        if options.docs {
            let path = links.docs[&id].clone();
            let page = doc::render(design, map, id, &links, options);
            files.push((path, page));
        }
    }
    if options.sources {
        for (&file, path) in &links.sources {
            files.push((path.clone(), source::render(map, file)));
        }
    }
    files.push((
        "index.html".to_owned(),
        index::render(design, map, &modules, &graphs, &links, options),
    ));

    files.sort();
    Site { files }
}

/// Every file a rendered module's objects point into.
fn referenced_sources(design: &Design, modules: &[ModuleId]) -> BTreeSet<SourceId> {
    let mut out = BTreeSet::new();
    for &id in modules {
        let module = design.module(id);
        let mut note = |span: Span| {
            out.insert(span.file);
        };
        note(module.span);
        module.ports.iter().for_each(|p| note(p.span));
        module.params.iter().for_each(|p| note(p.span));
        module.nets.values().for_each(|n| note(n.span));
        module.memories.values().for_each(|m| note(m.span));
        module.instances.values().for_each(|i| note(i.span));
        module.processes.values().for_each(|p| note(p.span));
        module.cells.values().for_each(|c| note(c.span));
        module.assigns.iter().for_each(|a| note(a.span));
    }
    out
}

/// Turns a name into a file-name component: letters, digits, `.`, `-` and
/// `_` survive, everything else becomes `_`.
///
/// A path separator in a source name is dropped along with the directories
/// before it, since listings all live in one directory.
fn slug(name: &str, fallback: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut out = String::with_capacity(base.len());
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('.').to_owned();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn design(text: &str) -> (Design, SourceMap) {
        let mut map = SourceMap::new();
        let file = map.add("t.rtl", text).unwrap();
        let design = Design::parse_text(text, file).unwrap();
        (design, map)
    }

    const NETLIST: &str = include_str!("../../testdata/ir/netlist.rtl");

    #[test]
    fn slugs_are_safe_and_stable() {
        assert_eq!(slug("counter", "module"), "counter");
        assert_eq!(slug("a/b/c.rtl", "source"), "c.rtl");
        assert_eq!(slug("top$mod", "module"), "top_mod");
        assert_eq!(slug("", "module"), "module");
        assert_eq!(slug("...", "module"), "module");
    }

    #[test]
    fn renders_a_page_per_view_plus_an_index() {
        let (design, map) = design(NETLIST);
        let site = render(&design, &map, &ViewerOptions::default());
        let paths: Vec<&str> = site.paths().collect();
        assert_eq!(
            paths,
            [
                "counter_synth.doc.html",
                "counter_synth.schematic.html",
                "index.html",
                "source/0-t.rtl.html",
            ]
        );
        assert!(site.get("index.html").unwrap().contains("counter_synth"));
        assert!(!site.is_empty());
        assert_eq!(site.len(), 4);
        assert_eq!(site.get("nope.html"), None);
    }

    #[test]
    fn options_select_and_suppress() {
        let (design, map) = design(NETLIST);
        let options = ViewerOptions {
            schematics: false,
            sources: false,
            only: vec!["nothing".to_owned()],
            ..ViewerOptions::default()
        };
        let site = render(&design, &map, &options);
        assert_eq!(site.paths().collect::<Vec<_>>(), ["index.html"]);

        let options = ViewerOptions {
            max_nodes: 1,
            ..ViewerOptions::default()
        };
        let site = render(&design, &map, &options);
        assert!(site.get("counter_synth.schematic.html").is_none());
        assert!(site.get("index.html").unwrap().contains("too large"));
    }

    #[test]
    fn colliding_names_get_distinct_files() {
        let text = "module a$b\nend\nmodule a_b\nend\n";
        let (design, map) = design(text);
        let options = ViewerOptions {
            schematics: false,
            sources: false,
            ..ViewerOptions::default()
        };
        let site = render(&design, &map, &options);
        assert_eq!(
            site.paths().collect::<Vec<_>>(),
            ["a_b-2.doc.html", "a_b.doc.html", "index.html"]
        );
    }

    #[test]
    fn an_empty_design_still_gets_an_index() {
        let site = render(&Design::new(), &SourceMap::new(), &ViewerOptions::default());
        assert_eq!(site.paths().collect::<Vec<_>>(), ["index.html"]);
        assert!(site.get("index.html").unwrap().contains("No modules"));
    }
}
