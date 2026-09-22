//! The schematic page: a module's netlist as inline SVG.
//!
//! [`render`] lays the module's [`Graph`] out with [`super::layout`] and
//! writes the result as one `<svg>` inside a page that carries its own
//! script. Nothing is fetched and nothing is generated in the browser: the
//! geometry is in the file, and the script only pans, zooms, highlights
//! and searches it.
//!
//! What the drawing says:
//!
//! - A **combinational cell** is a light box with its kind in the heading
//!   and its name below.
//! - A **flip-flop, latch or memory port** is a heavier box in another
//!   colour with a clock mark at its clock pin, so state is what the eye
//!   lands on first.
//! - An **instance**, a **black box** or a **process** is a box with a
//!   thicker border and the module (or process kind) as its heading.
//! - A **constant** is a small triangle labelled with the literal.
//! - A **port** is a tag on the left or right edge of the canvas, pointed
//!   in the direction the value travels.
//! - A **bus** is drawn with a thicker stroke and its label carries the
//!   width after a slash, as `q /4`.
//!
//! Every wire and pin carries `data-net`, which is what the script
//! highlights on hover; every node carries the index of its detail panel,
//! so a click needs no name to be interpolated into a selector.

use super::html::{Crumb, Html, page};
use super::layout::{Layout, Point};
use super::{Graph, Links, NodeKind, Shape, ViewerOptions};
use crate::ir::{Design, ModuleId, PortDir};
use crate::source::SourceMap;

/// Renders the schematic page of one module.
pub fn render(
    design: &Design,
    map: &SourceMap,
    id: ModuleId,
    graph: &Graph,
    links: &Links,
    options: &ViewerOptions,
) -> String {
    let module = design.module(id);
    let layout = Layout::of_graph(graph, &options.layout);

    let mut h = Html::new();
    h.open("h1", &[]);
    h.text(module.name.as_str());
    h.element("span", &[("class", "kind")], " schematic");
    h.close();

    h.open("p", &[("class", "loc")]);
    let (text, href) = links.location(map, module.span);
    match href {
        Some(href) => h.element("a", &[("href", &href)], &text),
        None => h.text(&text),
    }
    if let Some(doc) = links.doc(id) {
        h.text(" \u{b7} ");
        h.element("a", &[("href", doc)], "documentation");
    }
    h.text(" \u{b7} ");
    h.element("a", &[("href", "index.html")], "index");
    h.close();

    toolbar(&mut h);

    h.open("div", &[("class", "canvas")]);
    h.open("div", &[("class", "frame")]);
    let view_box = format!("0 0 {} {}", layout.width, layout.height);
    h.open(
        "svg",
        &[
            ("class", "schematic"),
            ("id", "schematic"),
            ("viewBox", &view_box),
            ("preserveAspectRatio", "xMinYMin meet"),
            ("role", "img"),
        ],
    );
    h.element(
        "title",
        &[],
        &format!("schematic of module {}", module.name),
    );
    h.open("g", &[("id", "view")]);
    wires(&mut h, graph, &layout);
    nodes(&mut h, graph, &layout);
    h.close();
    h.close();
    h.close();
    details(&mut h, graph, map, links);
    h.close();

    legend(&mut h, graph);

    let crumbs: Vec<Crumb> = vec![
        ("index.html".to_owned(), "index".to_owned()),
        (String::new(), format!("{} (schematic)", module.name)),
    ];
    page(
        &format!("{} schematic", module.name),
        &crumbs,
        &h.finish(),
        true,
        true,
    )
}

/// The controls above the canvas.
fn toolbar(h: &mut Html) {
    h.open("div", &[("class", "toolbar")]);
    h.void(
        "input",
        &[
            ("type", "search"),
            ("id", "search"),
            ("placeholder", "find a cell or a net"),
            ("aria-label", "find a cell or a net"),
        ],
    );
    h.element(
        "button",
        &[("type", "button"), ("id", "zoom-out")],
        "\u{2212}",
    );
    h.element("button", &[("type", "button"), ("id", "zoom-in")], "+");
    h.element(
        "button",
        &[("type", "button"), ("id", "zoom-reset")],
        "reset",
    );
    h.element(
        "span",
        &[("class", "hint")],
        "drag to pan, wheel to zoom, hover a wire to trace it, click a cell for its details",
    );
    h.close();
}

