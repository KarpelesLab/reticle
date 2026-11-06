//! Each primitive probe, on each part it was measured on, held to what
//! Vivado's own bitstream of it sets in the tiles its constraints pin.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/primitives/` has one design per 7-series primitive
//! (`tools/vivado-parity/gen_primitives.py`), with constraints for the
//! XC7A100T's FTG256, which the XC7A35T's FTG256 shares ball for ball, and a
//! set of its own for the XC7A35T's CPG236 in `xc7a35t-cpg236/`. Vivado 2026.1
//! built each one on each part (`run_all.sh`), and `pads.txt` beside each
//! part's ledger is what its bitstream sets in every pad tile, IO logic tile,
//! bank clock tile, configuration tile and the XADC's, reduced by
//! `tools/vivado-parity/probe_pads.py` to what does not depend on where either
//! tool put the logic: an IO logic tile's pips as the wires they drive, a bank
//! clock tile's pips not at all, nor its position or which of its regional
//! buffers was taken.
//!
//! # What these would and would not catch
//!
//! [`the_pinned_tiles_are_vivados`] builds every probe of the groups that
//! configure those tiles (`io`, `iologic`, `config`, `clock`) on all three
//! parts and compares the reduction, line for line. It would catch a pad's
//! standard, drive, slew, pull or termination bit wrong or missing, an IO
//! logic setting wrong, an XADC or configuration primitive's bit, and a pin of
//! an IO logic site that Vivado reaches and this flow leaves unrouted: before
//! 2026-11-06 an `ISERDESE2`'s `CLKB` and `OCLK` were left so, with `CLK` on
//! the global network, because only `CLK` moved onto the buffer. Since a pin
//! reached from the interconnect reads as reached, it also asks that an IO logic
//! site's clock pins all arrive on one kind of road, which an `OSERDESE2`'s did
//! not (`CLKDIV` from the interconnect beside `CLK` from the network, which
//! Vivado refuses as `REQP-1975`); and an XADC
//! with no `INIT_42` lost the library's `16'h0800`. And of every probe it
//! builds it asks that each flip-flop of a slice in use has its data mux
//! chosen, which every choice of `<L>FFMUX` and `<L>5FFMUX` does with a bit
//! (checked against the database's `segbits_clbl*`): an `IDELAYCTRL` no delay
//! needs is taken away and its `RDY` is a one, and until 2026-11-06 the
//! flip-flop reading it took a constant `D` after mapping had built every
//! constant's driver, and was written with neither input of its mux chosen,
//! where Vivado chose the bypass input. A probe built not fully routed fails it
//! too: an XADC's `CONVST` was, on every part, while the fabric loaded around an
//! XADC stopped at its tile's own row. It would not catch a wrong
//! bit anywhere else, which is `run_placed.sh`'s comparison of the same
//! implementation (`docs/fpga-vivado-parity.md`), nor what any of it does on a
//! part: **nothing here has been on a board**.
//!
//! One difference is known and allowed, and said where it is: Vivado drives
//! an output register's `OCLKM` from the clock that drives its `OCLK`, and this
//! flow leaves it, on an `ODDR`, an `OSERDESE2` and an `ISERDESE2`'s `OCLK`
//! alike. Closing it changes every build with an output register, the
//! ChipWhisperer-Husky's among them.
//!
//! [`every_probe_has_the_pinned_tiles_vivado_wrote`] does the same for every
//! probe, and is ignored for its time. Skips without the Project X-Ray
//! database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Every part, the directory of its ledger and `pads.txt`, and the package
/// file whose presence says the database can build for it.
const PARTS: [(&str, &str, &str); 3] = [
    (
        "xc7a100t-ftg256",
        "",
        "artix7/xc7a100tftg256-1/package_pins.csv",
    ),
    (
        "xc7a35t-ftg256",
        "xc7a35t-ftg256",
        "artix7/xc7a35tftg256-1/package_pins.csv",
    ),
    (
        "xc7a35t-cpg236",
        "xc7a35t-cpg236",
        "artix7/xc7a35tcpg236-1/package_pins.csv",
    ),
];

/// The groups of the ledger whose primitives configure a pinned tile.
const PINNED_GROUPS: [&str; 4] = ["io", "iologic", "config", "clock"];

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn primitives() -> PathBuf {
    manifest().join("testdata/fpga/vivado/primitives")
}

