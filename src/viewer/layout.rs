//! Laying the graph out: layering, ordering, placement and wire routing.
//!
//! The pipeline is the classic layered one, with every step kept small and
//! integer-only so the same design always produces the same bytes:
//!
//! 1. **Layering.** [`rank_nodes`] gives each node the length of the
//!    longest path reaching it, by the same memoised walk the synthesis
//!    depth report uses, with a *visiting* mark so a feedback loop breaks
//!    the path instead of hanging. Input ports are pinned to rank 0 and
//!    output ports to the last column, since the ports belong on the edges
//!    of the canvas; a constant is pulled to just before the pin it feeds,
//!    so literals sit next to their consumer rather than in a column of
//!    their own.
//! 2. **Dummies.** An edge spanning more than one rank gets a dummy node
//!    per rank it crosses. The dummy takes a slot in that column, so the
//!    wire has somewhere to pass that no cell body occupies, and the
//!    ordering step sees long edges too.
//! 3. **Ordering.** Four sweeps of the barycentre heuristic, down then up,
//!    each sorting a rank by the mean position of its neighbours in the
//!    previous one. A node with no neighbour in that direction keeps its
//!    place, and ties break on the current position, so the result is a
//!    deterministic function of the graph.
//! 4. **Placement.** Column width is the widest node in it; rows stack
//!    with a fixed gap, columns are centred against the tallest, and a
//!    second pass pulls each dummy towards the straight line between the
//!    pins its edge joins, clamped so it never overlaps its neighbours.
//! 5. **Routing.** Wires are orthogonal polylines. Every vertical segment
//!    lives in the gutter between two columns, where there are no cell
//!    bodies, and gutters are channels: segments are sorted and assigned
//!    to the first track whose intervals they do not overlap, which fixes
//!    both the gutter's width and each wire's x. An edge that points
//!    backwards (register feedback) leaves through the gutter after its
//!    source, runs along a lane below the drawing and comes back up the
//!    gutter before its target.
//!
//! Coordinates are `i32` SVG user units throughout: no floating point, so
//! nothing depends on rounding and the golden files are stable.

use std::collections::BTreeMap;

use super::graph::{Graph, NodeId, NodeKind, Shape};
use crate::ir::PortDir;

/// A point in the canvas, in SVG user units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    /// Distance from the left edge.
    pub x: i32,
    /// Distance from the top edge.
    pub y: i32,
}

/// The sizes the layout is built from.
///
/// They are chosen for the 12px monospaced type the style sheet uses;
/// changing one changes every golden file, which is the point of keeping
/// them in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutOptions {
    /// Narrowest a node box may be.
    pub min_width: i32,
    /// Widest a node box may be.
    pub max_width: i32,
    /// Shortest a node box may be.
    pub min_height: i32,
    /// Vertical space above the first pin, holding the kind and the name.
    pub header: i32,
    /// Vertical distance between pins.
    pub pin_pitch: i32,
    /// Vertical gap between two boxes of the same column.
    pub row_gap: i32,
    /// Horizontal distance between two wire tracks of one gutter.
    pub track_pitch: i32,
    /// Narrowest a gutter may be.
    pub min_gutter: i32,
    /// Blank border around the whole drawing.
    pub margin: i32,
    /// Vertical distance between two feedback lanes.
    pub lane_pitch: i32,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        LayoutOptions {
            min_width: 96,
            max_width: 260,
            min_height: 44,
            header: 30,
            pin_pitch: 18,
            row_gap: 22,
            track_pitch: 12,
            min_gutter: 40,
            margin: 28,
            lane_pitch: 14,
        }
    }
}

/// Where one node ended up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    /// The column, counted from the inputs.
    pub rank: usize,
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Box width.
    pub width: i32,
    /// Box height.
    pub height: i32,
    /// Where each input pin meets the box, top to bottom.
    pub inputs: Vec<Point>,
    /// Where each output pin leaves the box, top to bottom.
    pub outputs: Vec<Point>,
}

impl Placed {
    /// The centre of the box, which the search box scrolls to.
    pub fn centre(&self) -> Point {
        Point {
            x: self.x + self.width / 2,
            y: self.y + self.height / 2,
        }
    }

    /// True when this box and `other` share any area.
    pub fn overlaps(&self, other: &Placed) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
}

