//! Golden tests for the viewer, over the IR and synthesis corpora.
//!
//! Every `testdata/ir/*.rtl` and every `testdata/synth/*.cells.rtl` is
//! parsed and rendered, and the resulting site is compared with
//! `testdata/viewer/<name>/` byte for byte. The expectations are real
//! pages, not a digest: `xdg-open testdata/viewer/netlist/index.html`
//! shows exactly what a reviewer is being asked to approve, and a change
//! to the layout or the markup turns up in the diff.
//!
//! Set `UPDATE_EXPECT=1` to rewrite them after an intended change.

#![cfg(feature = "viewer")]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use reticle::ir::Design;
use reticle::source::SourceMap;
use reticle::viewer::{Site, ViewerOptions, render};

/// The corpus: `(name, path)` pairs, sorted by name.
///
/// `testdata/ir` contributes the hand-written designs (both forms of the
/// IR), `testdata/synth` the cell-form netlists synthesis produces, which
/// is what the schematic is really for.
fn corpus() -> Vec<(String, PathBuf)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut add = |dir: &Path, prefix: &str, keep: &dyn Fn(&str) -> bool| {
        let entries =
            fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.unwrap().path();
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            if !path.is_file() || !keep(&file) {
                continue;
            }
            let stem = file.split('.').next().unwrap_or(&file).to_owned();
            out.push((format!("{prefix}{stem}"), path));
        }
    };
    add(&root.join("testdata/ir"), "", &|name| {
        name.ends_with(".rtl")
    });
    add(&root.join("testdata/synth"), "synth-", &|name| {
        name.ends_with(".cells.rtl")
    });
    out.sort();
    out
}

/// Renders one design, naming its source file by its basename so the
/// locations on the pages do not depend on where the checkout lives.
fn site_of(path: &Path) -> Site {
    let text =
        fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let mut map = SourceMap::new();
    let file = map.add(name.clone(), text.clone()).unwrap();
    let design = Design::parse_text(&text, file)
        .unwrap_or_else(|d| panic!("{} does not parse: {}", path.display(), d.render(&map)));
    let options = ViewerOptions {
        title: format!("{name} \u{2014} design reference"),
        ..ViewerOptions::default()
    };
    render(&design, &map, &options)
}

