//! A slice's distributed RAM, held to what Vivado wrote for the same designs.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/lutram/` has one small design per cell by name and
//! per question about inference, written here, with what Vivado 2026.1 made of
//! each in `vivado.txt`: its own account of the cells it left, each pin's bel
//! pin and site pin (`tools/vivado-parity/lutram_probe.tcl`), and each slice
//! holding a RAM as decoded from its bitstream through this repository's
//! database. `xsim.txt` there is what Vivado's simulator says each design does
//! as Vivado built it, on Vivado's own models (`gen_lutram_vectors.py`), and
//! `sweep.txt` which memories its synthesis takes to block RAM.
//! `tools/vivado-parity/lutram.sh` makes the first; this test regenerates
//! `vivado.txt` when `RETICLE_LUTRAM_BITS` names that script's output and
//! `UPDATE_EXPECT` is set.
//!
//! A slice is one line: the modes of each table that is a RAM (`RAM`, `SMALL`,
//! `DI1MUX`), the slice's `WA7USED`, `WA8USED`, `WEMUX.CE` and `CLKINV`, and
//! each RAM table's contents, which is every bit Vivado set for the cell. What
//! the slice's flip-flops and output muxes do is the placement's and the
//! routes', and is left out; the pins the cell's signals arrive on are
//! compared apart, by their ends.
//!
//! # What these would and would not catch
//!
//! [`each_ram_slice_holds_what_vivados_does`] would catch a cell on the wrong
//! tables, a 32-word table in the wrong half, a mode bit missing or extra,
//! `WEMUX.CE` clear, contents permuted in any way, the seventh and eighth
//! address bits not enabled, and a dual-port cell with one port unread kept as
//! two tables. [`vivados_pins_are_the_fabrics`] would catch a data input on
//! the wrong half's `DI`, an address bit on another input, and the write
//! enable on `WE`: the pins the fabric declares for each shape against the
//! site pins Vivado's own report has for the same pin of the same cell. It
//! reads the fabric's declaration and not a build, so it would not catch the
//! router delivering a signal somewhere else, which every set bit decoding to
//! exactly the router's arcs is for, and the simulation.
//!
//! [`a_routed_netlist_answers_as_vivados_build_does`] simulates the netlist each
//! bitstream was made from against Vivado's simulator on Vivado's netlist, for
//! 2,000 cycles from power-up: it would catch contents in the wrong order, a
//! port reading the other's address, and inference building the wrong memory.
//! It reads the routed netlist with this repository's models of the cells
//! (`xc7_cells.v`, checked against Vivado's by `tests/fpga_xray_cells.rs`), so
//! a cell whose bits are wrong but whose netlist is right gets past it; the
//! slice comparison is what holds the bits. The two designs whose memory is a
//! `RAMB36E1` are left to `tests/fpga_xray_bram_vivado.rs`, which simulates
//! such a block on Vivado's own model, since this repository has none.
//!
//! None of it has been near a part. A slice so configured rests on Vivado's
//! having written the same bits.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli", feature = "sim"))]

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
    manifest().join("testdata/fpga/vivado/lutram")
}

/// The database root, or `None` with a line saying what is missing.
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

