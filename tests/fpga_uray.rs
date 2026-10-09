//! The Xilinx UltraScale+ frames against a **real** database and a
//! bitstream Vivado made for a ZCU104.
//!
//! # Everything here skips without its inputs
//!
//! - **The database**, Project U-Ray's ZU7EV die: `reticle fetch
//!   prjuray-db` puts the pinned copy where these tests find it, and
//!   `RETICLE_URAYDB` names another copy instead.
//! - **The reference bitstream**, `RETICLE_ZCU104_REF` pointing at a
//!   directory holding `base.bit`. That is the base overlay of AMD's PYNQ
//!   3.1 image for the ZCU104, built by Vivado 2024.1 for an
//!   `xczu7ev-ffvc1156-2-e`; the image installs it under
//!   `/usr/local/share/pynq-venv/lib/python3.10/site-packages/pynq/overlays/base/`.
//!   It is AMD's build output and is not in this repository.
//!
//! # What these tests would and would not catch
//!
//! They pin the layer every later UltraScale+ step stands on: the frame
//! layout, the frame address format, where each tile's bits sit, and
//! which bits are ECC. A layout off by one frame in any column would
//! shift every later column's bits and fail the ownership check by
//! hundreds of thousands of bits. A wrong ECC location would leave tens
//! of thousands unowned.
//!
//! They say nothing about whether Reticle could *write* a bitstream that
//! configures this part: it cannot yet, and the ECC is not understood
//! (see `docs/fpga-uray.md`).

#![cfg(feature = "fpga")]

use std::collections::HashMap;
use std::path::Path;

use reticle::fpga::uray::{
    self, FrameLayout, PAD_FRAMES_PER_ROW, TileGrid, TileTypeBits, WORDS_PER_FRAME,
};

/// The die directory, and the IDCODE Vivado writes for an XCZU7EV.
const DIE: &str = "xazu7ev";
const IDCODE: u32 = 0x04A5_A093;
/// Frames the layout has, before and after the pad frames.
const DATA_FRAMES: usize = 51_882;
const STREAM_FRAMES: usize = 51_906;

/// Where `reticle fetch <name>` puts the pinned copy, if it is there.
///
/// The name and version are spelled out because a test cannot see the
/// binary's `datadir` module; a unit test there checks that this file
/// asks for the version it pins.
fn fetched(name: &str, version: &str, probe: &str) -> Option<String> {
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let root = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")))?;
    let dir = format!("{root}/{name}/{version}");
    Path::new(&format!("{dir}/{probe}"))
        .is_file()
        .then_some(dir)
}

/// The database root, or `None` with a line saying what is missing.
fn uraydb() -> Option<String> {
    let probe = "xazu7ev/tilegrid.json";
    let Ok(root) = std::env::var("RETICLE_URAYDB") else {
        let pinned = fetched(
            "prjuray-db",
            "9e7d3e7965240fd260d6549a1883a01a152ae490",
            probe,
        );
        if pinned.is_none() {
            eprintln!(
                "skipped: needs Project U-Ray's UltraScale+ database; run \
                 `reticle fetch prjuray-db` or set RETICLE_URAYDB"
            );
        }
        return pinned;
    };
    if !Path::new(&format!("{root}/{probe}")).is_file() {
        eprintln!("skipped: RETICLE_URAYDB is `{root}` but `{probe}` is not in it");
        return None;
    }
    Some(root)
}

/// Vivado's ZCU104 base overlay, or `None` having said where to get it.
fn reference() -> Option<Vec<u8>> {
    let Ok(dir) = std::env::var("RETICLE_ZCU104_REF") else {
        eprintln!(
            "skipped: needs Vivado's own bitstream for a ZCU104; set RETICLE_ZCU104_REF to \
             a directory holding base.bit, the base overlay of the PYNQ 3.1 image. See \
             docs/fpga-uray.md"
        );
        return None;
    };
    let path = Path::new(&dir).join("base.bit");
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(err) => {
            eprintln!("skipped: `{}` would not read: {err}", path.display());
            None
        }
    }
}

fn grid(root: &str) -> TileGrid {
    let path = format!("{root}/{DIE}/tilegrid.json");
    let text = std::fs::read_to_string(&path).expect("the probe file reads");
    TileGrid::parse(&text, &path).expect("the tile grid parses")
}

