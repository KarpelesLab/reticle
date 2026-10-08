//! Every wire join the 7-series loader makes, against `tileconn.json`.
//!
//! A 7-series node is wires joined across tile boundaries, and
//! `tileconn.json` is the only statement of which wire of which tile is
//! the same metal as which wire of its neighbour. The loader turns each
//! pair into two zero-bit pips, so **every cross-tile pip of the loaded
//! fabric must be a pair that file describes, for the two tile types at
//! the two ends**. These tests read the file a second time, straight off
//! the disk and independently of the loader, and check exactly that.
//!
//! # Why this is not a tautology
//!
//! A `tileconn.json` entry names *two tile types* and a grid delta. The
//! loader declares its pips on a tile *type*, so a pip can only say
//! "the wire called N, `(dx, dy)` tiles from here". That is ambiguous
//! wherever two tile types that both have a wire called N can sit at the
//! same delta — and on this fabric they can. A tall `CMT_TOP` is cut
//! into `_UPPER_B`, `_UPPER_T`, `_LOWER_B` and `_LOWER_T`, all four
//! naming their wires `CMT_TOP_*`, and `tileconn.json` pairs
//! `CMT_FIFO_EE4A0_0` with `CMT_TOP_R_UPPER_B`'s `CMT_TOP_EE4A0_0` and
//! `CMT_FIFO_EE4A0_3` with `CMT_TOP_R_LOWER_T`'s wire **of the same
//! name**, both at delta `(1, 2)`. Sixty-four (owner type, delta) pairs
//! of this fabric have two different partner types really present.
//!
//! Until 2026-10-08 the loader dropped the partner type, so a
//! `CMT_FIFO_R` tile whose `(1, 2)` neighbour is `_LOWER_T` got **both**
//! joins, welding two different pieces of metal into one node. A route
//! could then enter the clock-management column on one interconnect row
//! and leave it three rows away: `io_exercise` had 68 such hops in its
//! routing and `carry_probe` six, every signal through them silently
//! going nowhere, while the flow reported every signal routed and every
//! set bit decoding.
//! `docs/fpga-xray.md` has the account.
//!
//! # What these tests would and would not catch
//!
//! They would catch that defect — [`no_join_of_the_loaded_fabric_is_one_tileconn_does_not_describe`]
//! fails on the commit before the fix — and any
//! future join the loader invents, mis-signs or mis-offsets. They would
//! **not** catch a join `tileconn.json` describes and the loader omits
//! (nothing here counts the file's pairs back), nor anything about bits,
//! placement or whether the fabric behaves. Only a board says that.
//!
//! Every test skips, saying why, without the database.

#![cfg(feature = "fpga")]

use std::collections::HashSet;
use std::path::Path;

use reticle::fpga::xray::{GridRegion, XrayDatabase, XrayOptions};
use reticle::ir::memfile::FileProvider;
use reticle::json::Json;

const DEVICE: &str = "xc7a35t-cpg236";