/// One line per slice holding a RAM; see the module docs.
fn ram_slices(features: &[(String, String)]) -> Vec<String> {
    let mut tiles: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (tile, feature) in features {
        tiles.entry(tile).or_default().push(feature);
    }
    let mut out = Vec::new();
    for here in tiles.values() {
        let mut rams: Vec<char> = Vec::new();
        let mut keep: Vec<&str> = Vec::new();
        let mut init: BTreeMap<char, u64> = BTreeMap::new();
        for feature in here {
            let Some(f) = feature.strip_prefix("SLICEM_X0.") else {
                continue;
            };
            let lut = f.chars().next().filter(|c| ('A'..='D').contains(c));
            let table = lut.and_then(|l| f[1..].strip_prefix("LUT.").map(|rest| (l, rest)));
            match table {
                Some((l, "RAM")) => {
                    rams.push(l);
                    keep.push(f);
                }
                Some((_, rest)) if rest == "SMALL" || rest.starts_with("DI1MUX.") => keep.push(f),
                Some((l, rest)) if rest.starts_with("INIT[") => {
                    let bit: u32 = rest[5..rest.len() - 1].parse().unwrap_or(0);
                    *init.entry(l).or_default() |= 1 << bit;
                }
                _ if matches!(f, "WA7USED" | "WA8USED" | "WEMUX.CE" | "CLKINV") => keep.push(f),
                _ => {}
            }
        }
        if rams.is_empty() {
            continue;
        }
        rams.sort_unstable();
        keep.sort_unstable();
        let contents: Vec<String> = rams
            .iter()
            .map(|l| format!("{l}={:016x}", init.get(l).copied().unwrap_or(0)))
            .collect();
        out.push(format!("{} init {}", keep.join(" "), contents.join(" ")));
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
    /// `leaf pin sitepin...` of each memory cell's leaves.
    sitepins: Vec<(String, String, Vec<String>)>,
    /// Each top-level memory cell by name and primitive.
    cells: Vec<(String, String)>,
    slices: Vec<String>,
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
        } else if let Some(rest) = line.strip_prefix("  sitepin ") {
            let mut words = rest.split_whitespace();
            let (Some(leaf), Some(pin)) = (words.next(), words.next()) else {
                continue;
            };
            answer.sitepins.push((
                leaf.to_owned(),
                pin.to_owned(),
                words.map(str::to_owned).collect(),
            ));
        } else if let Some(rest) = line.strip_prefix("  cell ") {
            let mut words = rest.split_whitespace();
            if let (Some(name), Some(primitive)) = (words.next(), words.next()) {
                answer.cells.push((name.to_owned(), primitive.to_owned()));
            }
        } else if let Some(slice) = line.strip_prefix("  slice ") {
            answer.slices.push(slice.to_owned());
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

/// One design through the flow.
struct Built {
    decoded: Decoded,
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
            .map(|name| scope.spawn(move || (name.clone(), build_one(root, name, &[]))))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    Some(results.into_iter().collect())
}

fn build_one(root: &str, name: &str, extra: &[&str]) -> Built {
    let dir = fixtures();
    let tag = if extra.is_empty() {
        "lutram"
    } else {
        "lutram-extra"
    };
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag).join(name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join(format!("{name}.bit"));
    let routed = out.join(format!("{name}.routed.v"));
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--chipdb", root, "--top", name])
        .args(extra)
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
        decoded: decode(root, &bit),
        log,
        routed,
    }
}

/// When `RETICLE_LUTRAM_BITS` and `UPDATE_EXPECT` are set, `vivado.txt` is made
/// again from `tools/vivado-parity/lutram.sh`'s output: never otherwise, so
/// that nothing here runs Vivado or needs to have.
fn regenerate(root: &str) {
    let (Some(bits), true) = (
        std::env::var_os("RETICLE_LUTRAM_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
    ) else {
        return;
    };
    let bits = PathBuf::from(bits);
    let mut out = String::from(
        "# What Vivado 2026.1 made of each design here, on xc7a100tftg256-1, from\n\
         # tools/vivado-parity/lutram.sh. `routed` is its cell count after routing,\n\
         # `cell` each memory cell it left and its bel, `sitepin` the site pins each\n\
         # pin of a named cell's leaves reaches at its own site, from its own report\n\
         # (lutram_probe.tcl), with the sites taken off. `slice` lines are its\n\
         # bitstream decoded through this repository's database, one per slice\n\
         # holding a RAM: tests/fpga_xray_lutram_vivado.rs says how to read one.\n",
    );
    for name in designs() {
        out.push_str(&format!("design {name}\n"));
        let dir = bits.join(&name);
        let report = std::fs::read_to_string(dir.join(format!("{name}.cells"))).unwrap();
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
                let bel = words
                    .iter()
                    .position(|w| *w == "bel")
                    .and_then(|at| words.get(at + 1))
                    .filter(|b| **b != "site")
                    .unwrap_or(&"-");
                out.push_str(&format!("  cell {} {} bel {bel}\n", words[0], words[1]));
            } else if let Some(rest) = line.strip_prefix("sitepin ") {
                // A named cell's leaves only: an inferred one's names are
                // Vivado's own.
                if rest.starts_with("dut/") {
                    out.push_str(&format!("  sitepin {rest}\n"));
                }
            }
        }
        let decoded = decode(root, &dir.join(format!("{name}.bit")));
        for slice in ram_slices(&decoded.features) {
            out.push_str(&format!("  slice {slice}\n"));
        }
    }
    std::fs::write(fixtures().join("vivado.txt"), out).unwrap();
}