/// The wires, and one label per driver pin.
fn wires(h: &mut Html, graph: &Graph, layout: &Layout) {
    h.open("g", &[("class", "wires")]);
    for (e, edge) in graph.edges.iter().enumerate() {
        let wire = &layout.wires[e];
        let class = if edge.width.is_some_and(|w| w > 1) {
            "wire bus"
        } else {
            "wire"
        };
        let points = points_attr(&wire.points);
        let mut attrs: Vec<(&str, &str)> = vec![("class", class), ("points", &points)];
        if edge.net.is_some() {
            attrs.push(("data-net", &edge.label));
        }
        h.void("polyline", &attrs);
    }
    // One label per driving pin: a net that fans out to six cells is
    // named once, not six times. A constant needs none, since its own
    // triangle already carries the literal.
    let mut labelled: Vec<(usize, usize)> = Vec::new();
    for edge in &graph.edges {
        if edge.net.is_none() || labelled.contains(&(edge.from, edge.from_pin)) {
            continue;
        }
        labelled.push((edge.from, edge.from_pin));
        let at = layout.nodes[edge.from].outputs[edge.from_pin];
        let text = match edge.width {
            Some(w) if w > 1 => format!("{} /{w}", edge.label),
            _ => edge.label.clone(),
        };
        // Cut the label to the room before the next box, so a long net
        // name is shortened rather than painted over a cell.
        let Some(text) = fit(&text, label_room(layout, at)) else {
            continue;
        };
        let (x, y) = (format!("{}", at.x + 7), format!("{}", at.y - 5));
        let mut attrs: Vec<(&str, &str)> = vec![("class", "label"), ("x", &x), ("y", &y)];
        if edge.net.is_some() {
            attrs.push(("data-net", &edge.label));
        }
        h.element("text", &attrs, &text);
    }
    h.close();
}

/// The horizontal room a wire label has at `at`, before the first box to
/// its right that shares the band the text occupies.
fn label_room(layout: &Layout, at: Point) -> i32 {
    let start = at.x + 7;
    // The text sits on the baseline `at.y - 5`, so it covers roughly the
    // nine units above the wire.
    let (top, bottom) = (at.y - 14, at.y - 4);
    layout
        .nodes
        .iter()
        .filter(|node| node.x > start && node.y < bottom && node.y + node.height > top)
        .map(|node| node.x - start)
        .min()
        .unwrap_or(i32::MAX)
}

/// Cuts `text` to what fits in `room` units of 10px monospaced type, or
/// returns `None` when there is not even room for an ellipsis.
fn fit(text: &str, room: i32) -> Option<String> {
    let chars = i32::try_from(text.chars().count()).unwrap_or(i32::MAX);
    if chars * 6 + 6 <= room {
        return Some(text.to_owned());
    }
    let keep = usize::try_from((room - 6) / 6).unwrap_or(0);
    if keep < 2 {
        return None;
    }
    let kept: String = text.chars().take(keep - 1).collect();
    Some(format!("{kept}\u{2026}"))
}