/// One routed wire: the polyline from a driver pin to a driven pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wire {
    /// The corners, starting at the driving pin and ending at the driven
    /// one. Always at least two points, and every segment is horizontal
    /// or vertical.
    pub points: Vec<Point>,
}

/// A laid-out graph: a box per node, a polyline per edge, and the canvas
/// they need.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// One entry per node of the graph, in the same order.
    pub nodes: Vec<Placed>,
    /// One entry per edge of the graph, in the same order.
    pub wires: Vec<Wire>,
    /// Canvas width.
    pub width: i32,
    /// Canvas height.
    pub height: i32,
    /// Number of columns.
    pub columns: usize,
}

/// Converts a count to canvas units; counts come from a design that fits
/// in memory, so the fallback is unreachable in practice.
fn units(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX / 4096)
}

/// The edges that close a cycle, as indices into `graph.edges`.
///
/// A depth-first walk over the successors marks an edge whose target is
/// still on the stack: that is the edge that has to be ignored for the
/// graph to be a DAG. Roots without a driver are tried first, so on a
/// normal netlist the walk starts at the inputs and the back edges are
/// the register feedback a reader expects to see pointing left.
pub fn back_edges(graph: &Graph) -> Vec<bool> {
    let n = graph.nodes.len();
    let mut out = vec![false; graph.edges.len()];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (e, edge) in graph.edges.iter().enumerate() {
        succ[edge.from].push(e);
    }
    let mut indegree = vec![0usize; n];
    for edge in &graph.edges {
        indegree[edge.to] += 1;
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let mut mark = vec![Mark::New; n];
    let roots = (0..n)
        .filter(|&i| indegree[i] == 0)
        .chain(0..n)
        .collect::<Vec<_>>();
    // Each frame is a node plus how many of its out-edges are done.
    let mut stack: Vec<(NodeId, usize)> = Vec::new();
    for root in roots {
        if mark[root] != Mark::New {
            continue;
        }
        mark[root] = Mark::Open;
        stack.push((root, 0));
        while let Some(&mut (node, ref mut at)) = stack.last_mut() {
            if *at == succ[node].len() {
                mark[node] = Mark::Done;
                stack.pop();
                continue;
            }
            let e = succ[node][*at];
            *at += 1;
            let next = graph.edges[e].to;
            match mark[next] {
                Mark::Open => out[e] = true,
                Mark::Done => {}
                Mark::New => {
                    mark[next] = Mark::Open;
                    stack.push((next, 0));
                }
            }
        }
    }
    out
}

/// The rank of every node: the longest path from a node with no driver.
///
/// Input ports are pinned to the first column and output (and
/// bidirectional) ports to the last, so the interface frames the drawing.
/// The edges [`back_edges`] found are left out, so a feedback loop shortens
/// the path rather than hanging the walk — the same treatment
/// `synth::report` gives a combinational loop when it estimates depth.
pub fn rank_nodes(graph: &Graph) -> Vec<usize> {
    let n = graph.nodes.len();
    let back = back_edges(graph);
    let mut preds: Vec<Vec<NodeId>> = vec![Vec::new(); n];
    for (e, edge) in graph.edges.iter().enumerate() {
        if !back[e] {
            preds[edge.to].push(edge.from);
        }
    }

    let pinned_first = |i: NodeId| {
        matches!(
            graph.nodes[i].kind,
            NodeKind::Port {
                dir: PortDir::In,
                ..
            }
        )
    };
    let pinned_last = |i: NodeId| {
        matches!(
            graph.nodes[i].kind,
            NodeKind::Port {
                dir: PortDir::Out | PortDir::InOut,
                ..
            }
        )
    };

    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Visiting,
        Done,
    }
    let mut mark = vec![Mark::New; n];
    let mut rank = vec![0usize; n];
    let mut stack: Vec<(NodeId, bool)> = Vec::new();
    for start in 0..n {
        if mark[start] != Mark::New {
            continue;
        }
        stack.push((start, false));
        while let Some((node, returning)) = stack.pop() {
            if returning {
                let mut best = 0;
                if !pinned_first(node) {
                    for &pred in &preds[node] {
                        if mark[pred] == Mark::Done {
                            best = best.max(rank[pred] + 1);
                        }
                    }
                }
                rank[node] = best;
                mark[node] = Mark::Done;
                continue;
            }
            match mark[node] {
                // Done, or an ancestor of itself: a loop, which stops here.
                Mark::Done | Mark::Visiting => continue,
                Mark::New => {}
            }
            mark[node] = Mark::Visiting;
            stack.push((node, true));
            for &pred in &preds[node] {
                if mark[pred] == Mark::New {
                    stack.push((pred, false));
                }
            }
        }
    }

    // The interface frames the drawing: outputs go one column past
    // everything else.
    let last = (0..n)
        .filter(|&i| !pinned_last(i))
        .map(|i| rank[i])
        .max()
        .unwrap_or(0);
    for (i, at) in rank.iter_mut().enumerate() {
        if pinned_last(i) {
            *at = last + 1;
        }
    }

    // A literal belongs beside the pin it feeds, not in the input column.
    let constants: Vec<(NodeId, usize)> = (0..n)
        .filter(|&i| graph.nodes[i].kind == NodeKind::Const)
        .filter_map(|i| {
            let edge = graph.edges.iter().find(|e| e.from == i)?;
            Some((i, rank[edge.to].saturating_sub(1)))
        })
        .collect();
    for (node, at) in constants {
        rank[node] = at;
    }
    rank
}