/// The database root, or `None` with a line saying what is missing. The same
/// search as `tests/fpga_xray.rs`.
fn chipdb(probe: &str) -> Option<String> {
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
                "skipped: needs a Project X-Ray database with `{probe}`; run \
                 `reticle fetch prjxray-db` or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

/// A part's ledger: `(group, primitive, status)`.
fn ledger(part: &str) -> Option<Vec<(String, String, String)>> {
    let text = std::fs::read_to_string(primitives().join(part).join("index.txt")).ok()?;
    Some(
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut words = l.split_whitespace();
                Some((
                    words.next()?.to_owned(),
                    words.next()?.to_owned(),
                    words.next()?.to_owned(),
                ))
            })
            .collect(),
    )
}

/// `pads.txt` of a part: each design's lines.
fn vivados(part: &str) -> Option<BTreeMap<String, Vec<String>>> {
    let text = std::fs::read_to_string(primitives().join(part).join("pads.txt")).ok()?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut at = String::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("design ") {
            at = name.to_owned();
            out.entry(at.clone()).or_default();
        } else if let Some(feature) = line.strip_prefix("  ") {
            out.entry(at.clone()).or_default().push(feature.to_owned());
        }
    }
    Some(out)
}

/// Whether a tile name is one of the pinned tiles' kinds.
fn pinned(tile: &str) -> bool {
    [
        "LIOB33",
        "RIOB33",
        "LIOI3",
        "RIOI3",
        "HCLK_IOI3",
        "CFG_CENTER",
        "MONITOR_BOT",
    ]
    .iter()
    .any(|kind| tile.starts_with(kind))
}

/// A tile's kind: its name without the trailing `_X<x>Y<y>`.
fn kind_of(tile: &str) -> &str {
    tile.rsplit_once("_X")
        .filter(|(_, at)| {
            at.split_once('Y').is_some_and(|(x, y)| {
                !x.is_empty()
                    && !y.is_empty()
                    && x.bytes().all(|b| b.is_ascii_digit())
                    && y.bytes().all(|b| b.is_ascii_digit())
            })
        })
        .map_or(tile, |(kind, _)| kind)
}

/// Whether a feature is a pip of one of those tiles: `<wire>.<wire>`, each
/// in capitals, digits and underscores, the first in the tile's own words.
fn is_pip(feature: &str) -> bool {
    let Some((to, from)) = feature.split_once('.') else {
        return false;
    };
    let word = |w: &str| {
        !w.is_empty()
            && w.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    };
    [
        "HCLK_IOI_",
        "IOI_",
        "LIOI_",
        "RIOI_",
        "LIOB_",
        "RIOB_",
        "CFG_CENTER_",
        "MONITOR_",
    ]
    .iter()
    .any(|p| to.starts_with(p))
        && word(to)
        && word(from)
}

/// Every run of digits written `#`.
fn numbers_hidden(text: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push('#');
            }
            digits = true;
        } else {
            out.push(c);
            digits = false;
        }
    }
    out
}

/// What `--decode` printed, reduced as `tools/vivado-parity/probe_pads.py`
/// reduces it, sorted.
fn reduce(decoded: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in decoded.lines() {
        if line.is_empty() || line.starts_with('!') || line.starts_with("decoded") {
            continue;
        }
        let Some((tile, feature)) = line.split_once(' ') else {
            continue;
        };
        if tile == "?" {
            if pinned(feature) {
                out.push(line.to_owned());
            }
            continue;
        }
        if !pinned(tile) {
            continue;
        }
        let kind = kind_of(tile);
        if kind == "HCLK_IOI3" {
            if is_pip(feature) {
                continue;
            }
            let mut feature = feature.to_owned();
            for buffer in ["BUFR_Y", "BUFIO_Y"] {
                if let Some(rest) = feature.strip_prefix(buffer) {
                    let tail = rest.trim_start_matches(|c: char| c.is_ascii_digit());
                    feature = format!("{buffer}#{tail}");
                }
            }
            out.push(format!("{kind} {feature}"));
        } else if is_pip(feature) {
            let to = feature.split_once('.').map_or(feature, |(to, _)| to);
            out.push(format!("{tile} {} driven", numbers_hidden(to)));
        } else {
            out.push(format!("{tile} {feature}"));
        }
    }
    out.sort();
    out
}

