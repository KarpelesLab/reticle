//! Readers for the individual Project X-Ray files.
//!
//! Every one of these takes text (or already-parsed JSON) and gives back
//! a value; none of them touches the filesystem. The formats are the
//! database's, not Reticle's, and each function says which file it reads
//! and what shape that file has.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::{Feature, FeatureSet, XrayError, XrayTile};
use crate::fpga::arch::{BelDecl, ConfigBit, ConfigEntry};
use crate::fpga::xc7::{ConfigRow, FrameLayout, Part, TileBits};
use crate::json::Json;

/// The family directory a die name belongs to.
///
/// From the layout of the `prjxray-db` repository: one directory per
/// 7-series family, named after the family and not after the die.
pub fn family_directory(die: &str) -> Option<&'static str> {
    let rest = die.strip_prefix("xc7")?;
    Some(match rest.as_bytes().first()? {
        b'a' => "artix7",
        b'k' => "kintex7",
        b's' => "spartan7",
        b'v' => "virtex7",
        b'z' => "zynq7",
        _ => return None,
    })
}

/// The fabric a die uses, from `mapping/devices.yaml`.
///
/// **This is not a YAML parser.** That file has exactly one shape —
///
/// ```text
/// "xc7a35t":
///   fabric: "xc7a50t"
/// ```
///
/// — a quoted die name at the left margin, then an indented `fabric:`
/// line, and this reader accepts that and nothing else. Writing a YAML
/// parser to read six lines would be the wrong trade, and pretending a
/// loose reader is a YAML parser would be worse; `part.json` is used in
/// preference to `part.yaml` for the same reason.
///
/// It matters because `xc7a35t` and `xc7a50t` are the same die: a
/// bitstream for a Basys 3 is built against the `xc7a50t` fabric.
pub fn fabric_of(text: &str, die: &str) -> Option<String> {
    let mut current: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            current = trimmed.strip_suffix(':').map(unquote);
            continue;
        }
        if current == Some(die)
            && let Some(rest) = trimmed.strip_prefix("fabric:")
        {
            return Some(unquote(rest.trim()).to_owned());
        }
    }
    None
}

/// Strips the quotes a `devices.yaml` key or value carries.
fn unquote(text: &str) -> &str {
    text.trim_matches('"').trim_matches('\'')
}

/// Whether a feature name is a programmable interconnect point rather
/// than a bel feature.
///
/// # Why this is a rule and not a lookup
///
/// `prjxray-db` ships the *bits*, in `segbits_<type>.db`, whose lines are
/// `<TILE_TYPE>.<A>.<B> <bits>`. It does not ship the tile-type wire and
/// pip lists that would say whether `<A>` is a wire or a site: prjxray
/// generates those from Vivado into `tile_type_*.json`, which is not in
/// the repository. So the distinction has to be recovered from the
/// feature names themselves.
///
/// The rule: a two-component feature is a pip unless its first component
/// also heads a *longer* feature of the same tile type, because that is
/// what a site does. `CLBLL_L.SLICEL_X0.AFF.ZINI` makes `SLICEL_X0` a
/// site, so `CLBLL_L.SLICEL_X0.CLKINV` is a bel feature; `INT_L` has no
/// feature longer than two components at all, so every one of its 3636
/// features is a pip, which is right.
///
/// Where it is wrong: `LIOB33.DIFF.ZIBUF_LOW_PWR` and its two siblings
/// describe the differential pair, not a pip, and `DIFF` heads nothing
/// longer. Three features of a 83-feature tile type; the wrong pips they
/// produce name wires that exist nowhere else, so the graph drops them
/// as dangling.
///
/// `sites` is the set of first components of the tile type's longer
/// features; [`FeatureSet`] computes it once.
pub fn is_pip_feature(name: &str, sites: &HashSet<String>) -> bool {
    let mut parts = name.split('.');
    let (Some(head), Some(_), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !sites.contains(head)
}

/// Reads a `segbits_<type>.db` file.
///
/// Each line is a feature name followed by its bits, and a bit is
/// `<frame>_<bit>` for one that must be set or `!<frame>_<bit>` for one
/// that must be clear. A mux code is several of each, which is why a pip
/// owns a list rather than a bit.
///
/// Some lines carry the word `always` or a similar keyword where a
/// `ppips` file would; those are connections with no bits, which the
/// architecture already means by an empty bit list.
///
/// # Errors
///
/// [`XrayError::Malformed`] with the line number for a bit that is not
/// `<number>_<number>`.
pub(super) fn segbits(text: &str, path: &str) -> Result<FeatureSet, XrayError> {
    let mut features = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        let Some(full) = words.next() else { continue };
        // The leading tile type is the same on every line of the file
        // and is not part of the feature's name.
        let name = match full.split_once('.') {
            Some((_, rest)) => rest.to_owned(),
            None => continue,
        };
        let mut ones = Vec::new();
        let mut zeros = Vec::new();
        for word in words {
            let (value, body) = match word.strip_prefix('!') {
                Some(rest) => (false, rest),
                None => (true, word),
            };
            let Some((frame, bit)) = body.split_once('_') else {
                // `always`, `default`, `hint`: a connection with no bits.
                continue;
            };
            let (Ok(frame), Ok(bit)) = (frame.parse::<u32>(), bit.parse::<u32>()) else {
                return Err(XrayError::Malformed {
                    path: path.to_owned(),
                    message: format!("line {}: `{body}` is not a <frame>_<bit>", number + 1),
                });
            };
            if value {
                ones.push(ConfigBit::new(frame, bit));
            } else {
                zeros.push(ConfigBit::new(frame, bit));
            }
        }
        features.push(Feature { name, ones, zeros });
    }
    Ok(FeatureSet::new(features))
}