/// Every tile type's bits that the grid uses.
fn tile_types(root: &str, grid: &TileGrid) -> HashMap<String, TileTypeBits> {
    let mut out = HashMap::new();
    for tile in grid.tiles() {
        if tile.windows.is_empty() || out.contains_key(&tile.kind) {
            continue;
        }
        let stem = TileTypeBits::file_stem(&tile.kind);
        let seg_path = format!("{root}/{stem}.db");
        let Ok(segbits) = std::fs::read_to_string(&seg_path) else {
            continue;
        };
        let dflt_path = format!("{root}/{}.db", stem.replacen("segbits_", "defaults_", 1));
        let defaults = std::fs::read_to_string(&dflt_path).ok();
        let bits = TileTypeBits::parse(
            &segbits,
            &seg_path,
            defaults.as_deref().map(|d| (d, dflt_path.as_str())),
        )
        .expect("the tile type's files parse");
        out.insert(tile.kind.clone(), bits);
    }
    out
}

#[test]
fn the_layout_is_the_frame_count_vivado_wrote() {
    let Some(root) = uraydb() else { return };
    let layout = FrameLayout::from_grid(&grid(&root));
    // Six clock region rows of each block type, each ending in its pads.
    assert_eq!(layout.rows().len(), 12);
    assert_eq!(layout.data_frames(), DATA_FRAMES);
    assert_eq!(layout.frames(), STREAM_FRAMES);
    assert_eq!(STREAM_FRAMES - DATA_FRAMES, 12 * PAD_FRAMES_PER_ROW);
    // Every row of a block type is as wide as every other: the rows under
    // the processor system borrow the widths of the rows above it.
    for block in [0u8, 1] {
        let widths: Vec<&Vec<u32>> = layout
            .rows()
            .iter()
            .filter(|((b, _), _)| *b == block)
            .map(|(_, w)| w)
            .collect();
        assert!(widths.windows(2).all(|p| p[0] == p[1]), "block {block}");
    }

    let Some(bytes) = reference() else { return };
    let bit = uray::read_bit(&bytes).expect("Vivado's bitstream reads, CRCs and all");
    assert_eq!(bit.part, "xczu7ev-ffvc1156-2-e");
    assert_eq!(bit.idcode, Some(IDCODE));
    assert_eq!(bit.frames.len(), STREAM_FRAMES * WORDS_PER_FRAME);
}

/// The strongest check this file has: every set bit Vivado wrote, ECC
/// aside, lies inside some tile's window, and almost all of them are
/// a feature the database names.
#[test]
fn every_bit_of_vivados_zcu104_bitstream_has_an_owner() {
    let Some(root) = uraydb() else { return };
    let Some(bytes) = reference() else { return };
    let grid = grid(&root);
    let layout = FrameLayout::from_grid(&grid);
    let bit = uray::read_bit(&bytes).unwrap();
    let decoded = uray::decode(&grid, &layout, &bit.frames).unwrap();

    assert_eq!(decoded.pad_bits, 0, "the pad frames are zero");
    assert_eq!(decoded.set_bits, 3_524_987);
    assert_eq!(
        decoded.unowned.len(),
        0,
        "set bits outside every tile, first few: {:?}",
        &decoded.unowned[..decoded.unowned.len().min(8)]
    );

    let explained = uray::explain(&grid, &decoded, &tile_types(&root, &grid));
    let unexplained = explained.unexplained_total();
    eprintln!(
        "{} set bits: {} explained, {unexplained} not; by tile type: {:?}",
        decoded.set_bits, explained.explained, explained.unexplained
    );
    assert_eq!(explained.explained + unexplained, decoded.set_bits);
    // Pinned exactly: the database and the bitstream are both pinned, so
    // any change here is a change in this code. The bits left are
    // dominated by slice-M modes (LUT RAM and shift registers, which the
    // base overlay's two MicroBlazes use) and interconnect interface
    // tiles; docs/fpga-uray.md lists them.
    assert_eq!(unexplained, UNEXPLAINED);
}

/// Measured with this file's own decoder; see the test above.
const UNEXPLAINED: usize = 10_827;

