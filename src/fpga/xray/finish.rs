//! What `write_bitstream` adds to the frames: the pads and configuration-tile
//! bits no netlist asks for.
//!
//! The rest of this crate paints what a design needs. Vivado also sets bits for
//! every design, or for an option, and these are they:
//!
//! - **The `PUDC_B` pad, always**: an input with a pull-up, in every `.bit`
//!   measured here on the XC7A100T and XC7A35T, whether or not the design goes
//!   near it. `reticle fpga --decode` on a Vivado `.bit` showed it, in a pad no
//!   design had named.
//! - **The `EMCCLK` pad**, the same, under `EXTMASTERCCLK_EN`.
//! - **`UNUSEDPIN`**: `PULLTYPE.<value>` on every other pad of the die, bonded
//!   or not, which is why this reads the database's tile list and not the loaded
//!   fabric: `--region` loads part of a die, and this is about all of it. The
//!   default, `PULLDOWN`, is the field with every bit clear.
//! - **The bits [`BitstreamOptions::config_tile_bits`] gives**.
//!
//! **Measured** by the decoded difference between Vivado's `.bit` with and
//! without each option; `tests/fpga_xray_bitstream.rs` holds the result to the
//! bytes of Vivado's stream, on both parts.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::{FeatureSet, Part, XrayDatabase, XrayError, XrayTile, block_ram_rows};
use crate::fpga::arch::{ConfigBit, RoutingGraph};
use crate::fpga::place::{Netlist, Placement};
use crate::fpga::xc7::options::{BitstreamOptions, ConfigTiles, UnusedPull};
use crate::fpga::xc7::{FrameAddress, FrameData, TileBits};
use crate::ir::memfile::FileProvider;

/// The configuration an otherwise unused pad of a bank gets: an input, with a
/// pull-up, as Vivado gives `PUDC_B` and `EMCCLK`.
const CONFIG_PAD: [&str; 3] = [
    "LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVDS_25_LVTTL_SSTL135_SSTL15_TMDS_33.IN_ONLY",
    "LVCMOS25_LVCMOS33_LVTTL.IN",
    "PULLTYPE.PULLUP",
];

/// What [`XrayDatabase::finish_frames`] did, for a build to report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinishedFrames {
    /// Pads configured for `PUDC_B` or `EMCCLK`.
    pub config_pads: usize,
    /// Pads `UNUSEDPIN` reached.
    pub unused_pads: usize,
    /// Bits set in the configuration tiles.
    pub config_tile_bits: usize,
}

/// Where a slice's flip-flop reads back, as bit of frame 31 of the tile, from
/// the tile's first bit, for the slice with the lower site `X` and the one with
/// the higher. **Measured** by placing a flip-flop on each bel of four slices:
/// `testdata/fpga/vivado/bitstream/ll_probe.ll.txt` is what Vivado said.
const CAPTURE_FRAME: u32 = 31;
const CAPTURE_BITS: [(&str, &str, u32, u32); 8] = [
    ("AFF", "AQ", 3, 4),
    ("A5FF", "A5FF.Q", 6, 5),
    ("B5FF", "B5FF.Q", 22, 23),
    ("BFF", "BQ", 28, 29),
    ("CFF", "CQ", 33, 34),
    ("C5FF", "C5FF.Q", 41, 42),
    ("D5FF", "D5FF.Q", 51, 52),
    ("DFF", "DQ", 58, 59),
];

/// One line's position in a logic location file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogicLocation {
    /// The bit's number from the start of the frames, pads counted.
    pub bit: usize,
    /// The frame address.
    pub frame: u32,
    /// The bit within the frame.
    pub offset: u32,
}

impl LogicLocation {
    fn of(part: &Part, address: u32, offset: u32) -> Option<LogicLocation> {
        let position = part.layout.position(FrameAddress::from_u32(address))?;
        Some(LogicLocation {
            bit: position * crate::fpga::xc7::WORDS_PER_FRAME * 32 + offset as usize,
            frame: address,
            offset,
        })
    }
}