/// The boxes, their labels and their pins.
fn nodes(h: &mut Html, graph: &Graph, layout: &Layout) {
    h.open("g", &[("class", "nodes")]);
    for (id, node) in graph.nodes.iter().enumerate() {
        let placed = &layout.nodes[id];
        let centre = placed.centre();
        let (detail, search) = (format!("{id}"), node.search_text());
        let (cx, cy) = (format!("{}", centre.x), format!("{}", centre.y));
        h.open(
            "g",
            &[
                ("class", node.shape.class()),
                ("data-detail", &detail),
                ("data-search", &search),
                ("data-cx", &cx),
                ("data-cy", &cy),
            ],
        );
        body(h, node.shape, node.kind, placed);

        if node.shape == Shape::Const {
            let (x, y) = (
                format!("{}", placed.x + 2),
                format!("{}", placed.y + placed.height / 2 + 4),
            );
            h.element(
                "text",
                &[("class", "kind"), ("x", &x), ("y", &y)],
                &node.title,
            );
        } else {
            let x = format!("{}", placed.x + 9);
            let y = format!("{}", placed.y + 17);
            h.element(
                "text",
                &[("class", "kind"), ("x", &x), ("y", &y)],
                &node.title,
            );
            if !node.name.is_empty() {
                let y = format!("{}", placed.y + 28);
                h.element(
                    "text",
                    &[("class", "label"), ("x", &x), ("y", &y)],
                    &node.name,
                );
            }
        }

        // A port's pin name repeats the box's own heading, so it is left
        // off; everywhere else the pin name is what tells `a` from `b`.
        let show_pins = node.shape != Shape::Port && node.shape != Shape::Const;
        for (index, pin) in node.inputs.iter().enumerate() {
            let at = placed.inputs[index];
            dot(h, at, pin.net_name.as_deref());
            if is_clock(&pin.name) {
                let marks = points_attr(&[
                    Point {
                        x: at.x + 3,
                        y: at.y - 5,
                    },
                    Point {
                        x: at.x + 10,
                        y: at.y,
                    },
                    Point {
                        x: at.x + 3,
                        y: at.y + 5,
                    },
                ]);
                h.void("polyline", &[("class", "clkmark"), ("points", &marks)]);
            }
            if show_pins && !pin.name.is_empty() {
                let (x, y) = (
                    format!("{}", at.x + if is_clock(&pin.name) { 14 } else { 7 }),
                    format!("{}", at.y + 3),
                );
                h.element(
                    "text",
                    &[("class", "pinname"), ("x", &x), ("y", &y)],
                    &pin.name,
                );
            }
        }
        for (index, pin) in node.outputs.iter().enumerate() {
            let at = placed.outputs[index];
            dot(h, at, pin.net_name.as_deref());
            if show_pins && !pin.name.is_empty() {
                let (x, y) = (format!("{}", at.x - 7), format!("{}", at.y + 3));
                h.element(
                    "text",
                    &[
                        ("class", "pinname"),
                        ("x", &x),
                        ("y", &y),
                        ("text-anchor", "end"),
                    ],
                    &pin.name,
                );
            }
        }
        h.close();
    }
    h.close();
}

/// The outline of one node.
fn body(h: &mut Html, shape: Shape, kind: NodeKind, placed: &super::Placed) {
    let (x, y, w, hh) = (placed.x, placed.y, placed.width, placed.height);
    match shape {
        Shape::Const => {
            let tip = points_attr(&[
                Point { x: x + w - 20, y },
                Point {
                    x: x + w,
                    y: y + hh / 2,
                },
                Point {
                    x: x + w - 20,
                    y: y + hh,
                },
            ]);
            h.void("polygon", &[("class", "box"), ("points", &tip)]);
        }
        Shape::Port => {
            let dir = match kind {
                NodeKind::Port { dir, .. } => dir,
                _ => PortDir::In,
            };
            let left_flat = dir == PortDir::In;
            let mut corners = Vec::new();
            if left_flat {
                corners.push(Point { x, y });
            } else {
                corners.push(Point { x: x + 11, y });
            }
            corners.push(Point { x: x + w - 11, y });
            corners.push(Point {
                x: x + w,
                y: y + hh / 2,
            });
            corners.push(Point {
                x: x + w - 11,
                y: y + hh,
            });
            if left_flat {
                corners.push(Point { x, y: y + hh });
            } else {
                corners.push(Point {
                    x: x + 11,
                    y: y + hh,
                });
                corners.push(Point { x, y: y + hh / 2 });
            }
            let tag = points_attr(&corners);
            h.void("polygon", &[("class", "box"), ("points", &tag)]);
        }
        Shape::Comb | Shape::Seq | Shape::Instance => {
            let (xs, ys) = (format!("{x}"), format!("{y}"));
            let (ws, hs) = (format!("{w}"), format!("{hh}"));
            h.void(
                "rect",
                &[
                    ("class", "box"),
                    ("x", &xs),
                    ("y", &ys),
                    ("width", &ws),
                    ("height", &hs),
                    ("rx", "4"),
                ],
            );
        }
    }
}

/// One pin dot.
fn dot(h: &mut Html, at: Point, net: Option<&str>) {
    let (cx, cy) = (format!("{}", at.x), format!("{}", at.y));
    let mut attrs: Vec<(&str, &str)> = vec![("class", "pin"), ("cx", &cx), ("cy", &cy), ("r", "3")];
    if let Some(net) = net {
        attrs.push(("data-net", net));
    }
    h.void("circle", &attrs);
}

/// True for the pin names that deserve a clock mark.
fn is_clock(name: &str) -> bool {
    name == "clk" || name == "trigger"
}

