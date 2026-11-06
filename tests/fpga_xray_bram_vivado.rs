//! A block RAM, named and inferred, held to what Vivado wrote for the same
//! designs.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/bram/` has one small design per mode of a
//! `RAMB18E1` or `RAMB36E1` named with its parameters, and one per shape of
//! memory to infer, written here (`tools/vivado-parity/gen_bram.py`), with what
//! Vivado 2026.1 made of each in `vivado.txt`: the cells it left, the site pins
//! each pin of a named cell reached (`lutram_probe.tcl`), and each block RAM
//! tile as decoded from its bitstream through this repository's database.
//! `xsim.txt` there is what Vivado's simulator says each design does as Vivado
//! built it, on Vivado's own models (`gen_lutram_vectors.py`). This test
//! regenerates `vivado.txt` when `RETICLE_BRAM_BITS` names
//! `FIXTURE=bram tools/vivado-parity/lutram.sh`'s output and `UPDATE_EXPECT` is
//! set.
//!
//! A tile is one line: every feature of its sites, `RAMB18_Y0.`, `RAMB18_Y1.`,
//! `RAMB36.` and the tile's own FIFO offsets, with the contents counted and
//! hashed rather than listed. A tile's own routing (`BRAM_ADDRARDADDRL14`
//! from the cascade below, and `CASCOUT_*_ACTIVE`) is interconnect and is left
//! out: Vivado takes a block's address through its neighbour's cascade, which
//! this flow does not route (`bram::is_cascade_input`). A block on one half
//! of its tile is compared whichever half it is on.
//!
//! # What these would and would not catch
//!
//! [`each_block_ram_tile_holds_what_vivados_does`] would catch a width on
//! the wrong half or at the wrong width, `SDP` written as `TDP`, a register,
//! an inversion, a write mode, a priority or a collision mode not written, an
//! initial or reset value in the wrong order (nextpnr's, which was), a
//! `RAMB36E1`'s words split between its halves any other way, its `BRAM36_*`
//! bits missing, and a `ZINV` set where Vivado holds a reset low through it.
//! [`vivados_pins_are_the_fabrics`] would catch a cell pin on another site
//! pin, a write enable on the wrong lanes, and an output register's clock not
//! routed, which is the fault [`XrayFabric::spread_block_ram_pins`] mends: it
//! reads the fabric's declaration and the lanes, not a build. The routes
//! reaching those pins are what every set bit decoding to exactly the
//! router's arcs is for, and the simulation; and
//! [`a_blocks_clocks_and_held_pins_are_vivados`] counts, in each build, the
//! pips that bring a clock or a one to a block's clock and control pins,
//! which is what would see a register's clock pin given a one again.
//!
//! [`a_routed_netlist_answers_as_vivados_build_does`] simulates the netlist each
//! bitstream was made from on Vivado's own models of the cells, under Vivado's
//! simulator, against `xsim.txt`, when `RETICLE_VIVADO` names an
//! installation. Several named designs read an `x` from the first cycle (a
//! narrow port's unused outputs, a collision of the two ports), and for those
//! it says only that the netlist reads `x` where Vivado's does.
//!
//! None of it has been near a part.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.
//!
//! [`XrayFabric::spread_block_ram_pins`]: reticle::fpga::xray::XrayFabric::spread_block_ram_pins

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use reticle::fpga::xc7;
use reticle::fpga::xray::{
    Decoded, XrayDatabase, XrayOptions, ramb36_input_roles, write_enable_lanes,
};
use reticle::ir::AttrValue;
use reticle::ir::memfile::FileProvider;

const DEVICE: &str = "xc7a100t-ftg256";

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures() -> PathBuf {
    manifest().join("testdata/fpga/vivado/bram")
}

fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a100tftg256-1/package_pins.csv";
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