/// The designs whose slices are not Vivado's line for line, and why. Vivado
/// packs more into a slice than one cell, and this flow places one cell a
/// slice but for the halves of a `RAM32X1D`:
///
/// | Design | Vivado | Here |
/// |---|---|---|
/// | `mem32x8r2`, `r3`, `r4` | a read port's two `RAM32X1D` and another's in one slice, all four tables | a slice each pair |
/// | `mem128x4r2` | two read ports' `RAM64X1D` in one slice | a slice each |
/// | `mem64x8`, `r2`, `r3` | a `RAM64M` whose unused `DIC` is `DID`'s, so `CLUT.DI1MUX.CI` is clear | the mux set, its input a constant |
/// | `mem32x8` | its two `RAM32X1D` in one slice, the `C` and `D` tables' upper halves and lower | a slice each |
///
/// `mem32x8` was Vivado's line for line until 2026-11-06, and nothing put
/// its two `RAM32X1D` in one slice but the anneal: no rule of the placer asks
/// for it. When `--synthesis vivado` mapped the XOR of its read word in two
/// tables deep where it had been three, the placement moved and the two went
/// a slice each. The cells are still Vivado's.
///
/// The cells are Vivado's in all of them ([`each_design_gets_the_cells_vivado_gave_it`]).
const PACKED: &[&str] = &[
    "mem128x4r2",
    "mem32x8",
    "mem32x8r2",
    "mem32x8r3",
    "mem32x8r4",
    "mem64x8",
    "mem64x8r2",
    "mem64x8r3",
];