#[test]
fn golden_sites() {
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/viewer");
    let corpus = corpus();
    assert!(!corpus.is_empty(), "no designs found");

    let mut failures: Vec<String> = Vec::new();
    for (name, path) in &corpus {
        let site = site_of(path);
        let dir = root.join(name);
        let expected = existing_files(&dir);
        let produced: BTreeSet<String> = site.paths().map(str::to_owned).collect();

        if update {
            for stale in expected.difference(&produced) {
                let _ = fs::remove_file(dir.join(stale));
            }
            for (file, contents) in &site.files {
                let target = dir.join(file);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::write(&target, contents).unwrap();
            }
            continue;
        }

        for stale in expected.difference(&produced) {
            failures.push(format!("{name}/{stale}: left over, no longer rendered"));
        }
        for (file, contents) in &site.files {
            let target = dir.join(file);
            match fs::read_to_string(&target) {
                Ok(text) if text == *contents => {}
                Ok(text) => failures.push(format!(
                    "{name}/{file}: {}",
                    first_difference(&text, contents)
                )),
                Err(_) => failures.push(format!("{name}/{file}: missing")),
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} mismatch(es) (set UPDATE_EXPECT=1 to rewrite):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Rendering twice gives the same bytes: nothing in the pipeline depends
/// on hash order or on an allocation address.
#[test]
fn rendering_is_deterministic() {
    for (_, path) in corpus().iter().take(4) {
        assert_eq!(site_of(path), site_of(path), "{}", path.display());
    }
}

/// Every page the site links to is a page the site contains.
#[test]
fn internal_links_resolve() {
    for (name, path) in corpus() {
        let site = site_of(&path);
        let paths: BTreeSet<&str> = site.paths().collect();
        for (page, text) in &site.files {
            let base = page.rsplit_once('/').map_or("", |(dir, _)| dir);
            for target in hrefs(text) {
                let (file, _) = target.split_once('#').unwrap_or((target.as_str(), ""));
                let resolved = resolve(base, file);
                assert!(
                    paths.contains(resolved.as_str()),
                    "{name}/{page} links to `{target}`, which the site does not have"
                );
            }
        }
    }
}

/// The geometry the layout promises, checked on real designs rather than
/// on the synthetic graphs the unit tests build: boxes do not overlap each
/// other, wires start and end on the pins they join, and no wire crosses a
/// cell body, which is what the channel routing is for.
#[test]
fn geometry_holds_on_the_corpus() {
    use reticle::viewer::{Graph, Layout, LayoutOptions};

    for (name, path) in corpus() {
        let text = fs::read_to_string(&path).unwrap();
        let mut map = SourceMap::new();
        let file = map.add(name.clone(), text.clone()).unwrap();
        let design = Design::parse_text(&text, file).expect("the corpus parses");
        for id in design.modules.ids() {
            let graph = Graph::of_module(&design, id);
            let layout = Layout::of_graph(&graph, &LayoutOptions::default());

            for a in 0..layout.nodes.len() {
                for b in a + 1..layout.nodes.len() {
                    assert!(
                        !layout.nodes[a].overlaps(&layout.nodes[b]),
                        "{name}: boxes {a} and {b} overlap"
                    );
                }
            }
            for (e, edge) in graph.edges.iter().enumerate() {
                let wire = &layout.wires[e];
                assert_eq!(
                    wire.points[0], layout.nodes[edge.from].outputs[edge.from_pin],
                    "{name}: wire {e} does not start on its driver pin"
                );
                assert_eq!(
                    *wire.points.last().unwrap(),
                    layout.nodes[edge.to].inputs[edge.to_pin],
                    "{name}: wire {e} does not end on its driven pin"
                );
                for pair in wire.points.windows(2) {
                    assert!(
                        pair[0].x == pair[1].x || pair[0].y == pair[1].y,
                        "{name}: wire {e} has a diagonal segment"
                    );
                    for (n, node) in layout.nodes.iter().enumerate() {
                        assert!(
                            !crosses(node, pair[0], pair[1]),
                            "{name}: wire {e} crosses box {n}"
                        );
                    }
                }
            }
        }
    }
}

/// True when the segment `a`-`b` passes through the inside of `node`. The
/// boundary does not count: every wire starts and ends on one.
fn crosses(
    node: &reticle::viewer::Placed,
    a: reticle::viewer::Point,
    b: reticle::viewer::Point,
) -> bool {
    let (x0, x1) = (node.x, node.x + node.width);
    let (y0, y1) = (node.y, node.y + node.height);
    let (lox, hix) = (a.x.min(b.x), a.x.max(b.x));
    let (loy, hiy) = (a.y.min(b.y), a.y.max(b.y));
    lox < x1 && x0 < hix && loy < y1 && y0 < hiy
}

/// Every `href` of a page, in order.
fn hrefs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("href=\"") {
        rest = &rest[at + 6..];
        let Some(end) = rest.find('"') else { break };
        out.push(rest[..end].to_owned());
        rest = &rest[end..];
    }
    out
}

/// Resolves a relative link against the directory of the page holding it.
fn resolve(base: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    for part in target.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// The files already under a golden directory, as relative paths.
fn existing_files(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        let Ok(entries) = fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(rest) = path.strip_prefix(dir) {
                out.insert(rest.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out
}

/// Where two texts first differ, for a readable failure.
fn first_difference(expected: &str, actual: &str) -> String {
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e != a {
            return format!("line {}:\n  expected: {e}\n  actual:   {a}", i + 1);
        }
    }
    format!(
        "expected {} lines, got {}",
        expected.lines().count(),
        actual.lines().count()
    )
}