/// One slot of a column: either a node, or a point an edge passes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    /// A node of the graph.
    Real(NodeId),
    /// A bend point of edge `edge`, the `seq`-th of its chain.
    Dummy { edge: usize, seq: usize },
}

/// The columns, each an ordered list of slots, plus the chain links that
/// the ordering sweeps follow.
struct Layered {
    /// Slot descriptions, indexed by slot id.
    items: Vec<Item>,
    /// The rank of each slot.
    rank: Vec<usize>,
    /// Slot ids per column, in drawing order.
    columns: Vec<Vec<usize>>,
    /// Slots one column to the right that each slot links to.
    succ: Vec<Vec<usize>>,
    /// Slots one column to the left that each slot links from.
    pred: Vec<Vec<usize>>,
    /// The chain of slots each edge runs through, source first; empty for
    /// an edge that points backwards.
    chains: Vec<Vec<usize>>,
}

impl Layered {
    fn build(graph: &Graph, ranks: &[usize]) -> Layered {
        let columns_count = ranks.iter().copied().max().unwrap_or(0) + 1;
        let mut layered = Layered {
            items: Vec::new(),
            rank: Vec::new(),
            columns: vec![Vec::new(); columns_count],
            succ: Vec::new(),
            pred: Vec::new(),
            chains: vec![Vec::new(); graph.edges.len()],
        };
        let mut slot_of_node = vec![usize::MAX; graph.nodes.len()];
        for node in 0..graph.nodes.len() {
            slot_of_node[node] = layered.push(Item::Real(node), ranks[node]);
        }
        for (e, edge) in graph.edges.iter().enumerate() {
            let (from, to) = (ranks[edge.from], ranks[edge.to]);
            if to <= from {
                // Backwards or within one column: routed through a lane
                // below the drawing, and left out of the ordering.
                continue;
            }
            let mut chain = vec![slot_of_node[edge.from]];
            for (seq, rank) in (from + 1..to).enumerate() {
                chain.push(layered.push(Item::Dummy { edge: e, seq }, rank));
            }
            chain.push(slot_of_node[edge.to]);
            for pair in chain.windows(2) {
                layered.succ[pair[0]].push(pair[1]);
                layered.pred[pair[1]].push(pair[0]);
            }
            layered.chains[e] = chain;
        }
        layered
    }

    fn push(&mut self, item: Item, rank: usize) -> usize {
        let id = self.items.len();
        self.items.push(item);
        self.rank.push(rank);
        self.succ.push(Vec::new());
        self.pred.push(Vec::new());
        self.columns[rank].push(id);
        id
    }

    /// The position of each slot inside its column.
    fn positions(&self) -> Vec<usize> {
        let mut pos = vec![0usize; self.items.len()];
        for column in &self.columns {
            for (at, &slot) in column.iter().enumerate() {
                pos[slot] = at;
            }
        }
        pos
    }

