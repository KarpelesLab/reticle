//! Inferred memories arranged as Vivado arranges them, held to what Vivado
//! 2026.1 built of the same designs.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/memarr/` has one small design per memory to infer,
//! written here (`tools/vivado-parity/gen_memarr.py`): depths from 32 words
//! to 65,536 by widths from 1 to 72, a write port and a read port, a single
//! port read first, written first or unchanged on a write, a true dual port,
//! two reads, the read registered twice or enabled or on no clock, initial
//! contents with and without a write, a memory written narrow and read wide,
//! each `ram_style`, and (`bw_`) memories written a byte lane at a time, of
//! 8 and 9 bits, simple, single and true dual port, read first, written
//! first and unchanged, with and without contents, in block and distributed
//! RAM; and memories deeper than a block (40,000 to 131,072 words), a single
//! port read on no clock (`sp_as_`), and single ports of up to 512 words as
//! wide as 72 bits. `vivado.txt` there keeps what Vivado made of each on
//! `xc7a100tftg256-1` (`FIXTURE=memarr tools/vivado-parity/lutram.sh`): the
//! cells left after routing, each block's parameters (`params`), and each
//! block RAM tile of its bitstream decoded through this repository's
//! database (`tile`, as `tests/fpga_xray_bram_vivado.rs` writes one);
//! `xsim.txt` what Vivado's simulator says each does as Vivado built it. This
//! test regenerates `vivado.txt` when `RETICLE_MEMARR_BITS` names
//! `lutram.sh`'s output and `UPDATE_EXPECT` is set.
//!
//! # What these would and would not catch
//!
//! [`each_memory_gets_the_cells_vivado_gave_it`] would catch any change to
//! the arrangement `vivado_columns` reads off Vivado (a block stacked rather
//! than side by side, a `RAMB18E1` not put beside `RAMB36E1`s for what they
//! leave, simple dual port not taken), a read written first left on no clock,
//! and a distributed RAM's shape changed.
//! [`each_block_ram_tile_holds_what_vivados_does`] holds every feature of
//! every block RAM tile, contents included, so it would catch a width, a
//! write mode, an output register, an unused port's widths, and a word's
//! bits on the wrong pins or in the wrong place of `INIT_` or `INITP_`; it
//! compares tiles as a set, so it does not see which tile holds which
//! column, which the simulation does. [`a_routed_netlist_answers_as_vivados_build_does`]
//! simulates each routed netlist on Vivado's own models under Vivado's
//! simulator, against `xsim.txt`, when `RETICLE_VIVADO` names an
//! installation: that is what sees a port pair's read-during-write, an
//! output register's latency, a word's address on the wrong pins, and
//! initial contents read back as Vivado's. Most designs fold the word read
//! to its parity, which two bits swapped leave as it was: a simple dual
//! port's halves written the wrong way round passed every test here until
//! the `bits_` designs, which fold it under a mask that changes every cycle,
//! were added for it.
//!
//! Every design must build at all, and `sdp_65536x4` is here for that:
//! its read multiplexer's table read one flip-flop on two inputs once its
//! copy was merged away, the router ended one of the two on the other's
//! input, and the build stopped (`` `$lut4` has two signals routed to one
//! input ``, `xray::lutswap`). It builds with either of the two fixes
//! alone, the table folded (`srl`) or the second input kept where it is
//! (`lutswap`), so it catches both undone and neither on its own; each has
//! its own unit test for that. Under `--memories legacy` six more failed
//! the same way, and [`legacy_memories_map_as_before`] routes one of them.
//!
//! A cascade's route is a fixed pip, which no tile feature shows: that the
//! lower block's `CASCADEOUT<x>` is the upper's `CASCADEIN<x>` is the
//! simulation's to see, and [`what_is_not_vivados_says_so`]'s, which reads it
//! off a routed netlist.
//! [`a_tight_clock_makes_a_small_memory_distributed_ram`] holds the choice by
//! the clock to Vivado's synthesis at the periods it was measured at.
//!
//! None of it has been near a part. Skips without the Project X-Ray
//! database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use reticle::fpga::xc7;
use reticle::fpga::xray::{Decoded, XrayDatabase, XrayOptions};
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
    manifest().join("testdata/fpga/vivado/memarr")
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

