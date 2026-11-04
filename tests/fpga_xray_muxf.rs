//! A slice's wide multiplexers, held to what Vivado wrote for the same designs.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/muxf/` has one small design per question, written
//! here, with what Vivado 2026.1 made of it in `vivado.txt`: its own account of
//! where each `MUXF7` and `MUXF8` went (`tools/vivado-parity/muxf_probe.tcl`), and
//! each slice holding one as decoded from its bitstream through this
//! repository's database. `tools/vivado-parity/muxf.sh` makes them; this test
//! regenerates the decoded part when `RETICLE_MUXF_BITS` names that script's
//! output and `UPDATE_EXPECT` is set.
//!
//! A multiplexer is one line: its data tables by how many ones each truth table
//! has, which says which table is on which input whatever its own inputs were
//! permuted to; whether its output is registered through `<L>FFMUX` or leaves
//! by `<L>OUTMUX`; and whether the bypass input its select needs is reached
//! from the interconnect. A `MUXF8` is one line with the two `MUXF7`s under it,
//! whose places follow from it; a `MUXF7` alone may be either of a slice's, and
//! Vivado uses both, so which it is is not asked.
//!
//! # What these would and would not catch
//!
//! [`each_slice_holds_what_vivados_does`] would catch a `MUXF7`'s `I0` and `I1`
//! on the wrong tables, which is the guess from the lanes' order that was going
//! to be wrong here (the higher letter is `I0`), a select on the wrong bypass
//! input, a wide multiplexer's output on the wrong lane's mux, and a flip-flop
//! that could have been beside its multiplexer and is not. It would not catch
//! a table's contents wrong within a count of ones, which
//! [`a_routed_netlist_answers_as_its_rtl_does`] does, by simulation.
//!
//! The second would catch a `MUXF8` wired `I1`/`I0` the other way in the netlist
//! or a table's inputs misread; it reads the netlist, so it would not catch a
//! netlist the bitstream then encodes wrongly, which the first and the flow's
//! own decode are for.
//!
//! Neither has been anywhere near a part. Whether a slice so configured computes
//! this on silicon rests on Vivado's having written the same bits.
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
    manifest().join("testdata/fpga/vivado/muxf")
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

/// The `BYP_ALT` wire of the interconnect beside it that reaches each bypass
/// input of a CLB's two slices, read off `ppips_clbll_l.db` (`CLBLL_LL_AX` from
/// `CLBLL_BYP1`, and so on) and the same in all four CLB tile types: slice
/// index, then `A`, `B`, `C`.
const BYPASS: [[u32; 3]; 2] = [[1, 4, 3], [0, 5, 2]];