impl XrayDatabase {
    /// Where a flip-flop on `bel` (`AFF`, `A5FF` ...) of slice `site` reads
    /// back, and what Vivado's `.ll` calls it, from the part and
    /// [`XrayDatabase::tiles`]. `None` for a site or a bel that is not a slice's.
    pub fn flip_flop_location(
        part: &Part,
        tiles: &[XrayTile],
        site: &str,
        bel: &str,
    ) -> Option<(LogicLocation, &'static str)> {
        let tile = tiles
            .iter()
            .find(|t| t.sites.iter().any(|(s, _)| s == site))?;
        flip_flop_location_in(part, tile, site, bel)
    }

    /// Where each contents bit of the block RAM `site` reads back, keyed by
    /// whether it is a parity bit and its number: the `BIT<n>` and `PARBIT<n>`
    /// of Vivado's `.ll`, which numbers a `RAMB18` as half of the `RAMB36` it
    /// is in. Empty for a site that is not a `RAMB18`'s.
    ///
    /// # Errors
    ///
    /// [`XrayError`] when the database cannot be read.
    pub fn block_ram_locations(
        &self,
        files: &dyn FileProvider,
        part: &Part,
        tiles: &[XrayTile],
        site: &str,
    ) -> Result<HashMap<(bool, u32), LogicLocation>, XrayError> {
        let mut out = HashMap::new();
        let Some(tile) = tiles
            .iter()
            .find(|t| t.sites.iter().any(|(s, _)| s == site))
        else {
            return Ok(out);
        };
        let Some(half) = (0..2).find(|h| block_ram_site(tile, *h) == Some(site)) else {
            return Ok(out);
        };
        let rows = block_ram_rows(tiles);
        let row = rows.get(tile.tile_type.as_str()).copied();
        let Some(set) = self.features_for(files, tile, row)? else {
            return Ok(out);
        };
        for feature in set.features() {
            let Some((kind, index)) = block_ram_feature(&feature.name, half) else {
                continue;
            };
            if let Some(at) = feature
                .ones
                .first()
                .and_then(|one| block_ram_bit(part, tile, *one))
            {
                out.insert((kind == "PARBIT", index), at);
            }
        }
        Ok(out)
    }

