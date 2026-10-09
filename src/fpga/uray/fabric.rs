//! An UltraScale+ routing fabric: the 2026 database's bits on the 2020
//! snapshot's wiring.
//!
//! # Two databases, because neither is enough
//!
//! [`mithro/prjuray-db-ultrascaleplus`] has the ZU7EV's tile grid and
//! every tile type's bits, but not one wire: it never says which wire of
//! a tile is the same metal as which wire of the next. The 2020
//! [`f4pga/prjuray-db`] snapshot does say, in its `tileconn.json` — but
//! only for the ZU3EG. What joins the two is that the dies share tile
//! types and Vivado's wire names. `docs/fpga-uray.md` has the
//! measurements.
//!
//! [`mithro/prjuray-db-ultrascaleplus`]: https://github.com/mithro/prjuray-db-ultrascaleplus
//! [`f4pga/prjuray-db`]: https://github.com/f4pga/prjuray-db
//!
//! # The two adjustments the ZU3EG's rules need
//!
//! A rule names two tile types and a grid offset. Applied as they are,
//! the rules leave tens of thousands of the ZU7EV's routed nodes
//! undriven, for two reasons, each repaired here and each measured
//! against Vivado's ZCU104 bitstream:
//!
//! - **Types the ZU3EG calls by another name** ([`ALIASES`]): five tile
//!   types of the ZU7EV are, as far as wiring goes, a ZU3EG type under a
//!   different name.
//! - **Neighbours the ZU3EG never puts side by side** ([`composed_joins`]):
//!   on the ZU3EG an east–west bus wire threads through clock-region
//!   break tiles between two columns that, on the ZU7EV, are often
//!   adjacent. Composing the rules through the break tiles gives the
//!   direct join, which [`build_arch`] declares on a *variant* of the
//!   western tile's type, because whether it applies depends on the
//!   instance and not on the type.
//!
//! # What this builds, and what it does not
//!
//! An [`Arch`] for a [`GridRegion`] of the die: every tile type's wires,
//! every one-bit pip the 2026 database names, and every join. It has no
//! bels yet: a design cannot be placed on it. It is enough to check a
//! bitstream's routing against it, which is what
//! `tests/fpga_uray_routing.rs` does.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::{TileGrid, TileTypeBits, UrayError};
use crate::fpga::arch::{Arch, ConfigBit, PipDecl, TileType, WireDecl, WireRef};
use crate::json::Json;

/// ZU7EV tile types whose wiring is a ZU3EG type's under another name.
///
/// **Measured, not documented.** Each was kept only because, applied on
/// its own, it reduced the number of Vivado's routed nodes that the
/// rules leave undriven or unread without creating a node driven twice;
/// every other name-derived candidate was tried and rejected the same
/// way. The four `_FT` names differ from the ZU3EG's by that suffix
/// alone. `INT_TERM_P`, at the top of the processor system, wires like
/// the ZU3EG's `INT_TERM_B` and not like `INT_TERM_T`.
pub const ALIASES: [(&str, &str); 5] = [
    ("AMS_M12BUF_BOT_L_FT", "AMS_M12BUF_BOT_L"),
    ("AMS_M12BUF_TOP_L_FT", "AMS_M12BUF_TOP_L"),
    ("CFG_M12BUF_CFG_BOT_R_FT", "CFG_M12BUF_CFG_BOT_R"),
    ("CFG_M12BUF_CFG_TOP_R_FT", "CFG_M12BUF_CFG_TOP_R"),
    ("INT_TERM_P", "INT_TERM_B"),
];

/// The ZU3EG type whose wiring a ZU7EV type has.
pub fn wiring_type(kind: &str) -> &str {
    ALIASES
        .iter()
        .find(|(from, _)| *from == kind)
        .map_or(kind, |(_, to)| to)
}

/// True for a clock-region break tile, which a bus wire passes through
/// on its way between two columns: the `CFRM_CBRK_*` and `*M12BUF*`
/// types.
pub fn is_break_type(kind: &str) -> bool {
    kind.contains("CBRK") || kind.contains("M12BUF")
}

