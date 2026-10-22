//! The 7-series global clock buffers a design instantiates by name — `BUFG`,
//! `BUFGCE`, `BUFGMUX` and the `IBUFG` in front of them — built by the
//! command line for NewAE's ChipWhisperer-Husky part and decoded back.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test
//! does: `reticle fetch prjxray-db` puts the pinned copy where this looks.
//!
//! # The oracle
//!
//! NewAE ships the Husky's bitstream, built by Vivado 2022.1 from source
//! that instantiates three `BUFG`s, two `BUFGCE`s and ten `BUFGMUX`es,
//! eight of them `CLK_SEL_TYPE("ASYNC")`. Decoded through this database,
//! its thirteen `BUFGCTRL`s hold three configurations and only three
//! (`src/fpga/xray/bufg.rs` lists them with their counts). This test
//! asks that the same primitives, built here, come out as exactly those
//! configurations — not as something that merely decodes.
//!
//! # What it would and would not catch
//!
//! It would catch a control inverter written the wrong way round, a
//! `BUFGMUX`'s select not reaching both enables, `CLK_SEL_TYPE` lost on the
//! way from the source (it was: a black box's parameters were dropped by
//! the elaborator until this test), a second `BUFG` put behind a clock the
//! design already buffers (it was: two of them), and a select or enable
//! with no path to the buffer (it had none: the region the command line
//! loads did not reach the interconnect beside the buffer column).
//!
//! It would not catch a wrong reading shared by this flow and the test —
//! which is why the configurations are Vivado's, from its image, and not
//! derived — nor anything about the part. Nothing here has run on a Husky.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use reticle::fpga::xc7;
use reticle::fpga::xray::{XrayDatabase, XrayOptions};
use reticle::ir::memfile::FileProvider;

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing. The
/// same search as `tests/fpga_xray.rs`.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a35tftg256-1/package_pins.csv";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but has no `{probe}`");
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
                "skipped: needs a Project X-Ray database with the FTG256 package; run \
                 `reticle fetch prjxray-db` or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

/// Each in-use `BUFGCTRL`'s features, sorted and joined, keyed by
/// `tile:site`.
fn buffer_configurations(features: &[(String, String)]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (tile, feature) in features {
        let Some(rest) = feature.strip_prefix("BUFGCTRL.") else {
            continue;
        };
        let Some((site, name)) = rest.split_once('.') else {
            continue;
        };
        out.entry(format!("{tile}:{site}"))
            .or_default()
            .push(name.to_owned());
    }
    for names in out.values_mut() {
        names.sort();
    }
    out
}

/// `clock_buffers.v` built by the command line and decoded, once for the
/// tests here: its features, or `None` when there is nothing to build it
/// with.
fn built() -> Option<&'static [(String, String)]> {
    static BUILT: OnceLock<Option<Vec<(String, String)>>> = OnceLock::new();
    BUILT.get_or_init(build).as_deref()
}