    /// Reduces crossings with alternating barycentre sweeps.
    fn order(&mut self, sweeps: usize) {
        for sweep in 0..sweeps {
            let down = sweep % 2 == 0;
            let range: Vec<usize> = if down {
                (1..self.columns.len()).collect()
            } else {
                (0..self.columns.len().saturating_sub(1)).rev().collect()
            };
            for rank in range {
                let pos = self.positions();
                let column = std::mem::take(&mut self.columns[rank]);
                // Scaled so the mean stays an integer comparison, with the
                // current position as the tie-break and as the answer for
                // a slot with no neighbour on that side.
                let mut keyed: Vec<((i64, usize), usize)> = column
                    .into_iter()
                    .map(|slot| {
                        let neighbours = if down {
                            &self.pred[slot]
                        } else {
                            &self.succ[slot]
                        };
                        let key = if neighbours.is_empty() {
                            (i64::from(units(pos[slot])) * 1024, pos[slot])
                        } else {
                            let sum: i64 =
                                neighbours.iter().map(|&n| i64::from(units(pos[n]))).sum();
                            let count = i64::from(units(neighbours.len()));
                            (sum * 1024 / count, pos[slot])
                        };
                        (key, slot)
                    })
                    .collect();
                keyed.sort_by_key(|entry| entry.0);
                self.columns[rank] = keyed.into_iter().map(|(_, slot)| slot).collect();
            }
        }
    }
}

impl Layout {
    /// Lays `graph` out with the given sizes.
    pub fn of_graph(graph: &Graph, options: &LayoutOptions) -> Layout {
        let ranks = rank_nodes(graph);
        let mut layered = Layered::build(graph, &ranks);
        layered.order(4);
        place(graph, &ranks, &layered, options)
    }

    /// The order the nodes ended up in, column by column, top to bottom.
    ///
    /// Constants and dummy bend points are left out, so this is what a
    /// reader sees.
    pub fn column_order(&self, graph: &Graph) -> Vec<Vec<NodeId>> {
        let mut columns: Vec<Vec<(i32, NodeId)>> = vec![Vec::new(); self.columns];
        for (id, placed) in self.nodes.iter().enumerate() {
            if graph.nodes[id].kind == NodeKind::Const {
                continue;
            }
            columns[placed.rank].push((placed.y, id));
        }
        columns
            .into_iter()
            .map(|mut column| {
                column.sort_unstable();
                column.into_iter().map(|(_, id)| id).collect()
            })
            .collect()
    }
}

/// The size of one node's box.
fn size_of(graph: &Graph, node: NodeId, options: &LayoutOptions) -> (i32, i32) {
    let info = &graph.nodes[node];
    if info.shape == Shape::Const {
        let label = units(info.title.chars().count()) * 6;
        return (label + 30, 22);
    }
    let rows = info.inputs.len().max(info.outputs.len());
    let height = (options.header + units(rows) * options.pin_pitch + 10).max(options.min_height);
    let title = units(info.title.chars().count()) * 7 + 22;
    let name = units(info.name.chars().count()) * 6 + 22;
    let longest = |pins: &[super::graph::Pin]| {
        pins.iter()
            .map(|p| units(p.name.chars().count()))
            .max()
            .unwrap_or(0)
    };
    let pins = (longest(&info.inputs) + longest(&info.outputs)) * 6 + 34;
    let width = title
        .max(name)
        .max(pins)
        .clamp(options.min_width, options.max_width);
    (width, height)
}

/// Where a pin meets its box, relative to the box's top.
fn pin_offset(
    graph: &Graph,
    node: NodeId,
    index: usize,
    height: i32,
    options: &LayoutOptions,
) -> i32 {
    if graph.nodes[node].shape == Shape::Const {
        return height / 2;
    }
    options.header + units(index) * options.pin_pitch + options.pin_pitch / 2
}

