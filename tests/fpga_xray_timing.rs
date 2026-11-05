//! The 7-series node delays a timing-driven route weighs, against the
//! database they are read from.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test
//! does: `reticle fetch prjxray-db` puts the pinned copy where this looks.

#![cfg(feature = "fpga")]

use std::path::Path;

use reticle::fpga::xray::{DelayModel, GridRegion, UNTIMED_PIP_PS, XrayDatabase, XrayOptions};
use reticle::ir::memfile::FileProvider;

/// The part the region is cut from, as `tests/fpga_xray_joins.rs` has it.
const DEVICE: &str = "xc7a35t-cpg236";

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing: the
/// same search `tests/fpga_xray.rs` makes.
fn chipdb() -> Option<String> {
    let probe = "artix7/tile_type_INT_L.json";
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

/// **A node costs the delay of the pip into it, as the database times
/// it.** Over a corner of the 50T-sized grid `tests/fpga_xray_joins.rs`
/// loads: for each of four `INT_L` wires, every node of it costs the
/// largest delay `tile_type_INT_L.json` gives a pip into it, read here
/// independently of the loader (0.165 ns into `EE4BEG0`, also read by
/// hand off the file). A wire whose largest delay is the untimed
/// stand-in's 0.175 ns — `CLK_L0` is, by coincidence — could not tell a
/// delay read from one missed, so it is not counted.
///
/// **And a bit-less pip costs what the file says, which is nothing for
/// a join.** An `INT_L` node only bit-less pips enter costs the largest
/// delay the file gives a pip into its wire: 0.178 ns into `BYP_BOUNCE0`,
/// which an always-on buffer drives (read by hand too), and nothing for
/// the pieces of a wire the loader joins across tiles, which the file
/// does not list. Both kinds must be seen. Every bit-less pip once cost
/// nothing, and a timing-driven route on the Husky Plus took a USB data
/// lane up thirty-five tiles of bounces that were free but for the tile
/// charge. The lane read back as zero.
///
/// Would catch the delays not read (every node at the stand-in), the
/// wrong tile type's file read, the wrong corner taken, a fixed buffer
/// left free, or a join charged the stand-in; would not catch the
/// database's delays being wrong about the part, nor a buffer outside
/// `INT_L`.
#[test]
fn a_node_costs_the_delay_of_the_pip_into_it() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let mut options = XrayOptions::new();
    options.region = Some(GridRegion::new(0, 120, 24, 160));
    let fabric = db.load(&DiskFiles, &options).unwrap();
    let graph = fabric.arch.build_graph();
    let delays = db
        .node_delays(&DiskFiles, &fabric.arch, &graph, DelayModel::Legacy)
        .unwrap();
    assert_eq!(delays.len(), graph.nodes.len());

    let text = std::fs::read_to_string(format!("{root}/artix7/tile_type_INT_L.json")).unwrap();
    let json = reticle::json::Json::parse(&text).unwrap();
    let pips = json
        .get("pips")
        .and_then(reticle::json::Json::as_object)
        .unwrap();
    let largest_into = |wire: &str| -> u32 {
        let ns = pips
            .iter()
            .filter(|(_, p)| p.get("dst_wire").and_then(reticle::json::Json::as_str) == Some(wire))
            .filter_map(|(_, p)| {
                p.get("src_to_dst")?
                    .get("delay")?
                    .as_array()?
                    .get(3)?
                    .as_str()?
                    .parse::<f64>()
                    .ok()
            })
            .fold(0.0f64, f64::max);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ps = (ns * 1000.0).round() as u32;
        ps
    };
    assert_eq!(largest_into("EE4BEG0"), 165);

    let mut checked = 0;
    for name in ["EE4BEG0", "NN6BEG0", "SW2BEG1", "IMUX_L10"] {
        let want = largest_into(name);
        if want == 0 || want == UNTIMED_PIP_PS {
            continue;
        }
        for (index, wire) in graph.nodes.iter().enumerate() {
            let tile_type = fabric
                .arch
                .tile_index_at(wire.tile.0, wire.tile.1)
                .map(|t| fabric.arch.tile_types[t].name.as_str());
            if tile_type == Some("INT_L") && wire.name == name {
                assert_eq!(delays[index], want, "INT_L {:?} {name}", wire.tile);
                checked += 1;
            }
        }
    }
    assert!(checked > 40, "only {checked} nodes checked");
    assert_eq!(largest_into("BYP_BOUNCE0"), 178);

    let mut known: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let (mut buffers, mut joins) = (0, 0);
    for (index, wire) in graph.nodes.iter().enumerate() {
        let id = u32::try_from(index).unwrap();
        let into = graph.incoming(id);
        let tile_type = fabric
            .arch
            .tile_index_at(wire.tile.0, wire.tile.1)
            .map(|t| fabric.arch.tile_types[t].name.as_str());
        if into.is_empty()
            || tile_type != Some("INT_L")
            || !into.iter().all(|p| graph.pip_bits(*p).is_empty())
        {
            continue;
        }
        let want = *known
            .entry(wire.name.clone())
            .or_insert_with(|| largest_into(&wire.name));
        assert_eq!(
            delays[index], want,
            "INT_L {:?} {} is entered by bit-less pips only",
            wire.tile, wire.name
        );
        if want == 0 {
            joins += 1;
        } else {
            buffers += 1;
        }
    }
    assert!(
        buffers > 40 && joins > 40,
        "{buffers} buffered and {joins} joined nodes checked"
    );
}

