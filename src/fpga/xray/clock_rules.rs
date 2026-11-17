//! Two bits beside a global clock that no pip carries, set as Vivado sets
//! them: the rebuffer enables at the ends of each live segment of a global
//! track, and a row's `HCLK_CMT_CK_BUFHCLK<n>_USED` for each `BUFHCE` in use.
//!
//! # The rules, and how they were read
//!
//! Both are read off Vivado 2026.1's bitstreams and held to them by
//! `tools/vivado-parity/clock_rules.py`, which recomputes each from a
//! bitstream's own buffers, rows and `BUFHCE`s: no disagreement on all 528 of
//! the XC7A35T's probes (both packages, Vivado's own flow and its routes of
//! Reticle's placements), all 335 of the XC7A100T's and eleven designs made to
//! ask (`gen_clock_rules.py`); `docs/fpga-vivado-parity.md`, "Vivado's bits
//! beside a clock, answered".
//!
//! - **`GCLK<k>_ENABLE_*`.** A global track `k` leaves its buffer's tile
//!   (`CLK_BUFG_CK_GCLK<k>` in a `CLK_BUFG_*_R`) and is cut at every
//!   `CLK_BUFG_REBUF` of the column. The live segments are the buffer's own
//!   and every one between it and each clock row the track is used in; each
//!   live segment sets `GCLK<k>_ENABLE_ABOVE` in the rebuffer above it and
//!   `GCLK<k>_ENABLE_BELOW` in the one below, and no other enable is set.
//!   A segment's far end may be a tile the region did not load (the
//!   rebuffer at `Y142` above the row at `Y130`, on both dies), and its bit
//!   is set all the same, straight into the frames
//!   ([`XrayFabric::clock_rule_bits`]).
//! - **`HCLK_CMT_CK_BUFHCLK<n>_USED`**, in the row's `HCLK_CMT` for a
//!   `BUFHCE_X0Y<n>` in use in its `CLK_HROW` and in its `HCLK_CMT_L` for an
//!   `X1`, and nowhere when that side of the row has no such tile (the
//!   XC7A35T's top row, whose right is a transceiver column).
//!
//! # What it replaced
//!
//! Until 2026-11-18 the first was nextpnr-xilinx's: both enables of a track in
//! every rebuffer of a column a route crossed, as far as the region went, and
//! none at all for a track no route took across a rebuffer. The second was set
//! only where a route crossed the clock management tile, or for a `BUFHCE`
//! the design instantiated. Vivado's rules were known and left unapplied
//! because they change the ChipWhisperer-Husky's bitstreams, which had been
//! measured on a board; the user decided that day that parity with Vivado
//! comes first. **Neither rule has been measured on a part**: what is known
//! is that Vivado writes these bits and no others.

use std::collections::{BTreeMap, BTreeSet};

use super::XrayFabric;
use super::sites;
use crate::fpga::arch::{ConfigBit, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, BitstreamError};
use crate::fpga::route::Routing;
use crate::fpga::xc7::{FrameData, Xc7Error};

/// The bits of the two rules and where their tiles are, over the whole die:
/// what [`XrayFabric::enable_global_clocks`] and
/// [`XrayFabric::clock_rule_bits`] need beyond the region's architecture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockColumn {
    /// The tile type that holds the rebuffers, empty when the database has
    /// none.
    pub tile_type: String,
    /// Each track's `GCLK<k>_ENABLE_ABOVE` bits.
    pub above: BTreeMap<u32, Vec<ConfigBit>>,
    /// Each track's `GCLK<k>_ENABLE_BELOW` bits.
    pub below: BTreeMap<u32, Vec<ConfigBit>>,
    /// Every rebuffer of the die, by grid position, sorted.
    pub rebuffers: Vec<(u32, u32)>,
    /// Every row's clock management tile, by grid position, with whether it
    /// is the left one (`HCLK_CMT`) or the right (`HCLK_CMT_L`).
    pub row_managers: Vec<((u32, u32), bool)>,
    /// `HCLK_CMT_CK_BUFHCLK<n>_USED`'s bits, by `n`, for the left tile type
    /// and the right.
    pub used: [BTreeMap<u32, Vec<ConfigBit>>; 2],
    /// A `CLK_HROW` tile type's `BUFHCE.BUFHCE_X<side>Y<n>.IN_USE` bits, by
    /// tile type and then `(side, n)`.
    pub in_use: BTreeMap<String, BTreeMap<(u32, u32), Vec<ConfigBit>>>,
}