/// Reads `part.json`: the IDCODE and the frame layout.
///
/// The layout is nested clock region half, then row, then configuration
/// bus, then column, and the leaves are frame counts. The block type a
/// bus becomes is the frame address register's, from UG470: `CLB_IO_CLK`
/// is 0 and `BLOCK_RAM` is 1.
///
/// # Errors
///
/// [`XrayError::Malformed`] when a field this needs is absent or not a
/// number.
pub(super) fn part_from_json(json: &Json, path: &str, name: &str) -> Result<Part, XrayError> {
    let bad = |message: String| XrayError::Malformed {
        path: path.to_owned(),
        message,
    };
    let idcode = json
        .get("idcode")
        .and_then(Json::as_i64)
        .ok_or_else(|| bad("no `idcode`".to_owned()))?;
    let idcode = u32::try_from(idcode)
        .map_err(|_| bad(format!("`idcode` is {idcode}, which is not a 32-bit word")))?;

    let regions = json
        .get("global_clock_regions")
        .and_then(Json::as_object)
        .ok_or_else(|| bad("no `global_clock_regions`".to_owned()))?;
    let mut rows = Vec::new();
    for (half_name, half_json) in regions {
        let half = match half_name.as_str() {
            "top" => 0u8,
            "bottom" => 1u8,
            other => return Err(bad(format!("`{other}` is neither `top` nor `bottom`"))),
        };
        let row_map = half_json
            .get("rows")
            .and_then(Json::as_object)
            .ok_or_else(|| bad(format!("`{half_name}` has no `rows`")))?;
        for (row_name, row) in row_map {
            let row_number = row_name
                .parse::<u8>()
                .map_err(|_| bad(format!("`{row_name}` is not a row number")))?;
            let buses = row
                .get("configuration_buses")
                .and_then(Json::as_object)
                .ok_or_else(|| bad(format!("row {row_name} has no `configuration_buses`")))?;
            for (bus_name, bus) in buses {
                let block = match bus_name.as_str() {
                    "CLB_IO_CLK" => 0u8,
                    "BLOCK_RAM" => 1u8,
                    other => {
                        return Err(bad(format!(
                            "`{other}` is not a configuration bus this build knows"
                        )));
                    }
                };
                let columns = bus
                    .get("configuration_columns")
                    .and_then(Json::as_object)
                    .ok_or_else(|| bad(format!("`{bus_name}` has no `configuration_columns`")))?;
                let mut counts: BTreeMap<u32, u32> = BTreeMap::new();
                for (column_name, column) in columns {
                    let number = column_name
                        .parse::<u32>()
                        .map_err(|_| bad(format!("`{column_name}` is not a column number")))?;
                    let frames = column
                        .get("frame_count")
                        .and_then(Json::as_u32)
                        .ok_or_else(|| bad(format!("column {column_name} has no `frame_count`")))?;
                    counts.insert(number, frames);
                }
                let highest = counts.keys().next_back().copied().unwrap_or(0);
                let mut columns = vec![0u32; usize::try_from(highest + 1).unwrap_or(0)];
                for (number, frames) in counts {
                    columns[usize::try_from(number).unwrap_or(0)] = frames;
                }
                rows.push(ConfigRow {
                    block,
                    half,
                    row: row_number,
                    columns,
                });
            }
        }
    }
    if rows.is_empty() {
        return Err(bad("it describes no configuration rows".to_owned()));
    }
    Ok(Part {
        name: name.to_owned(),
        idcode,
        layout: FrameLayout::new(rows),
    })
}