/// Places the slots and routes the wires.
fn place(graph: &Graph, ranks: &[usize], layered: &Layered, options: &LayoutOptions) -> Layout {
    let columns = layered.columns.len();
    let mut size: Vec<(i32, i32)> = Vec::with_capacity(graph.nodes.len());
    for node in 0..graph.nodes.len() {
        size.push(size_of(graph, node, options));
    }

    // --- vertical placement -------------------------------------------
    let slot_height = |slot: usize| match layered.items[slot] {
        Item::Real(node) => size[node].1,
        Item::Dummy { .. } => 6,
    };
    let mut y = vec![0i32; layered.items.len()];
    let mut column_height = vec![0i32; columns];
    for (rank, column) in layered.columns.iter().enumerate() {
        let mut cursor = 0;
        for &slot in column {
            y[slot] = cursor;
            cursor += slot_height(slot) + options.row_gap;
        }
        column_height[rank] = (cursor - options.row_gap).max(0);
    }
    let tallest = column_height.iter().copied().max().unwrap_or(0);
    for (rank, column) in layered.columns.iter().enumerate() {
        let offset = options.margin + (tallest - column_height[rank]) / 2;
        for &slot in column {
            y[slot] += offset;
        }
    }

    // Pull each bend point towards the straight line between the pins its
    // edge joins, then restack the column so nothing overlaps.
    let pin_y = |node: NodeId, index: usize, y: &[i32], slot_of: &[usize]| {
        let slot = slot_of[node];
        let (_, height) = size[node];
        y[slot] + pin_offset(graph, node, index, height, options)
    };
    let mut slot_of_node = vec![0usize; graph.nodes.len()];
    for (slot, item) in layered.items.iter().enumerate() {
        if let Item::Real(node) = item {
            slot_of_node[*node] = slot;
        }
    }
    let mut desired = y.clone();
    for (e, chain) in layered.chains.iter().enumerate() {
        if chain.len() < 3 {
            continue;
        }
        let edge = &graph.edges[e];
        let start = pin_y(edge.from, edge.from_pin, &y, &slot_of_node);
        let end = pin_y(edge.to, edge.to_pin, &y, &slot_of_node);
        let steps = units(chain.len() - 1);
        for (step, &slot) in chain.iter().enumerate().skip(1).take(chain.len() - 2) {
            let at = units(step);
            desired[slot] = start + (end - start) * at / steps - 3;
        }
    }
    for column in &layered.columns {
        let mut cursor = options.margin;
        for &slot in column {
            y[slot] = desired[slot].max(cursor);
            cursor = y[slot] + slot_height(slot) + options.row_gap;
        }
    }

    // --- feedback lanes ------------------------------------------------
    // Lanes are assigned over column spans, not pixels, so they are known
    // before the horizontal placement that depends on them.
    let bottom = (0..layered.items.len())
        .map(|slot| y[slot] + slot_height(slot))
        .max()
        .unwrap_or(options.margin);
    let mut lane_spans: Vec<(usize, usize)> = Vec::new();
    let mut lane_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (e, edge) in graph.edges.iter().enumerate() {
        if !layered.chains[e].is_empty() {
            continue;
        }
        let (from, to) = (ranks[edge.from], ranks[edge.to]);
        let span = (to.min(from), from.max(to) + 1);
        let lane = lane_spans
            .iter()
            .position(|&(_, end)| span.0 > end)
            .unwrap_or(lane_spans.len());
        if lane == lane_spans.len() {
            lane_spans.push(span);
        } else {
            lane_spans[lane].1 = lane_spans[lane].1.max(span.1);
        }
        lane_of.insert(e, lane);
    }
    let lane_y = |lane: usize| bottom + options.row_gap + units(lane) * options.lane_pitch;

    // --- channels ------------------------------------------------------
    // One gutter before each column, plus one after the last. Every
    // vertical segment of every wire is assigned a track in its gutter.
    let gutters = columns + 1;
    let mut segments: Vec<Vec<(i32, i32, usize)>> = vec![Vec::new(); gutters];
    // Keyed by (edge, step) so the track can be looked up while routing.
    let mut track_of: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut keys: Vec<(usize, usize)> = Vec::new();
    /// Records a vertical run in a gutter, unless it is degenerate: a
    /// wire that comes out level with its target needs no track.
    fn register(
        segments: &mut [Vec<(i32, i32, usize)>],
        keys: &mut Vec<(usize, usize)>,
        gutter: usize,
        a: i32,
        b: i32,
        key: (usize, usize),
    ) {
        if a == b {
            return;
        }
        let id = keys.len();
        keys.push(key);
        segments[gutter].push((a.min(b), a.max(b), id));
    }
    for (e, edge) in graph.edges.iter().enumerate() {
        let chain = &layered.chains[e];
        if chain.is_empty() {
            let Some(&lane) = lane_of.get(&e) else {
                continue;
            };
            let start = pin_y(edge.from, edge.from_pin, &y, &slot_of_node);
            let end = pin_y(edge.to, edge.to_pin, &y, &slot_of_node);
            let at = lane_y(lane);
            register(
                &mut segments,
                &mut keys,
                ranks[edge.from] + 1,
                start,
                at,
                (e, 0),
            );
            register(&mut segments, &mut keys, ranks[edge.to], at, end, (e, 1));
            continue;
        }
        let mut previous = pin_y(edge.from, edge.from_pin, &y, &slot_of_node);
        for (step, &slot) in chain.iter().enumerate().skip(1) {
            let next = if step + 1 == chain.len() {
                pin_y(edge.to, edge.to_pin, &y, &slot_of_node)
            } else {
                y[slot] + 3
            };
            register(
                &mut segments,
                &mut keys,
                layered.rank[slot],
                previous,
                next,
                (e, step),
            );
            previous = next;
        }
    }
    let mut tracks = vec![0usize; gutters];
    for (gutter, list) in segments.iter_mut().enumerate() {
        list.sort_unstable();
        // Greedy interval colouring: the first track whose last segment
        // ended far enough above takes this one.
        let mut last: Vec<i32> = Vec::new();
        for &(lo, hi, id) in list.iter() {
            let track = last
                .iter()
                .position(|&end| end + 8 < lo)
                .unwrap_or(last.len());
            if track == last.len() {
                last.push(hi);
            } else {
                last[track] = hi;
            }
            track_of.insert(keys[id], track);
        }
        tracks[gutter] = last.len();
    }

    // --- horizontal placement -----------------------------------------
    let mut column_width = vec![0i32; columns];
    for (node, placed) in size.iter().enumerate() {
        let rank = ranks[node];
        column_width[rank] = column_width[rank].max(placed.0);
    }
    let mut gutter_x = vec![0i32; gutters];
    let mut column_x = vec![0i32; columns];
    let mut cursor = options.margin;
    for rank in 0..columns {
        gutter_x[rank] = cursor;
        cursor += gutter_width(tracks[rank], options);
        column_x[rank] = cursor;
        cursor += column_width[rank];
    }
    gutter_x[columns] = cursor;
    cursor += gutter_width(tracks[columns], options);
    let width = cursor + options.margin;
    let track_x =
        |gutter: usize, track: usize| gutter_x[gutter] + 12 + units(track) * options.track_pitch;

    // --- boxes ---------------------------------------------------------
    let mut nodes = Vec::with_capacity(graph.nodes.len());
    for node in 0..graph.nodes.len() {
        let (w, h) = size[node];
        let slot = slot_of_node[node];
        let (x, top) = (column_x[ranks[node]], y[slot]);
        let anchors = |count: usize, at: i32| -> Vec<Point> {
            (0..count)
                .map(|index| Point {
                    x: at,
                    y: top + pin_offset(graph, node, index, h, options),
                })
                .collect()
        };
        nodes.push(Placed {
            rank: ranks[node],
            x,
            y: top,
            width: w,
            height: h,
            inputs: anchors(graph.nodes[node].inputs.len(), x),
            outputs: anchors(graph.nodes[node].outputs.len(), x + w),
        });
    }

    // --- wires ---------------------------------------------------------
    let mut wires = Vec::with_capacity(graph.edges.len());
    for (e, edge) in graph.edges.iter().enumerate() {
        let start = nodes[edge.from].outputs[edge.from_pin];
        let end = nodes[edge.to].inputs[edge.to_pin];
        let mut points = vec![start];
        let chain = &layered.chains[e];
        if chain.is_empty() {
            let lane = lane_y(lane_of.get(&e).copied().unwrap_or(0));
            let out_gutter = ranks[edge.from] + 1;
            let in_gutter = ranks[edge.to];
            let ax = track_of
                .get(&(e, 0))
                .map_or_else(|| track_x(out_gutter, 0), |&t| track_x(out_gutter, t));
            let bx = track_of
                .get(&(e, 1))
                .map_or_else(|| track_x(in_gutter, 0), |&t| track_x(in_gutter, t));
            points.push(Point { x: ax, y: start.y });
            points.push(Point { x: ax, y: lane });
            points.push(Point { x: bx, y: lane });
            points.push(Point { x: bx, y: end.y });
        } else {
            let mut previous = start.y;
            for (step, &slot) in chain.iter().enumerate().skip(1) {
                let last = step + 1 == chain.len();
                let next = if last { end.y } else { y[slot] + 3 };
                if let Some(&track) = track_of.get(&(e, step)) {
                    let at = track_x(layered.rank[slot], track);
                    points.push(Point { x: at, y: previous });
                    points.push(Point { x: at, y: next });
                }
                if !last {
                    points.push(Point {
                        x: column_x[layered.rank[slot]],
                        y: next,
                    });
                }
                previous = next;
            }
        }
        points.push(end);
        wires.push(Wire {
            points: simplify(points),
        });
    }

    let height = wires
        .iter()
        .flat_map(|w| w.points.iter().map(|p| p.y))
        .chain(nodes.iter().map(|n| n.y + n.height))
        .max()
        .unwrap_or(options.margin)
        + options.margin;
    Layout {
        nodes,
        wires,
        width,
        height,
        columns,
    }
}