    /// The text of a logic location file for a placed design: what Vivado's
    /// `-logic_location_file` writes, for what this flow places that it has a
    /// table of. That is the slices' flip-flops and the contents of a
    /// `RAMB18E1`, by `tests/fpga_xray_bitstream.rs` the same positions as
    /// Vivado's. **It is not all of Vivado's file**: an IO register and a
    /// lookup table used as memory have lines there and none here, and the net
    /// names are this flow's own.
    ///
    /// # Errors
    ///
    /// [`XrayError`] when the database cannot be read.
    pub fn logic_location_text(
        &self,
        files: &dyn FileProvider,
        part: &Part,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
    ) -> Result<String, XrayError> {
        let tiles = self.tiles(files)?;
        let by_position: HashMap<(u32, u32), &XrayTile> =
            tiles.iter().map(|t| ((t.grid_x, t.grid_y), t)).collect();
        let rows = block_ram_rows(&tiles);
        let mut lines: Vec<(usize, String)> = Vec::new();
        for (index, instance) in netlist.instances.iter().enumerate() {
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(tile) = by_position.get(&site.tile) else {
                continue;
            };
            if instance.kind == "ff" {
                let Some((prefix, bel)) = site.bel.rsplit_once('_') else {
                    continue;
                };
                let Some((name, _)) = super::sites::site_of_prefix(tile, prefix) else {
                    continue;
                };
                let Some((at, latch)) = flip_flop_location_in(part, tile, name, bel) else {
                    continue;
                };
                let net = instance
                    .pins
                    .iter()
                    .filter_map(|p| netlist.pins.get(*p))
                    .find(|p| p.output)
                    .and_then(|p| p.signal)
                    .and_then(|s| netlist.signals.get(s))
                    .map_or("", |s| s.name.as_str());
                lines.push((
                    at.bit,
                    format!(
                        "Bit {} 0x{:08x} {:>6} Block={name} Latch={latch} Net={net}",
                        at.bit, at.frame, at.offset
                    ),
                ));
            } else if instance.kind == "bram" {
                let half = u32::from(site.bel == "RAMB18_Y1");
                let Some(name) = block_ram_site(tile, half) else {
                    continue;
                };
                let row = rows.get(tile.tile_type.as_str()).copied();
                let Some(set) = self.features_for(files, tile, row)? else {
                    continue;
                };
                for feature in set.features() {
                    let Some((kind, index)) = block_ram_feature(&feature.name, half) else {
                        continue;
                    };
                    let Some(at) = feature
                        .ones
                        .first()
                        .and_then(|one| block_ram_bit(part, tile, *one))
                    else {
                        continue;
                    };
                    lines.push((
                        at.bit,
                        format!(
                            "Bit {} 0x{:08x} {:>6} Block={name} Ram=B:{kind}{index}",
                            at.bit, at.frame, at.offset
                        ),
                    ));
                }
            }
        }
        lines.sort();
        let mut out = String::new();
        let _ = writeln!(out, "Revision 3");
        let _ = writeln!(
            out,
            "; Logic location file, by Reticle. Only slice flip-flops and RAMB18E1 contents."
        );
        let _ = writeln!(out, "Info STARTSEL0=1");
        for (_, line) in lines {
            let _ = writeln!(out, "{line}");
        }
        Ok(out)
    }

    /// Where the two configuration tiles that hold options' bits start.
    ///
    /// # Errors
    ///
    /// [`XrayError::Malformed`] when the database has neither, which no 7-series
    /// die has been seen to do.
    pub fn config_tiles(&self, files: &dyn FileProvider) -> Result<ConfigTiles, XrayError> {
        let tiles = self.tiles(files)?;
        config_tiles_of(&tiles).ok_or_else(|| XrayError::Malformed {
            path: format!("{}/{}", self.root, self.family),
            message: "no CFG_CENTER_MID and MONITOR_BOT tiles in the tile grid".to_owned(),
        })
    }