/// Reads `tilegrid.json`.
///
/// One member per tile, named after the tile, holding its `type`, its
/// `grid_x` and `grid_y`, its `sites` (name to site type) and its `bits`
/// (one entry per configuration bus, with `baseaddr`, `frames`, `offset`
/// and `words`).
///
/// # Errors
///
/// [`XrayError::Malformed`] when the top level is not an object or a
/// tile has no type or position.
pub(super) fn tilegrid(json: &Json, path: &str) -> Result<Vec<XrayTile>, XrayError> {
    let bad = |message: String| XrayError::Malformed {
        path: path.to_owned(),
        message,
    };
    let members = json
        .as_object()
        .ok_or_else(|| bad("the top level is not an object of tiles".to_owned()))?;
    let mut out = Vec::with_capacity(members.len());
    for (name, tile) in members {
        let tile_type = tile
            .get("type")
            .and_then(Json::as_str)
            .ok_or_else(|| bad(format!("tile `{name}` has no `type`")))?
            .to_owned();
        let grid_x = tile
            .get("grid_x")
            .and_then(Json::as_u32)
            .ok_or_else(|| bad(format!("tile `{name}` has no `grid_x`")))?;
        let grid_y = tile
            .get("grid_y")
            .and_then(Json::as_u32)
            .ok_or_else(|| bad(format!("tile `{name}` has no `grid_y`")))?;
        let mut sites = Vec::new();
        if let Some(members) = tile.get("sites").and_then(Json::as_object) {
            for (site, kind) in members {
                sites.push((site.clone(), kind.as_str().unwrap_or("").to_owned()));
            }
            sites.sort();
        }
        let mut bits = Vec::new();
        if let Some(members) = tile.get("bits").and_then(Json::as_object) {
            for (bus, entry) in members {
                let base = entry
                    .get("baseaddr")
                    .and_then(Json::as_str)
                    .and_then(|t| u32::from_str_radix(t.trim_start_matches("0x"), 16).ok())
                    .ok_or_else(|| bad(format!("tile `{name}` has no usable `baseaddr`")))?;
                bits.push((
                    bus.clone(),
                    TileBits {
                        baseaddr: base,
                        frames: entry.get("frames").and_then(Json::as_u32).unwrap_or(0),
                        offset: entry.get("offset").and_then(Json::as_u32).unwrap_or(0),
                        words: entry.get("words").and_then(Json::as_u32).unwrap_or(0),
                    },
                ));
            }
            bits.sort_by(|a, b| a.0.cmp(&b.0));
        }
        out.push(XrayTile {
            name: name.clone(),
            tile_type,
            grid_x,
            grid_y,
            sites,
            bits,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// One entry of `tileconn.json`: which wires of two neighbouring tile
/// types are the same piece of metal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TileConn {
    /// The two tile types, the first one being where the delta is
    /// measured from.
    pub types: (String, String),
    /// How far the second tile is from the first, in grid coordinates.
    pub delta: (i32, i32),
    /// The wire of the first tile and the wire of the second that are
    /// one node.
    pub pairs: Vec<(String, String)>,
}

/// Reads `tileconn.json`.
///
/// An array of objects with `tile_types`, `grid_deltas` and
/// `wire_pairs`. This is the file that makes a 7-series long line one
/// node: the same metal is `EE4BEG0` in one tile and `EE4A0` in the next.
///
/// # Errors
///
/// [`XrayError::Malformed`] when the top level is not an array or an
/// entry is missing a field.
pub(super) fn tileconn(json: &Json, path: &str) -> Result<Vec<TileConn>, XrayError> {
    let bad = |message: String| XrayError::Malformed {
        path: path.to_owned(),
        message,
    };
    let items = json
        .as_array()
        .ok_or_else(|| bad("the top level is not an array".to_owned()))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let types = item
            .get("tile_types")
            .and_then(Json::as_array)
            .ok_or_else(|| bad("an entry has no `tile_types`".to_owned()))?;
        let deltas = item
            .get("grid_deltas")
            .and_then(Json::as_array)
            .ok_or_else(|| bad("an entry has no `grid_deltas`".to_owned()))?;
        if types.len() != 2 || deltas.len() != 2 {
            return Err(bad(
                "`tile_types` and `grid_deltas` both want two elements".to_owned()
            ));
        }
        let first = types[0].as_str().unwrap_or("").to_owned();
        let second = types[1].as_str().unwrap_or("").to_owned();
        let dx = i32::try_from(deltas[0].as_i64().unwrap_or(0)).unwrap_or(0);
        let dy = i32::try_from(deltas[1].as_i64().unwrap_or(0)).unwrap_or(0);
        let mut pairs = Vec::new();
        for pair in item
            .get("wire_pairs")
            .and_then(Json::as_array)
            .unwrap_or(&[])
        {
            let Some(pair) = pair.as_array() else {
                continue;
            };
            if pair.len() != 2 {
                continue;
            }
            pairs.push((
                pair[0].as_str().unwrap_or("").to_owned(),
                pair[1].as_str().unwrap_or("").to_owned(),
            ));
        }
        out.push(TileConn {
            types: (first, second),
            delta: (dx, dy),
            pairs,
        });
    }
    Ok(out)
}

/// Reads `package_pins.csv`: package pin to site.
///
/// The file has a header row naming its columns; this takes `pin` and
/// `site` by name, so a column order change does not matter.
pub(super) fn package_pins(text: &str) -> Vec<(String, String)> {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let columns: Vec<&str> = header.split(',').map(str::trim).collect();
    let pin = columns.iter().position(|c| *c == "pin");
    let site = columns.iter().position(|c| *c == "site");
    let (Some(pin), Some(site)) = (pin, site) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        let (Some(pin), Some(site)) = (fields.get(pin), fields.get(site)) else {
            continue;
        };
        if pin.is_empty() || site.is_empty() {
            continue;
        }
        out.push(((*pin).to_owned(), (*site).to_owned()));
    }
    out
}

/// What a site type can hold, in the vocabulary
/// [`BelRole`](crate::fpga::BelRole) uses.
///
/// A site whose type is not here becomes one bel of its own lowercased
/// type name, which the placer will simply never match a cell to. That
/// is the right failure: a cell that cannot be placed is reported, not
/// put somewhere plausible.
fn site_kind(site_type: &str) -> &'static str {
    match site_type {
        "SLICEL" | "SLICEM" => "slice",
        "IOB33M" | "IOB33S" | "IOB33" => "io",
        "BUFGCTRL" => "gb",
        "RAMB18E1" | "RAMB36E1" | "RAMBFIFO36E1" | "FIFO18E1" => "bram",
        "DSP48E1" => "dsp",
        _ => "other",
    }
}