/// The width a gutter needs for its tracks.
fn gutter_width(tracks: usize, options: &LayoutOptions) -> i32 {
    if tracks == 0 {
        return options.min_gutter;
    }
    (units(tracks - 1) * options.track_pitch + 24).max(options.min_gutter)
}

/// Drops repeated points and merges runs that carry on in one direction,
/// so a straight wire is two points rather than six.
fn simplify(points: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for point in points {
        if out.last() == Some(&point) {
            continue;
        }
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let collinear = (a.x == b.x && b.x == point.x) || (a.y == b.y && b.y == point.y);
            if collinear {
                out.pop();
            }
        }
        out.push(point);
    }
    if out.len() == 1 {
        out.push(out[0]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::graph::{Edge, Node, Pin};
    use super::*;
    use crate::ir::{CellId, NetId, Span};
    use crate::source::{SourceId, SourceMap};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id: SourceId = map.add("viewer-test", "").unwrap();
        Span::new(id, 0, 0)
    }

    fn node(name: &str, inputs: usize, outputs: usize) -> Node {
        let pin = |i: usize| Pin {
            name: format!("p{i}"),
            label: format!("{name}{i}"),
            net: None,
            net_name: None,
            width: Some(1),
        };
        Node {
            kind: NodeKind::Cell(CellId(0)),
            shape: Shape::Comb,
            title: "and".to_owned(),
            name: name.to_owned(),
            inputs: (0..inputs).map(pin).collect(),
            outputs: (0..outputs).map(pin).collect(),
            span: span(),
            details: Vec::new(),
        }
    }

    fn edge(from: NodeId, from_pin: usize, to: NodeId, to_pin: usize) -> Edge {
        Edge {
            from,
            from_pin,
            to,
            to_pin,
            net: Some(NetId(0)),
            label: "n".to_owned(),
            width: Some(1),
        }
    }

    /// `a -> b -> c -> d`: every node one column further right.
    fn chain() -> Graph {
        Graph {
            nodes: vec![
                node("a", 1, 1),
                node("b", 1, 1),
                node("c", 1, 1),
                node("d", 1, 1),
            ],
            edges: vec![edge(0, 0, 1, 0), edge(1, 0, 2, 0), edge(2, 0, 3, 0)],
        }
    }

    /// `a` fans out to `b` and `c`, both feeding `d`.
    fn diamond() -> Graph {
        Graph {
            nodes: vec![
                node("a", 1, 1),
                node("b", 1, 1),
                node("c", 1, 1),
                node("d", 2, 1),
            ],
            edges: vec![
                edge(0, 0, 1, 0),
                edge(0, 0, 2, 0),
                edge(1, 0, 3, 0),
                edge(2, 0, 3, 1),
            ],
        }
    }

    #[test]
    fn a_chain_ranks_monotonically() {
        let ranks = rank_nodes(&chain());
        assert_eq!(ranks, [0, 1, 2, 3]);
    }

    #[test]
    fn a_diamond_puts_both_middles_on_one_rank() {
        let ranks = rank_nodes(&diamond());
        assert_eq!(ranks, [0, 1, 1, 2]);
    }

    #[test]
    fn a_cycle_does_not_hang() {
        let mut graph = chain();
        graph.edges.push(edge(3, 0, 0, 0));
        let ranks = rank_nodes(&graph);
        // The back edge is ignored, so the chain still ranks in order.
        assert_eq!(ranks, [0, 1, 2, 3]);
        // A two-node loop resolves too, and laying it out terminates.
        let loop_graph = Graph {
            nodes: vec![node("a", 1, 1), node("b", 1, 1)],
            edges: vec![edge(0, 0, 1, 0), edge(1, 0, 0, 0)],
        };
        assert_eq!(rank_nodes(&loop_graph), [0, 1]);
        let layout = Layout::of_graph(&loop_graph, &LayoutOptions::default());
        assert_eq!(layout.wires.len(), 2);
    }

    #[test]
    fn ordering_pulls_a_crossing_apart() {
        // Two independent chains wired across each other: after the
        // sweeps, the middle column is ordered to match its neighbours.
        let graph = Graph {
            nodes: vec![
                node("i0", 0, 1),
                node("i1", 0, 1),
                node("m0", 1, 1),
                node("m1", 1, 1),
                node("o", 2, 0),
            ],
            edges: vec![
                edge(0, 0, 3, 0),
                edge(1, 0, 2, 0),
                edge(3, 0, 4, 0),
                edge(2, 0, 4, 1),
            ],
        };
        let layout = Layout::of_graph(&graph, &LayoutOptions::default());
        let order = layout.column_order(&graph);
        assert_eq!(order[0], [0, 1]);
        // `m1` feeds the first input of `o`, so it sorts above `m0`.
        assert_eq!(order[1], [3, 2]);
    }

    #[test]
    fn boxes_never_overlap() {
        for graph in [chain(), diamond()] {
            let layout = Layout::of_graph(&graph, &LayoutOptions::default());
            for a in 0..layout.nodes.len() {
                for b in a + 1..layout.nodes.len() {
                    assert!(
                        !layout.nodes[a].overlaps(&layout.nodes[b]),
                        "boxes {a} and {b} overlap"
                    );
                }
            }
        }
    }

    #[test]
    fn wires_start_and_end_on_their_pins() {
        let graph = diamond();
        let layout = Layout::of_graph(&graph, &LayoutOptions::default());
        for (e, edge) in graph.edges.iter().enumerate() {
            let wire = &layout.wires[e];
            assert_eq!(
                wire.points[0],
                layout.nodes[edge.from].outputs[edge.from_pin]
            );
            assert_eq!(
                *wire.points.last().unwrap(),
                layout.nodes[edge.to].inputs[edge.to_pin]
            );
            // Orthogonal: every segment is horizontal or vertical.
            for pair in wire.points.windows(2) {
                assert!(pair[0].x == pair[1].x || pair[0].y == pair[1].y);
            }
        }
    }

    #[test]
    fn long_edges_get_a_bend_point() {
        // `a` reaches `c` directly and through `b`, so the direct edge
        // spans two ranks and is routed around the middle column.
        let graph = Graph {
            nodes: vec![node("a", 0, 1), node("b", 1, 1), node("c", 2, 0)],
            edges: vec![edge(0, 0, 1, 0), edge(1, 0, 2, 0), edge(0, 0, 2, 1)],
        };
        let layout = Layout::of_graph(&graph, &LayoutOptions::default());
        assert_eq!(rank_nodes(&graph), [0, 1, 2]);
        assert!(layout.wires[2].points.len() >= 3, "{:?}", layout.wires[2]);
        // The detour leaves the source column, which a direct wire would
        // not do.
        let widest = layout.wires[2].points.iter().map(|p| p.x).max().unwrap();
        assert!(widest >= layout.nodes[2].x);
    }

    #[test]
    fn simplify_merges_collinear_runs() {
        let points = vec![
            Point { x: 0, y: 0 },
            Point { x: 5, y: 0 },
            Point { x: 9, y: 0 },
            Point { x: 9, y: 4 },
            Point { x: 9, y: 4 },
        ];
        assert_eq!(
            simplify(points),
            [
                Point { x: 0, y: 0 },
                Point { x: 9, y: 0 },
                Point { x: 9, y: 4 }
            ]
        );
        assert_eq!(
            simplify(vec![Point { x: 1, y: 1 }]),
            [Point { x: 1, y: 1 }, Point { x: 1, y: 1 }]
        );
    }
}