/// The panel a click fills in.
fn details(h: &mut Html, graph: &Graph, map: &SourceMap, links: &Links) {
    h.open("aside", &[("class", "info"), ("id", "info")]);
    h.element(
        "p",
        &[("id", "info-empty"), ("class", "none")],
        "Click a cell to see its parameters and where it came from.",
    );
    for (id, node) in graph.nodes.iter().enumerate() {
        let pane = format!("d{id}");
        h.open(
            "div",
            &[("class", "detail"), ("id", &pane), ("hidden", "hidden")],
        );
        let heading = if node.name.is_empty() {
            node.title.clone()
        } else {
            node.name.clone()
        };
        h.element("h3", &[], &heading);
        h.open("dl", &[]);
        h.element("dt", &[], "kind");
        h.element("dd", &[], &node.title);
        for (label, value) in &node.details {
            h.element("dt", &[], label);
            h.element("dd", &[], value);
        }
        for pin in &node.inputs {
            h.element("dt", &[], &format!("in {}", pin.name));
            h.element("dd", &[], &pin.label);
        }
        for pin in &node.outputs {
            h.element("dt", &[], &format!("out {}", pin.name));
            h.element("dd", &[], &pin.label);
        }
        h.close();
        h.open("p", &[("class", "loc")]);
        let (text, href) = links.location(map, node.span);
        match href {
            Some(href) => h.element("a", &[("href", &href)], &text),
            None => h.text(&text),
        }
        h.close();
        h.close();
    }
    h.close();
}

/// What the colours mean, and how big the drawing is.
fn legend(h: &mut Html, graph: &Graph) {
    let cells = graph
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Cell(_)))
        .count();
    let state = graph.nodes.iter().filter(|n| n.is_sequential()).count();
    h.open("p", &[("class", "legend")]);
    h.element("span", &[("class", "l-comb")], "combinational");
    h.element("span", &[("class", "l-seq")], "state");
    h.element("span", &[("class", "l-inst")], "instance or process");
    h.element("span", &[("class", "l-port")], "port");
    h.element(
        "span",
        &[],
        &format!(
            "{cells} cell{}, {state} holding state, {} wire{}",
            if cells == 1 { "" } else { "s" },
            graph.edges.len(),
            if graph.edges.len() == 1 { "" } else { "s" }
        ),
    );
    h.close();
}

/// An SVG `points` attribute.
fn points_attr(points: &[Point]) -> String {
    let mut out = String::with_capacity(points.len() * 10);
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{},{}", p.x, p.y));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer::{Comments, LayoutOptions};

    fn rendered(text: &str) -> String {
        let mut map = SourceMap::new();
        let file = map.add("t.rtl", text).unwrap();
        let design = Design::parse_text(text, file).unwrap();
        let id = design.modules.ids().next().unwrap();
        let graph = Graph::of_module(&design, id);
        let options = ViewerOptions {
            comments: Comments::new(),
            layout: LayoutOptions::default(),
            ..ViewerOptions::default()
        };
        render(&design, &map, id, &graph, &Links::default(), &options)
    }

    #[test]
    fn draws_cells_wires_and_ports() {
        let html = rendered(include_str!("../../testdata/ir/netlist.rtl"));
        assert!(html.contains("<svg class=\"schematic\""));
        assert!(html.contains("class=\"node seq\""));
        assert!(html.contains("class=\"node port\""));
        assert!(html.contains("class=\"wire bus\""));
        assert!(html.contains("data-net=\"q\""));
        // The bus label carries its width.
        assert!(html.contains("q /4"));
        assert!(html.contains("<polygon class=\"box\""));
    }

    #[test]
    fn a_net_named_like_a_tag_stays_text() {
        let text = "module m\n  net %<script> u1 wire\n  net %y u1 wire\n  \
                    port y out %y\n  cell c0 not (a=%<script>) -> (y=%y)\nend\n";
        let mut map = SourceMap::new();
        let file = map.add("t.rtl", text).unwrap();
        let Ok(design) = Design::parse_text(text, file) else {
            // The text format may not accept the name; the escaping is
            // then covered by `html::escape` alone.
            return;
        };
        let id = design.modules.ids().next().unwrap();
        let graph = Graph::of_module(&design, id);
        let html = render(
            &design,
            &map,
            id,
            &graph,
            &Links::default(),
            &ViewerOptions::default(),
        );
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn points_render_as_pairs() {
        assert_eq!(
            points_attr(&[Point { x: 1, y: 2 }, Point { x: 3, y: -4 }]),
            "1,2 3,-4"
        );
        assert_eq!(points_attr(&[]), "");
    }
}