/// The site prefixes a tile type's features use, in name order.
///
/// `CLBLL_L` gives `["SLICEL_X0", "SLICEL_X1"]`, which is the
/// vocabulary its bels are named in. The order is the order
/// [`tilegrid`] puts a tile's own site names in, so the two zip: the
/// i-th site of a tile is the i-th prefix.
pub(super) fn site_prefixes(features: &FeatureSet) -> Vec<&str> {
    let mut out: Vec<&str> = features.sites().iter().map(String::as_str).collect();
    out.sort_unstable();
    out
}

/// The bel name a tile's site becomes, which is what a package pin has
/// to name to reach it.
///
/// The site is `SLICE_X0Y0` in `tilegrid.json` and the bel is
/// `SLICEL_X0`, because a bel belongs to the tile *type* and a site
/// name does not. They are matched by position in each list. A tile
/// type with no features at all keeps the site's own name, which at
/// least names something.
pub(super) fn bel_of_site(tile: &XrayTile, site: &str, features: &FeatureSet) -> Option<String> {
    let index = tile.sites.iter().position(|(name, _)| name == site)?;
    let prefixes = site_prefixes(features);
    Some(match prefixes.get(index) {
        Some(prefix) => (*prefix).to_owned(),
        None => site.to_owned(),
    })
}