    /// Sets in `frames` what `write_bitstream` would for these options; see the
    /// module's documentation. Bits only ever go from clear to set, and a pad a
    /// design has configured keeps what it has.
    ///
    /// # Errors
    ///
    /// [`XrayError`] when the database cannot be read, or a feature this names
    /// is not in it.
    pub fn finish_frames(
        &self,
        files: &dyn FileProvider,
        frames: &mut FrameData,
        options: &BitstreamOptions,
    ) -> Result<FinishedFrames, XrayError> {
        let tiles = self.tiles(files)?;
        let mut report = FinishedFrames::default();
        let mut sets: HashMap<String, FeatureSet> = HashMap::new();
        let malformed = |message: String| XrayError::Malformed {
            path: format!("{}/{}", self.root, self.family),
            message,
        };

        // The config pads, by what the package says the balls are.
        let mut wanted = vec!["PUDC_B"];
        if options.ext_master_cclk.is_some() {
            wanted.push("EMCCLK");
        }
        let path = format!(
            "{}/{}/{}/package_pins.csv",
            self.root, self.family, self.part_dir
        );
        let pins = self.read(files, &path, "the part's package pins")?;
        for line in pins.lines() {
            let fields: Vec<&str> = line.split(',').collect();
            let [_, _, site, tile_name, function, ..] = fields.as_slice() else {
                continue;
            };
            if !wanted.iter().any(|w| function.contains(w)) {
                continue;
            }
            let Some(tile) = tiles.iter().find(|t| t.name == *tile_name) else {
                continue;
            };
            let set = self.feature_set(files, tile, &mut sets)?;
            let half = half_of_site(tile, site, &set);
            let bits = half_bits(&set, half);
            if bits.iter().any(|b| is_set(frames, tile, *b)) {
                continue;
            }
            for feature in CONFIG_PAD {
                let name = format!("{half}.{feature}");
                let Some(f) = set.features().iter().find(|f| f.name == name) else {
                    return Err(malformed(format!(
                        "`{}` has no feature `{name}`",
                        tile.name
                    )));
                };
                for bit in &f.ones {
                    set_bit(frames, tile, *bit).map_err(malformed)?;
                }
            }
            report.config_pads += 1;
        }

        // Every other pad of the die.
        let pull = match options.unused_pin {
            UnusedPull::Down => None,
            UnusedPull::Up => Some("PULLTYPE.PULLUP"),
            UnusedPull::None => Some("PULLTYPE.NONE"),
        };
        if let Some(pull) = pull {
            for tile in tiles
                .iter()
                .filter(|t| t.tile_type.starts_with("LIOB") || t.tile_type.starts_with("RIOB"))
            {
                let set = self.feature_set(files, tile, &mut sets)?;
                for half in ["IOB_Y0", "IOB_Y1"] {
                    let name = format!("{half}.{pull}");
                    let Some(f) = set.features().iter().find(|f| f.name == name) else {
                        continue;
                    };
                    if half_bits(&set, half)
                        .iter()
                        .any(|b| is_set(frames, tile, *b))
                    {
                        continue;
                    }
                    for bit in &f.ones {
                        set_bit(frames, tile, *bit).map_err(malformed)?;
                    }
                    report.unused_pads += 1;
                }
            }
        }

        // The configuration tiles.
        let bits = config_tiles_of(&tiles).map_or_else(Vec::new, |t| options.config_tile_bits(t));
        for (address, word, bit) in bits {
            frames
                .set(address, word, bit)
                .map_err(|e| malformed(e.to_string()))?;
            report.config_tile_bits += 1;
        }
        Ok(report)
    }

    /// One tile's feature set, kept by its feature type.
    fn feature_set(
        &self,
        files: &dyn FileProvider,
        tile: &XrayTile,
        sets: &mut HashMap<String, FeatureSet>,
    ) -> Result<FeatureSet, XrayError> {
        let key = tile.feature_type();
        if let Some(set) = sets.get(&key) {
            return Ok(set.clone());
        }
        let set = self.features_for(files, tile, None)?.unwrap_or_default();
        sets.insert(key, set.clone());
        Ok(set)
    }
}

/// [`XrayDatabase::flip_flop_location`], given the tile.
fn flip_flop_location_in(
    part: &Part,
    tile: &XrayTile,
    site: &str,
    bel: &str,
) -> Option<(LogicLocation, &'static str)> {
    let (_, latch, low, high) = CAPTURE_BITS.iter().find(|(name, ..)| *name == bel)?;
    let x = |name: &str| {
        let tail = name.rsplit_once('X')?.1;
        tail.split_once('Y')?.0.parse::<u32>().ok()
    };
    let this = x(site)?;
    let lowest = tile.sites.iter().filter_map(|(name, _)| x(name)).min()?;
    let window = &tile.bits.first()?.1;
    let bit = if this == lowest { *low } else { *high };
    let at = LogicLocation::of(
        part,
        window.baseaddr + CAPTURE_FRAME,
        window.offset * 32 + bit,
    )?;
    Some((at, latch))
}

/// The site of a half of a block RAM tile: the half `RAMB18_Y0` is the even
/// site `Y`.
fn block_ram_site(tile: &XrayTile, half: u32) -> Option<&str> {
    tile.sites
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| name.starts_with("RAMB18"))
        .find(|name| {
            name.rsplit_once('Y')
                .and_then(|(_, y)| y.parse::<u32>().ok())
                .is_some_and(|y| y % 2 == half)
        })
}