/// One rule of `tileconn.json`: two tile types, the grid offset from a
/// tile of the first to a tile of the second, and the wires joined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileConn {
    /// The two types, in the order the offset runs.
    pub types: (String, String),
    /// From the first to the second, in grid columns and rows.
    pub delta: (i32, i32),
    /// Each pair is a wire of the first type and a wire of the second.
    pub pairs: Vec<(String, String)>,
}

/// Reads a `tileconn.json`.
///
/// # Errors
///
/// [`UrayError::Malformed`] for text that is not the file's shape.
pub fn parse_tileconn(text: &str, path: &str) -> Result<Vec<TileConn>, UrayError> {
    let bad = |message: String| UrayError::Malformed {
        path: path.to_owned(),
        message,
    };
    let json = Json::parse(text).map_err(|e| bad(e.to_string()))?;
    let Json::Array(rules) = json else {
        return Err(bad("the top level is not an array".to_owned()));
    };
    let mut out = Vec::with_capacity(rules.len());
    for (index, rule) in rules.iter().enumerate() {
        let strings = |value: Option<&Json>| -> Option<(String, String)> {
            let Some(Json::Array(two)) = value else {
                return None;
            };
            match two.as_slice() {
                [a, b] => Some((a.as_str()?.to_owned(), b.as_str()?.to_owned())),
                _ => None,
            }
        };
        let types = strings(rule.get("tile_types"))
            .ok_or_else(|| bad(format!("rule {index} has no `tile_types` pair")))?;
        let delta = match rule.get("grid_deltas") {
            Some(Json::Array(two)) => match two.as_slice() {
                [a, b] => a
                    .as_i64()
                    .zip(b.as_i64())
                    .and_then(|(a, b)| Some((i32::try_from(a).ok()?, i32::try_from(b).ok()?))),
                _ => None,
            },
            _ => None,
        }
        .ok_or_else(|| bad(format!("rule {index} has no `grid_deltas` pair")))?;
        let Some(Json::Array(list)) = rule.get("wire_pairs") else {
            return Err(bad(format!("rule {index} has no `wire_pairs`")));
        };
        let mut pairs = Vec::with_capacity(list.len());
        for pair in list {
            let Json::Array(two) = pair else {
                return Err(bad(format!(
                    "rule {index} has a wire pair that is not a pair"
                )));
            };
            match two.as_slice() {
                [Json::String(a), Json::String(b)] => pairs.push((a.clone(), b.clone())),
                _ => {
                    return Err(bad(format!(
                        "rule {index} has a wire pair that is not a pair"
                    )));
                }
            }
        }
        out.push(TileConn {
            types,
            delta,
            pairs,
        });
    }
    Ok(out)
}

/// One site of a tile type, as the 2020 `tile_type_<T>.json` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteDecl {
    /// The site's name inside the tile (`X0Y0`).
    pub name: String,
    /// The prefix a site of the die is named with (`SLICE`).
    pub prefix: String,
    /// Its site type (`SLICEL`).
    pub site_type: String,
    /// Each pin and the tile wire it is.
    pub pins: Vec<(String, String)>,
}

/// What one 2020 `tile_type_<T>.json` gives: the wires and the sites.
///
/// Its pips are not used: which of them are programmable, and with which
/// bits, comes from the 2026 database's `segbits` instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileTypeWiring {
    /// Every wire of the type.
    pub wires: Vec<String>,
    /// Every site.
    pub sites: Vec<SiteDecl>,
}

