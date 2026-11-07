//! Eight flip-flops in a slice, through a lookup table, held to what Vivado
//! wrote for them.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/ffpack/` has designs that hold eight flip-flops fed
//! from elsewhere to one slice (`tools/vivado-parity/gen_ffpack.py`), with the
//! slice's tables free or holding a function of two, four or five inputs, the
//! flip-flops of every reset kind, and two that Vivado cannot hold there (one
//! flip-flop's enable or clock not the others'). `ffpack.sh` ran them through
//! Vivado 2026.1, and `vivado.txt` is each held slice of its bitstreams decoded
//! through this repository's database, a line for the slice's own features and
//! one a lane ([`lanes`] says what a line holds).
//!
//! A slice has four bypass inputs for its eight flip-flops, so four reach
//! theirs through a lookup table: `<L>FFMUX.O6` with the table `FFFFFFFF00000000`
//! (its `A6` passed on) when the table holds nothing, `<L>FFMUX.O5` with the
//! lower half passing one input on under the table's own function when it holds
//! one of four inputs or fewer, and `<L>5FF` on the bypass in both. That is
//! `xray::thru`'s reading, and [`vivados_slices_are_as_thru_reads_them`] holds
//! the fixture to it.
//!
//! `free16` and `lut2x16` are built here too: sixteen flip-flops in one tile
//! (Vivado by `LOC`, this flow by a region, since it cannot fix a cell to a
//! site), and in the second a table of two inputs on each of the tile's eight
//! lanes. Sixteen in a tile is eight in each slice, which neither tool can do
//! without a way through a table for half of them. The second is built with
//! `--route-throughs all`, since the way beside a table's function is not this
//! flow's default (`xray::RouteThroughMode`), and the first with the default.
//!
//! # What this would and would not catch
//!
//! [`the_tiles_both_tools_fill_are_vivados`] compares the tile's lanes and
//! slices with Vivado's. It would catch the way through a table missing (the
//! design does not fit its region and the build fails), a way from another
//! input than Vivado's or into the other half, the five-input flip-flop sent
//! through a table where Vivado has it on the bypass, a way's bits written on
//! top of a table's own function's lower half, and a flip-flop's initial or
//! reset value lost to the move. It runs each build with `--verify`, so a
//! netlist the bitstream was made from that does not do what the source does
//! fails too. It would not catch which lane holds which flip-flop, which is
//! the placer's, nor which input of `A1` to `A5` a way beside a table takes,
//! the router's, nor anything on a part: nothing here has been on a board.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli", feature = "sim"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use reticle::fpga::xc7;
use reticle::fpga::xray::{Decoded, XrayDatabase, XrayOptions};
use reticle::ir::memfile::FileProvider;

const DEVICE: &str = "xc7a100t-ftg256";

/// The tile both tools fill, and its two slices.
const TILE: &str = "CLBLL_L_X4Y50";
const BOTH: [&str; 2] = ["free16", "lut2x16"];

/// A slice as [`lanes`] writes it: its own features, and a line a lane.
type Slice = (String, Vec<String>);

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/ffpack")
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

fn decode(root: &str, bit: &Path) -> Decoded {
    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let read = xc7::read_bit(&std::fs::read(bit).unwrap()).unwrap();
    let frames = xc7::FrameData::from_stream(part.layout.clone(), read.frames).unwrap();
    db.decode(&DiskFiles, &frames).unwrap()
}

/// Whether a truth table reads input `k`.
fn reads(init: u64, k: u32) -> bool {
    (0..64u32)
        .filter(|i| i & (1 << k) == 0)
        .any(|i| (init >> i & 1) != (init >> (i | 1 << k) & 1))
}

/// What a lane's table holds, in words: `-` for nothing, `through6` for
/// `A6` passed on to `O6` (`FFFFFFFF00000000`), `through5+<n>` for a lower
/// half passing one of `A1` to `A5` on under an upper half reading `n`
/// inputs, and `fn<n>` for any other function of `n` inputs. Which input a
/// way takes beside a function is left out: it is the router's choice among
/// the ones the function leaves free.
fn table(init: u64) -> String {
    if init == 0 {
        return "-".to_owned();
    }
    if init == 0xFFFF_FFFF_0000_0000 {
        return "through6".to_owned();
    }
    let lower = init & 0xFFFF_FFFF;
    for k in 0..5u32 {
        let pass = (0..32u32)
            .filter(|i| i & (1 << k) != 0)
            .fold(0u64, |a, i| a | 1 << i);
        if lower == pass {
            let upper = init >> 32;
            let both = upper | upper << 32;
            let n = (0..5).filter(|j| reads(both, *j)).count();
            return format!("through5+{n}");
        }
    }
    format!("fn{}", (0..6).filter(|j| reads(init, *j)).count())
}