/// Every flip-flop of a slice that a bitstream configures and whose data mux
/// it does not: `<tile> <slice>.<lane>FF` (or `5FF`) with a feature of its
/// own and none of `<lane>FFMUX` (`5FFMUX`). Every choice of either mux sets a
/// bit, so a flip-flop with none reads no input that was chosen.
fn unselected_flip_flops(decoded: &str) -> Vec<String> {
    let mut used: BTreeMap<String, bool> = BTreeMap::new();
    for line in decoded.lines() {
        let Some((tile, feature)) = line.split_once(' ') else {
            continue;
        };
        if !tile.starts_with("CLBL") {
            continue;
        }
        let Some((slice, rest)) = feature.split_once('.') else {
            continue;
        };
        let Some((element, _)) = rest.split_once('.') else {
            continue;
        };
        let (ff, mux) = if let Some(lane) = element.strip_suffix("FFMUX") {
            (format!("{lane}FF"), true)
        } else if element.ends_with("FF") && element.len() <= 4 {
            (element.to_owned(), false)
        } else {
            continue;
        };
        let key = format!("{tile} {slice}.{ff}");
        let entry = used.entry(key).or_insert(false);
        *entry |= mux;
    }
    used.into_iter()
        .filter(|(_, mux)| !mux)
        .map(|(ff, _)| ff)
        .collect()
}

/// Every IO logic site whose clock pins are reached on two kinds of road,
/// some from a clock leaf and some from the interconnect: Vivado refuses an
/// `OSERDESE2` whose `CLK` and `CLKDIV` arrive so (`REQP-1975`), and builds
/// none. `<tile> <site index>` for each.
fn mixed_clock_roads(decoded: &str) -> Vec<String> {
    let mut roads: BTreeMap<String, std::collections::BTreeSet<&str>> = BTreeMap::new();
    for line in decoded.lines() {
        let Some((tile, feature)) = line.split_once(' ') else {
            continue;
        };
        if !(tile.starts_with("LIOI3") || tile.starts_with("RIOI3")) || !is_pip(feature) {
            continue;
        }
        let Some((to, from)) = feature.split_once('.') else {
            continue;
        };
        // `IOI_ILOGIC1_CLKB`, `IOI_OLOGIC0_CLKDIV`, `IOI_OCLK_1`, `IOI_OCLKM_0`.
        let site = if let Some(rest) = to
            .strip_prefix("IOI_ILOGIC")
            .or_else(|| to.strip_prefix("IOI_OLOGIC"))
        {
            let (index, pin) = rest.split_once('_').unwrap_or((rest, ""));
            if !pin.starts_with("CLK") {
                continue;
            }
            index.to_owned()
        } else if let Some(index) = to
            .strip_prefix("IOI_OCLKM_")
            .or_else(|| to.strip_prefix("IOI_OCLK_"))
        {
            index.to_owned()
        } else {
            continue;
        };
        let road = if from.contains("GCLK") || from.contains("IOCLK") || from.contains("RCLK") {
            "clock"
        } else {
            "interconnect"
        };
        roads
            .entry(format!("{tile} {site}"))
            .or_default()
            .insert(road);
    }
    roads
        .into_iter()
        .filter(|(_, kinds)| kinds.len() > 1)
        .map(|(site, _)| site)
        .collect()
}