impl TileTypeWiring {
    /// Reads a `tile_type_<T>.json`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for text that is not the file's shape.
    pub fn parse(text: &str, path: &str) -> Result<TileTypeWiring, UrayError> {
        let bad = |message: String| UrayError::Malformed {
            path: path.to_owned(),
            message,
        };
        let json = Json::parse(text).map_err(|e| bad(e.to_string()))?;
        let mut out = TileTypeWiring::default();
        match json.get("wires") {
            Some(Json::Object(wires)) => out.wires = wires.iter().map(|(n, _)| n.clone()).collect(),
            Some(Json::Array(wires)) => {
                out.wires = wires
                    .iter()
                    .filter_map(Json::as_str)
                    .map(str::to_owned)
                    .collect();
            }
            _ => return Err(bad("there is no `wires` list".to_owned())),
        }
        if let Some(Json::Array(sites)) = json.get("sites") {
            for site in sites {
                let text = |key: &str| {
                    site.get(key)
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_owned()
                };
                let mut pins = Vec::new();
                if let Some(Json::Object(list)) = site.get("site_pins") {
                    for (pin, info) in list {
                        if let Some(wire) = info.get("wire").and_then(Json::as_str) {
                            pins.push((pin.clone(), wire.to_owned()));
                        }
                    }
                }
                out.sites.push(SiteDecl {
                    name: text("name"),
                    prefix: text("prefix"),
                    site_type: text("type"),
                    pins,
                });
            }
        }
        Ok(out)
    }
}

/// Which pins of a site type drive and which are driven:
/// `site_type_<T>.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteTypePins {
    /// Pins the site drives.
    pub outputs: BTreeSet<String>,
    /// Pins the site reads.
    pub inputs: BTreeSet<String>,
}

impl SiteTypePins {
    /// Reads a `site_type_<T>.json`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for text that is not the file's shape.
    pub fn parse(text: &str, path: &str) -> Result<SiteTypePins, UrayError> {
        let bad = |message: String| UrayError::Malformed {
            path: path.to_owned(),
            message,
        };
        let json = Json::parse(text).map_err(|e| bad(e.to_string()))?;
        let Some(Json::Object(pins)) = json.get("site_pins") else {
            return Err(bad("there is no `site_pins` object".to_owned()));
        };
        let mut out = SiteTypePins::default();
        for (pin, info) in pins {
            match info.get("direction").and_then(Json::as_str) {
                Some("OUT") => {
                    out.outputs.insert(pin.clone());
                }
                Some("IN") => {
                    out.inputs.insert(pin.clone());
                }
                _ => {}
            }
        }
        Ok(out)
    }
}

/// The routing a feature name describes, or `None` for a feature that is
/// not routing.
///
/// `A->B` and `A->>B` are a pip from `A` to `B`; `A<->B` is a pip each
/// way. A name with `&` is two pips the database could not tell apart,
/// because they share a bit: which of them a set bit means is unknown,
/// so it gives none. A name with a `.` before its arrow is inside a site.
pub fn routing_of(feature: &str) -> Option<Vec<(&str, &str)>> {
    if feature.contains('&') {
        return None;
    }
    if let Some((a, b)) = feature.split_once("<->") {
        if a.contains('.') {
            return None;
        }
        return Some(vec![(a, b), (b, a)]);
    }
    let (a, b) = feature.split_once("->")?;
    if a.contains('.') {
        return None;
    }
    Some(vec![(a, b.trim_start_matches('>'))])
}