/// The bels of one tile type, from its sites and its features.
///
/// A bel has to be named in the tile type's own vocabulary, because it
/// becomes one site per tile of that type. Project X-Ray already has
/// such a vocabulary — the site prefix its features use, `SLICEL_X0` and
/// `SLICEL_X1` for a `CLBLL_L` — so that is what the bels are named
/// after, and the tile's own site list (`SLICE_X0Y0`, `SLICE_X1Y0`)
/// supplies the site *type* by position.
///
/// A slice is not one placeable element but several, so it is split:
/// each `<prefix>_<x>LUT` feature makes a `lut` bel, each `<prefix>_<x>FF`
/// an `ff` bel, and the carry chain a `carry` bel. Anything else
/// attaches to a bel named after the site prefix itself.
///
/// # What is missing, and it is the important part
///
/// **No pins.** Which wire a LUT input or a flip-flop output reaches is
/// in prjxray's `tile_type_*.json`, which `prjxray-db` does not ship.
/// A bel with no pins can be placed onto and cannot be routed to, so a
/// design taken through this loader gets a real placement and no
/// connections. See `docs/fpga-xray.md`.
pub(super) fn bels_of(tile: &XrayTile, features: &FeatureSet) -> Vec<BelDecl> {
    let prefixes = site_prefixes(features);

    // Zip the feature files' site prefixes onto the tile's sites, in
    // name order, which is the order both lists are in.
    let mut kinds: HashMap<&str, &str> = HashMap::new();
    for (index, prefix) in prefixes.iter().enumerate() {
        let site_type = tile.sites.get(index).map_or("", |(_, kind)| kind.as_str());
        kinds.insert(prefix, site_kind(site_type));
    }

    // Which sub-elements each prefix has, and which features belong to
    // the prefix itself.
    let mut subs: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for feature in features.features() {
        let mut parts = feature.name.split('.');
        let (Some(head), Some(sub)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !features.sites().contains(head) {
            continue;
        }
        if parts.next().is_some() {
            subs.entry(head).or_default().insert(sub);
        }
    }

    let mut bels: BTreeMap<String, BelDecl> = BTreeMap::new();
    for prefix in &prefixes {
        let kind = kinds.get(prefix).copied().unwrap_or("other");
        if kind == "slice" {
            for sub in subs.get(prefix).into_iter().flatten() {
                if let Some(sub_kind) = slice_sub_kind(sub) {
                    let name = format!("{prefix}_{sub}");
                    bels.insert(name.clone(), BelDecl::new(name, sub_kind));
                }
            }
        }
        bels.insert(
            (*prefix).to_owned(),
            BelDecl::new(*prefix, if kind == "slice" { "site" } else { kind }),
        );
    }

    // Now attach every feature to the bel it names.
    for feature in features.features() {
        let mut parts = feature.name.split('.');
        let (Some(head), Some(second)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !features.sites().contains(head) {
            continue;
        }
        let rest: Vec<&str> = parts.collect();
        let (bel_name, tail) = if rest.is_empty() {
            ((*head).to_owned(), second.to_owned())
        } else {
            let candidate = format!("{head}_{second}");
            if bels.contains_key(&candidate) {
                (candidate, rest.join("."))
            } else {
                ((*head).to_owned(), format!("{second}.{}", rest.join(".")))
            }
        };
        let Some(bel) = bels.get_mut(&bel_name) else {
            continue;
        };
        if feature.ones.is_empty() {
            continue;
        }
        match parameter_bit(&tail) {
            Some((name, index)) => {
                for bit in &feature.ones {
                    bel.config.push(ConfigEntry::Param {
                        name: name.clone(),
                        index,
                        at: *bit,
                    });
                }
            }
            None => bel.config.push(ConfigEntry::Cell {
                primitive: tail,
                bits: feature.ones.clone(),
            }),
        }
    }

    bels.into_values().collect()
}

/// What a slice sub-element is, in Reticle's bel vocabulary.
fn slice_sub_kind(sub: &str) -> Option<&'static str> {
    if sub.ends_with("LUT") {
        return Some("lut");
    }
    if sub.ends_with("FF") {
        return Some("ff");
    }
    if sub == "CARRY4" || sub == "PRECYINIT" {
        return Some("carry");
    }
    None
}

/// Splits `INIT[63]` into the parameter and the bit of it.
///
/// A feature whose tail is a parameter with an index is a
/// [`ConfigEntry::Param`] — which is how a LUT's truth table reaches the
/// bitstream without a line of Rust knowing what a LUT is. Anything else
/// is a mode feature and becomes a [`ConfigEntry::Cell`] keyed on the
/// database's own name for it.
fn parameter_bit(tail: &str) -> Option<(String, u32)> {
    let (name, rest) = tail.split_once('[')?;
    let index = rest.strip_suffix(']')?.parse::<u32>().ok()?;
    Some((name.to_owned(), index))
}