/// One line per wide multiplexer outside a `MUXF8`, and per `MUXF8`; see the
/// module docs.
fn slices(features: &[(String, String)]) -> Vec<String> {
    let mut tiles: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (tile, feature) in features {
        tiles.entry(tile).or_default().push(feature);
    }
    let mut out = Vec::new();
    for (tile, here) in &tiles {
        if !tile.starts_with("CLB") {
            continue;
        }
        let at = tile.rsplit_once('_').map_or("", |(_, at)| at);
        let bypass: BTreeSet<&str> = ["INT_L", "INT_R"]
            .iter()
            .filter_map(|side| tiles.get(format!("{side}_{at}").as_str()))
            .flatten()
            .filter_map(|f| f.split_once('.').map(|(to, _)| to))
            .filter(|to| to.starts_with("BYP_ALT"))
            .collect();
        for (index, prefix) in [(0usize, "SLICEL_X0"), (0, "SLICEM_X0"), (1, "SLICEL_X1")] {
            let of = |name: &str| here.contains(&format!("{prefix}.{name}").as_str());
            let f8 = of("BFFMUX.F8") || of("BOUTMUX.F8");
            let f7a = f8 || of("AFFMUX.F7") || of("AOUTMUX.F7");
            let f7b = f8 || of("CFFMUX.F7") || of("COUTMUX.F7");
            if !f7a && !f7b {
                continue;
            }
            let ones = |lane: char| {
                let table = format!("{prefix}.{lane}LUT.INIT[");
                here.iter().filter(|f| f.starts_with(&table)).count()
            };
            let exit = |lane: char, value: &str| {
                let registered = of(&format!("{lane}FFMUX.{value}"));
                let leaves = of(&format!("{lane}OUTMUX.{value}"));
                match (registered, leaves) {
                    (true, true) => "both",
                    (true, false) => "ff",
                    (false, true) => "out",
                    (false, false) => "none",
                }
            };
            let select = |column: usize| {
                let wire = format!("BYP_ALT{}", BYPASS[index][column]);
                let lane = ['A', 'B', 'C'][column];
                let another = of(&format!("{lane}FFMUX.{lane}X"))
                    || of(&format!("{lane}5FFMUX.IN_B"))
                    || (lane == 'A' && of("PRECYINIT.AX"));
                if !bypass.contains(wire.as_str()) {
                    "unreached"
                } else if another {
                    // A flip-flop of the slice reads it too, so that it is
                    // reached says nothing about this select.
                    "reached"
                } else {
                    "reached:alone"
                }
            };
            if f8 {
                out.push(format!(
                    "f8 I0.I0={} I0.I1={} I1.I0={} I1.I1={} out={} f7b={} f7a={} selects={},{},{}",
                    ones('D'),
                    ones('C'),
                    ones('B'),
                    ones('A'),
                    exit('B', "F8"),
                    exit('C', "F7"),
                    exit('A', "F7"),
                    select(0),
                    select(1),
                    select(2),
                ));
                continue;
            }
            // Which of a slice's two `MUXF7`s holds one is the placer's, and
            // Vivado uses both: what matters is the same on either.
            for (used, i0, i1, lane, column) in [(f7a, 'B', 'A', 'A', 0), (f7b, 'D', 'C', 'C', 2)] {
                if used {
                    out.push(format!(
                        "f7 I0={} I1={} out={} select={}",
                        ones(i0),
                        ones(i1),
                        exit(lane, "F7"),
                        select(column),
                    ));
                }
            }
        }
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

/// `vivado.txt`, by design: what it says of its cells, and its slices.
#[derive(Default)]
struct Answer {
    cells: Vec<String>,
    slices: Vec<String>,
    delays: Vec<String>,
}

fn answers() -> Option<BTreeMap<String, Answer>> {
    let path = fixtures().join("vivado.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut out: BTreeMap<String, Answer> = BTreeMap::new();
    let mut at = String::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("design ") {
            at = name.to_owned();
            out.entry(at.clone()).or_default();
        } else if let Some(cell) = line.strip_prefix("  cell ") {
            out.get_mut(&at).unwrap().cells.push(cell.to_owned());
        } else if let Some(slice) = line.strip_prefix("  slice ") {
            out.get_mut(&at).unwrap().slices.push(slice.to_owned());
        } else if let Some(delay) = line.strip_prefix("  delay ") {
            out.get_mut(&at).unwrap().delays.push(delay.to_owned());
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

/// Every design built once, for the tests here, or `None` when there is
/// nothing to build them with.
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
        .join("muxf")
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
        decoded: decode(root, &bit),
        log,
        routed,
    }
}

/// When `RETICLE_MUXF_BITS` and `UPDATE_EXPECT` are set, `vivado.txt` is made
/// again from `tools/vivado-parity/muxf.sh`'s output: never otherwise, so
/// that nothing here runs Vivado or needs to have.
fn regenerate(root: &str) {
    let (Some(bits), true) = (
        std::env::var_os("RETICLE_MUXF_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
    ) else {
        return;
    };
    let bits = PathBuf::from(bits);
    let mut out = String::from(
        "# What Vivado 2026.1 made of each design here, on xc7a100tftg256-1, from\n\
         # tools/vivado-parity/muxf.sh. `cell` lines are its own report (BEL and pin\n\
         # drivers, from muxf_probe.tcl), with the sites taken off; `slice` lines are\n\
         # its bitstream decoded through this repository's database, one line per\n\
         # wide multiplexer: tests/fpga_xray_muxf.rs says how to read one. `delay`\n\
         # lines are from its report of the worst path.\n",
    );
    for name in designs() {
        out.push_str(&format!("design {name}\n"));
        let dir = bits.join(&name);
        let report = std::fs::read_to_string(dir.join(format!("{name}.cells"))).unwrap();
        for line in report.lines() {
            // Which slice, of which type, and on which flip-flop a select is
            // are the placement's business.
            let mut kept = Vec::new();
            for word in line.split_whitespace() {
                let word = word.split("@SLICE_").next().unwrap_or(word);
                let word = word.replace("@SLICEL.", "@").replace("@SLICEM.", "@");
                let word = word.replace("SLICEL.", "").replace("SLICEM.", "");
                if word.starts_with("SLICE_") {
                    continue;
                }
                kept.push(word);
            }
            let line = kept.join(" ");
            let line = line.split(" loc").next().unwrap_or(&line);
            let line = if line.starts_with("S from") {
                "S from a flip-flop"
            } else {
                line.strip_prefix("cell ").unwrap_or(line)
            };
            out.push_str(&format!("  cell {line}\n"));
        }
        let decoded = decode(root, &dir.join(format!("{name}.bit")));
        for slice in slices(&decoded.features) {
            out.push_str(&format!("  slice {slice}\n"));
        }
        // The multiplexers' own delays on its worst path, from its report.
        let timing = std::fs::read_to_string(dir.join(format!("{name}.timing"))).unwrap();
        for line in timing.lines().filter(|l| l.contains("(Prop_muxf")) {
            let mut words = line.split_whitespace().skip(2);
            if let (Some(arc), Some(delay)) = (words.next(), words.next()) {
                let arc = arc.trim_matches(['(', ')']);
                out.push_str(&format!("  delay {arc} {delay}\n"));
            }
        }
    }
    std::fs::write(fixtures().join("vivado.txt"), out).unwrap();
}

/// The designs Vivado and this flow cover alike, so that their slices must be
/// the same line for line. The others differ for a reason, and
/// [`where_the_cover_differs_it_differs_as_written`] holds them to that.
const SAME_COVER: &[&str] = &[
    "f7_d",
    "f7_fanout",
    "f7_ff",
    "f7_out",
    "f7_two",
    "f8_ff",
    "f8_out",
    "mux16",
    "mux64",
    "mux8",
];

/// **The slices are Vivado's.** See the module docs for what a line says and
/// what this would catch.
#[test]
fn each_slice_holds_what_vivados_does() {
    let Some(root) = chipdb() else { return };
    regenerate(&root);
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    for name in SAME_COVER {
        // Whether a flip-flop shares a select's bypass input is the placer's.
        let plain = |lines: &[String]| -> Vec<String> {
            lines.iter().map(|l| l.replace(":alone", "")).collect()
        };
        let theirs = plain(&answers[*name].slices);
        let ours = plain(&slices(&built[*name].decoded.features));
        assert!(!theirs.is_empty(), "{name} has no slice in the fixture");
        if ours != theirs {
            wrong.push(format!("{name}:\n  ours   {ours:?}\n  Vivado {theirs:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **Each `MUXF7` and `MUXF8` Vivado reports sits as this flow's tile tables
/// have it**: `I0` on the higher letter, in every design that has one. This is
/// the fixture checked against itself, and it is here because `sites.rs` states
/// the rule as a reading of it. Would catch that reading going stale when the
/// fixture is made again, and nothing about this flow.
#[test]
fn vivado_puts_the_higher_letter_on_i0() {
    let Some(answers) = answers() else { return };
    let mut seen = 0;
    for answer in answers.values() {
        for pair in answer.cells.windows(3) {
            let [head, i0, i1] = pair else { continue };
            let Some(bel) = head.split(" bel ").nth(1) else {
                continue;
            };
            let (want0, want1) = match bel {
                "F7AMUX" => ("B6LUT", "A6LUT"),
                "F7BMUX" => ("D6LUT", "C6LUT"),
                "F8MUX" => ("F7BMUX", "F7AMUX"),
                other => panic!("`{other}` is no wide multiplexer"),
            };
            // A flip-flop on a data pin says nothing about which is which.
            if !i0.contains("FDRE") && !i0.contains("FDSE") {
                assert!(i0.ends_with(want0), "{head}: {i0}");
                seen += 1;
            }
            if !i1.contains("FDRE") && !i1.contains("FDSE") {
                assert!(i1.ends_with(want1), "{head}: {i1}");
                seen += 1;
            }
        }
    }
    assert!(seen >= 40, "only {seen} data pins looked at");
}

/// **Vivado's selects are on `AX`, `BX` and `CX`**, which `sites::wide_mux_bels`
/// states and its own test pins. The evidence is a slice whose bypass input
/// nothing else there reads: reached all the same, it can only be the select.
/// This is the fixture checked against itself. Would catch the fixture no longer
/// saying so; `a_slices_wide_multiplexers_are_where_vivado_has_them` is what
/// would catch this flow disagreeing, since a shared bypass input hides a select
/// misplaced from [`each_slice_holds_what_vivados_does`], which is how a `MUXF7`
/// with its select on `DX` got past it when that mutation was tried.
#[test]
fn vivados_selects_are_on_those_bypass_inputs() {
    let Some(answers) = answers() else { return };
    let lines: Vec<&String> = answers.values().flat_map(|a| &a.slices).collect();
    let alone = |prefix: &str, at: usize| {
        lines.iter().any(|l| {
            l.starts_with(prefix)
                && l.rsplit_once('=')
                    .is_some_and(|(_, s)| s.split(',').nth(at) == Some("reached:alone"))
        })
    };
    assert!(alone("f7 ", 0), "no `MUXF7` shows its select: {lines:?}");
    for at in 0..3 {
        assert!(
            alone("f8 ", at),
            "no `MUXF8` slice shows select {at}: {lines:?}"
        );
    }
}

/// **Where the cover is not Vivado's, it is what this says.** Each of these
/// has a reason, and each reason would be silently overturned by a change to
/// the mapper or to `legalise_wide_muxes` if nothing held it.
///
/// | Design | Vivado | Here |
/// |---|---|---|
/// | `f7_from_ff` | `LUT3` | one table |
/// | `f7_half` | `MUXF7`, a table, and a route-through table | two tables |
/// | `f8_from_ff` | `MUXF8` and four route-through tables, selects grounded | one table |
/// | `f8_half` | `MUXF8`, a `MUXF7`, two tables and route-throughs | a `MUXF7`, three tables |
/// | `f7_shared` | the shared table copied, two `MUXF7` | one `MUXF7`, a three-input table |
/// | `mux32` | nine tables and four `MUXF7` | nine tables, four `MUXF7`, two `MUXF8` |
/// | `tie` | no `MUXF7` either, four tables, with a `LUT5` over the two: logic it was not told was a cell | the same |
/// | `f7_pair` | a `MUXF7` over `LUT3`s, the unread half of each table holding something else, so a count of ones is no comparison | a `MUXF7` over narrow tables |
/// | `f7_carry` | keeps the `MUXF7`, a table feeding it and a carry's propagate | a table for the `MUXF7`, as the placer holds a carry lane's table to it |
///
/// In every row the table count is Vivado's or lower, sites counted as
/// `vivado.txt`'s are; route-through tables are sites too.
#[test]
fn where_the_cover_differs_it_differs_as_written() {
    let Some(built) = built() else { return };
    let cells = |name: &str, cell: &str| {
        built[name]
            .log
            .lines()
            .find_map(|l| {
                let (count, what) = l.trim().split_once(" x ")?;
                (what == cell).then(|| count.parse::<u32>().ok())?
            })
            .unwrap_or(0)
    };
    for (name, luts, f7, f8) in [
        ("f7_from_ff", 1, 0, 0),
        ("f7_half", 2, 0, 0),
        ("f8_from_ff", 1, 0, 0),
        ("f8_half", 3, 1, 0),
        ("f7_shared", 5, 1, 0),
        ("mux32", 9, 4, 2),
        ("f7_carry", 12, 0, 0),
        ("f7_pair", 6, 1, 0),
        ("tie", 4, 0, 0),
    ] {
        let got = (
            cells(name, "LUT6"),
            cells(name, "MUXF7"),
            cells(name, "MUXF8"),
        );
        assert_eq!(got, (luts, f7, f8), "{name}: {}", built[name].log);
    }
}

/// **A multiplexer's delay is one Vivado reports for one, on a connection of no
/// delay at all.** Vivado's routed reports (`delay` lines in `vivado.txt`) give
/// each `MUXF7` and `MUXF8` it crossed, and this flow's report of `mux64`'s
/// latest path has a step for each of its own. Would catch either missing from
/// the timing model, which leaves it counted as a register that ends the path,
/// the two figures exchanged, and a connection into one timed as a wire when
/// it has none. It would not catch the rest of that path's estimate being
/// wrong: a table's figure and a wire's are what the module docs of
/// `xray::timing` say they are, and they are not Vivado's.
#[test]
fn a_wide_multiplexer_is_timed_with_vivados_figures() {
    let Some(root) = chipdb() else { return };
    let Some(answers) = answers() else { return };
    // `f8_from_ff`'s `MUXF8` carries the route-through under it in its figure.
    let range = |arc: &str| -> (f64, f64) {
        let figures: Vec<f64> = answers
            .iter()
            .filter(|(name, _)| !name.ends_with("_from_ff") && !name.ends_with("_half"))
            .flat_map(|(_, a)| &a.delays)
            .filter_map(|d| {
                let (which, delay) = d.split_once(' ')?;
                (which.starts_with(arc) && !which.ends_with("_S_O")).then(|| delay.parse().ok())?
            })
            .collect();
        assert!(figures.len() > 2, "{arc}: {figures:?}");
        let low = figures.iter().copied().fold(f64::MAX, f64::min);
        let high = figures.iter().copied().fold(0.0, f64::max);
        (low, high)
    };
    let (f7, f8) = (range("Prop_muxf7"), range("Prop_muxf8"));
    let dir = fixtures();
    let mut crossed = BTreeSet::new();
    // Which of them a latest path crosses depends on whether a select or a
    // table starts it, so more than one design is asked.
    for name in ["mux64", "mux16", "f8_ff", "f7_ff"] {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("muxf-timed")
            .join(name);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        // A latest path is only reported late, so it is made late, and the
        // pads, which would be the latest, are taken out of it.
        let rcf = std::fs::read_to_string(dir.join(format!("{name}.rcf")))
            .unwrap()
            .replace("-period 10", "-period 1")
            + "set_false_path -to so\nset_false_path -from si\n";
        let constraints = out.join("late.rcf");
        std::fs::write(&constraints, rcf).unwrap();
        let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args(["fpga", "--device", DEVICE, "--chipdb", &root, "--top", name])
            .args(["--timing", "--timing-driven"])
            .arg("--constraints")
            .arg(&constraints)
            .arg("--output-dir")
            .arg(&out)
            .arg("--bitstream")
            .arg(out.join(format!("{name}.bit")))
            .arg(dir.join(format!("{name}.v")))
            .output()
            .expect("the reticle binary runs");
        let log = String::from_utf8_lossy(&run.stderr);
        assert!(run.status.success(), "{log}");
        let path: Vec<(f64, &str)> = log
            .lines()
            .skip_while(|l| !l.contains("latest path"))
            .skip(1)
            .take_while(|l| l.starts_with("  "))
            .filter_map(|l| {
                let (ns, rest) = l.trim().split_once(" ns  ")?;
                Some((ns.parse().ok()?, rest))
            })
            .collect();
        let netlist = std::fs::read_to_string(out.join(format!("{name}.v"))).unwrap();
        for pair in path.windows(2) {
            let ((before, _), (after, what)) = (pair[0], pair[1]);
            let step = after - before;
            let cell = what.split('.').next().unwrap_or(what);
            let kind = ["MUXF7", "MUXF8"].into_iter().find(|kind| {
                netlist.contains(&format!("{kind} {cell}"))
                    || netlist.contains(&format!("{kind} \\{cell} "))
            });
            let Some(kind) = kind else { continue };
            if what.ends_with(".o") {
                let (low, high) = if kind == "MUXF7" { f7 } else { f8 };
                assert!(
                    step > low - 0.011 && step < high + 0.011,
                    "{cell} takes {step:.3} ns, and Vivado's are {low} to {high}:\n{log}"
                );
                crossed.insert(kind);
            } else if !what.contains(".s ") {
                assert!(step.abs() < 0.005, "{what} is {step:.3} ns of wire:\n{log}");
            }
        }
    }
    assert_eq!(
        crossed.len(),
        2,
        "both kinds were not on a path: {crossed:?}"
    );
}

/// **Every bit decodes, and the arcs are the router's.** `reticle fpga` checks
/// the second itself and refuses to finish without it, which `build_one` would
/// report; this reads its verdicts and the decode's count, for designs chosen to
/// have every exit of every wide multiplexer. Would catch a hop whose bits
/// are in nothing the decoder knows, and a hop with the router's choice and no
/// bits at all, as `<L>FFMUX.F7` would be were its feature misspelled, since
/// the loader drops such a hop and the route would fail.
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
            one.log
                .contains("the interconnect arcs those bits select are exactly the ones"),
            "{name}: {}",
            one.log
        );
    }
}

/// **`reticle fpga --verify` proves each.** Would catch a wide network mapped
/// from a graph that lost something when its trees were taken out and put
/// back, since the proof is against the graph from before; would not catch a
/// named `MUXF7`, which is a cut in that proof and is simulated below.
#[test]
fn the_mapping_of_each_design_is_proved() {
    let Some(built) = built() else { return };
    for (name, one) in built {
        assert!(
            one.log.contains("mapping: equivalent"),
            "{name}: {}",
            one.log
        );
    }
}

/// **The netlist the bitstream is made from answers as the RTL does**, cycle by
/// cycle, over a pseudo-random input long enough to walk the selects through
/// their values. See the module docs. The first cycles are not compared: in
/// RTL a register starts unknown, in a flip-flop at its `INIT`.
#[test]
fn a_routed_netlist_answers_as_its_rtl_does() {
    let Some(built) = built() else { return };
    // The cell library for what names a cell of it, and the simulation
    // models for the buffers, which it has no reason to hold.
    let cells = manifest().join("src/fpga/devices/xc7_cells.v");
    let sim = manifest().join("examples/husky/sim");
    let results: Vec<(String, bool, String)> = std::thread::scope(|scope| {
        let handles: Vec<_> = built
            .iter()
            .map(|(name, one)| {
                let (cells, sim) = (&cells, &sim);
                scope.spawn(move || {
                    let dir = one.routed.parent().unwrap();
                    let text = std::fs::read_to_string(&one.routed).unwrap();
                    // Its top alone: what flattening inlined is written too,
                    // and a library cell's copy would be declared twice.
                    let at = text
                        .find(&format!("module {name} "))
                        .expect("the routed netlist's top is named for the design");
                    let end = text[at..].find("endmodule").expect("it ends") + at;
                    let top = text[at..end].replacen(
                        &format!("module {name} "),
                        &format!("module {name}_routed "),
                        1,
                    );
                    let netlist = dir.join("routed_top.v");
                    std::fs::write(&netlist, format!("{top}endmodule\n")).unwrap();
                    let tb = dir.join("tb.v");
                    std::fs::write(&tb, testbench(name)).unwrap();
                    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
                        .args(["sim", "--quiet", "--top", "tb"])
                        .arg(&tb)
                        .arg(fixtures().join(format!("{name}.v")))
                        .arg(&netlist)
                        .arg(cells)
                        .arg(sim.join("xilinx_primitives.v"))
                        .output()
                        .expect("the reticle binary runs");
                    let out = String::from_utf8_lossy(&run.stdout).into_owned();
                    let err = String::from_utf8_lossy(&run.stderr).into_owned();
                    (
                        name.clone(),
                        run.status.success() && out.contains("PASS:"),
                        format!("{out}\n{err}"),
                    )
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let wrong: Vec<String> = results
        .iter()
        .filter(|(_, ok, _)| !ok)
        .map(|(name, _, log)| format!("{name}: {log}"))
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Both the RTL and the routed netlist under one input: they must agree on
/// every cycle after the registers have all been written.
fn testbench(name: &str) -> String {
    format!(
        "module tb;\n\
         \x20 reg clk = 0;\n\
         \x20 reg si = 0;\n\
         \x20 reg [15:0] lfsr = 16'hACE1;\n\
         \x20 wire a, b;\n\
         \x20 {name} rtl (.clk(clk), .si(si), .so(a));\n\
         \x20 {name}_routed routed (.clk(clk), .si(si), .so(b));\n\
         \x20 integer cycle;\n\
         \x20 integer seen1 = 0;\n\
         \x20 integer seen0 = 0;\n\
         \x20 initial begin\n\
         \x20   for (cycle = 0; cycle < 3000; cycle = cycle + 1) begin\n\
         \x20     #5 clk = 1;\n\
         \x20     #1 if (cycle > 100) begin\n\
         \x20       if (a !== b) begin\n\
         \x20         $display(\"FAIL: cycle %0d rtl %b routed %b\", cycle, a, b);\n\
         \x20         $finish;\n\
         \x20       end\n\
         \x20       if (a === 1'b1) seen1 = seen1 + 1;\n\
         \x20       if (a === 1'b0) seen0 = seen0 + 1;\n\
         \x20     end\n\
         \x20     #4 clk = 0;\n\
         \x20     lfsr = {{lfsr[14:0], lfsr[15] ^ lfsr[13] ^ lfsr[12] ^ lfsr[10]}};\n\
         \x20     si = lfsr[0];\n\
         \x20   end\n\
         \x20   if (seen1 == 0 || seen0 == 0) $display(\"FAIL: the output never moved\");\n\
         \x20   else $display(\"PASS: %0d ones, %0d zeros\", seen1, seen0);\n\
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

/// **A design that saves nothing is the design it was, to the bit**: `--wide-muxes`
/// on and off give one bitstream. `gray_cdc` has no choice between tables to
/// offer, and `tie` has one where a `MUXF7` would leave as many tables as a wider
/// table over the two does, which is also what Vivado makes of it. This holds
/// the promise made to any build measured on a board: inference does not move a
/// design it has nothing to offer. Would catch a tie going to the wide network
/// (`tie`), and anything else the option reached beyond the mapping. It would
/// not catch the fabric's own new sites and wires moving a placement, since both
/// builds have those; that was measured against the commit before them, and
/// `docs/fpga-vivado-parity.md` says on what.
#[test]
fn a_design_that_saves_nothing_is_not_moved() {
    let Some(root) = chipdb() else { return };
    let qor = manifest().join("testdata/fpga/vivado/qor");
    if !qor.join("gray_cdc.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    for (dir, name) in [(qor, "gray_cdc"), (fixtures(), "tie")] {
        let build = |flag: &str| -> Vec<u8> {
            let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
                .join("muxf-nothing")
                .join(format!("{name}-{flag}"));
            let _ = std::fs::remove_dir_all(&out);
            std::fs::create_dir_all(&out).unwrap();
            let bit = out.join(format!("{name}.bit"));
            let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
                .args(["fpga", "--device", DEVICE, "--chipdb", &root, "--report"])
                .args(["--top", name, "--wide-muxes", flag])
                .arg("--constraints")
                .arg(dir.join(format!("{name}.rcf")))
                .arg("--output-dir")
                .arg(&out)
                .arg("--bitstream")
                .arg(&bit)
                .arg(dir.join(format!("{name}.v")))
                .output()
                .expect("the reticle binary runs");
            let log = String::from_utf8_lossy(&run.stderr);
            assert!(run.status.success(), "{log}");
            assert!(!log.contains(" x MUXF"), "{name}: {log}");
            std::fs::read(&bit).unwrap()
        };
        assert!(build("on") == build("off"), "{name}: two bitstreams");
    }
}
