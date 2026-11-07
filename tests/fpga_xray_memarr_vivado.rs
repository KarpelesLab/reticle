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
//! and each `ram_style`. `vivado.txt` there keeps what Vivado made of each on
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
/// | `asym_*` | written narrow and read wide: four blocks | refused for block RAM, the loop's four reads taken as reads on no clock; distributed RAM |
/// | `async_1024x8` | the read's address register taken into a `RAMB18E1` | distributed RAM: no block RAM reads on no clock here |
/// | `rom_512x36` to `rom_4096x8` | lookup tables, 174 to 321 of them | a block RAM, as before: one block is the smaller |
/// | `rom_16384x2` | the address register taken into the block, and the read into its output register | the read in the block's latch |
/// | `sdp_65536x1` | two `RAMB36E1` as one through the cascade | two stacked and a multiplexer: no cascade is built |
/// | `style_registers_128x8` | 1,024 flip-flops | distributed RAM, as before: measured faster and smaller than either flow's flip-flops |
const NOT_AS_VIVADO: &[&str] = &[
    "asym_2048x9_r36",
    "asym_4096x8_r32",
    "async_1024x8",
    "rom_1024x16",
    "rom_1024x18",
    "rom_16384x2",
    "rom_2048x9",
    "rom_4096x8",
    "rom_512x36",
    "sdp_65536x1",
    "style_registers_128x8",
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
/// here runs Vivado or needs to have.
fn regenerate(root: &str) {
    let (Some(bits), true) = (
        std::env::var_os("RETICLE_MEMARR_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
    ) else {
        return;
    };
    let bits = PathBuf::from(bits);
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
                    "RAM32M" | "RAM64M" | "RAM32X1D" | "RAM64X1D" | "RAM128X1D"
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
    assert!(compared >= 97, "only {compared} compared");
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
    assert!(compared >= 177, "only {compared} tiles compared");
}

/// **What is not arranged as Vivado arranges it is what [`NOT_AS_VIVADO`]
/// says**: the cascade stacked, the asymmetric memories and the read on no
/// clock in distributed RAM, the read-only memories Vivado builds of
/// lookup tables in a block, and the register file Vivado builds of
/// flip-flops in distributed RAM. Would catch any of them changing without
/// the list and the document changing with it.
#[test]
fn what_is_not_vivados_says_so() {
    let Some(built) = built() else { return };
    let cells = |name: &str| memory_cells(&netlist_cells(&built[name].log));
    let one = |cell: &str, n: u32| BTreeMap::from([(cell.to_owned(), n)]);
    assert_eq!(cells("sdp_65536x1"), one("RAMB36E1", 2));
    assert!(built["sdp_65536x1"].log.contains("2 deep"));
    assert_eq!(cells("style_registers_128x8"), one("RAM64M", 6));
    for name in ["asym_2048x9_r36", "asym_4096x8_r32", "async_1024x8"] {
        let found = cells(name);
        assert!(
            found.keys().all(|c| !c.starts_with("RAMB")) && !found.is_empty(),
            "{name}: {found:?}"
        );
    }
    for (name, cell) in [
        ("rom_512x36", "RAMB36E1"),
        ("rom_1024x16", "RAMB18E1"),
        ("rom_1024x18", "RAMB18E1"),
        ("rom_2048x9", "RAMB18E1"),
        ("rom_4096x8", "RAMB36E1"),
        ("rom_16384x2", "RAMB36E1"),
    ] {
        assert_eq!(cells(name), one(cell, 1), "{name}");
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

/// The designs not simulated: each is 384 or 768 `RAM64M` here (see
/// [`NOT_AS_VIVADO`]), and Vivado's elaborator had not finished one of them
/// after forty minutes, where every other design takes seconds.
const NOT_SIMULATED: &[&str] = &["asym_2048x9_r36", "asym_4096x8_r32"];

/// **The netlist each bitstream was made from answers as Vivado's build
/// does**, on Vivado's models under Vivado's simulator, for every design
/// but [`NOT_SIMULATED`]'s, those [`NOT_AS_VIVADO`] arranges otherwise
/// included: what a memory computes does not depend on its arrangement.
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
        .filter(|(name, _)| !NOT_SIMULATED.contains(&name.as_str()))
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
    assert!(runs.len() >= 106, "only {} simulated", runs.len());
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
/// says, and is not held here.
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