/// One bit of one tile, by the tile's grid position: what the rules set.
pub type TileBit = ((u32, u32), ConfigBit);

/// The two halves of the global buffer column, whose `CLK_BUFG_CK_GCLK<k>`
/// is a track's start.
const BUFFER_TILES: [&str; 2] = ["CLK_BUFG_BOT_R", "CLK_BUFG_TOP_R"];

/// The clock rows' spine tiles, where a track is used.
const ROW_TILES: [&str; 2] = ["CLK_HROW_BOT_R", "CLK_HROW_TOP_R"];

impl ClockColumn {
    /// The table of a database's features and tiles: `features` by tile type,
    /// `tiles` as `(type, grid x, grid y)` over the whole die.
    pub(super) fn new<'a>(
        features: &std::collections::HashMap<String, super::FeatureSet>,
        tiles: impl Iterator<Item = (&'a str, u32, u32)>,
    ) -> ClockColumn {
        let mut out = ClockColumn::default();
        let mut names: Vec<&String> = features.keys().collect();
        names.sort();
        for name in names {
            let set = &features[name];
            let mut above = BTreeMap::new();
            let mut below = BTreeMap::new();
            for feature in set.features() {
                let Some(track) = sites::global_clock_enable_track(&feature.name) else {
                    continue;
                };
                let into = if feature.name.ends_with("_ABOVE") {
                    &mut above
                } else {
                    &mut below
                };
                into.entry(track)
                    .or_insert_with(Vec::new)
                    .extend(feature.ones.iter().copied());
            }
            if out.tile_type.is_empty() && !(above.is_empty() && below.is_empty()) {
                out.tile_type = name.clone();
                out.above = above;
                out.below = below;
            }
        }
        for (side, tile_type) in ["HCLK_CMT", "HCLK_CMT_L"].into_iter().enumerate() {
            let Some(set) = features.get(tile_type) else {
                continue;
            };
            for feature in set.features() {
                if let Some(n) = feature
                    .name
                    .strip_prefix("HCLK_CMT_CK_BUFHCLK")
                    .and_then(|r| r.strip_suffix("_USED"))
                    .and_then(|n| n.parse::<u32>().ok())
                {
                    out.used[side].insert(n, feature.ones.clone());
                }
            }
        }
        for tile_type in ROW_TILES {
            let Some(set) = features.get(tile_type) else {
                continue;
            };
            for feature in set.features() {
                let Some((side, n)) = feature
                    .name
                    .strip_prefix("BUFHCE.BUFHCE_X")
                    .and_then(|r| r.strip_suffix(".IN_USE"))
                    .and_then(|r| r.split_once('Y'))
                else {
                    continue;
                };
                if let (Ok(side), Ok(n)) = (side.parse::<u32>(), n.parse::<u32>()) {
                    out.in_use
                        .entry(tile_type.to_owned())
                        .or_default()
                        .insert((side, n), feature.ones.clone());
                }
            }
        }
        for (tile_type, x, y) in tiles {
            if !out.tile_type.is_empty() && tile_type == out.tile_type {
                out.rebuffers.push((x, y));
            } else if tile_type == "HCLK_CMT" {
                out.row_managers.push(((x, y), true));
            } else if tile_type == "HCLK_CMT_L" {
                out.row_managers.push(((x, y), false));
            }
        }
        out.rebuffers.sort_unstable();
        out.row_managers.sort_unstable();
        out
    }
}

/// The track a wire of a clock row's spine or a buffer tile names: `k` of
/// `CLK_HROW_R_CK_GCLK<k>` or `CLK_BUFG_CK_GCLK<k>`.
fn track_of(wire: &str, prefix: &str) -> Option<u32> {
    wire.strip_prefix(prefix)?.parse().ok()
}