fn build() -> Option<Vec<(String, String)>> {
    let root = chipdb()?;
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/husky");
    if !dir.join("clock_buffers.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("husky-clock-buffers");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join("clock_buffers.bit");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a35t-ftg256",
            "--chipdb",
            &root,
            "--constraints",
        ])
        .arg(dir.join("clock_buffers.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join("clock_buffers.v"))
        .output()
        .expect("the reticle binary runs");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    assert!(
        !stderr.contains("NOT FULLY ROUTED"),
        "every signal must route, the buffers' controls included:\n{stderr}"
    );

    let db = XrayDatabase::open(&DiskFiles, &root, "xc7a35t-ftg256", &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let read = xc7::read_bit(&std::fs::read(&bit).unwrap()).unwrap();
    let frames = xc7::FrameData::from_stream(part.layout.clone(), read.frames).unwrap();
    let decoded = db.decode(&DiskFiles, &frames).unwrap();
    assert_eq!(decoded.unexplained, 0, "{}", decoded.to_text());
    Some(decoded.features)
}

#[test]
fn the_husky_clock_buffers_are_the_ones_vivado_wrote() {
    let Some(features) = built() else { return };
    let configurations = buffer_configurations(features);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for names in configurations.values() {
        *counts.entry(names.join(" ")).or_default() += 1;
    }
    // Vivado's three, from NewAE's image, and nothing else.
    let bufg = "IN_USE IS_IGNORE1_INVERTED ZINV_CE0 ZINV_S0";
    let mux_async = "IN_USE IS_IGNORE0_INVERTED IS_IGNORE1_INVERTED ZINV_CE1 ZINV_S0 ZINV_S1";
    let mux_sync = "IN_USE ZINV_CE1 ZINV_S0 ZINV_S1";
    for configuration in counts.keys() {
        assert!(
            [bufg, mux_async, mux_sync].contains(&configuration.as_str()),
            "`{configuration}` is not one of Vivado's:\n{configurations:#?}"
        );
    }
    // One `ASYNC` `BUFGMUX` and one `SYNC`, exactly.
    assert_eq!(counts.get(mux_async), Some(&1), "{configurations:#?}");
    assert_eq!(counts.get(mux_sync), Some(&1), "{configurations:#?}");
    // The `BUFG` and the `BUFGCE` at least. A buffer's output reaches
    // another buffer's input directly only from the sites beside it
    // (`CLK_BUFG_R_FBG_OUT<n±1>`), and a clock feeding three of them cannot
    // be beside all three; the router then passes it through a free
    // `BUFGCTRL`, which it configures as a `BUFG` — a cascade, legal and
    // the same function, costing a site. So more than two is allowed here,
    // and the configuration of every one is still Vivado's.
    assert!(
        counts.get(bufg).copied().unwrap_or(0) >= 2,
        "{configurations:#?}"
    );
}

/// **The 96 MHz clock reaches its buffer the way Vivado's does**: from
/// `D4`, a clock-capable pin, through the clock management tile beside
/// its bank and along the clock row to the buffer column, on wires that
/// go nowhere else, and not over general routing.
///
/// The features are the ones NewAE's own image sets for the same pin, read
/// off it through this database (`HCLK_CMT_L_X106Y78`, `CLK_HROW_TOP_R_X60Y78`):
/// the pad's `CCIO0` marked active and used, taken onto the row's
/// `CK_IN0`, and the row's `CK_IN_R0` marked active where it arrives. Which
/// buffer cascade (`CASCO<n>`) it then takes follows from which buffer the
/// placer chose, and that is not asked.
///
/// It would catch the clock going from its pad over general routing, which
/// it did, on this design and on the ChipWhisperer-Husky Plus: the router's
/// distance estimate charged the dedicated road's first step a full wire a
/// tile while general routing got three tiles closer, so the road was never
/// tried (`XrayFabric::node_costs`). On the Plus that was 4.4 to 7.7 ns of
/// clock insertion that moved with every placement, on a bus synchronous
/// to the microcontroller driving it, and builds failed to read back their
/// registers. It would also catch the road's enables left unset, which
/// leaves it undriven. It would not catch a road that is longer than it
/// need be, nor say anything about timing. Nor would it catch the second
/// half of what the Plus needed, the pad's buffers routed before its data
/// readers (`RouteOptions::first`): given a flip-flop reading the pad as
/// data, this design passed with that order taken away, its data branch
/// going nowhere near the buffer column. The router's own
/// `a_pin_named_first_is_reached_before_the_tree_grows` catches that.
///
/// **And the same on the left half**, from the board PLL's `D13`, whose
/// clock tile names the pip onto the row `HCLK_CMT_MUX_CLK_<n>.HCLK_CMT_CCIO0`
/// rather than `CK_IN` (`is_clock_input_wire`). NewAE's image takes
/// `MUX_CLK_1` onto the row's `CK_IN_L1`; which `n` is the router's to
/// choose, so the test follows the one it took to the row. It would catch
/// this pad routed over general routing, as every left-side clock pad on
/// the ChipWhisperer-Husky Plus was — **but it did not catch why**: with
/// `HCLK_CMT_MUX_CLK_<n>` left off the road this still passed, the router
/// finding the road unaided on a design this small. The Plus's failure was
/// in placement, and `a_clock_pad_on_either_side_reaches_its_buffers_by_road`
/// is what catches that.
#[test]
fn the_husky_clock_reaches_its_buffer_on_the_clock_pins_own_road() {
    let Some(features) = built() else { return };
    let at = |tile: &str| -> Vec<&str> {
        features
            .iter()
            .filter(|(t, _)| t == tile)
            .map(|(_, f)| f.as_str())
            .collect()
    };
    let cmt = at("HCLK_CMT_L_X106Y78");
    for feature in [
        "HCLK_CMT_CCIO0_ACTIVE",
        "HCLK_CMT_CCIO0_USED",
        "HCLK_CMT_CK_IN0.HCLK_CMT_CCIO0",
    ] {
        assert!(cmt.contains(&feature), "{feature} missing: {cmt:?}");
    }
    let row = at("CLK_HROW_TOP_R_X60Y78");
    assert!(row.contains(&"CLK_HROW_CK_IN_R0_ACTIVE"), "{row:?}");
    assert!(
        row.iter()
            .any(|f| f.starts_with("CLK_HROW_TOP_R_CK_BUFG_CASCO")
                && f.ends_with(".CLK_HROW_CK_IN_R0")),
        "the row's input goes on to no buffer: {row:?}"
    );

    let cmt = at("HCLK_CMT_X8Y78");
    for feature in ["HCLK_CMT_CCIO0_ACTIVE", "HCLK_CMT_CCIO0_USED"] {
        assert!(cmt.contains(&feature), "{feature} missing: {cmt:?}");
    }
    let lane = cmt
        .iter()
        .find_map(|f| {
            f.strip_prefix("HCLK_CMT_MUX_CLK_")?
                .strip_suffix(".HCLK_CMT_CCIO0")
        })
        .unwrap_or_else(|| panic!("the PLL's pad is taken onto no row input: {cmt:?}"));
    let input = format!("CLK_HROW_CK_IN_L{lane}");
    assert!(row.contains(&format!("{input}_ACTIVE").as_str()), "{row:?}");
    assert!(
        row.iter()
            .any(|f| f.starts_with("CLK_HROW_TOP_R_CK_BUFG_CASCO")
                && f.ends_with(&format!(".{input}"))),
        "{input} goes on to no buffer: {row:?}"
    );
}

/// **A clock-capable pad on either half of the die reaches its buffers on
/// the road the placer walks**: from the pad's input, over the wires
/// `XrayFabric::clock_roads` names and pips that set no bit, to the
/// `BUFGCTRL` inputs of its half of the column — sixteen, from `D4` on the
/// right and from `D13` on the left. It is the walk `confine_to_roads`
/// makes to keep a pad's buffer where its road arrives.
///
/// It would catch a link of either road left out of the set, as the left
/// clock tile's `HCLK_CMT_MUX_CLK_<n>` was: `D13` then reached none, the
/// placer kept the board PLL's buffers nowhere in particular, and on the
/// ChipWhisperer-Husky Plus its clock went to them over general routing.
/// The build above did not catch that — on a design this small the router
/// found the road unaided — which is why this walks the graph. It would
/// not catch a road that reaches buffers it should not, nor anything the
/// router then does.
#[test]
fn a_clock_pad_on_either_side_reaches_its_buffers_by_road() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, "xc7a35t-ftg256", &XrayOptions::new()).unwrap();
    let tiles = db.tiles(&DiskFiles).unwrap();
    let fabric = db.load(&DiskFiles, &XrayOptions::new()).unwrap();
    let graph = fabric.arch.build_graph();
    let roads = fabric.clock_roads(&graph);
    let targets: BTreeMap<u32, usize> = graph
        .sites
        .iter()
        .enumerate()
        .filter(|(_, site)| site.kind == "gb")
        .flat_map(|(index, site)| {
            ["i", "i1"]
                .into_iter()
                .filter_map(move |role| site.pin(role).map(|node| (node, index)))
        })
        .collect();
    // `D4` is `IOB_X1Y76` in `RIOB33_X43Y75` and `D13` is `IOB_X0Y76` in
    // `LIOB33_X0Y75` (`package_pins.csv`); each is the higher of its
    // tile's two sites, which is `IOB_Y0`.
    for (ball, tile) in [("D4", "RIOB33_X43Y75"), ("D13", "LIOB33_X0Y75")] {
        let tile = tiles.iter().find(|t| t.name == tile).unwrap();
        let pad = graph
            .sites
            .iter()
            .find(|site| site.tile == (tile.grid_x, tile.grid_y) && site.name.ends_with("/IOB_Y0"))
            .unwrap_or_else(|| panic!("{ball} has no site"));
        let mut seen: std::collections::HashSet<u32> = pad.pin_nodes("din").collect();
        let mut work: Vec<u32> = seen.iter().copied().collect();
        let mut reached = std::collections::BTreeSet::new();
        while let Some(node) = work.pop() {
            for pip in graph.outgoing(node) {
                let to = graph.pip(*pip).to;
                if !seen.insert(to) {
                    continue;
                }
                if let Some(site) = targets.get(&to) {
                    reached.insert(*site);
                } else if roads[to as usize] || graph.pip_bits(*pip).is_empty() {
                    work.push(to);
                }
            }
        }
        assert_eq!(
            reached.len(),
            16,
            "{ball} reaches {} buffer(s) by road",
            reached.len()
        );
    }
}