/// **The slices are Vivado's.** See the module docs for what a line says and
/// what this would catch.
#[test]
fn each_ram_slice_holds_what_vivados_does() {
    let Some(root) = chipdb() else { return };
    regenerate(&root);
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    let mut compared = 0;
    for (name, answer) in &answers {
        if PACKED.contains(&name.as_str()) {
            continue;
        }
        let ours = ram_slices(&built[name].decoded.features);
        compared += answer.slices.len();
        if ours != answer.slices {
            wrong.push(format!(
                "{name}:\n  ours   {ours:?}\n  Vivado {:?}",
                answer.slices
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(compared >= 40, "only {compared} slices compared");
}

/// **Where Vivado packs tighter, the slices hold no other kind of RAM.** Each
/// design of [`PACKED`] has every line of Vivado's but the packed ones, and
/// none Vivado has no shape for. Would catch a packing difference turning into
/// a different cell; would not catch the packing itself, which is not done.
#[test]
fn where_vivado_packs_tighter_it_is_only_the_packing() {
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    for name in PACKED {
        let ours: BTreeSet<String> = ram_slices(&built[*name].decoded.features)
            .into_iter()
            .collect();
        let theirs: BTreeSet<String> = answers[*name].slices.iter().cloned().collect();
        let extra: Vec<&String> = ours.difference(&theirs).collect();
        // Ours may only be the unpacked tables of a shape Vivado also used.
        for line in extra {
            assert!(
                line.starts_with("CLUT.RAM CLUT.SMALL DLUT.RAM DLUT.SMALL WEMUX.CE")
                    || line.starts_with("CLUT.RAM DLUT.RAM WEMUX.CE")
                    || line.starts_with(
                        "ALUT.DI1MUX.AI ALUT.RAM BLUT.DI1MUX.BI BLUT.RAM CLUT.DI1MUX.CI"
                    ),
                "{name}: {line}"
            );
        }
    }
}

/// The memory cells of a count, the `_1` of each counted as the cell, and the
/// one-bit cells inside a two-bit one left out, as Vivado counts its macros.
fn memory_cells(counts: &BTreeMap<String, u32>) -> BTreeMap<String, u32> {
    let mut out: BTreeMap<String, u32> = BTreeMap::new();
    for (cell, n) in counts {
        let cell = cell.strip_suffix("_1").unwrap_or(cell);
        if cell.starts_with("RAM") && !cell.starts_with("RAMD") && !cell.starts_with("RAMS") {
            *out.entry(cell.to_owned()).or_default() += n;
        }
    }
    for (two, one) in [("RAM32X2S", "RAM32X1S"), ("RAM64X2S", "RAM64X1S")] {
        if let Some(n) = out.get(two).copied()
            && let Some(m) = out.get_mut(one)
        {
            *m -= (2 * n).min(*m);
        }
    }
    out.retain(|_, n| *n > 0);
    out
}

/// **Each design gets the cells Vivado gave it**, by kind and count: the
/// shapes inference picks for every memory of the fixture, and a named cell
/// kept as that cell, a dual-port one with its second port unread made the
/// single-port one. Would catch a rule of `vivado_lutram_plan` changed, a
/// memory under Vivado's threshold sent to block RAM, and a named cell made
/// logic. The two that differ wait for inference of the block RAM's simple
/// dual port: Vivado puts 32 words of 32 bits in one `RAMB18E1` read 36 bits
/// wide, and Reticle in one `RAMB36E1` in true dual port, the same tile. It was
/// two `RAMB18E1` before `RAMB36E1` was inferred, which is the same tile too.
#[test]
fn each_design_gets_the_cells_vivado_gave_it() {
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    for (name, answer) in &answers {
        let mut ours: BTreeMap<String, u32> = BTreeMap::new();
        for line in built[name].log.lines() {
            if let Some((n, cell)) = line.trim().split_once(" x ")
                && let Ok(n) = n.parse::<u32>()
                && !cell.contains(' ')
            {
                ours.insert(cell.to_owned(), n);
            }
        }
        let (ours, theirs) = (memory_cells(&ours), memory_cells(&answer.routed));
        let expected = match name.as_str() {
            "mem32x32" => BTreeMap::from([("RAMB36E1".to_owned(), 1)]),
            "mem32x32r2" => BTreeMap::from([("RAMB36E1".to_owned(), 2)]),
            _ => theirs.clone(),
        };
        if ours != expected {
            wrong.push(format!("{name}: ours {ours:?}, Vivado {theirs:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The role of `xc7.dev`'s bel line a pin of one of Vivado's leaf cells is,
/// for a named cell of `primitive`.
fn role_of(primitive: &str, leaf: &str, pin: &str) -> Option<String> {
    let primitive = primitive.strip_suffix("_1").unwrap_or(primitive);
    let sub = leaf.rsplit('/').next().unwrap_or(leaf);
    let parent = leaf.split('/').nth(1).unwrap_or("");
    let digits = |p: &str| {
        p.trim_start_matches(|c: char| !c.is_ascii_digit())
            .to_owned()
    };
    match pin {
        "CLK" => return Some("wclk".to_owned()),
        "WE" => return Some("we".to_owned()),
        _ => {}
    }
    match primitive {
        "RAM32M" | "RAM64M" => {
            let port = sub.chars().nth(3)?.to_ascii_lowercase();
            let bit = if sub.ends_with("_D1") { "1" } else { "0" };
            let bit = if primitive == "RAM32M" { bit } else { "" };
            if pin == "I" {
                Some(format!("di{port}{bit}"))
            } else if pin == "O" {
                Some(format!("do{port}{bit}"))
            } else if pin.starts_with("RADR") {
                Some(format!("addr{port}{}", digits(pin)))
            } else if pin.starts_with("ADR") || pin.starts_with("WADR") {
                // Port `D`'s own tables are single-port cells, read at the
                // write address.
                Some(format!("addrd{}", digits(pin)))
            } else {
                None
            }
        }
        "RAM32X1D" | "RAM64X1D" => match (sub, pin) {
            (_, "I") => Some("din".to_owned()),
            ("SP", "O") => Some("spo".to_owned()),
            ("DP", "O") => Some("dout".to_owned()),
            ("SP", p) if p.starts_with("RADR") => Some(format!("waddr{}", digits(p))),
            ("DP", p) if p.starts_with("RADR") => Some(format!("raddr{}", digits(p))),
            (_, p) if p.starts_with("WADR") => Some(format!("waddr{}", digits(p))),
            _ => None,
        },
        "RAM32X1S" | "RAM64X1S" => match pin {
            "I" => Some("din".to_owned()),
            "O" => Some("spo".to_owned()),
            p if p.starts_with("ADR") => Some(format!("waddr{}", digits(p))),
            _ => None,
        },
        "RAM32X2S" | "RAM64X2S" => {
            let index: String = parent.chars().last().into_iter().collect();
            match pin {
                "I" => Some(format!("din{index}")),
                "O" => Some(format!("spo{index}")),
                p if p.starts_with("ADR") => Some(format!("waddr{}", digits(p))),
                _ => None,
            }
        }
        "RAM128X1S" | "RAM256X1S" => match pin {
            "I" => Some("din".to_owned()),
            p if p.starts_with("ADR") || p.starts_with("WADR") => {
                Some(format!("waddr{}", digits(p)))
            }
            _ => None,
        },
        "RAM128X1D" => match pin {
            "I" => Some("din".to_owned()),
            // The write port's tables read at the write address.
            p if p.starts_with("RADR") && sub.starts_with("SP") => {
                Some(format!("waddr{}", digits(p)))
            }
            p if p.starts_with("RADR") => Some(format!("raddr{}", digits(p))),
            p if p.starts_with("WADR") => Some(format!("waddr{}", digits(p))),
            _ => None,
        },
        _ => None,
    }
}

/// A site pin of a `SLICEM` as the fabric's wire name: a table's lower half
/// leaves by its output mux, and the fabric declares it on its own wire.
fn pin_wire(pin: &str) -> String {
    match pin.strip_suffix("MUX") {
        Some(lut) if lut.len() == 1 => format!("CLBLM_M_MUX_{lut}O5"),
        _ => format!("CLBLM_M_{pin}"),
    }
}

/// **The pins the fabric declares for each shape are Vivado's site pins**, for
/// every pin of every named cell's leaves that reaches one, by role. See the
/// module docs for what this would and would not catch.
#[test]
fn vivados_pins_are_the_fabrics() {
    let Some(root) = chipdb() else { return };
    let Some(answers) = answers() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let region = db
        .region_for_pins(&DiskFiles, &["L4".to_owned()], 12)
        .unwrap()
        .unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(region))
        .unwrap();
    let tile = fabric
        .arch
        .tile_types
        .iter()
        .find(|t| t.bels.iter().any(|b| b.kind == "lutram_m32"))
        .expect("a SLICEM in the region");
    let mut checked = 0;
    let mut wrong = Vec::new();
    for (name, answer) in &answers {
        let Some((_, primitive)) = answer.cells.iter().find(|(cell, _)| cell == "dut") else {
            continue;
        };
        let primitive = primitive.as_str();
        // The bel of this shape: the first of its kind, its usual place.
        let kind = match primitive {
            "RAM64X1D" => "lutram",
            "RAM32X1D" => "lutram_d32",
            "RAM64X1S" => "lutram_s64",
            "RAM32X1S" => "lutram_s32",
            "RAM64X2S" => "lutram_s64x2",
            "RAM32X2S" => "lutram_s32x2",
            "RAM128X1S" => "lutram_s128",
            "RAM128X1D" => "lutram_d128",
            "RAM256X1S" => "lutram_s256",
            "RAM32M" => "lutram_m32",
            "RAM64M" => "lutram_m64",
            other => panic!("{name}: no shape for `{other}`"),
        };
        let bel = tile.bels.iter().find(|b| b.kind == kind).unwrap();
        let mut theirs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (leaf, pin, sites) in &answer.sitepins {
            if sites.is_empty() {
                continue;
            }
            let Some(role) = role_of(primitive, leaf, pin) else {
                wrong.push(format!("{name}: no role for {leaf} {pin}"));
                continue;
            };
            theirs
                .entry(role)
                .or_default()
                .extend(sites.iter().map(|s| pin_wire(s)));
        }
        for (role, wires) in theirs {
            let ours: BTreeSet<String> = bel
                .pins
                .iter()
                .filter(|(r, _)| *r == role)
                .map(|(_, w)| w.name.clone())
                .collect();
            checked += 1;
            if ours != wires {
                wrong.push(format!("{name} {role}: ours {ours:?}, Vivado {wires:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(checked >= 200, "only {checked} roles checked");
}

/// **Every bit decodes, the arcs are the router's, and the mapping is proved**,
/// for every design here: `reticle fpga` checks the second itself and refuses to
/// finish without it. Would catch a RAM bit in nothing the decoder knows, a
/// mode feature misspelled (the loader would drop the bel and the build fail),
/// and an inferred RAM built from a graph the proof says differs.
#[test]
fn every_bit_decodes_and_every_arc_is_the_routers() {
    let Some(built) = built() else { return };
    for (name, one) in built {
        assert_eq!(
            one.decoded.unexplained,
            0,
            "{name}: {}",
            one.decoded.to_text()
        );
        assert!(
            one.decoded.ecc_faults.is_empty(),
            "{name}: {:?}",
            one.decoded.ecc_faults
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

/// **A RAM's write enable does not enable its slice's flip-flops.** It takes
/// the slice's `CE` pin, which the flip-flops read through `CEUSEDMUX`, and no
/// flip-flop of these designs has an enable, so no slice of them may have
/// `CEUSEDMUX` set. Would catch the bit put back on the hop into the pin, where
/// a RAM's write enable set it and every flip-flop beside the RAM loaded only
/// when it wrote: a fault no simulation of the netlist could see.
#[test]
fn a_rams_write_enable_does_not_enable_its_slices_flip_flops() {
    let Some(built) = built() else { return };
    for (name, one) in built {
        let enabled: Vec<&(String, String)> = one
            .decoded
            .features
            .iter()
            .filter(|(_, f)| f.ends_with(".CEUSEDMUX"))
            .collect();
        assert!(enabled.is_empty(), "{name}: {enabled:?}");
    }
}

/// **Vivado's frames have the ECC this decode expects of them**, with a RAM's
/// contents left out of it: every `.bit` of `lutram.sh`'s output, when
/// `RETICLE_LUTRAM_BITS` names it. Would catch the exclusion dropped, which
/// leaves 71 of those frames wrong, or widened to a whole frame.
#[test]
fn vivados_frames_have_the_ecc_expected_of_them() {
    let Some(root) = chipdb() else { return };
    let Some(bits) = std::env::var_os("RETICLE_LUTRAM_BITS") else {
        eprintln!("skipped: RETICLE_LUTRAM_BITS does not name lutram.sh's output");
        return;
    };
    for name in designs() {
        let bit = PathBuf::from(&bits).join(&name).join(format!("{name}.bit"));
        let decoded = decode(&root, &bit);
        assert!(
            decoded.ecc_faults.is_empty(),
            "{name}: {:?}",
            decoded.ecc_faults
        );
    }
}

/// **The netlist each bitstream was made from answers as Vivado's build does**
/// under Vivado's simulator, cycle for cycle from power-up. See the module docs.
#[test]
fn a_routed_netlist_answers_as_vivados_build_does() {
    let Some(built) = built() else { return };
    let expected: BTreeMap<String, String> = std::fs::read_to_string(fixtures().join("xsim.txt"))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once(' '))
        .map(|(n, b)| (n.to_owned(), b.to_owned()))
        .collect();
    let cells = manifest().join("src/fpga/devices/xc7_cells.v");
    let sim = manifest().join("examples/husky/sim");
    // A design whose memory went to a `RAMB36E1` (`mem32x32`, `mem32x32r2`):
    // this repository's simulator has no model of one, and
    // `tests/fpga_xray_bram_vivado.rs` simulates that cell on Vivado's.
    let whole: Vec<&String> = built
        .iter()
        .filter(|(_, one)| {
            std::fs::read_to_string(&one.routed).is_ok_and(|t| t.contains("RAMB36E1 "))
        })
        .map(|(name, _)| name)
        .collect();
    for name in &whole {
        eprintln!("not simulated here: `{name}` is a RAMB36E1");
    }
    assert!(whole.len() <= 2, "{whole:?}");
    let results: Vec<(String, Option<String>, String)> = std::thread::scope(|scope| {
        let handles: Vec<_> = built
            .iter()
            .filter(|(name, _)| !whole.contains(name))
            .map(|(name, one)| {
                let (cells, sim) = (&cells, &sim);
                scope.spawn(move || {
                    let dir = one.routed.parent().unwrap();
                    let text = std::fs::read_to_string(&one.routed).unwrap();
                    let at = text
                        .find(&format!("module {name} "))
                        .expect("the routed netlist's top is named for the design");
                    let end = text[at..].find("endmodule").expect("it ends") + at;
                    let netlist = dir.join("routed_top.v");
                    std::fs::write(&netlist, format!("{}endmodule\n", &text[at..end])).unwrap();
                    let tb = dir.join("tb.v");
                    std::fs::write(&tb, testbench(name)).unwrap();
                    // A block RAM, which `xc7_cells.v` has no model of: the
                    // Husky's simulation models have one, in the one mode the
                    // mapper writes, beside cells that are `xc7_cells.v`'s too.
                    let models = std::fs::read_to_string(sim.join("xilinx_cells.v")).unwrap();
                    let at = models.find("module RAMB18E1").expect("a RAMB18E1 model");
                    let end = models[at..].find("endmodule").expect("it ends") + at;
                    let bram = dir.join("ramb18e1.v");
                    std::fs::write(&bram, format!("{}endmodule\n", &models[at..end])).unwrap();
                    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
                        .args(["sim", "--quiet", "--top", "tb"])
                        .arg(&tb)
                        .arg(&netlist)
                        .arg(cells)
                        .arg(sim.join("xilinx_primitives.v"))
                        .arg(&bram)
                        .output()
                        .expect("the reticle binary runs");
                    let out = String::from_utf8_lossy(&run.stdout).into_owned();
                    let err = String::from_utf8_lossy(&run.stderr).into_owned();
                    let bits = out
                        .lines()
                        .find_map(|l| l.strip_prefix("R "))
                        .map(|b| b.trim().to_owned());
                    (name.clone(), bits, format!("{out}\n{err}"))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut wrong = Vec::new();
    for (name, bits, log) in &results {
        let want = &expected[name];
        match bits {
            Some(bits) if bits == want => {}
            Some(bits) => {
                let at = bits
                    .chars()
                    .zip(want.chars())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                wrong.push(format!("{name}: differs from cycle {at}"));
            }
            None => wrong.push(format!("{name}: no answer\n{log}")),
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert_eq!(results.len() + whole.len(), expected.len());
}

/// The stimulus of `tools/vivado-parity/gen_lutram_vectors.py`, on the routed
/// netlist.
fn testbench(name: &str) -> String {
    format!(
        "module tb;\n\
         \x20 reg clk = 0;\n\
         \x20 reg si = 0;\n\
         \x20 reg [15:0] lfsr = 16'hACE1;\n\
         \x20 wire so;\n\
         \x20 {name} dut (.clk(clk), .si(si), .so(so));\n\
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
         endmodule\n\
         module IBUF (output O, input I);\n\
         \x20 assign O = I;\n\
         endmodule\n\
         module OBUF (output O, input I);\n\
         \x20 assign O = I;\n\
         endmodule\n"
    )
}

/// **Vivado's threshold, read off its synthesis**: every memory of `sweep.txt`'s
/// `s_` designs is block RAM exactly when `vivado_prefers_block_ram` says, and
/// so are the ones written near the line. This is the fixture checked against
/// the rule; that the mapper applies it is
/// [`each_design_gets_the_cells_vivado_gave_it`]'s, for the designs here. Would
/// catch the rule's rounding or its threshold changed, and the fixture saying
/// otherwise when it is made again.
#[test]
fn vivados_threshold_is_the_rule() {
    use reticle::fpga::primitives::vivado_prefers_block_ram;
    let Ok(text) = std::fs::read_to_string(fixtures().join("sweep.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    };
    let mut seen = 0;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let (name, cells) = line.split_once(' ').unwrap();
        let block = cells.contains("RAMB");
        let shape = if let Some(rest) = name.strip_prefix("s_") {
            let mut parts = rest.split('_').skip(1);
            let depth: u64 = parts.next().unwrap().parse().unwrap();
            let width: u32 = parts.next().unwrap().parse().unwrap();
            Some((depth, width))
        } else {
            // `v_<depth>x<width>`, and the two that write fewer bits than
            // they declare.
            match name {
                "v_lutram32" | "v_32x32_const" => Some((32, 31)),
                "v_lutram32_full" => Some((32, 32)),
                _ => name.strip_prefix("v_").and_then(|r| {
                    let (d, w) = r.split_once('x')?;
                    Some((d.parse().ok()?, w.parse().ok()?))
                }),
            }
        };
        let Some((depth, width)) = shape else {
            continue;
        };
        assert_eq!(vivado_prefers_block_ram(depth, width), block, "{line}");
        seen += 1;
    }
    assert!(seen >= 319, "only {seen} lines");
}

/// **`--memories legacy` is the mapping before**: `qor/lutram32` in two
/// `RAMB18E1` and `qor/regfile_alu` in 64 `RAM64X1D` on the slice's `WE` pin,
/// with `WEMUX.CE` clear, as `docs/fpga-vivado-parity.md`'s first table had
/// them. Would catch the option not reaching the mapper, the fabric or the
/// threshold; that the build is the one from before byte for byte was
/// measured once, on what the document says, and is not held here.
#[test]
fn legacy_memories_map_as_before() {
    let Some(root) = chipdb() else { return };
    let qor = manifest().join("testdata/fpga/vivado/qor");
    if !qor.join("lutram32.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    for (name, cell, count) in [("lutram32", "RAMB18E1", 2), ("regfile_alu", "RAM64X1D", 64)] {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("lutram-legacy")
            .join(name);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let bit = out.join(format!("{name}.bit"));
        let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args(["fpga", "--device", DEVICE, "--chipdb", &root, "--report"])
            .args(["--top", name, "--memories", "legacy"])
            .arg("--constraints")
            .arg(qor.join(format!("{name}.rcf")))
            .arg("--output-dir")
            .arg(&out)
            .arg("--bitstream")
            .arg(&bit)
            .arg(qor.join(format!("{name}.v")))
            .output()
            .expect("the reticle binary runs");
        let log = String::from_utf8_lossy(&run.stderr);
        assert!(run.status.success(), "{log}");
        assert!(
            log.contains(&format!("  {count} x {cell}\n")),
            "{name}: {log}"
        );
        // No slice holding a RAM has `WEMUX.CE`: a shift register's slice
        // does (`regfile_alu` has one), and a shift register never shares a
        // slice with the legacy RAM (`xray::srl` blocks it).
        let decoded = decode(&root, &bit);
        let rams: BTreeSet<(&str, &str)> = decoded
            .features
            .iter()
            .filter_map(|(t, f)| f.strip_suffix(".DLUT.RAM").map(|slice| (t.as_str(), slice)))
            .collect();
        assert_eq!(rams.is_empty(), cell != "RAM64X1D", "{name}");
        for (tile, slice) in &rams {
            let enabled = format!("{slice}.WEMUX.CE");
            assert!(
                !decoded
                    .features
                    .iter()
                    .any(|(t, f)| t == tile && *f == enabled),
                "{name}: {tile} {slice}"
            );
        }
    }
}