/// The rebuffers either side of grid row `at` in one column: the nearest
/// above (a smaller row) and the nearest below.
fn segment(rebuffers: &[u32], at: u32) -> (Option<u32>, Option<u32>) {
    let above = rebuffers.iter().copied().filter(|r| *r < at).max();
    let below = rebuffers.iter().copied().filter(|r| *r > at).min();
    (above, below)
}

impl XrayFabric {
    /// The tile type at a grid position of the loaded region.
    fn type_at(&self, tile: (u32, u32)) -> Option<&str> {
        self.arch
            .tile_index_at(tile.0, tile.1)
            .map(|t| self.arch.tile_types[t].name.as_str())
    }

    /// Every rebuffer enable Vivado's rule sets for the global clocks
    /// `routing` uses, over the whole die: see the module documentation.
    pub(super) fn global_clock_enable_bits(
        &self,
        graph: &RoutingGraph,
        routing: &Routing,
    ) -> Vec<TileBit> {
        // Each track's buffer, as its tile, and the rows it is used in.
        let mut buffer: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
        let mut rows: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for route in routing.routes() {
            for id in &route.pips {
                let pip = graph.pip(*id);
                let Some(kind) = self.type_at(pip.tile) else {
                    continue;
                };
                if BUFFER_TILES.contains(&kind) {
                    if let Some(k) = track_of(&graph.wire(pip.to).name, "CLK_BUFG_CK_GCLK") {
                        buffer.insert(k, pip.tile);
                    }
                } else if ROW_TILES.contains(&kind) {
                    for node in [pip.from, pip.to] {
                        if let Some(k) = track_of(&graph.wire(node).name, "CLK_HROW_R_CK_GCLK") {
                            rows.entry(k).or_default().insert(pip.tile.1);
                        }
                    }
                }
            }
        }
        let mut out = BTreeSet::new();
        for (k, (x, at)) in buffer {
            let column: Vec<u32> = self
                .clocks
                .rebuffers
                .iter()
                .filter(|(rx, _)| *rx == x)
                .map(|(_, ry)| *ry)
                .collect();
            let used = rows.get(&k).cloned().unwrap_or_default();
            let top = used.iter().copied().chain([at]).min().unwrap_or(at);
            let bottom = used.iter().copied().chain([at]).max().unwrap_or(at);
            // Every segment from the topmost row's to the bottommost's, the
            // buffer's among them: the rule's union, since a segment between
            // the buffer and a row is between two of these.
            let mut live = BTreeSet::new();
            live.insert(segment(&column, top));
            live.insert(segment(&column, bottom));
            for r in column.iter().copied().filter(|r| top < *r && *r < bottom) {
                live.insert(segment(&column, r - 1));
                live.insert(segment(&column, r + 1));
            }
            for (above, below) in live {
                if let (Some(y), Some(bits)) = (above, self.clocks.above.get(&k)) {
                    out.extend(bits.iter().map(|b| ((x, y), *b)));
                }
                if let (Some(y), Some(bits)) = (below, self.clocks.below.get(&k)) {
                    out.extend(bits.iter().map(|b| ((x, y), *b)));
                }
            }
        }
        out.into_iter().collect()
    }