/// **A buffer that feeds another through the cascade has the cascade
/// switched on.** For every `CLK_BUFG_BUFGCTRL<m>_I<k>.CLK_BUFG_R_FBG_OUT<n>`
/// the build sets, the same tile sets `CLK_BUFG_R_FBG_OUT<n>_ACTIVE`, the
/// bit that drives buffer `n`'s output onto it (`xray::bufg`, "The
/// cascade"), and the design — a `BUFG` feeding a `BUFGMUX`, a second
/// `BUFGMUX` and a `BUFGCE` — makes at least one such join.
///
/// It would catch the enable left off, as it was in every bitstream this
/// flow wrote until 2026-10-22: the cascade decoded as Vivado's, every bit
/// was explained, and on the ChipWhisperer-Husky Plus the receiving
/// buffer carried nothing — the glitch module's clock through its two
/// multiplexers, the logic analyser's and the trace front end's. It would
/// also catch the cascade going unused, a join routed some other way. It
/// would not catch the enable put on the wrong bit: that the bit is
/// `26_<16n+14>` was measured on the part, in both halves of the column,
/// and is quoted in the module documentation rather than checked here.
#[test]
fn a_buffer_cascade_has_its_enable() {
    let Some(features) = built() else { return };
    let mut cascades = 0;
    for (tile, feature) in features {
        let Some((_, source)) = feature.split_once(".CLK_BUFG_R_FBG_OUT") else {
            continue;
        };
        if !feature.starts_with("CLK_BUFG_BUFGCTRL") {
            continue;
        }
        cascades += 1;
        let enable = format!("CLK_BUFG_R_FBG_OUT{source}_ACTIVE");
        assert!(
            features.iter().any(|(t, f)| t == tile && *f == enable),
            "{tile}: {feature} with no {enable}"
        );
    }
    assert!(cascades > 0, "no buffer feeds another through the cascade");
}