/// The writer is Vivado's recipe word for word: given the reference
/// bitstream's own header and frames, it writes the reference bitstream,
/// byte for byte. And the `.bin` derived from it is the one PYNQ loads,
/// when that is beside it.
///
/// This would catch any word of the sequence out of place, a wrong
/// register value, a CRC computed over the wrong words, or a wrong
/// preamble. It would not catch a sequence that Vivado happens to use for
/// this design but that some other set of frames needs differently.
#[test]
fn the_writer_reproduces_vivados_bitstream_byte_for_byte() {
    let Some(bytes) = reference() else { return };
    let Some(root) = uraydb() else { return };
    let layout = FrameLayout::from_grid(&grid(&root));
    let bit = uray::read_bit(&bytes).unwrap();
    let written = uray::write_bit(&bit.header, bit.idcode.unwrap(), &layout, &bit.frames).unwrap();
    assert_eq!(written.len(), bytes.len());
    let first = written.iter().zip(&bytes).position(|(a, b)| a != b);
    assert_eq!(first, None, "the first byte that differs");

    let Ok(dir) = std::env::var("RETICLE_ZCU104_REF") else {
        return;
    };
    match std::fs::read(Path::new(&dir).join("base.bin")) {
        Ok(bin) => assert!(uray::bin_from_bit(&written).unwrap() == bin),
        Err(_) => eprintln!("skipped the .bin half: no base.bin beside base.bit"),
    }
}

/// What an unused tile looks like in Vivado's output: how many tiles of
/// each type with `defaults` carry every default bit, some, or none.
/// Printed, not asserted: it is what tells whether a bitstream Reticle
/// writes must set the defaults of the tiles it does not use.
#[test]
#[ignore = "prints a measurement"]
fn how_vivado_leaves_the_default_bits() {
    let Some(root) = uraydb() else { return };
    let Some(bytes) = reference() else { return };
    let grid = grid(&root);
    let layout = FrameLayout::from_grid(&grid);
    let bit = uray::read_bit(&bytes).unwrap();
    let decoded = uray::decode(&grid, &layout, &bit.frames).unwrap();
    let types = tile_types(&root, &grid);
    let mut counts: std::collections::BTreeMap<&str, [usize; 3]> = Default::default();
    for (index, tile) in grid.tiles().iter().enumerate() {
        let Some(info) = types.get(&tile.kind) else {
            continue;
        };
        if info.defaults.is_empty() {
            continue;
        }
        let set: std::collections::HashSet<_> = decoded
            .tiles
            .get(&index)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let on = info.defaults.iter().filter(|b| set.contains(b)).count();
        let slot = if on == info.defaults.len() {
            0
        } else if on == 0 {
            2
        } else {
            1
        };
        counts.entry(tile.kind.as_str()).or_default()[slot] += 1;
    }
    for (kind, [all, some, none]) in counts {
        eprintln!("{kind}: all {all}, some {some}, none {none}");
    }
}

/// The two `INIT` bits the database files as defaults are ordinary
/// truth-table bits: read through the full 64-bit layout, the slice where
/// Vivado ties two processor inputs low holds two tables of contents zero
/// (the `D` and `F` tables, whose outputs reach the interface column) and
/// six of exactly the unused pattern. See `uray::slice`.
///
/// If the missing bits were stored inverted, the two zero tables would
/// read `0x8000_0000_8000_0000` and the unused ones zero; if the layout
/// were wrong, none would read either value.
#[test]
fn vivados_constant_zero_tables_read_zero_through_the_full_layout() {
    use reticle::fpga::uray::slice::{self, LUT_LETTERS, UNUSED_LUT_INIT};
    let Some(root) = uraydb() else { return };
    let Some(bytes) = reference() else { return };
    let grid = grid(&root);
    let layout = FrameLayout::from_grid(&grid);
    let bit = uray::read_bit(&bytes).unwrap();
    let decoded = uray::decode(&grid, &layout, &bit.frames).unwrap();
    let types = tile_types(&root, &grid);
    let index = grid
        .tiles()
        .iter()
        .position(|t| t.name == "CLEL_R_X27Y180")
        .unwrap();
    let set: std::collections::HashSet<_> = decoded.tiles[&index].iter().copied().collect();
    let mut contents = Vec::new();
    for letter in LUT_LETTERS {
        let init = slice::lut_init_bits(&types["CLEL_R"], letter).unwrap();
        contents.push((letter, slice::read_lut(&init, &|b| set.contains(&b))));
    }
    for (letter, value) in contents {
        let expected = if matches!(letter, 'D' | 'F') {
            0
        } else {
            UNUSED_LUT_INIT
        };
        assert_eq!(value, expected, "{letter}6LUT reads {value:#018x}");
    }
}