/// Builds one probe for one part and reduces its bitstream, or says why not.
fn reticles(root: &str, device: &str, part: &str, top: &str) -> Result<Vec<String>, String> {
    let dir = primitives();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("pads-{device}-{top}"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let own = dir.join(part).join(format!("{top}.rcf"));
    let rcf = if own.is_file() {
        own
    } else {
        dir.join(format!("{top}.rcf"))
    };
    let bit = out.join(format!("{top}.bit"));
    // Every regional buffer on a site of its own, as Vivado has it: the
    // default keeps a plain `BUFR` and a `BUFMRCE` before it on global
    // buffers, for a reason the Husky Plus's build gives (`xray::regional`).
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", device, "--chipdb", root])
        .args(["--regional-clocks", "on", "--top", top, "--constraints"])
        .arg(rcf)
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join(format!("{top}.v")))
        .output()
        .expect("the reticle binary runs");
    let stderr = String::from_utf8_lossy(&run.stderr);
    // A design that is not fully routed still writes a bitstream, of less than
    // it asks for: the XADC's `CONVST` was one, on every part, until the
    // fabric loaded around an XADC reached the rows its pins enter by.
    if !run.status.success() || stderr.contains("NOT FULLY ROUTED") {
        return Err(stderr.into_owned());
    }
    let decoded = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", device, "--chipdb", root, "--decode"])
        .arg(&bit)
        .output()
        .expect("the reticle binary runs");
    let decoded = String::from_utf8_lossy(&decoded.stdout);
    let unselected = unselected_flip_flops(&decoded);
    if !unselected.is_empty() {
        return Err(format!(
            "flip-flops with no data input chosen: {}",
            unselected.join(", ")
        ));
    }
    let mixed = mixed_clock_roads(&decoded);
    if !mixed.is_empty() {
        return Err(format!(
            "IO logic with its clocks on two kinds of road: {}",
            mixed.join(", ")
        ));
    }
    Ok(reduce(&decoded))
}

/// The lines Vivado has that this flow is known not to, and why: see the
/// module documentation.
fn known(line: &str) -> bool {
    line.ends_with(" IOI_OCLKM_# driven")
}

/// The differences on every probe of `groups` (all of them when `None`), on
/// every part that has its fixtures and its database.
fn differences(groups: Option<&[&str]>) -> Vec<String> {
    let mut work: Vec<(String, &str, &str, String)> = Vec::new();
    let mut wrong: Vec<String> = Vec::new();
    for (device, part, probe) in PARTS {
        let (Some(ledger), Some(vivado)) = (ledger(part), vivados(part)) else {
            eprintln!("skipped: `testdata/` is not in the published crate");
            return Vec::new();
        };
        let Some(root) = chipdb(probe) else { continue };
        for (group, primitive, status) in ledger {
            if status != "ok" || groups.is_some_and(|g| !g.contains(&group.as_str())) {
                continue;
            }
            let top = format!("{}_probe", primitive.to_ascii_lowercase());
            if !vivado.contains_key(&top) {
                wrong.push(format!("{device} {top}: no line in pads.txt"));
                continue;
            }
            work.push((root.clone(), device, part, top));
        }
    }
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let found = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((root, device, part, top)) = work.get(at) else {
                        break;
                    };
                    let mut here = Vec::new();
                    match reticles(root, device, part, top) {
                        Err(log) => here.push(format!("{device} {top}: did not build\n{log}")),
                        Ok(ours) => {
                            let theirs = vivados(part)
                                .and_then(|v| v.get(top).cloned())
                                .unwrap_or_default();
                            let mut left = ours.clone();
                            for line in theirs.iter().filter(|l| !known(l)) {
                                match left.iter().position(|l| l == line) {
                                    Some(i) => {
                                        left.remove(i);
                                    }
                                    None => {
                                        here.push(format!("{device} {top}: Vivado only: {line}"))
                                    }
                                }
                            }
                            here.extend(
                                left.iter()
                                    .map(|line| format!("{device} {top}: Reticle only: {line}")),
                            );
                        }
                    }
                    found.lock().unwrap().extend(here);
                }
            });
        }
    });
    wrong.extend(found.into_inner().unwrap());
    wrong.sort();
    wrong
}