/// The slice whose features start `prefix` in `tile`, as one line of its own
/// features and one a lane: `ff=` and `ff5=` the value of the two flip-flops'
/// data muxes, `out=` the output mux's, `lut=` the table ([`table`]), and
/// `init=` which of the two have `ZINI` and `ZRST` set. A lane's letter is
/// written `L`, and the slice's own features are those of no lane.
fn lanes(features: &[(String, String)], tile: &str, prefix: &str) -> (String, Vec<String>) {
    let here: Vec<&str> = features
        .iter()
        .filter(|(t, _)| t == tile)
        .filter_map(|(_, f)| f.strip_prefix(prefix)?.strip_prefix('.'))
        .collect();
    let mut own: Vec<&str> = here.clone();
    let mut out = Vec::new();
    for letter in ['A', 'B', 'C', 'D'] {
        let of = |head: &str| -> Vec<&str> {
            here.iter()
                .filter_map(|f| f.strip_prefix(&format!("{letter}{head}")))
                .collect()
        };
        let one = |head: &str| -> String {
            let v = of(head);
            if v.is_empty() {
                return "-".to_owned();
            }
            // A value naming its own lane (`AX`, `A5Q`) names it `L`.
            v.iter()
                .map(|value| match value.strip_prefix(letter) {
                    Some(rest) => format!("L{rest}"),
                    None => (*value).to_owned(),
                })
                .collect::<Vec<_>>()
                .join("|")
        };
        let mut init = 0u64;
        for bit in of("LUT.INIT[") {
            if let Ok(i) = bit.trim_end_matches(']').parse::<u32>() {
                init |= 1 << i;
            }
        }
        let mut flags = Vec::new();
        for (head, name) in [("FF.", "ff"), ("5FF.", "ff5")] {
            for flag in of(head) {
                flags.push(format!("{name}.{}", flag.to_lowercase()));
            }
        }
        flags.sort();
        out.push(format!(
            "ff={} ff5={} out={} lut={} init={}",
            one("FFMUX."),
            one("5FFMUX."),
            one("OUTMUX."),
            table(init),
            if flags.is_empty() {
                "-".to_owned()
            } else {
                flags.join(",")
            }
        ));
        own.retain(|f| {
            !["FF.", "5FF.", "FFMUX.", "5FFMUX.", "OUTMUX.", "LUT."]
                .iter()
                .any(|h| f.starts_with(&format!("{letter}{h}")))
        });
    }
    own.sort_unstable();
    (own.join(" "), out)
}

/// The prefix of a slice's features from its site name and type in a tile:
/// the site of the lower `X` is a tile's `X0`.
fn prefix_of(site: &str, kind: &str) -> String {
    let x: u32 = site
        .strip_prefix("SLICE_X")
        .and_then(|r| r.split('Y').next())
        .and_then(|x| x.parse().ok())
        .unwrap_or(0);
    format!("{kind}_X{}", x % 2)
}