/// Every wire `(type, wire)` reaches eastwards *through* break tiles,
/// arriving at a tile of another kind in the same row.
///
/// The ZU3EG's rules only ever join, say, a `CLEL_R` to a `CLEM` through
/// the `CFRM_CBRK_L` and `CFG_M12BUF` tiles between them; following the
/// rules through those tiles and keeping the far ends whose rows add up
/// to zero gives the join directly. The walk only moves east and gives
/// up after a few hundred steps, which no real chain approaches.
///
/// With `only`, the walk starts from those tile types alone, which is
/// all [`build_arch`] needs and a small fraction of the work.
pub fn composed_joins(
    rules: &[TileConn],
    only: Option<&BTreeSet<&str>>,
) -> BTreeMap<(String, String), BTreeSet<(String, String)>> {
    let mut adj: Adjacency<'_> = HashMap::new();
    for rule in rules {
        let (ta, tb) = (rule.types.0.as_str(), rule.types.1.as_str());
        let (dx, dy) = rule.delta;
        for (wa, wb) in &rule.pairs {
            adj.entry((ta, wa.as_str()))
                .or_default()
                .push((dx, dy, (tb, wb.as_str())));
            adj.entry((tb, wb.as_str()))
                .or_default()
                .push((-dx, -dy, (ta, wa.as_str())));
        }
    }
    let mut out: BTreeMap<(String, String), BTreeSet<(String, String)>> = BTreeMap::new();
    let mut starts: Vec<&End<'_>> = adj
        .keys()
        .filter(|(t, _)| !is_break_type(t) && only.is_none_or(|o| o.contains(t)))
        .collect();
    starts.sort();
    for start in starts {
        // Each first step is walked on its own, with its own budget: a
        // bus wire's first steps are dozens of break tiles at different
        // row offsets, and one budget shared between them runs out
        // before most are followed.
        for &(dx, dy, next) in &adj[start] {
            if dx <= 0 || !is_break_type(next.0) {
                continue;
            }
            let mut stack: Vec<(End<'_>, i32, i32)> = vec![(next, dx, dy)];
            let mut seen: HashSet<(End<'_>, i32, i32)> = HashSet::from([(next, dx, dy)]);
            walk(*start, &adj, &mut stack, &mut seen, &mut out);
        }
    }
    out
}

/// One end of a join: a tile type and one of its wires.
type End<'a> = (&'a str, &'a str);

/// Every join each end takes part in, with the grid offset to the other.
type Adjacency<'a> = HashMap<End<'a>, Vec<(i32, i32, End<'a>)>>;

/// One first step's walk of [`composed_joins`]: through break tiles,
/// keeping every non-break end it reaches back in the starting row.
fn walk<'a>(
    start: End<'a>,
    adj: &Adjacency<'a>,
    stack: &mut Vec<(End<'a>, i32, i32)>,
    seen: &mut HashSet<(End<'a>, i32, i32)>,
    out: &mut BTreeMap<(String, String), BTreeSet<(String, String)>>,
) {
    while let Some((at, x, y)) = stack.pop() {
        for &(dx, dy, next) in adj.get(&at).into_iter().flatten() {
            let (nx, ny) = (x + dx, y + dy);
            if nx <= 0 {
                continue;
            }
            if is_break_type(next.0) {
                if seen.len() < 200 && seen.insert((next, nx, ny)) {
                    stack.push((next, nx, ny));
                }
            } else if ny == 0 {
                out.entry((start.0.to_owned(), start.1.to_owned()))
                    .or_default()
                    .insert((next.0.to_owned(), next.1.to_owned()));
            }
        }
    }
}

/// An inclusive rectangle of the database's grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridRegion {
    /// The first column.
    pub x0: u32,
    /// The first row.
    pub y0: u32,
    /// The last column.
    pub x1: u32,
    /// The last row.
    pub y1: u32,
}

impl GridRegion {
    /// The region from `(x0, y0)` to `(x1, y1)`, both included.
    pub fn new(x0: u32, y0: u32, x1: u32, y1: u32) -> GridRegion {
        GridRegion { x0, y0, x1, y1 }
    }

    /// True when `(x, y)` is in the region.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }
}

/// Counts of what [`build_arch`] did, for a test to pin and a person to
/// read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FabricStats {
    /// Tile types declared, variants included.
    pub tile_types: usize,
    /// Variants made for composed joins.
    pub variants: usize,
    /// Tiles placed in the region.
    pub tiles: usize,
    /// Pips with bits, summed over the region's tiles.
    pub pips: usize,
    /// Bitless joins, summed over the region's tiles.
    pub joins: usize,
    /// Features skipped because they name two pips with one bit.
    pub compound_features: usize,
    /// Tile types in the region that the 2020 snapshot does not describe
    /// even through [`ALIASES`]: their wires come only from their own
    /// pips, and no rule joins them to anything.
    pub unwired_types: BTreeSet<String>,
}

/// What [`build_arch`] returns: the architecture, and for each of its
/// tile types the database type its bits are under.
#[derive(Clone, Debug)]
pub struct UrayFabric {
    /// The architecture.
    pub arch: Arch,
    /// For each tile type of `arch`, by index, the database type.
    pub database_type: Vec<String>,
    /// What was built.
    pub stats: FabricStats,
}

/// Everything [`build_arch`] reads.
pub struct FabricInputs<'a> {
    /// The die's tile grid (2026).
    pub grid: &'a TileGrid,
    /// Each database tile type's bits (2026), by type.
    pub bits: &'a HashMap<String, TileTypeBits>,
    /// Each tile type's wires and sites (2020), by type.
    pub wiring: &'a HashMap<String, TileTypeWiring>,
    /// The ZU3EG's `tileconn.json` (2020).
    pub rules: &'a [TileConn],
}

/// The name of the variant of `base` that also joins east to `other`,
/// `distance` columns away.
fn variant_name(base: &str, other: &str, distance: u32) -> String {
    format!("{base}+{other}@{distance}")
}

/// Builds the architecture of `region`.
///
/// Every tile of the region gets its database type, or the variant of
/// it that carries a composed join. Each type declares the wires its
/// 2020 tile type lists — through [`ALIASES`] — and any its own pips or
/// joins name besides; a pip for every routing feature of its `segbits`
/// file ([`routing_of`]); and two bitless pips, one each way, for every
/// wire pair of every rule whose first type is its wiring type.
pub fn build_arch(inputs: &FabricInputs<'_>, region: GridRegion) -> UrayFabric {
    let tiles = inputs.grid.tiles();
    let mut width = 0;
    let mut height = 0;
    let mut at: HashMap<(u32, u32), usize> = HashMap::new();
    for (index, tile) in tiles.iter().enumerate() {
        width = u32::max(width, tile.grid.0 + 1);
        height = u32::max(height, tile.grid.1 + 1);
        at.insert(tile.grid, index);
    }
    let mut stats = FabricStats::default();

    // Composed joins: for each tile of the region, the next tile east
    // that is neither blank nor a break, when the ZU3EG's rules do not
    // already join the two at that offset and the composition does.
    let direct: HashSet<(&str, &str, i32, i32)> = inputs
        .rules
        .iter()
        .flat_map(|r| {
            let (a, b) = (r.types.0.as_str(), r.types.1.as_str());
            let (dx, dy) = r.delta;
            [(a, b, dx, dy), (b, a, -dx, -dy)]
        })
        .collect();
    let mut candidates: Vec<(usize, String, u32)> = Vec::new();
    for (index, tile) in tiles.iter().enumerate() {
        let (x, y) = tile.grid;
        if !region.contains(x, y) || tile.kind == "NULL" {
            continue;
        }
        for step in 1..8u32 {
            let Some(&other) = at.get(&(x + step, y)) else {
                continue;
            };
            let other = &tiles[other];
            if other.kind == "NULL" || is_break_type(&other.kind) {
                continue;
            }
            let (a, b) = (wiring_type(&tile.kind), wiring_type(&other.kind));
            let offset = i32::try_from(step).unwrap_or(i32::MAX);
            if !direct.contains(&(a, b, offset, 0)) {
                candidates.push((index, other.kind.clone(), step));
            }
            break;
        }
    }
    let starts: BTreeSet<&str> = candidates
        .iter()
        .map(|(index, _, _)| wiring_type(&tiles[*index].kind))
        .collect();
    let composed = composed_joins(inputs.rules, Some(&starts));
    let composed_pairs: HashSet<(&str, &str)> = composed
        .iter()
        .flat_map(|((a, _), ends)| ends.iter().map(move |(b, _)| (a.as_str(), b.as_str())))
        .collect();
    let mut variant_of: HashMap<usize, (String, u32)> = HashMap::new();
    for (index, other, step) in candidates {
        let pair = (wiring_type(&tiles[index].kind), wiring_type(&other));
        if composed_pairs.contains(&pair) {
            variant_of.insert(index, (other, step));
        }
    }

    // The types the region uses: each database type, and each variant.
    let mut used: BTreeMap<String, (String, Option<(String, u32)>)> = BTreeMap::new();
    for (index, tile) in tiles.iter().enumerate() {
        if !region.contains(tile.grid.0, tile.grid.1) || tile.kind == "NULL" {
            continue;
        }
        let key = match variant_of.get(&index) {
            Some((other, distance)) => variant_name(&tile.kind, other, *distance),
            None => tile.kind.clone(),
        };
        used.entry(key)
            .or_insert_with(|| (tile.kind.clone(), variant_of.get(&index).cloned()));
    }
    // A rule names the far end's wiring type; the tiles there may be of
    // any arch type that stands for it.
    let mut stands_for: HashMap<&str, Vec<&str>> = HashMap::new();
    for (key, (kind, _)) in &used {
        stands_for
            .entry(wiring_type(kind))
            .or_default()
            .push(key.as_str());
    }
    let mut rules_from: HashMap<&str, Vec<&TileConn>> = HashMap::new();
    for rule in inputs.rules {
        rules_from
            .entry(rule.types.0.as_str())
            .or_default()
            .push(rule);
    }
    let mut rules_to: HashMap<&str, Vec<&TileConn>> = HashMap::new();
    for rule in inputs.rules {
        rules_to
            .entry(rule.types.1.as_str())
            .or_default()
            .push(rule);
    }

    let mut arch = Arch::new("zynqusp-xazu7ev", "uray", width, height);
    let mut database_type = Vec::new();
    let mut index_of: HashMap<&str, usize> = HashMap::new();
    let no_bits = TileTypeBits::default();
    let no_wiring = TileTypeWiring::default();
    for (key, (kind, variant)) in &used {
        let wiring_kind = wiring_type(kind);
        let bits = inputs.bits.get(kind).unwrap_or(&no_bits);
        let wiring = inputs.wiring.get(wiring_kind).unwrap_or_else(|| {
            stats.unwired_types.insert(kind.clone());
            &no_wiring
        });
        let (rows, cols) = tiles.iter().find(|t| t.kind == *kind).map_or((0, 0), |t| {
            (
                t.windows.iter().map(|w| w.frames).sum(),
                t.windows.iter().map(|w| w.bits).max().unwrap_or(0),
            )
        });
        let mut tile_type = TileType::new(key.as_str(), kind.as_str(), rows, cols);

        let mut wires: BTreeSet<&str> = wiring.wires.iter().map(String::as_str).collect();
        let mut pips: Vec<(&str, &str, Vec<ConfigBit>)> = Vec::new();
        for feature in &bits.features {
            if feature.name.contains('&') {
                stats.compound_features += 1;
                continue;
            }
            for (from, to) in routing_of(&feature.name).into_iter().flatten() {
                wires.insert(from);
                wires.insert(to);
                pips.push((from, to, feature.ones.clone()));
            }
        }
        for rule in rules_from.get(wiring_kind).into_iter().flatten() {
            wires.extend(rule.pairs.iter().map(|(mine, _)| mine.as_str()));
        }
        for rule in rules_to.get(wiring_kind).into_iter().flatten() {
            wires.extend(rule.pairs.iter().map(|(_, theirs)| theirs.as_str()));
        }
        tile_type.wires = wires
            .iter()
            .map(|w| WireDecl {
                name: (*w).to_owned(),
                dx: 0,
                dy: 0,
            })
            .collect();
        for (from, to, bits) in pips {
            tile_type.pips.push(PipDecl {
                from: WireRef::local(from),
                to: WireRef::local(to),
                bits,
            });
        }
        let mut join = |mine: &str, theirs: &str, dx: i32, dy: i32, other: &str| {
            tile_type.pips.push(PipDecl {
                from: WireRef::local(mine),
                to: WireRef::at_in(theirs, dx, dy, other),
                bits: Vec::new(),
            });
            tile_type.pips.push(PipDecl {
                from: WireRef::at_in(theirs, dx, dy, other),
                to: WireRef::local(mine),
                bits: Vec::new(),
            });
        };
        for rule in rules_from.get(wiring_kind).into_iter().flatten() {
            let (dx, dy) = rule.delta;
            for other in stands_for.get(rule.types.1.as_str()).into_iter().flatten() {
                for (mine, theirs) in &rule.pairs {
                    join(mine, theirs, dx, dy, other);
                }
            }
        }
        if let Some((other_kind, distance)) = variant {
            let other_wiring = wiring_type(other_kind);
            let dx = i32::try_from(*distance).unwrap_or(i32::MAX);
            for ((a, mine), ends) in composed.range((wiring_kind.to_owned(), String::new())..) {
                if a != wiring_kind {
                    break;
                }
                for (b, theirs) in ends {
                    if b != other_wiring {
                        continue;
                    }
                    for other in stands_for.get(b.as_str()).into_iter().flatten() {
                        join(mine, theirs, dx, 0, other);
                    }
                }
            }
            stats.variants += 1;
        }
        index_of.insert(key.as_str(), arch.tile_types.len());
        arch.tile_types.push(tile_type);
        database_type.push(kind.clone());
    }

    for (index, tile) in tiles.iter().enumerate() {
        let (x, y) = tile.grid;
        if !region.contains(x, y) || tile.kind == "NULL" {
            continue;
        }
        let key = match variant_of.get(&index) {
            Some((other, distance)) => variant_name(&tile.kind, other, *distance),
            None => tile.kind.clone(),
        };
        if let Some(&type_index) = index_of.get(key.as_str()) {
            arch.set_tile(x, y, type_index);
            stats.tiles += 1;
            let t = &arch.tile_types[type_index];
            stats.pips += t.pips.iter().filter(|p| !p.bits.is_empty()).count();
            stats.joins += t.pips.iter().filter(|p| p.bits.is_empty()).count();
        }
    }
    stats.tile_types = arch.tile_types.len();
    UrayFabric {
        arch,
        database_type,
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_feature_name_says_which_pips_it_is() {
        assert_eq!(
            routing_of("EE2_E_BEG0->>NN1_E_END0"),
            Some(vec![("EE2_E_BEG0", "NN1_E_END0")])
        );
        assert_eq!(routing_of("A->B"), Some(vec![("A", "B")]));
        assert_eq!(routing_of("A<->B"), Some(vec![("A", "B"), ("B", "A")]));
        assert_eq!(routing_of("A<->B&C->D"), None);
        assert_eq!(routing_of("SLICE_X0Y0.A6LUT.INIT[0]"), None);
        assert_eq!(routing_of("SLICE_X0Y0.AFFMUX->D6"), None);
    }

    fn rule(a: &str, b: &str, delta: (i32, i32), pairs: &[(&str, &str)]) -> TileConn {
        TileConn {
            types: (a.to_owned(), b.to_owned()),
            delta,
            pairs: pairs
                .iter()
                .map(|(x, y)| ((*x).to_owned(), (*y).to_owned()))
                .collect(),
        }
    }

    /// The shape of the ZU3EG's: a slice-L, a break tile in the next
    /// column two rows down, a second break tile beside it, and a
    /// slice-M two columns on, back in the slice-L's row.
    #[test]
    fn a_join_through_break_tiles_composes_into_a_direct_one() {
        let rules = vec![
            rule("CLEL_R", "CFRM_CBRK_L", (1, 2), &[("BUS_3", "BUS_2_3")]),
            rule(
                "CFRM_CBRK_L",
                "CFG_M12BUF",
                (1, 0),
                &[("BUS_2_3", "BUS_2_3")],
            ),
            rule("CFG_M12BUF", "CLEM", (1, -2), &[("BUS_2_3", "BUS_3")]),
            // A rule that leaves the row is not a composition.
            rule("CFG_M12BUF", "INT", (1, 5), &[("BUS_2_3", "X")]),
        ];
        let composed = composed_joins(&rules, None);
        let ends = &composed[&("CLEL_R".to_owned(), "BUS_3".to_owned())];
        assert_eq!(
            ends.iter().cloned().collect::<Vec<_>>(),
            vec![("CLEM".to_owned(), "BUS_3".to_owned())]
        );
        // Nothing starts from a break tile, and nothing composes westward.
        assert!(composed.keys().all(|(t, _)| !is_break_type(t)));
        assert!(!composed.contains_key(&("CLEM".to_owned(), "BUS_3".to_owned())));
    }

    #[test]
    fn an_alias_gives_the_zu3eg_type() {
        assert_eq!(wiring_type("INT_TERM_P"), "INT_TERM_B");
        assert_eq!(wiring_type("AMS_M12BUF_BOT_L_FT"), "AMS_M12BUF_BOT_L");
        assert_eq!(wiring_type("INT"), "INT");
    }
}