/// Whether a feature is a contents bit of this half, and what `.ll` calls it.
fn block_ram_feature(name: &str, half: u32) -> Option<(&'static str, u32)> {
    let rest = name.strip_prefix(&format!("RAMB18_Y{half}."))?;
    let (kind, rest) = if let Some(rest) = rest.strip_prefix("INITP_") {
        ("PARBIT", rest)
    } else {
        ("BIT", rest.strip_prefix("INIT_")?)
    };
    let (block, bit) = rest.strip_suffix(']')?.split_once('[')?;
    let index = u32::from_str_radix(block, 16).ok()? * 256 + bit.parse::<u32>().ok()?;
    Some((kind, index * 2 + half))
}

/// A contents bit's place, through the tile's stacked windows.
fn block_ram_bit(part: &Part, tile: &XrayTile, one: ConfigBit) -> Option<LogicLocation> {
    let mut row = one.row;
    for (_, window) in &tile.bits {
        if row < window.frames {
            return LogicLocation::of(part, window.baseaddr + row, window.offset * 32 + one.col);
        }
        row -= window.frames;
    }
    None
}

/// Which tiles hold the options' bits, from a tile list.
fn config_tiles_of(tiles: &[XrayTile]) -> Option<ConfigTiles> {
    let base = |wanted: &str| {
        tiles
            .iter()
            .find(|t| t.tile_type == wanted)
            .and_then(|t| t.bits.first().map(|(_, w)| w.baseaddr))
    };
    Some(ConfigTiles {
        center: base("CFG_CENTER_MID")?,
        monitor: base("MONITOR_BOT")?,
    })
}

/// A site's half of its tile: the higher-numbered of two is `IOB_Y0`. A `_SING`
/// tile has one site and whichever half its features are named for.
fn half_of_site(tile: &XrayTile, site: &str, set: &FeatureSet) -> &'static str {
    let has = |half: &str| {
        let prefix = format!("{half}.");
        set.features().iter().any(|f| f.name.starts_with(&prefix))
    };
    if !has("IOB_Y0") {
        return "IOB_Y1";
    }
    if !has("IOB_Y1") {
        return "IOB_Y0";
    }
    let y = |name: &str| {
        name.rsplit('Y')
            .next()
            .and_then(|digits| digits.parse::<u32>().ok())
            .unwrap_or(0)
    };
    let highest = tile
        .sites
        .iter()
        .filter(|(name, _)| name.starts_with("IOB"))
        .map(|(name, _)| y(name))
        .max()
        .unwrap_or(0);
    if y(site) == highest {
        "IOB_Y0"
    } else {
        "IOB_Y1"
    }
}

/// Every bit any feature of a pad's half sets.
fn half_bits(set: &FeatureSet, half: &str) -> Vec<ConfigBit> {
    let prefix = format!("{half}.");
    set.features()
        .iter()
        .filter(|f| f.name.starts_with(&prefix))
        .flat_map(|f| f.ones.iter().copied())
        .collect()
}

/// Where a tile bit lands, in the tile's first window: a pad tile has one.
fn locate(tile: &XrayTile, bit: ConfigBit) -> Option<(FrameAddress, usize, u32)> {
    tile.bits
        .first()
        .and_then(|(_, window): &(_, TileBits)| window.locate(bit))
}

/// Whether a tile bit is set in the frames.
fn is_set(frames: &FrameData, tile: &XrayTile, bit: ConfigBit) -> bool {
    locate(tile, bit)
        .and_then(|(address, word, index)| frames.get(address, word, index))
        .unwrap_or(false)
}

/// Sets a tile bit in the frames.
fn set_bit(frames: &mut FrameData, tile: &XrayTile, bit: ConfigBit) -> Result<(), String> {
    let Some((address, word, index)) = locate(tile, bit) else {
        return Err(format!(
            "bit {}.{} is outside tile `{}`",
            bit.row, bit.col, tile.name
        ));
    };
    frames.set(address, word, index).map_err(|e| e.to_string())
}