/// A small, stable hash of the contents, the same on every platform:
/// FNV-1a over the sorted feature names.
fn fnv(names: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for name in names {
        for byte in name.bytes().chain(std::iter::once(b'\n')) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

/// One line per block RAM tile; see the module docs.
fn block_ram_tiles(features: &[(String, String)]) -> Vec<String> {
    let mut tiles: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for (tile, feature) in features {
        if !tile.starts_with("BRAM_")
            || feature.starts_with("BRAM_")
            || feature.starts_with("CASCOUT_")
        {
            continue;
        }
        tiles.entry(tile).or_default().insert(feature.clone());
    }
    let mut out = Vec::new();
    for here in tiles.into_values() {
        let halves: BTreeSet<&str> = here
            .iter()
            .filter(|f| f.starts_with("RAMB18_Y"))
            .filter_map(|f| f.split('.').next())
            .collect();
        let alone = halves.len() == 1 && !here.iter().any(|f| f.starts_with("RAMB36."));
        // A block on one half, whichever: without the two things that half
        // decides, which `the_half_a_block_is_on_decides_two_features`
        // holds apart.
        let here: BTreeSet<String> = match halves.iter().next() {
            Some(half) if alone => {
                let sdp = here.contains(&format!("{half}.SDP_READ_WIDTH_36"));
                here.iter()
                    .filter(|f| !f.starts_with("ZALMOST_"))
                    .filter(|f| !(sdp && **f == format!("{half}.READ_WIDTH_A_18")))
                    .map(|f| f.replacen(&format!("{half}."), "RAMB18_Y?.", 1))
                    .collect()
            }
            _ => here,
        };
        let contents: Vec<&str> = here
            .iter()
            .filter(|f| f.contains(".INIT_") || f.contains(".INITP_"))
            .map(String::as_str)
            .collect();
        let site: Vec<&str> = here
            .iter()
            .filter(|f| !f.contains(".INIT_") && !f.contains(".INITP_"))
            .map(String::as_str)
            .collect();
        out.push(format!(
            "{} contents {} {:016x}",
            site.join(" "),
            contents.len(),
            fnv(&contents)
        ));
    }
    out.sort();
    out
}

fn decode(root: &str, bit: &Path) -> Decoded {
    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let read = xc7::read_bit(&std::fs::read(bit).unwrap()).unwrap();
    let frames = xc7::FrameData::from_stream(part.layout.clone(), read.frames).unwrap();
    db.decode(&DiskFiles, &frames).unwrap()
}

/// `vivado.txt`, by design.
#[derive(Default)]
struct Answer {
    /// Cell counts after routing.
    routed: BTreeMap<String, u32>,
    /// Each block RAM cell by name and primitive.
    cells: Vec<(String, String)>,
    /// `pin net sitepin...` of the named cell `dut`.
    pins: Vec<(String, String, Vec<String>)>,
    tiles: Vec<String>,
    /// [`clocks_and_ones`] of its bitstream.
    clocks: String,
}

fn answers() -> Option<BTreeMap<String, Answer>> {
    let Ok(text) = std::fs::read_to_string(fixtures().join("vivado.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut out: BTreeMap<String, Answer> = BTreeMap::new();
    let mut at = String::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("design ") {
            at = name.to_owned();
            out.entry(at.clone()).or_default();
            continue;
        }
        let Some(answer) = out.get_mut(&at) else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("  routed ") {
            for pair in rest.split_whitespace() {
                if let Some((k, v)) = pair.split_once('=') {
                    answer.routed.insert(k.to_owned(), v.parse().unwrap_or(0));
                }
            }
        } else if let Some(rest) = line.strip_prefix("  pin ") {
            let mut words = rest.split_whitespace();
            let (Some(pin), Some(net)) = (words.next(), words.next()) else {
                continue;
            };
            answer.pins.push((
                pin.to_owned(),
                net.to_owned(),
                words.map(str::to_owned).collect(),
            ));
        } else if let Some(rest) = line.strip_prefix("  cell ") {
            let mut words = rest.split_whitespace();
            if let (Some(name), Some(primitive)) = (words.next(), words.next()) {
                answer.cells.push((name.to_owned(), primitive.to_owned()));
            }
        } else if let Some(tile) = line.strip_prefix("  tile ") {
            answer.tiles.push(tile.to_owned());
        } else if let Some(clocks) = line.strip_prefix("  clocks ") {
            clocks.clone_into(&mut answer.clocks);
        }
    }
    Some(out)
}

/// The designs, in order.
fn designs() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_suffix(".v").map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// The designs this flow does not build as Vivado does, and why. Each is
/// held to what it does instead by [`what_is_not_built_says_so`].
///
/// | Design | Vivado | Here |
/// |---|---|---|
/// | `b36_cascade` | two `RAMB36E1` as one 64 kbit block (`RAM_EXTENSION_A`) | refused: a cascade is not built |
/// | `ib_1024x18_tdp` | one block, each port reading and writing | refused: the mapper gives a read and a write a port each, and has four for two |
/// | `ib_1024x36_reg2` | the read's second register in the block, `DOB_REG` | the register in a slice |
/// | `ib_512x36` | one `RAMB18E1` in `SDP`, 36 bits wide | one `RAMB36E1`, 36 bits wide in `TDP` |
/// | `ib_512x72` | one `RAMB36E1` in `SDP`, 72 bits wide | two `RAMB36E1`s, 36 wide each |
/// | `ib_1024x32_bytes` | none: Vivado's synthesis stopped on it | refused: its byte writes, `mem[a][7:0] <= d`, assign a select of an array element, which the frontend has no target for |
///
/// `ib_1024x32_bytes` used to be "built", and what was built was wrong: the
/// frontend read `mem[a]` as `mem` and dropped every byte write without a
/// word, so the block held a memory nothing wrote. Every check here passed on
/// it, since nothing here compares what it computes with Vivado's (Vivado
/// built nothing) and the mapping was proved against a source already
/// without its writes. It is refused now, by name, until the frontend has a
/// target for part of an element.
const NOT_AS_VIVADO: &[&str] = &[
    "b36_cascade",
    "ib_1024x18_tdp",
    "ib_1024x36_reg2",
    "ib_512x36",
    "ib_512x72",
    "ib_1024x32_bytes",
];

/// The three this flow refuses, and a word of each refusal.
const REFUSED: &[(&str, &str)] = &[
    ("b36_cascade", "a cascade"),
    ("ib_1024x18_tdp", "it did not become block RAM or logic"),
    ("ib_1024x32_bytes", "a select of an array element"),
];

/// One design through the flow.
struct Built {
    decoded: Option<Decoded>,
    log: String,
    routed: PathBuf,
}

fn built() -> Option<&'static BTreeMap<String, Built>> {
    static BUILT: OnceLock<Option<BTreeMap<String, Built>>> = OnceLock::new();
    BUILT.get_or_init(build_all).as_ref()
}

fn build_all() -> Option<BTreeMap<String, Built>> {
    let root = chipdb()?;
    answers()?;
    let names = designs();
    let results: Vec<(String, Built)> = std::thread::scope(|scope| {
        let root = &root;
        let handles: Vec<_> = names
            .iter()
            .map(|name| scope.spawn(move || (name.clone(), build_one(root, name))))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    Some(results.into_iter().collect())
}

fn build_one(root: &str, name: &str) -> Built {
    let dir = fixtures();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("bram")
        .join(name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join(format!("{name}.bit"));
    let routed = out.join(format!("{name}.routed.v"));
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--chipdb", root, "--top", name])
        .arg("--constraints")
        .arg(dir.join(format!("{name}.rcf")))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg("--routed-netlist")
        .arg(&routed)
        .args(["--verify", "--report"])
        .arg(dir.join(format!("{name}.v")))
        .output()
        .expect("the reticle binary runs");
    let log = String::from_utf8_lossy(&run.stderr).into_owned();
    let refused = REFUSED.iter().any(|(n, _)| *n == name);
    assert_eq!(run.status.success(), !refused, "{name}: {log}");
    assert!(!log.contains("NOT FULLY ROUTED"), "{name}: {log}");
    Built {
        decoded: run.status.success().then(|| decode(root, &bit)),
        log,
        routed,
    }
}

/// When `RETICLE_BRAM_BITS` and `UPDATE_EXPECT` are set, `vivado.txt` is made
/// again from `lutram.sh`'s output: never otherwise, so that nothing here runs
/// Vivado or needs to have.
fn regenerate(root: &str) {
    let (Some(bits), true) = (
        std::env::var_os("RETICLE_BRAM_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
    ) else {
        return;
    };
    let bits = PathBuf::from(bits);
    let mut out = String::from(
        "# What Vivado 2026.1 made of each design here, on xc7a100tftg256-1, from\n\
         # `FIXTURE=bram tools/vivado-parity/lutram.sh`. `routed` is its cell count\n\
         # after routing, `cell` each block RAM it left, `pin` each pin of the named\n\
         # cell `dut`, its net and the site pins that net reaches at the cell's site\n\
         # (lutram_probe.tcl), and `tile` each block RAM tile of its bitstream\n\
         # decoded through this repository's database:\n\
         # tests/fpga_xray_bram_vivado.rs says how to read one. A design with no\n\
         # lines but its name is one Vivado did not finish.\n",
    );
    for name in designs() {
        out.push_str(&format!("design {name}\n"));
        let dir = bits.join(&name);
        let Ok(report) = std::fs::read_to_string(dir.join(format!("{name}.cells"))) else {
            continue;
        };
        let mut nets: BTreeMap<String, String> = BTreeMap::new();
        let mut cell = String::new();
        for line in report.lines() {
            if let Some(rest) = line.strip_prefix("cell ") {
                cell = rest.split_whitespace().next().unwrap_or("").to_owned();
            } else if let Some(rest) = line.strip_prefix("  pin ")
                && cell == "dut"
            {
                let words: Vec<&str> = rest.split_whitespace().collect();
                if let [pin, "net", net, ..] = words.as_slice() {
                    nets.insert((*pin).to_owned(), (*net).to_owned());
                }
            }
        }
        for line in report.lines() {
            if let Some(rest) = line.strip_prefix("routed ") {
                let words: Vec<&str> = rest.split_whitespace().collect();
                let pairs: Vec<String> = words
                    .chunks(2)
                    .filter(|p| p.len() == 2 && !matches!(p[0], "GND" | "VCC"))
                    .map(|p| format!("{}={}", p[0], p[1]))
                    .collect();
                out.push_str(&format!("  routed {}\n", pairs.join(" ")));
            } else if let Some(rest) = line.strip_prefix("cell ") {
                let words: Vec<&str> = rest.split_whitespace().collect();
                out.push_str(&format!("  cell {} {}\n", words[0], words[1]));
            } else if let Some(rest) = line.strip_prefix("sitepin dut ") {
                let mut words = rest.split_whitespace();
                let Some(pin) = words.next() else { continue };
                let net = nets.get(pin).map_or("-", String::as_str);
                let sites: Vec<&str> = words.collect();
                out.push_str(&format!("  pin {pin} {net} {}\n", sites.join(" ")));
            }
        }
        let bit = dir.join(format!("{name}.bit"));
        if bit.is_file() {
            let decoded = decode(root, &bit);
            for tile in block_ram_tiles(&decoded.features) {
                out.push_str(&format!("  tile {tile}\n"));
            }
            out.push_str(&format!(
                "  clocks {}\n",
                clocks_and_ones(&decoded.features)
            ));
        }
    }
    std::fs::write(fixtures().join("vivado.txt"), out).unwrap();
}

/// **The block RAM tiles are Vivado's**, site feature for site feature and
/// contents bit for contents bit, for every design but [`NOT_AS_VIVADO`]'s.
/// See the module docs for what a line says and what this would catch.
#[test]
fn each_block_ram_tile_holds_what_vivados_does() {
    let Some(root) = chipdb() else { return };
    regenerate(&root);
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    let mut compared = 0;
    for (name, answer) in &answers {
        if NOT_AS_VIVADO.contains(&name.as_str()) {
            continue;
        }
        let ours = block_ram_tiles(&built[name].decoded.as_ref().unwrap().features);
        compared += answer.tiles.len();
        if ours != answer.tiles {
            wrong.push(format!(
                "{name}:\n  ours   {ours:?}\n  Vivado {:?}",
                answer.tiles
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(compared >= 35, "only {compared} tiles compared");
}

/// **The half a `RAMB18E1` is on decides two features, as Vivado has them**:
/// the tile's FIFO offsets (`ZALMOST_*`) beside a block on the lower half
/// and not beside one on the upper alone, and port A's `READ_WIDTH_A_18`
/// beside `SDP_READ_WIDTH_36` on the upper half and not the lower. Vivado
/// placed every fixture on the lower half; on the upper, where Reticle put
/// some of the same cells, its `write_bitstream` of Reticle's placement
/// is the evidence (`tools/vivado-parity/run_placed.sh`, in
/// `docs/fpga-vivado-parity.md`). This holds Reticle's own bitstreams to
/// the rule, for every lone half here; it would catch either rule dropped
/// or applied to the wrong half, and is only as good as that reading.
#[test]
fn the_half_a_block_is_on_decides_two_features() {
    let Some(built) = built() else { return };
    let mut halves = BTreeSet::new();
    for (name, one) in built {
        let Some(decoded) = &one.decoded else {
            continue;
        };
        let mut tiles: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (tile, feature) in &decoded.features {
            if tile.starts_with("BRAM_") {
                tiles.entry(tile).or_default().insert(feature);
            }
        }
        for here in tiles.values() {
            let used: Vec<&str> = ["RAMB18_Y0", "RAMB18_Y1"]
                .into_iter()
                .filter(|h| here.contains(format!("{h}.IN_USE").as_str()))
                .collect();
            let [half] = used.as_slice() else { continue };
            if here.iter().any(|f| f.starts_with("RAMB36.")) {
                continue;
            }
            halves.insert(*half);
            let fifo = here.iter().any(|f| f.starts_with("ZALMOST_EMPTY_OFFSET["));
            assert_eq!(fifo, *half == "RAMB18_Y0", "{name}: {half}");
            if here.contains(format!("{half}.SDP_READ_WIDTH_36").as_str()) {
                let wide = here.contains(format!("{half}.READ_WIDTH_A_18").as_str());
                assert_eq!(wide, *half == "RAMB18_Y1", "{name}: {half}");
            }
        }
    }
    assert_eq!(halves.len(), 2, "both halves seen: {halves:?}");
}

/// What reaches a block RAM's clock and control pins, counted over the
/// interconnect tiles beside each block (the five rows of its tile): a clock
/// pin's pips from a global clock, its pips from a wire that reads one
/// (`CLK0.FAN_BOUNCE5`), and a control pin's from one (`CTRL0.BYP_BOUNCE4`), as
/// `gclk=N one=M ctrl_one=K`. A port's clock and its register's clock are two
/// clock pins, so a registered port is two from a global clock and an
/// unregistered one a global clock and a one.
fn clocks_and_ones(features: &[(String, String)]) -> String {
    let at = |tile: &str, kind: &str| -> Option<(u32, u32)> {
        let rest = tile
            .strip_prefix(&format!("{kind}_L_X"))
            .or_else(|| tile.strip_prefix(&format!("{kind}_R_X")))?;
        let (x, y) = rest.split_once('Y')?;
        Some((x.parse().ok()?, y.parse().ok()?))
    };
    let mut rows: BTreeSet<(u32, u32)> = BTreeSet::new();
    for (tile, feature) in features {
        if feature.ends_with(".IN_USE")
            && let Some((x, y)) = at(tile, "BRAM")
        {
            rows.extend((0..5).map(|dy| (x, y + dy)));
        }
    }
    let (mut gclk, mut one, mut ctrl_one) = (0, 0, 0);
    for (tile, feature) in features {
        if !at(tile, "INT").is_some_and(|here| rows.contains(&here)) {
            continue;
        }
        let feature = feature.replace("_L", "");
        let Some((to, from)) = feature.split_once('.') else {
            continue;
        };
        if to.starts_with("CLK") && from.starts_with("GCLK") {
            gclk += 1;
        } else if to.starts_with("CLK") && from.starts_with("FAN_BOUNCE") {
            one += 1;
        } else if to.starts_with("CTRL") && from.starts_with("BYP_BOUNCE") {
            ctrl_one += 1;
        }
    }
    format!("gclk={gclk} one={one} ctrl_one={ctrl_one}")
}

/// **A block's clocks are Vivado's, and so are the ones it holds its
/// controls with**, counted as [`clocks_and_ones`] counts them: for every
/// design Vivado's global clock pips and its ones on clock pins, and for the
/// inferred ones its ones on control pins too, which a named design's
/// signals also reach through those wires. Would catch an output register's
/// clock not routed (the fault [`XrayFabric::spread_block_ram_pins`]
/// mends, which every other test here passes), a port nothing clocks left
/// alone where Vivado gives it a one, and a reset grounded where Vivado
/// holds it through its inverter. It counts pips and does not follow them,
/// so a clock on the wrong pin of the right tiles gets past it; the pins
/// test is for that.
///
/// [`XrayFabric::spread_block_ram_pins`]: reticle::fpga::xray::XrayFabric::spread_block_ram_pins
#[test]
fn a_blocks_clocks_and_held_pins_are_vivados() {
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    let mut compared = 0;
    for (name, answer) in &answers {
        if NOT_AS_VIVADO.contains(&name.as_str()) {
            continue;
        }
        let ours = clocks_and_ones(&built[name].decoded.as_ref().unwrap().features);
        let inferred = name.starts_with("ib_");
        let keep = |line: &str| -> String {
            line.split(' ')
                .filter(|part| inferred || !part.starts_with("ctrl_one="))
                .collect::<Vec<_>>()
                .join(" ")
        };
        compared += 1;
        if keep(&ours) != keep(&answer.clocks) {
            wrong.push(format!("{name}: ours {ours}, Vivado {}", answer.clocks));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(compared >= 30, "only {compared} compared");
}

/// The block RAM cells of a log's or a report's count.
fn block_rams(counts: &BTreeMap<String, u32>) -> BTreeMap<String, u32> {
    counts
        .iter()
        .filter(|(cell, _)| cell.starts_with("RAMB"))
        .map(|(cell, n)| (cell.clone(), *n))
        .collect()
}

/// **Each design gets the block RAMs Vivado gave it**, by kind and count:
/// one `RAMB36E1` where Vivado has one and not two `RAMB18E1`s, two side by
/// side for 2048 words of 36 rather than one above the other. Would catch the
/// cost by capacity, the fewest-blocks rule or the stacking rule of
/// `choose_bram` changed, and a named cell made something else.
#[test]
fn each_design_gets_the_block_rams_vivado_gave_it() {
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    for (name, answer) in &answers {
        if REFUSED.iter().any(|(n, _)| n == name) || answer.routed.is_empty() {
            continue;
        }
        let mut ours: BTreeMap<String, u32> = BTreeMap::new();
        for line in built[name].log.lines() {
            if let Some((n, cell)) = line.trim().split_once(" x ")
                && let Ok(n) = n.parse::<u32>()
                && !cell.contains(' ')
            {
                ours.insert(cell.to_owned(), n);
            }
        }
        let (ours, theirs) = (block_rams(&ours), block_rams(&answer.routed));
        let expected = match name.as_str() {
            "ib_512x36" => BTreeMap::from([("RAMB36E1".to_owned(), 1)]),
            "ib_512x72" => BTreeMap::from([("RAMB36E1".to_owned(), 2)]),
            _ => theirs.clone(),
        };
        if ours != expected {
            wrong.push(format!("{name}: ours {ours:?}, Vivado {theirs:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **What is not built as Vivado builds it says so**: the two refused with
/// a reason, and the others' differences only the ones [`NOT_AS_VIVADO`]
/// names. Would catch a refusal turning into a build (a cascade built
/// without its neighbour's route) and a known difference growing.
#[test]
fn what_is_not_built_says_so() {
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    for (name, reason) in REFUSED {
        assert!(
            built[*name].log.contains(reason),
            "{name}: {}",
            built[*name].log
        );
    }
    // The second register is in a slice: every other feature is Vivado's.
    let ours = block_ram_tiles(&built["ib_1024x36_reg2"].decoded.as_ref().unwrap().features);
    let theirs = answers["ib_1024x36_reg2"].tiles.clone();
    let strip = |line: &str| -> String {
        line.split(' ')
            .filter(|f| !f.ends_with(".DOB_REG") && !f.ends_with(".ZINV_REGCLKB"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(
        ours.iter().map(|l| strip(l)).collect::<Vec<_>>(),
        theirs.iter().map(|l| strip(l)).collect::<Vec<_>>()
    );
}

/// The role of a cell pin, `ADDRARDADDR[3]` or `CLKBWRCLK`, in the device's
/// port map; the write enables and the register clocks are the caller's.
fn role_of(pin: &str) -> Option<String> {
    let (base, bit) = match pin.split_once('[') {
        Some((base, rest)) => (base, rest.trim_end_matches(']').parse::<u32>().ok()),
        None => (pin, None),
    };
    let role = match base {
        "CLKARDCLK" => "p0_clk",
        "CLKBWRCLK" => "p1_clk",
        "ENARDEN" => "p0_en",
        "ENBWREN" => "p1_en",
        "RSTRAMARSTRAM" => "p0_rst",
        "RSTRAMB" => "p1_rst",
        "RSTREGARSTREG" => "p0_rstreg",
        "RSTREGB" => "p1_rstreg",
        "REGCEAREGCE" => "p0_regce",
        "REGCEB" => "p1_regce",
        "ADDRARDADDR" => "p0_addr",
        "ADDRBWRADDR" => "p1_addr",
        "DIADI" => "p0_din",
        "DIBDI" => "p1_din",
        "DIPADIP" => "p0_dinp",
        "DIPBDIP" => "p1_dinp",
        "DOADO" => "p0_dout",
        "DOBDO" => "p1_dout",
        "DOPADOP" => "p0_doutp",
        "DOPBDOP" => "p1_doutp",
        _ => return None,
    };
    Some(match bit {
        Some(bit) => format!("{role}{bit}"),
        None => role.to_owned(),
    })
}

/// A design's integer parameters of its `dut`, read off the fixture.
fn params_of(name: &str) -> BTreeMap<String, AttrValue> {
    let text = std::fs::read_to_string(fixtures().join(format!("{name}.v"))).unwrap();
    let mut out = BTreeMap::new();
    for key in [
        "READ_WIDTH_A",
        "READ_WIDTH_B",
        "WRITE_WIDTH_A",
        "WRITE_WIDTH_B",
        "DOA_REG",
        "DOB_REG",
    ] {
        let at = format!(".{key}(");
        if let Some(start) = text.find(&at) {
            let rest = &text[start + at.len()..];
            let value = &rest[..rest.find(')').unwrap_or(0)];
            if let Ok(v) = value.trim().parse::<i64>() {
                out.insert(key.to_owned(), AttrValue::Int(v));
            }
        }
    }
    if text.contains(".RAM_MODE(\"SDP\")") {
        out.insert("RAM_MODE".to_owned(), AttrValue::String("SDP".to_owned()));
    }
    out
}

/// The wire each site pin of a block RAM site of `tile_type` is on, by the
/// database's own description of the site, for the sites of `site_type`.
fn site_pin_wires(
    root: &str,
    tile_type: &str,
    site_type: &str,
) -> Option<BTreeMap<String, String>> {
    let path = format!("{root}/artix7/tile_type_{tile_type}.json");
    let text = std::fs::read_to_string(&path).ok()?;
    let json = reticle::json::Json::parse(&text).ok()?;
    let mut out = BTreeMap::new();
    for site in json.get("sites")?.as_array()? {
        if site.get("type")?.as_str()? != site_type {
            continue;
        }
        for (pin, value) in site.get("site_pins")?.as_object()? {
            let wire = value
                .get("wire")
                .and_then(|w| w.as_str())
                .or(value.as_str())?;
            out.insert(pin.clone(), wire.to_owned());
        }
    }
    Some(out)
}

/// **Every pin of every named block RAM reaches the site pins Vivado's
/// reached**, net by net: for each net on the cell, the wires of the site
/// pins Vivado's report has it on against the wires of the fabric's pins for
/// the roles of the cell pins on it, the write enables by their lanes and a
/// registered port's clock on its register's pin too. See the module docs.
#[test]
fn vivados_pins_are_the_fabrics() {
    let Some(root) = chipdb() else { return };
    let Some(answers) = answers() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let region = db
        .region_for_pins(&DiskFiles, &["L4".to_owned()], 30)
        .unwrap()
        .unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(region))
        .unwrap();
    // Vivado's sites are on `BRAM_R` tiles and the region's may be `BRAM_L`:
    // the two name their site pins' wires alike.
    let tile = fabric
        .arch
        .tile_types
        .iter()
        .find(|t| t.name == "BRAM_R" || t.name == "BRAM_L")
        .expect("a block RAM tile in the region");
    let Some(y0) = site_pin_wires(&root, &tile.name, "FIFO18E1") else {
        eprintln!(
            "skipped: the database has no `tile_type_{}.json`",
            tile.name
        );
        return;
    };
    let Some(whole) = site_pin_wires(&root, &tile.name, "RAMBFIFO36E1") else {
        return;
    };
    let mut checked = 0;
    let mut wrong = Vec::new();
    for (name, answer) in &answers {
        let Some((_, primitive)) = answer.cells.iter().find(|(cell, _)| cell == "dut") else {
            continue;
        };
        if NOT_AS_VIVADO.contains(&name.as_str()) {
            continue;
        }
        let (bel, sites) = match primitive.as_str() {
            "RAMB18E1" => ("RAMB18_Y0", &y0),
            "RAMB36E1" => ("RAMB36", &whole),
            other => panic!("{name}: `{other}`"),
        };
        let bel = tile.bels.iter().find(|b| b.name == bel).unwrap();
        let wires_of = |role: &str| -> BTreeSet<String> {
            bel.pins
                .iter()
                .filter(|(r, _)| r == role)
                .map(|(_, w)| w.name.clone())
                .collect()
        };
        let params = params_of(name);
        let param = |key: &str| params.get(key).cloned();
        let registered =
            |port: usize| param(["DOA_REG", "DOB_REG"][port]).and_then(|v| v.as_int()) == Some(1);
        let mut theirs: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        let mut ours: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        for (pin, net, site_pins) in &answer.pins {
            if net.contains("const") || net.starts_with("GND") || net.starts_with("VCC") {
                continue;
            }
            theirs.entry(net).or_default().extend(
                site_pins
                    .iter()
                    .map(|s| sites.get(s).cloned().unwrap_or_else(|| format!("?{s}"))),
            );
            let mine = ours.entry(net).or_default();
            let we = pin
                .strip_prefix("WEA[")
                .map(|b| (0, b))
                .or_else(|| pin.strip_prefix("WEBWE[").map(|b| (1, b)));
            if let Some((port, bit)) = we {
                let bit: u32 = bit.trim_end_matches(']').parse().unwrap();
                for (lane, carries) in write_enable_lanes(primitive, port, &param)
                    .into_iter()
                    .enumerate()
                {
                    if carries == Some(bit) {
                        mine.extend(wires_of(&format!("p{port}_we{lane}")));
                    }
                }
                continue;
            }
            let Some(role) = role_of(pin) else {
                wrong.push(format!("{name}: no role for {pin}"));
                continue;
            };
            // A narrow port's data inputs, as `narrow_ramb36_inputs` wires
            // them.
            let reaches = if primitive == "RAMB36E1" {
                let port = usize::from(role.starts_with("p1_"));
                let width = param(["WRITE_WIDTH_A", "WRITE_WIDTH_B"][port])
                    .and_then(|v| v.as_int())
                    .unwrap_or(0);
                ramb36_input_roles(width, &role)
            } else {
                vec![role.clone()]
            };
            for to in &reaches {
                mine.extend(wires_of(to));
            }
            for port in 0..2 {
                if role == format!("p{port}_clk") && registered(port) {
                    mine.extend(wires_of(&format!("p{port}_regclk")));
                }
            }
        }
        for (net, wires) in theirs {
            checked += 1;
            if ours.get(net) != Some(&wires) {
                wrong.push(format!(
                    "{name} {net}: ours {:?}, Vivado {wires:?}",
                    ours.get(net)
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(checked >= 2000, "only {checked} nets checked");
}

/// **Every bit decodes, the arcs are the router's, and the mapping is proved**,
/// for every design this flow builds. Would catch a block RAM feature in
/// nothing the decoder knows, a tie's pip the decoder finds and the flow did
/// not record, a `RAMB36.*` feature taken for an arc, and an inferred block
/// built from a graph the proof says differs.
#[test]
fn every_bit_decodes_and_every_arc_is_the_routers() {
    let Some(built) = built() else { return };
    for (name, one) in built {
        let Some(decoded) = &one.decoded else {
            continue;
        };
        assert_eq!(decoded.unexplained, 0, "{name}: {}", decoded.to_text());
        assert!(
            decoded.ecc_faults.is_empty(),
            "{name}: {:?}",
            decoded.ecc_faults
        );
        assert!(
            one.log
                .contains("the interconnect arcs those bits select are exactly the ones"),
            "{name}: {}",
            one.log
        );
        assert!(
            one.log.contains("mapping: equivalent"),
            "{name}: {}",
            one.log
        );
    }
}

/// The Vivado installation `RETICLE_VIVADO` names, when it has a simulator.
fn vivado() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("RETICLE_VIVADO") else {
        eprintln!("skipped: RETICLE_VIVADO does not name a Vivado installation");
        return None;
    };
    let dir = PathBuf::from(dir);
    if !dir.join("settings64.sh").is_file() {
        eprintln!("skipped: RETICLE_VIVADO has no `settings64.sh`");
        return None;
    }
    Some(dir)
}

/// The stimulus of `tools/vivado-parity/gen_lutram_vectors.py`, connected by
/// position: Vivado's simulator does not connect a port the routed netlist
/// declares as `.clk(clk$pad)`, so the netlist's header is made a plain list.
fn testbench(name: &str) -> String {
    format!(
        "`timescale 1ns / 1ps\n\
         module tb;\n\
         \x20 reg clk = 0;\n\
         \x20 reg si = 0;\n\
         \x20 reg [15:0] lfsr = 16'hACE1;\n\
         \x20 wire so;\n\
         \x20 {name} dut (clk, si, so);\n\
         \x20 integer cycle;\n\
         \x20 initial begin\n\
         \x20   #200;\n\
         \x20   $write(\"R \");\n\
         \x20   for (cycle = 0; cycle < 2000; cycle = cycle + 1) begin\n\
         \x20     #5 clk = 1;\n\
         \x20     #1 $write(\"%b\", so);\n\
         \x20     #4 clk = 0;\n\
         \x20     lfsr = {{lfsr[14:0], lfsr[15] ^ lfsr[13] ^ lfsr[12] ^ lfsr[10]}};\n\
         \x20     si = lfsr[0];\n\
         \x20   end\n\
         \x20   $display(\"\");\n\
         \x20   $finish;\n\
         \x20 end\n\
         endmodule\n"
    )
}

/// The routed netlist with its header a plain list of the inner names.
fn plain_header(text: &str) -> Option<String> {
    let open = text.find('(')?;
    let close = open + text[open..].find(");")?;
    let inner: Vec<&str> = text[open + 1..close]
        .split("),")
        .filter_map(|port| {
            port.split_once('(')
                .map(|(_, inner)| inner.trim_end_matches(')'))
        })
        .collect();
    Some(format!(
        "{}({});{}",
        &text[..open],
        inner.join(", "),
        &text[close + 2..]
    ))
}

/// What Vivado's simulator printed after `R `, with a block RAM model's
/// collision reports, a heading and a sentence each, taken out.
fn answer_of(out: &str) -> Option<String> {
    let at = out
        .find("\nR ")
        .map(|i| i + 3)
        .or_else(|| out.strip_prefix("R ").map(|_| 2))?;
    let mut rest = String::new();
    let mut lines = out[at..].split_inclusive('\n');
    while let Some(line) = lines.next() {
        if let Some(at) = line.find("Memory Collision Error on ") {
            rest.push_str(&line[..at]);
            // The sentence after it, when it is one.
            let mut peek = lines.clone();
            if peek
                .next()
                .is_some_and(|l| l.starts_with(|c: char| c.is_ascii_uppercase()))
            {
                lines.next();
            }
            continue;
        }
        rest.push_str(line);
        break;
    }
    Some(rest.trim().to_lowercase())
}

/// **The netlist each bitstream was made from answers as Vivado's build
/// does**, on Vivado's models under Vivado's simulator, when `RETICLE_VIVADO`
/// names one. See the module docs.
#[test]
fn a_routed_netlist_answers_as_vivados_build_does() {
    let Some(vivado) = vivado() else { return };
    let Some(built) = built() else { return };
    let expected: BTreeMap<String, String> = std::fs::read_to_string(fixtures().join("xsim.txt"))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once(' '))
        .map(|(n, b)| (n.to_owned(), b.to_owned()))
        .collect();
    let glbl = vivado.join("data/verilog/src/glbl.v");
    let mut wrong = Vec::new();
    let mut seen = 0;
    for (name, one) in built {
        let (Some(want), Some(_)) = (expected.get(name), &one.decoded) else {
            continue;
        };
        let dir = one.routed.parent().unwrap().join("xsim");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let text = std::fs::read_to_string(&one.routed).unwrap();
        std::fs::write(dir.join("netlist.v"), plain_header(&text).unwrap()).unwrap();
        std::fs::write(dir.join("tb.v"), testbench(name)).unwrap();
        let script = format!(
            ". '{}/settings64.sh' >/dev/null 2>&1; cd '{}' && \
             xvlog netlist.v tb.v '{}' >x.log 2>&1 && \
             xelab -L unisims_ver -debug off tb glbl -s s >>x.log 2>&1 && \
             xsim s -R",
            vivado.display(),
            dir.display(),
            glbl.display()
        );
        let run = Command::new("sh")
            .args(["-c", &script])
            .output()
            .expect("a shell runs");
        let out = String::from_utf8_lossy(&run.stdout);
        seen += 1;
        match answer_of(&out) {
            Some(bits) if bits == *want => {}
            Some(bits) => {
                let at = bits
                    .chars()
                    .zip(want.chars())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                wrong.push(format!("{name}: differs from cycle {at}"));
            }
            None => wrong.push(format!("{name}: no answer")),
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(seen >= 37, "only {seen} simulated");
}

/// **`--memories legacy` keeps two `RAMB18E1`s** where the Vivado mapping
/// makes `qor/bram_fifo` one `RAMB36E1`, as Vivado does. Would catch the
/// option not reaching the mapper's choice of shape; that the legacy build is
/// the one from before byte for byte was measured, on what the document
/// says, and is not held here.
#[test]
fn legacy_block_rams_map_as_before() {
    let Some(root) = chipdb() else { return };
    let qor = manifest().join("testdata/fpga/vivado/qor");
    if !qor.join("bram_fifo.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    for (memories, cell, count) in [("legacy", "RAMB18E1", 2), ("vivado", "RAMB36E1", 1)] {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("bram-legacy")
            .join(memories);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args(["fpga", "--device", DEVICE, "--chipdb", &root, "--report"])
            .args(["--top", "bram_fifo", "--memories", memories])
            .arg("--constraints")
            .arg(qor.join("bram_fifo.rcf"))
            .arg("--output-dir")
            .arg(&out)
            .arg("--bitstream")
            .arg(out.join("bram_fifo.bit"))
            .arg(qor.join("bram_fifo.v"))
            .output()
            .expect("the reticle binary runs");
        let log = String::from_utf8_lossy(&run.stderr);
        assert!(run.status.success(), "{log}");
        assert!(
            log.contains(&format!("  {count} x {cell}\n")),
            "{memories}: {log}"
        );
    }
}