/// One line per block RAM tile, as `tests/fpga_xray_bram_vivado.rs` writes
/// one: every feature of its sites, a block on one half written for either
/// half, and the contents counted and hashed; but with `halves`, said of a
/// build with no `RAMB36E1`, a tile with a block on each half is a line for
/// each. A tile does not say whether it holds a `RAMB36E1` or two
/// `RAMB18E1`s (only some widths set a `RAMB36.` feature), so the caller
/// says, from the build's cells.
fn block_ram_tiles(features: &[(String, String)], halves: bool) -> Vec<String> {
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
    // Two `RAMB18E1` in one tile are two blocks, each compared as a block
    // on either half: Vivado puts a memory's two copies in one tile
    // (`r2_1024x18`) where Reticle's placer may not, and which tile holds a
    // block is the placement's.
    let mut blocks: Vec<BTreeSet<String>> = Vec::new();
    for here in tiles.into_values() {
        let pair = halves
            && ["RAMB18_Y0.IN_USE", "RAMB18_Y1.IN_USE"]
                .iter()
                .all(|f| here.contains(*f));
        if pair {
            for half in ["RAMB18_Y0.", "RAMB18_Y1."] {
                blocks.push(
                    here.iter()
                        .filter(|f| f.starts_with(half))
                        .cloned()
                        .collect(),
                );
            }
        } else {
            blocks.push(here);
        }
    }
    let mut out = Vec::new();
    for here in blocks {
        let halves: BTreeSet<&str> = here
            .iter()
            .filter(|f| f.starts_with("RAMB18_Y"))
            .filter_map(|f| f.split('.').next())
            .collect();
        let alone = halves.len() == 1 && !here.iter().any(|f| f.starts_with("RAMB36."));
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
    tiles: Vec<String>,
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
        } else if let Some(tile) = line.strip_prefix("  tile ") {
            answer.tiles.push(tile.to_owned());
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

/// The designs this flow does not arrange as Vivado does, and why; each
/// held to what it does instead by [`what_is_not_vivados_says_so`].
///
/// | Design | Vivado | Here |
/// |---|---|---|
/// | `async_1024x8` | the read's address register taken into a `RAMB18E1` | distributed RAM: no block RAM reads on no clock here, and Vivado's is not the source's on a write to the word being read (`docs/fpga-vivado-parity.md`) |
/// | `rom_512x36` to `rom_4096x8`, `rom_65536x1` | lookup tables, 174 to 321 of them, 60 and the multiplexers for 65,536 by 1 | a block RAM, or two cascaded, as before: the blocks are the smaller |
/// | `rom_16384x2` | the address register taken into the block, and the read into its output register | the read in the block's latch |
/// | `style_registers_128x8` | 1,024 flip-flops | distributed RAM, as before: measured faster and smaller than either flow's flip-flops |
/// | `sdp_40000x1` to `sdp_131072x1`, `sdp_65536x4`, `sp_*_65536x1`, `tdp_65536x1`, `init_65536x1`, `bw_sdp_65536x16_b8`, `bw_sp_rf_65536x16_b8` | `RAMB36E1`s cascaded in pairs | stacked with a read multiplexer, as before: Vivado's timer finds the cascade slower (`sdp_65536x1` 3.36 ns cascaded here and 3.27 by Vivado, 2.72 stacked); only a read registered again, which the cascade takes into its blocks, is cascaded (`reg_65536x1`, `reg_init_65536x1`) |
///
/// Four more were here until "Deeper than a block, and single ports, as
/// Vivado builds them" (`docs/fpga-vivado-parity.md`): `bw_sp_rf_32x32_b8`,
/// one `RAMB36E1` on
/// port A, is a `RAMB18E1`'s two ports a half each; `bw_sp_wf_1024x32_b8` and
/// `bw_sp_wf_2048x18_b9`, `RAM64M`s, are `RAM256X1S`s; and the two `asym_*`,
/// distributed RAM read on no clock, are four blocks each, a read on the
/// clock a copy.
const NOT_AS_VIVADO: &[&str] = &[
    "async_1024x8",
    "bw_sdp_65536x16_b8",
    "bw_sp_rf_65536x16_b8",
    "init_65536x1",
    "rom_1024x16",
    "rom_1024x18",
    "rom_16384x2",
    "rom_2048x9",
    "rom_4096x8",
    "rom_512x36",
    "rom_65536x1",
    "sdp_131072x1",
    "sdp_40000x1",
    "sdp_65536x1",
    "sdp_65536x2",
    "sdp_65536x4",
    "sp_nc_65536x1",
    "sp_rf_65536x1",
    "sp_wf_65536x1",
    "style_registers_128x8",
    "tdp_65536x1",
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
    // Forty builds at a time: each holds the part's database, nearly two
    // gigabytes, and two hundred at once is more than a machine may have.
    let mut results: Vec<(String, Built)> = Vec::new();
    for chunk in names.chunks(40) {
        results.extend(std::thread::scope(|scope| {
            let root = &root;
            let handles: Vec<_> = chunk
                .iter()
                .map(|name| scope.spawn(move || (name.clone(), build_one(root, name))))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        }));
    }
    Some(results.into_iter().collect())
}

fn build_one(root: &str, name: &str) -> Built {
    let dir = fixtures();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("memarr")
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
    assert!(run.status.success(), "{name}: {log}");
    assert!(!log.contains("NOT FULLY ROUTED"), "{name}: {log}");
    Built {
        decoded: Some(decode(root, &bit)),
        log,
        routed,
    }
}

/// When `RETICLE_MEMARR_BITS` and `UPDATE_EXPECT` are set, `vivado.txt` is
/// made again from `lutram.sh`'s output: never otherwise, so that nothing
/// here runs Vivado or needs to have. A design with no output there keeps
/// what `vivado.txt` said of it, so a run of the designs added is enough.
fn regenerate(root: &str) {
    let (Some(bits), true) = (
        std::env::var_os("RETICLE_MEMARR_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
    ) else {
        return;
    };
    let bits = PathBuf::from(bits);
    // A design `lutram.sh` was not run on this time keeps what `vivado.txt`
    // said of it, so that a run over the designs added is enough.
    let before = std::fs::read_to_string(fixtures().join("vivado.txt")).unwrap_or_default();
    let mut kept: BTreeMap<String, String> = BTreeMap::new();
    let mut at = String::new();
    for line in before.lines() {
        if let Some(name) = line.strip_prefix("design ") {
            at = name.to_owned();
            kept.entry(at.clone()).or_default();
        } else if line.starts_with("  ")
            && let Some(lines) = kept.get_mut(&at)
        {
            lines.push_str(line);
            lines.push('\n');
        }
    }
    let mut out = String::from(
        "# What Vivado 2026.1 made of each design here, on xc7a100tftg256-1, from\n\
         # `FIXTURE=memarr tools/vivado-parity/lutram.sh`. `routed` is its cell count\n\
         # after routing, `params` each block RAM it left with the parameters that\n\
         # arrange it (lutram_probe.tcl), and `tile` each block RAM tile of its\n\
         # bitstream decoded through this repository's database:\n\
         # tests/fpga_xray_memarr_vivado.rs says how to read one.\n",
    );
    for name in designs() {
        out.push_str(&format!("design {name}\n"));
        let dir = bits.join(&name);
        let Ok(report) = std::fs::read_to_string(dir.join(format!("{name}.cells"))) else {
            out.push_str(kept.get(&name).map_or("", String::as_str));
            continue;
        };
        for line in report.lines() {
            if let Some(rest) = line.strip_prefix("routed ") {
                let words: Vec<&str> = rest.split_whitespace().collect();
                let pairs: Vec<String> = words
                    .chunks(2)
                    .filter(|p| p.len() == 2 && !matches!(p[0], "GND" | "VCC"))
                    .map(|p| format!("{}={}", p[0], p[1]))
                    .collect();
                out.push_str(&format!("  routed {}\n", pairs.join(" ")));
            } else if let Some(rest) = line.strip_prefix("params ") {
                out.push_str(&format!("  params {rest}\n"));
            }
        }
        let bit = dir.join(format!("{name}.bit"));
        if bit.is_file() {
            let decoded = decode(root, &bit);
            let halves = !report
                .lines()
                .any(|l| l.starts_with("routed ") && l.contains(" RAMB36E1 "));
            for tile in block_ram_tiles(&decoded.features, halves) {
                out.push_str(&format!("  tile {tile}\n"));
            }
        }
    }
    std::fs::write(fixtures().join("vivado.txt"), out).unwrap();
}

/// The cells of a build's log: its `netlist:` section, `<n> x <cell>`.
fn netlist_cells(log: &str) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    let mut inside = false;
    for line in log.lines() {
        if !line.starts_with("  ") {
            inside = line == "netlist:";
            continue;
        }
        if inside
            && let Some((n, cell)) = line.trim().split_once(" x ")
            && let Ok(n) = n.parse::<u32>()
        {
            out.insert(cell.to_owned(), n);
        }
    }
    out
}

/// The memory cells of a count: block RAMs, and distributed RAMs by their
/// macro (Vivado's count has the macros' leaves too, `RAMD64E`, which are
/// left out).
fn memory_cells(counts: &BTreeMap<String, u32>) -> BTreeMap<String, u32> {
    counts
        .iter()
        .filter(|(cell, _)| {
            cell.starts_with("RAMB")
                || matches!(
                    cell.as_str(),
                    "RAM32M"
                        | "RAM64M"
                        | "RAM32X1D"
                        | "RAM64X1D"
                        | "RAM128X1D"
                        | "RAM32X1S"
                        | "RAM64X1S"
                        | "RAM128X1S"
                        | "RAM256X1S"
                )
        })
        .map(|(cell, n)| (cell.clone(), *n))
        .collect()
}

/// **Each memory gets the cells Vivado gave it**, block RAMs and
/// distributed RAMs by kind and count, for every design but
/// [`NOT_AS_VIVADO`]'s. See the module docs.
#[test]
fn each_memory_gets_the_cells_vivado_gave_it() {
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
        let ours = netlist_cells(&built[name].log);
        compared += 1;
        let (mine, theirs) = (memory_cells(&ours), memory_cells(&answer.routed));
        if mine != theirs {
            wrong.push(format!("{name}: ours {mine:?}, Vivado {theirs:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(compared >= 183, "only {compared} compared");
}

/// **The block RAM tiles are Vivado's**, site feature for site feature and
/// contents bit for contents bit, for every design but [`NOT_AS_VIVADO`]'s.
/// See the module docs for what a line says and what this would catch.
#[test]
fn each_block_ram_tile_holds_what_vivados_does() {
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    let mut compared = 0;
    for (name, answer) in &answers {
        if NOT_AS_VIVADO.contains(&name.as_str()) {
            continue;
        }
        let halves = !netlist_cells(&built[name].log).contains_key("RAMB36E1");
        let ours = block_ram_tiles(&built[name].decoded.as_ref().unwrap().features, halves);
        compared += answer.tiles.len();
        if ours != answer.tiles {
            wrong.push(format!(
                "{name}:\n  ours   {ours:?}\n  Vivado {:?}",
                answer.tiles
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(compared >= 277, "only {compared} tiles compared");
}

/// **What is not arranged as Vivado arranges it is what [`NOT_AS_VIVADO`]
/// says**: the read on no clock in distributed RAM, the read-only memories
/// Vivado builds of lookup tables in blocks, and the register file Vivado
/// builds of flip-flops in distributed RAM; and what left the list is held
/// to it here by its report: a cascade is said, the split port is said, and
/// the asymmetric memories are a copy a read. Would catch any of them
/// changing without the list and the document changing with it.
#[test]
fn what_is_not_vivados_says_so() {
    let Some(built) = built() else { return };
    let cells = |name: &str| memory_cells(&netlist_cells(&built[name].log));
    let one = |cell: &str, n: u32| BTreeMap::from([(cell.to_owned(), n)]);
    assert_eq!(cells("style_registers_128x8"), one("RAM64M", 6));
    let found = cells("async_1024x8");
    assert!(
        found.keys().all(|c| !c.starts_with("RAMB")) && !found.is_empty(),
        "async_1024x8: {found:?}"
    );
    for (name, cell, n) in [
        ("rom_512x36", "RAMB36E1", 1),
        ("rom_1024x16", "RAMB18E1", 1),
        ("rom_1024x18", "RAMB18E1", 1),
        ("rom_2048x9", "RAMB18E1", 1),
        ("rom_4096x8", "RAMB36E1", 1),
        ("rom_16384x2", "RAMB36E1", 1),
        ("rom_65536x1", "RAMB36E1", 2),
    ] {
        assert_eq!(cells(name), one(cell, n), "{name}");
    }
    // Stacked, two rows of 32,768 or more and a multiplexer; cascaded only
    // where the read's register goes into the blocks.
    for (name, deep) in [
        ("sdp_65536x1", "2 deep"),
        ("sdp_65536x4", "2 deep"),
        ("bw_sdp_65536x16_b8", "2 deep"),
        ("bw_sp_rf_65536x16_b8", "2 deep"),
        ("sdp_131072x1", "4 deep"),
    ] {
        assert!(built[name].log.contains(deep), "{name}");
        assert!(!built[name].log.contains("cascaded"), "{name}");
    }
    for name in ["reg_65536x1", "reg_init_65536x1"] {
        assert!(
            built[name].log.contains("each column two blocks cascaded"),
            "{name}"
        );
        assert!(
            built[name]
                .log
                .contains("1 output register(s) in the block"),
            "{name}"
        );
    }
    // The cascade is a net from the lower block to the upper on each port
    // used, which no tile feature shows (a fixed pip): without it the
    // upper reads nothing of the lower half, which only the simulation
    // would otherwise see.
    let routed = std::fs::read_to_string(&built["reg_65536x1"].routed).unwrap();
    for (port, n) in [("A", "mem$casc0_w0_d0"), ("B", "mem$casc1_w0_d0")] {
        assert!(
            routed.contains(&format!(".CASCADEOUT{port}({n})")),
            "{routed}"
        );
        assert!(
            routed.contains(&format!(".CASCADEIN{port}({n})")),
            "{routed}"
        );
    }
    for name in ["bw_sp_rf_32x32_b8", "sp_rf_32x32", "sp_rf_512x64"] {
        assert!(
            built[name].log.contains("one port split over both"),
            "{name}"
        );
    }
    for name in ["asym_2048x9_r36", "asym_4096x8_r32"] {
        assert!(
            built[name].log.contains("4 copies, one per read port"),
            "{name}"
        );
    }
}

/// **Every bit decodes, the arcs are the router's, and the mapping is
/// proved**, for every design. Would catch a block RAM feature in nothing
/// the decoder knows (a width of zero, simple dual port), a tie's pip the
/// decoder finds and the flow did not record, and logic around a block
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
/// does**, on Vivado's models under Vivado's simulator, for every design,
/// those [`NOT_AS_VIVADO`] arranges otherwise included: what a memory
/// computes does not depend on its arrangement. (The two `asym_*` were left
/// out while they were 384 and 768 `RAM64M` here, which Vivado's elaborator
/// had not finished after forty minutes; they are four blocks now.)
/// See the module docs.
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
    let jobs: Vec<(&String, &Built)> = built
        .iter()
        .filter(|(name, one)| expected.contains_key(*name) && one.decoded.is_some())
        .collect();
    // Sixteen simulators at a time: each takes a gigabyte or so.
    let mut runs: Vec<(String, Option<String>)> = Vec::new();
    for chunk in jobs.chunks(16) {
        runs.extend(simulate(&vivado, &glbl, chunk));
    }
    let mut wrong = Vec::new();
    for (name, answer) in &runs {
        let want = &expected[name];
        match answer {
            Some(bits) if bits == want => {}
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
    assert!(runs.len() >= 203, "only {} simulated", runs.len());
}

/// Each of `jobs`' routed netlists under Vivado's simulator at once, and
/// what each printed.
fn simulate(
    vivado: &Path,
    glbl: &Path,
    jobs: &[(&String, &Built)],
) -> Vec<(String, Option<String>)> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(name, one)| {
                scope.spawn(move || {
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
                    ((*name).clone(), answer_of(&out))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

/// **`--memories legacy` arranges as before**: `sdp_16384x18`, nine
/// `RAMB36E1` of 16,384 by 2 with the default, is sixteen `RAMB18E1` with
/// it, as every build before this arrangement had; `sdp_512x36` one
/// `RAMB18E1` 36 wide in simple dual port against two of 18; and
/// `sp_wf_1024x18`, a single port written first, is distributed RAM, its
/// read on no clock. Would catch the arrangement, the pairing or the
/// output registers leaking into the legacy mapping; that the legacy build
/// is the one from before byte for byte was measured, on what the document
/// says, and is not held here. Each is routed to a bitstream, since only
/// routing reaches `xray::lutswap`: `sdp_4096x9` is one of the six legacy
/// builds that stopped there (`` `$lut10` has two signals routed to one
/// input ``) before a table reading a flip-flop and its merged copy was
/// folded; like `sdp_65536x4` above, it builds with either fix alone.
#[test]
fn legacy_memories_map_as_before() {
    let Some(root) = chipdb() else { return };
    if answers().is_none() {
        return;
    }
    for (name, memories, cell, count) in [
        ("sdp_16384x18", "legacy", "RAMB18E1", 16),
        ("sdp_16384x18", "vivado", "RAMB36E1", 9),
        ("sdp_512x36", "legacy", "RAMB18E1", 2),
        ("sdp_512x36", "vivado", "RAMB18E1", 1),
        ("sp_wf_1024x18", "legacy", "RAM64X1D", 288),
        ("sp_wf_1024x18", "vivado", "RAMB18E1", 1),
        ("sdp_4096x9", "legacy", "RAMB18E1", 2),
    ] {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("memarr-legacy")
            .join(format!("{name}-{memories}"));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let dir = fixtures();
        let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args(["fpga", "--device", DEVICE, "--chipdb", &root, "--report"])
            .args(["--top", name, "--memories", memories])
            .arg("--constraints")
            .arg(dir.join(format!("{name}.rcf")))
            .arg("--output-dir")
            .arg(&out)
            .arg("--bitstream")
            .arg(out.join(format!("{name}.bit")))
            .arg(dir.join(format!("{name}.v")))
            .output()
            .expect("the reticle binary runs");
        let log = String::from_utf8_lossy(&run.stderr);
        assert!(run.status.success(), "{log}");
        let cells = netlist_cells(&log);
        assert_eq!(
            cells.get(cell),
            Some(&count),
            "{name} {memories}: {cells:?}"
        );
    }
}

/// **Under a tight clock a small memory is distributed RAM, as Vivado's
/// synthesis makes it**: each case is a design built with its clock's period
/// changed, and what Vivado's synthesis made of it at that period
/// (`Synth 8-5584`, measured, "Deeper than a block, and single ports, as
/// Vivado builds them"): blocks at 4 ns and distributed RAM at 3.5 for up to
/// 1,024 words, and a block at any period for a read registered again or a
/// memory over 32 kbit; 2,048 words, which Vivado moves at 3.2 ns and this
/// flow's timing does not gain by, stay a block. Would catch the line moved,
/// the clock not found, or a memory moved that Vivado keeps.
#[test]
fn a_tight_clock_makes_a_small_memory_distributed_ram() {
    let Some(root) = chipdb() else { return };
    if answers().is_none() {
        return;
    }
    for (name, period, block) in [
        ("sdp_512x36", "4", true),
        ("sdp_512x36", "3.5", false),
        ("sp_wf_1024x18", "3.5", false),
        ("sdp_2048x9", "3.5", true),
        // Vivado's is distributed RAM here, measured slower: a block still.
        ("sdp_2048x9", "3.2", true),
        ("reg_1024x18", "1.4", true),
        ("sdp_1024x36", "1.4", true),
    ] {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("memarr-clock")
            .join(format!("{name}-{period}"));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let dir = fixtures();
        let rcf = std::fs::read_to_string(dir.join(format!("{name}.rcf")))
            .unwrap()
            .replace("-period 5 ", &format!("-period {period} "));
        let constraints = out.join("c.rcf");
        std::fs::write(&constraints, rcf).unwrap();
        let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args(["fpga", "--device", DEVICE, "--chipdb", &root, "--report"])
            .args(["--top", name])
            .arg("--constraints")
            .arg(&constraints)
            .arg("--output-dir")
            .arg(&out)
            .arg(dir.join(format!("{name}.v")))
            .output()
            .expect("the reticle binary runs");
        let log = String::from_utf8_lossy(&run.stderr);
        assert!(run.status.success(), "{log}");
        let cells = memory_cells(&netlist_cells(&log));
        let blocks = cells.keys().any(|c| c.starts_with("RAMB"));
        assert_eq!(blocks, block, "{name} at {period} ns: {cells:?}");
        assert!(!cells.is_empty(), "{name} at {period} ns");
    }
}

/// Per (design, clock): Vivado's own figure, and each build's by seed, from
/// `achieved.txt`.
type Achieved = BTreeMap<(String, String), (f64, BTreeMap<String, Vec<f64>>)>;

fn achieved() -> Option<Achieved> {
    let Ok(text) = std::fs::read_to_string(fixtures().join("achieved.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut table = Achieved::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["vivado", design, clock, ns] => {
                table
                    .entry(((*design).to_owned(), (*clock).to_owned()))
                    .or_default()
                    .0 = ns.parse().expect("a figure");
            }
            ["reticle", build, _seed, design, clock, ns] => table
                .entry(((*design).to_owned(), (*clock).to_owned()))
                .or_default()
                .1
                .entry((*build).to_owned())
                .or_default()
                .push(ns.parse().expect("a figure")),
            ["netlist", ..] => {}
            other => panic!("a line of achieved.txt this does not know: {other:?}"),
        }
    }
    Some(table)
}

fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// **Vivado's arrangement achieves a shorter period than the one before**,
/// under Vivado's timer, in the quoted run of `achieved.txt`: every clock has
/// Vivado's figure and five seeds of `after`, and five of `before` but where
/// `before` did not build; over the clocks both have, the geometric mean of
/// the medians against Vivado's is at least 5 % lower after; no clock's median
/// is more than 5 % slower (the worst is `init_16384x9`, 4 %), and the two
/// that a read multiplexer made slow, `sdp_16384x18` and `analyser_fifo`'s
/// read clock, are faster. Reads the quoted file and runs nothing: it would
/// catch a regeneration that lost a design or a seed, or one that recorded a
/// slower arrangement, and not the flow moving on since.
#[test]
fn vivados_arrangement_achieves_a_shorter_period_than_the_one_before() {
    let Some(table) = achieved() else { return };
    let mut ratios = (Vec::new(), Vec::new());
    for ((design, clock), (vivado, builds)) in &table {
        assert!(*vivado > 0.0, "{design} {clock}: no Vivado figure");
        let after = &builds["after"];
        assert_eq!(after.len(), 5, "{design} {clock}: after");
        let Some(before) = builds.get("before") else {
            assert!(design.starts_with("tdp_"), "{design} {clock}: no before");
            continue;
        };
        assert_eq!(before.len(), 5, "{design} {clock}: before");
        let (b, a) = (median(before), median(after));
        assert!(a <= b * 1.05, "{design} {clock}: {b:.3} to {a:.3} ns");
        ratios.0.push(b / vivado);
        ratios.1.push(a / vivado);
    }
    let geo = |v: &[f64]| (v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64).exp();
    let (before, after) = (geo(&ratios.0), geo(&ratios.1));
    assert!(after <= before * 0.95, "{before:.3} to {after:.3}");
    for key in [("sdp_16384x18", "clk"), ("analyser_fifo", "rd_clk")] {
        let builds = &table[&(key.0.to_owned(), key.1.to_owned())].1;
        assert!(
            median(&builds["after"]) < median(&builds["before"]),
            "{key:?}"
        );
    }
    assert_eq!(table.len(), 28);
}