/// **Every probe of the pinned tiles' groups sets what Vivado set there**, on
/// all three parts. See the module documentation for what it would catch.
#[test]
fn the_pinned_tiles_are_vivados() {
    let wrong = differences(Some(&PINNED_GROUPS));
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The same of every probe of the ledger, a few hundred builds.
#[test]
#[ignore = "builds every probe of three parts to a bitstream"]
fn every_probe_has_the_pinned_tiles_vivado_wrote() {
    let wrong = differences(None);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **A flip-flop is unselected only with no mux bit**: one with `<L>FFMUX`
/// or `<L>5FFMUX` is not reported, one without is, and a slice's other
/// elements (`FFSYNC`, a table) are not flip-flops. Would catch the five-input
/// half folded into the main one's name, which hid `idelayctrl_probe`'s.
#[test]
fn a_flip_flop_with_no_data_mux_is_found() {
    let decoded = "\
CLBLM_L_X36Y47 SLICEL_X1.AFF.ZINI
CLBLM_L_X36Y47 SLICEL_X1.AFFMUX.O6
CLBLM_L_X36Y47 SLICEL_X1.B5FF.ZINI
CLBLM_L_X36Y47 SLICEL_X1.B5FF.ZRST
CLBLM_L_X36Y47 SLICEL_X1.BOUTMUX.B5Q
CLBLM_L_X36Y47 SLICEL_X1.DFF.ZINI
CLBLM_L_X36Y47 SLICEL_X1.DFFMUX.DX
CLBLM_L_X36Y47 SLICEL_X1.FFSYNC
CLBLM_L_X36Y47 SLICEL_X1.ALUT.INIT[01]
";
    assert_eq!(
        unselected_flip_flops(decoded),
        ["CLBLM_L_X36Y47 SLICEL_X1.B5FF"]
    );
}

/// **A site's clocks on two kinds of road are found**, and one site's on one
/// kind is not, whichever. Would catch the class read off the wrong end of the
/// pip, or `OCLK` not taken for a clock pin, which is how an `ISERDESE2` with
/// `CLK` on the network and `CLKB` and `OCLK` on the interconnect passed.
#[test]
fn a_sites_clocks_on_two_roads_are_found() {
    let decoded = "\
RIOI3_X43Y45 IOI_ILOGIC1_CLK.IOI_LEAF_GCLK0
RIOI3_X43Y45 IOI_ILOGIC1_CLKB.IOI_IMUX20_0
RIOI3_X43Y45 IOI_OCLK_1.IOI_IMUX31_0
RIOI3_X43Y45 IOI_ILOGIC1_D.IOI_IMUX1_0
RIOI3_X43Y47 IOI_ILOGIC0_CLK.IOI_IMUX3_1
RIOI3_X43Y47 IOI_OCLK_0.IOI_IMUX4_1
LIOI3_X0Y23 IOI_OLOGIC0_CLKDIV.IOI_LEAF_GCLK2
LIOI3_X0Y23 IOI_OCLK_0.IOI_LEAF_GCLK2
";
    assert_eq!(mixed_clock_roads(decoded), ["RIOI3_X43Y45 1"]);
}

/// **The reduction is `probe_pads.py`'s**, on lines of each kind it treats
/// apart. Would catch a pip of an IO logic tile kept with its numbers (which
/// would make every route a difference), a bank clock tile's position kept, a
/// pad's feature mistaken for a pip and its standard lost, or a tile of
/// another kind let in.
#[test]
fn the_reduction_is_the_fixtures() {
    let decoded = "\
decoded: something
RIOI3_X43Y45 IOI_ILOGIC1_CLKB.IOI_LEAF_GCLK5
RIOI3_X43Y45 ILOGIC_Y1.ZINV_D
RIOB33_X43Y45 IOB_Y1.PULLTYPE.NONE
HCLK_IOI3_X113Y26 HCLK_IOI_CK_IGCLK11.HCLK_IOI_CK_BUFHCLK11
HCLK_IOI3_X113Y26 BUFR_Y2.IN_USE
HCLK_IOI3_X113Y26 VREF.V_900_MV
MONITOR_BOT_X46Y79 XADC.INIT_42[11]
CLBLM_L_X36Y47 SLICEL_X1.AFF.ZINI
INT_L_X36Y47 IMUX_L10.SR1BEG_S0
? MONITOR_BOT_X46Y79 26_1928
? INT_L_X18Y79 26_40
! 0x0000091d ecc 0x0000 wants 0x0a15
";
    assert_eq!(
        reduce(decoded),
        [
            "? MONITOR_BOT_X46Y79 26_1928",
            "HCLK_IOI3 BUFR_Y#.IN_USE",
            "HCLK_IOI3 VREF.V_900_MV",
            "MONITOR_BOT_X46Y79 XADC.INIT_42[11]",
            "RIOB33_X43Y45 IOB_Y1.PULLTYPE.NONE",
            "RIOI3_X43Y45 ILOGIC_Y1.ZINV_D",
            "RIOI3_X43Y45 IOI_ILOGIC#_CLKB driven",
        ]
    );
}