    /// Switches on, in the region, the rebuffer enables Vivado sets for the
    /// global clocks `routing` uses, and says how many bits that was.
    ///
    /// # Why this is not a pip's business
    ///
    /// A 7-series global clock leaves its `BUFGCTRL` onto one of 32 vertical
    /// tracks, and the track is **cut** at every `CLK_BUFG_REBUF` tile of the
    /// column. Each cut has a buffer enable on each side
    /// (`sites::global_clock_enable` derives which, and says how that was
    /// measured). A route charges the two of each rebuffer it crosses; the
    /// ends of the segments it does not cross are tiles it never enters, so
    /// no pip of theirs is taken. This sets them, by Vivado's rule (the module
    /// documentation), where the region has their tile;
    /// [`XrayFabric::clock_rule_bits`] gives the rest.
    ///
    /// # Errors
    ///
    /// [`BitstreamError`] when a rebuffer's bits fall outside the tile's
    /// bitmap, which would mean the `segbits` file and `tilegrid.json`
    /// disagree about the tile type's shape.
    pub fn enable_global_clocks(
        &self,
        graph: &RoutingGraph,
        routing: &Routing,
        bits: &mut Bitstream,
    ) -> Result<usize, BitstreamError> {
        let mut count = 0usize;
        for (tile, bit) in self.global_clock_enable_bits(graph, routing) {
            if self.type_at(tile).is_some() {
                bits.set(tile, bit)?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Each `HCLK_CMT_CK_BUFHCLK<n>_USED` Vivado sets for the `BUFHCE`s in use
    /// in `bits`, over the whole die: see the module documentation. A `BUFHCE`
    /// is in use when its `IN_USE` is set, by a global clock's way into the
    /// row or by one the design placed.
    fn horizontal_buffer_bits(&self, bits: &Bitstream) -> Vec<TileBit> {
        let mut out = BTreeSet::new();
        for y in 0..self.arch.height {
            for x in 0..self.arch.width {
                let Some(table) = self.type_at((x, y)).and_then(|t| self.clocks.in_use.get(t))
                else {
                    continue;
                };
                for ((side, n), ones) in table {
                    if ones.is_empty() || !ones.iter().all(|b| bits.get((x, y), *b) == Some(true)) {
                        continue;
                    }
                    let left = *side == 0;
                    let Some(used) = self.clocks.used[usize::from(!left)].get(n) else {
                        continue;
                    };
                    for (tile, _) in self
                        .clocks
                        .row_managers
                        .iter()
                        .filter(|(at, l)| at.1 == y && *l == left)
                    {
                        out.extend(used.iter().map(|b| (*tile, *b)));
                    }
                }
            }
        }
        out.into_iter().collect()
    }

    /// The bits of both rules of the module documentation that are not set
    /// yet: the `BUFHCLK<n>_USED`s in the region are set in `bits` here, and
    /// what falls outside the region, of either rule, is returned for
    /// [`XrayFabric::set_beyond_region`] to put in the frames. Called once
    /// every pass that sets a `BUFHCE`'s `IN_USE` has run.
    ///
    /// # Errors
    ///
    /// [`BitstreamError`] as [`XrayFabric::enable_global_clocks`].
    pub fn clock_rule_bits(
        &self,
        graph: &RoutingGraph,
        routing: &Routing,
        bits: &mut Bitstream,
    ) -> Result<Vec<TileBit>, BitstreamError> {
        let mut beyond = Vec::new();
        let rows = self.horizontal_buffer_bits(bits);
        for (tile, bit) in rows {
            if self.type_at(tile).is_some() {
                bits.set(tile, bit)?;
            } else {
                beyond.push((tile, bit));
            }
        }
        beyond.extend(
            self.global_clock_enable_bits(graph, routing)
                .into_iter()
                .filter(|(tile, _)| self.type_at(*tile).is_none()),
        );
        Ok(beyond)
    }

    /// Sets `bits`, of tiles the region did not load, in the frames, where
    /// every tile of the die has its window.
    ///
    /// # Errors
    ///
    /// [`Xc7Error::OutOfTile`] for a bit no window of its tile holds.
    pub fn set_beyond_region(
        &self,
        frames: &mut FrameData,
        bits: &[TileBit],
    ) -> Result<(), Xc7Error> {
        for (tile, bit) in bits {
            let (address, word, index) =
                self.frames.locate(*tile, *bit).ok_or(Xc7Error::OutOfTile {
                    tile: *tile,
                    bit: *bit,
                })?;
            frames.set(address, word, index)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::segment;

    /// The XC7A100T's rebuffers and the far end of the row at `Y130`: grid row
    /// 78 sits between the rebuffers at grid rows 66 (`Y142`) and 91 (`Y117`).
    #[test]
    fn a_rows_segment_is_between_the_rebuffers_either_side() {
        let column = [14, 39, 66, 91, 118, 143, 170, 195];
        assert_eq!(segment(&column, 78), (Some(66), Some(91)));
        assert_eq!(segment(&column, 5), (None, Some(14)));
        assert_eq!(segment(&column, 200), (Some(195), None));
    }
}