/// **With the model fitted to Vivado, a wire costs its class's figure where a
/// pip in its own tile enters it, and nothing where a join does.** Over the
/// same corner: every node of an interconnect tile's `IMUX`, `SL1BEG` or
/// `NN6BEG` wires, on either side, costs what `xray::delay` says of `INT:IMUX`,
/// `INT:SL1BEG` and `INT:NN6BEG`; every node of a slice tile's lookup table
/// input `CLBLL_LL_A3` or `CLBLL_L_A6` costs its own pin's figure, which is not
/// the same for the two; and an interconnect node only joins enter costs
/// nothing.
///
/// Would catch the class looked up with the wrong tile type, a side's wires not
/// matched (`IMUX_L10` against `IMUX10`), a join charged the class of the wire
/// it continues, or the legacy delays returned for this model; would not catch
/// a figure in the table that is wrong, which `tests/fpga_xray_delay.rs` holds
/// against Vivado's.
#[test]
fn a_node_costs_its_class_under_the_vivado_model() {
    use reticle::fpga::xray::delay;
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let mut options = XrayOptions::new();
    options.region = Some(GridRegion::new(0, 120, 24, 160));
    let fabric = db.load(&DiskFiles, &options).unwrap();
    let graph = fabric.arch.build_graph();
    let delays = db
        .node_delays(&DiskFiles, &fabric.arch, &graph, DelayModel::Vivado)
        .unwrap();
    let class = |name: &str| delay::class_ps(name).unwrap();
    assert_ne!(class("CLBLL:LL_A3"), class("CLBLL:L_A6"));
    let mut seen = std::collections::BTreeMap::new();
    let mut joins = 0;
    for (index, wire) in graph.nodes.iter().enumerate() {
        let Some(t) = fabric.arch.tile_index_at(wire.tile.0, wire.tile.1) else {
            continue;
        };
        let tile_type = fabric.arch.tile_types[t].name.as_str();
        let id = u32::try_from(index).unwrap();
        let into = graph.incoming(id);
        let from_here = into
            .iter()
            .any(|p| graph.wire(graph.pip(*p).from).tile == wire.tile);
        if !into.is_empty() && !from_here && tile_type.starts_with("INT_") {
            assert_eq!(
                delays[index], 0,
                "{tile_type} {:?} {}",
                wire.tile, wire.name
            );
            joins += 1;
            continue;
        }
        if !from_here {
            continue;
        }
        let stem = wire.name.trim_end_matches(|c: char| c.is_ascii_digit());
        let want = match (tile_type, stem) {
            ("INT_L", "IMUX_L") | ("INT_R", "IMUX") => class("INT:IMUX"),
            ("INT_L" | "INT_R", "SL1BEG") => class("INT:SL1BEG"),
            ("INT_L" | "INT_R", "NN6BEG") => class("INT:NN6BEG"),
            ("CLBLL_L" | "CLBLL_R", _) if wire.name == "CLBLL_LL_A3" => class("CLBLL:LL_A3"),
            ("CLBLL_L" | "CLBLL_R", _) if wire.name == "CLBLL_L_A6" => class("CLBLL:L_A6"),
            _ => continue,
        };
        assert_eq!(
            delays[index], want,
            "{tile_type} {:?} {}",
            wire.tile, wire.name
        );
        *seen.entry(format!("{tile_type} {stem}")).or_insert(0) += 1;
    }
    assert!(seen.len() >= 8, "{seen:?}");
    assert!(seen.values().all(|n| *n > 5), "{seen:?}");
    assert!(joins > 100, "{joins} joined nodes");
}