/// `vivado.txt`: each design's slices, as `(own features, lanes)`.
fn answers() -> Option<BTreeMap<String, Vec<Slice>>> {
    let Ok(text) = std::fs::read_to_string(fixtures().join("vivado.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut out: BTreeMap<String, Vec<Slice>> = BTreeMap::new();
    let mut at = String::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("design ") {
            at = name.to_owned();
            out.entry(at.clone()).or_default();
        } else if let Some(own) = line.strip_prefix("  slice ") {
            let own = own.split_once(' ').map_or("", |(_, rest)| rest);
            out.get_mut(&at).unwrap().push((own.to_owned(), Vec::new()));
        } else if let Some(lane) = line.strip_prefix("    lane ")
            && let Some(slice) = out.get_mut(&at).unwrap().last_mut()
        {
            slice.1.push(lane.to_owned());
        }
    }
    Some(out)
}

/// When `RETICLE_FFPACK_BITS` and `UPDATE_EXPECT` are set, `vivado.txt` is made
/// again from `tools/vivado-parity/ffpack.sh`'s output; never otherwise, so
/// that nothing here runs Vivado or needs to have.
fn regenerate(root: &str) {
    let (Some(bits), true) = (
        std::env::var_os("RETICLE_FFPACK_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
    ) else {
        return;
    };
    let bits = PathBuf::from(bits);
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .into_string()
                .ok()?
                .strip_suffix(".v")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    let mut out = String::from(
        "# What Vivado 2026.1 made of each design here, on xc7a100tftg256-3, from\n\
         # tools/vivado-parity/ffpack.sh: each slice holding a held flip-flop (`r*`),\n\
         # its bitstream decoded through this repository's database. `slice` is the\n\
         # site's type and its features of no lane; `lane` lines are its four lanes,\n\
         # A to D. tests/fpga_xray_ffpack_vivado.rs says how to read one.\n",
    );
    for name in names {
        let dir = bits.join(&name);
        let cells = std::fs::read_to_string(dir.join(format!("{name}.cells"))).unwrap();
        out.push_str(&format!("design {name}\n"));
        if let Some(refused) = cells.lines().find(|l| l.starts_with("refused")) {
            out.push_str(&format!("  {refused}\n"));
            continue;
        }
        let decoded = decode(root, &dir.join(format!("{name}.bit")));
        for line in cells.lines().filter(|l| l.starts_with("site ")) {
            let words: Vec<&str> = line.split_whitespace().collect();
            let (site, kind, tile) = (words[1], words[2], words[3]);
            let (own, lanes) = lanes(&decoded.features, tile, &prefix_of(site, kind));
            out.push_str(&format!("  slice {kind} {own}\n"));
            for lane in lanes {
                out.push_str(&format!("    lane {lane}\n"));
            }
        }
    }
    std::fs::write(fixtures().join("vivado.txt"), out).unwrap();
}

/// **Vivado's slices are as `xray::thru` reads them.** In every slice of the
/// fixture, a flip-flop that reads its table's `O6` beside a table no cell
/// holds reads `A6` passed on (`through6`); one that reads `O5` reads a lower
/// half that passes one input on under a function of four inputs or fewer; a
/// five-input flip-flop beside either is on the bypass; and a slice of eight is
/// eight that way. This is the fixture checked against itself, and it is here
/// because `xray::thru` states those rules as a reading of it: it would catch
/// that reading going stale when the fixture is made again, and nothing about
/// this flow.
#[test]
fn vivados_slices_are_as_thru_reads_them() {
    let Some(root) = chipdb() else { return };
    regenerate(&root);
    let Some(answers) = answers() else { return };
    let mut through = 0usize;
    for (name, slices) in &answers {
        for (own, lanes) in slices {
            for lane in lanes {
                let field = |key: &str| {
                    lane.split_whitespace()
                        .find_map(|w| w.strip_prefix(&format!("{key}=")))
                        .unwrap_or("-")
                };
                let (ff, ff5, lut) = (field("ff"), field("ff5"), field("lut"));
                match ff {
                    "O6" if !lut.starts_with("fn") => {
                        assert_eq!(lut, "through6", "{name}: {lane}");
                        through += 1;
                    }
                    "O5" => {
                        let n: usize = lut
                            .strip_prefix("through5+")
                            .and_then(|n| n.parse().ok())
                            .unwrap_or_else(|| panic!("{name}: {lane}"));
                        assert!(n <= 4, "{name}: {lane}");
                        through += 1;
                    }
                    _ => {}
                }
                if lut.starts_with("through") {
                    assert!(ff5 == "IN_B" || ff5 == "-", "{name}: {lane}");
                }
            }
            assert!(!own.contains("LATCH"), "{name}: {own}");
        }
    }
    assert!(through >= 40, "{through} lanes through a table");
}

/// One of `BOTH` through the flow, and its tile's slices.
fn build(root: &str, name: &str) -> Vec<Slice> {
    let dir = fixtures();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("ffpack")
        .join(name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join(format!("{name}.bit"));
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--chipdb", root, "--top", name])
        .arg("--constraints")
        .arg(dir.join(format!("{name}.rcf")))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .args(["--verify", "--report"])
        // A way beside a table's own function is not the default: see
        // `xray::RouteThroughMode`.
        .args([
            "--route-throughs",
            if name == "lut2x16" { "all" } else { "empty" },
        ])
        .arg(dir.join(format!("{name}.v")))
        .output()
        .expect("the reticle binary runs");
    let log = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(run.status.success(), "{name}: {log}");
    assert!(!log.contains("NOT FULLY ROUTED"), "{name}: {log}");
    assert!(
        log.contains("decodes to exactly what was placed and routed"),
        "{name}: {log}"
    );
    let decoded = decode(root, &bit);
    assert_eq!(decoded.unexplained, 0, "{name}");
    ["SLICEL_X0", "SLICEL_X1"]
        .iter()
        .map(|prefix| lanes(&decoded.features, TILE, prefix))
        .collect()
}

/// A tile's slices as what a comparison may ask of them: each slice's own
/// features, and every lane of the tile, sorted, since which lane and which
/// slice of the tile a flip-flop is in is the placer's.
fn as_compared(slices: &[Slice]) -> (BTreeSet<String>, Vec<String>) {
    let own = slices.iter().map(|(own, _)| own.clone()).collect();
    let mut lanes: Vec<String> = slices.iter().flat_map(|(_, l)| l.clone()).collect();
    lanes.sort();
    (own, lanes)
}

/// **The tile both tools fill holds what Vivado's does**, lane for lane. See
/// the module docs for what this would and would not catch.
#[test]
fn the_tiles_both_tools_fill_are_vivados() {
    let Some(root) = chipdb() else { return };
    let Some(answers) = answers() else { return };
    let built: Vec<(&str, Vec<Slice>)> = std::thread::scope(|scope| {
        let root = &root;
        let handles: Vec<_> = BOTH
            .iter()
            .map(|name| scope.spawn(move || (*name, build(root, name))))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut wrong = Vec::new();
    for (name, ours) in &built {
        let theirs = &answers[*name];
        assert_eq!(theirs.len(), 2, "{name}: Vivado's two slices");
        let (ours, theirs) = (as_compared(ours), as_compared(theirs));
        if ours != theirs {
            wrong.push(format!("{name}:\n  ours   {ours:?}\n  Vivado {theirs:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