/// Reads files from the filesystem, which is what the CLI does and what
/// the library never does.
struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing: the
/// same search `tests/fpga_xray.rs` makes.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a50t/tilegrid.json";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but `{root}/{probe}` is not there");
        return None;
    }
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let cache = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")));
    let dir = cache.map(|c| format!("{c}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3"));
    match dir {
        Some(dir) if Path::new(&format!("{dir}/{probe}")).is_file() => Some(dir),
        _ => {
            eprintln!(
                "skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db` \
                 or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

/// Every join `tileconn.json` states, as
/// `(type here, type there, dx, dy, wire here, wire there)`, both ways
/// round because a join is metal and metal has no direction.
///
/// Read with the crate's JSON parser but **not** with the loader's
/// reader, so that a wrong reading of the file cannot agree with itself.
fn stated_joins(root: &str) -> HashSet<(String, String, i32, i32, String, String)> {
    let path = format!("{root}/artix7/xc7a50t/tileconn.json");
    let text = std::fs::read_to_string(&path).expect("tileconn.json");
    let json = Json::parse(&text).expect("tileconn.json parses");
    let mut out = HashSet::new();
    for entry in json.as_array().expect("an array of entries") {
        let types = entry
            .get("tile_types")
            .and_then(Json::as_array)
            .expect("tile_types");
        let deltas = entry
            .get("grid_deltas")
            .and_then(Json::as_array)
            .expect("grid_deltas");
        let here = types[0].as_str().expect("a type name").to_owned();
        let there = types[1].as_str().expect("a type name").to_owned();
        let dx = i32::try_from(deltas[0].as_i64().expect("a delta")).expect("a small delta");
        let dy = i32::try_from(deltas[1].as_i64().expect("a delta")).expect("a small delta");
        for pair in entry
            .get("wire_pairs")
            .and_then(Json::as_array)
            .expect("wire_pairs")
        {
            let pair = pair.as_array().expect("a pair");
            let mine = pair[0].as_str().expect("a wire name").to_owned();
            let theirs = pair[1].as_str().expect("a wire name").to_owned();
            out.insert((
                here.clone(),
                there.clone(),
                dx,
                dy,
                mine.clone(),
                theirs.clone(),
            ));
            out.insert((there.clone(), here.clone(), -dx, -dy, theirs, mine));
        }
    }
    out
}

/// **Every cross-tile pip of a loaded region is a join `tileconn.json`
/// states, for the tile types at both of its ends.**
///
/// The region is a 25-by-41 corner of the grid — the left-hand IO
/// column, the clock-management column beside it and the first logic
/// columns. It holds `CMT_FIFO_R` beside both `CMT_TOP_R_LOWER_T` and
/// `CMT_TOP_R_LOWER_B`, which is the shape that was wrong, and four
/// hundred thousand cross-tile pips of everything else.
///
/// Would catch: a join the loader invents, or one applied at the wrong
/// offset, or — the defect this exists for — one applied to a tile of
/// the wrong type. Would not catch a join the file states and the loader
/// never makes.
#[test]
fn no_join_of_the_loaded_fabric_is_one_tileconn_does_not_describe() {
    let Some(root) = chipdb() else { return };
    let stated = stated_joins(&root);
    assert!(stated.len() > 100_000, "{} stated join(s)", stated.len());

    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let mut options = XrayOptions::new();
    options.region = Some(GridRegion::new(0, 120, 24, 160));
    let fabric = db.load(&DiskFiles, &options).unwrap();
    let graph = fabric.arch.build_graph();

    // Grid position to tile type, from `tilegrid.json`, so that this
    // test names types the same way `tileconn.json` does.
    let mut types = std::collections::BTreeMap::new();
    for tile in db.tiles(&DiskFiles).unwrap() {
        types.insert((tile.grid_x, tile.grid_y), tile.tile_type);
    }
    // The region really holds the tile types the ambiguity is about,
    // or this test is checking the wrong corner of the die.
    for want in [
        "CMT_FIFO_R",
        "CMT_TOP_R_LOWER_T",
        "CMT_TOP_R_LOWER_B",
        "HCLK_CLB",
        "LIOI3",
    ] {
        assert!(
            types
                .iter()
                .any(|((x, y), t)| options.region.unwrap().contains(*x, *y) && t == want),
            "the region has no {want}"
        );
    }

    let mut checked = 0usize;
    let mut wrong = 0usize;
    let mut bad: Vec<String> = Vec::new();
    for pip in &graph.pips {
        let (from, to) = (graph.wire(pip.from), graph.wire(pip.to));
        if from.tile == to.tile || from.global || to.global {
            continue;
        }
        checked += 1;
        let dx = i64::from(to.tile.0) - i64::from(from.tile.0);
        let dy = i64::from(to.tile.1) - i64::from(from.tile.1);
        let key = (
            types[&from.tile].clone(),
            types[&to.tile].clone(),
            i32::try_from(dx).unwrap(),
            i32::try_from(dy).unwrap(),
            from.name.clone(),
            to.name.clone(),
        );
        if stated.contains(&key) {
            continue;
        }
        wrong += 1;
        if bad.len() < 20 {
            bad.push(format!(
                "{}/{} ({}) -> {}/{} ({}) at {dx},{dy}",
                from.tile.0, from.name, key.0, to.tile.0, to.name, key.1
            ));
        }
    }
    eprintln!("{checked} cross-tile pip(s) checked against tileconn.json");
    assert!(checked > 400_000, "{checked} is not a loaded fabric");
    assert!(
        wrong == 0,
        "{wrong} cross-tile pip(s) `tileconn.json` does not state, first {}:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// **The one join that was wrong, named.**
///
/// `CMT_FIFO_R_X7Y20` sits at grid `(7, 136)` and the tile at
/// `(8, 138)` is `CMT_TOP_R_LOWER_T`. `tileconn.json` pairs that type's
/// `CMT_TOP_EE4A0_0` with `CMT_FIFO_EE4A0_3`, and pairs
/// `CMT_FIFO_EE4A0_0` with `CMT_TOP_EE4A0_0` of `CMT_TOP_R_UPPER_B`
/// instead — a tile that is not there. So exactly one of the two joins
/// may exist here.
///
/// Would catch the specific defect even if the sweep above were
/// weakened. Would not catch anything about the other sixty-three
/// ambiguous (type, delta) pairs; the sweep is what covers those.
#[test]
fn the_cmt_column_joins_the_row_it_belongs_to_and_not_its_neighbour() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let mut options = XrayOptions::new();
    options.region = Some(GridRegion::new(0, 120, 24, 160));
    let fabric = db.load(&DiskFiles, &options).unwrap();
    let graph = fabric.arch.build_graph();

    let node = |x: u32, y: u32, name: &str| {
        graph
            .nodes
            .iter()
            .position(|w| w.tile == (x, y) && w.name == name)
            .map(|i| u32::try_from(i).unwrap())
    };
    let fifo0 = node(7, 136, "CMT_FIFO_EE4A0_0").expect("CMT_FIFO_EE4A0_0 of (7, 136)");
    let fifo3 = node(7, 136, "CMT_FIFO_EE4A0_3").expect("CMT_FIFO_EE4A0_3 of (7, 136)");
    let top0 = node(8, 138, "CMT_TOP_EE4A0_0").expect("CMT_TOP_EE4A0_0 of (8, 138)");

    let joined = |a, b| {
        graph
            .pips
            .iter()
            .any(|p| (p.from, p.to) == (a, b) || (p.from, p.to) == (b, a))
    };
    assert!(
        joined(fifo3, top0),
        "the join `tileconn.json` states for CMT_TOP_R_LOWER_T is missing"
    );
    assert!(
        !joined(fifo0, top0),
        "lane 0 is joined to the lane-3 wire: the partner tile type is \
         being ignored and two rows of the interconnect are one node"
    );
}
