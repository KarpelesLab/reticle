//! A pad's IO standard and its attributes, held to what Vivado wrote and what
//! it refused.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/io/` has what `tools/vivado-parity/gen_io.py` writes:
//! a design per standard and direction with a pad per setting, and a design per
//! question about a bank. Vivado 2026.1 built each to a bitstream or refused it
//! (`run_all.sh`), and `vivado.txt` is each bitstream decoded through this
//! repository's database, or the rule that refused it. `pairs.txt` is its design
//! rule check on every pair of (standard, direction) in one bank. This regenerates
//! `vivado.txt` when `RETICLE_IO_BITS` names `run_all.sh`'s output and
//! `UPDATE_EXPECT` is set.
//!
//! An answer is one line a pad: everything set on its half of its IO tile and of
//! the IO logic beside it, and what is set on the tile for a pair. A bank is one
//! line: its clock row's features, and how many of its pads have `STEPDOWN`,
//! which is every one or none. And a line for each pad tile's and clock row's
//! **raw bits**, set against none of those names: a name this flow gave a group of
//! bits decodes to itself whatever its bits are, so without these a wrong one
//! would agree with Vivado's answer. Interconnect is left out, and is the router's.
//!
//! # What these would and would not catch
//!
//! [`every_recipe_is_what_vivado_wrote`] would catch any feature of any pad wrong
//! or missing, in any setting of any standard, on either half, without building
//! anything. It would not catch a bank's features, a `_SING` tile, or a pad the
//! flow never reached, which [`what_reticle_writes_is_what_vivado_wrote`] does
//! on the designs it builds, since it compares the whole bitstream's decoding.
//! That one in the gate builds a few, and `every_design_builds_to_vivados_bits`,
//! which is ignored, builds them all.
//!
//! [`the_bank_rules_are_vivados_on_every_pair`] would catch a supply or a reference
//! wrong in any standard in any direction. It says nothing of a third pad, which
//! `DIFFSTDLIMIT-1` needs, and [`reticle_refuses_what_vivado_refuses`] has.
//!
//! None of this has been near a part. That these bits drive a pad at that
//! standard rests on Vivado's having written them.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use reticle::fpga::arch::ConfigBit;
use reticle::fpga::xc7;
use reticle::fpga::xc7::pads::{Pad, Way, check_pads, standard};
use reticle::fpga::xray::{Decoded, PadRequest, XrayDatabase, XrayOptions, pad_features};
use reticle::ir::memfile::FileProvider;
use reticle::source::{SourceMap, Span};

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
    manifest().join("testdata/fpga/vivado/io")
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

/// A tile's place, without its type: `X57Y115`.
fn place(tile: &str) -> &str {
    tile.rsplit_once('_').map_or(tile, |(_, at)| at)
}

fn is_pad_tile(tile: &str) -> bool {
    tile.starts_with("LIOB33") || tile.starts_with("RIOB33")
}

fn is_logic_tile(tile: &str) -> bool {
    tile.starts_with("LIOI3") || tile.starts_with("RIOI3")
}

/// What a bitstream says of pads and banks, as lines an answer can be compared
/// with: `pad <place> <half> <features>` and `bank <clock row> ... stepdown=<n>`.
fn answer_of(decoded: &Decoded) -> Vec<String> {
    let mut halves: BTreeMap<(String, char), BTreeSet<String>> = BTreeMap::new();
    let mut tile_level: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut banks: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut stepdown = 0usize;
    for (tile, feature) in &decoded.features {
        if tile.starts_with("HCLK_IOI") {
            if !feature.contains("BUFIO")
                && !feature.contains("BUFR")
                && !feature.starts_with("HCLK_IOI")
                && !feature.contains("IDELAYCTRL")
                && (feature == "STEPDOWN"
                    || feature.starts_with("VREF.")
                    || feature.ends_with("_IN_USE"))
            {
                banks
                    .entry(place(tile).to_owned())
                    .or_default()
                    .insert(feature.clone());
            }
            continue;
        }
        if is_pad_tile(tile) {
            let at = place(tile).to_owned();
            if feature.contains(".STEPDOWN") {
                stepdown += 1;
            } else if let Some(rest) = feature.strip_prefix("IOB_Y") {
                let half = rest.chars().next().unwrap_or('?');
                halves
                    .entry((at, half))
                    .or_default()
                    .insert(rest[2..].to_owned());
            } else {
                tile_level.entry(at).or_default().insert(feature.clone());
            }
        } else if is_logic_tile(tile) {
            for (prefix, name) in [
                ("ILOGIC_Y", "ILOGIC"),
                ("OLOGIC_Y", "OLOGIC"),
                ("IDELAY_Y", "IDELAY"),
            ] {
                if let Some(rest) = feature.strip_prefix(prefix) {
                    let half = rest.chars().next().unwrap_or('?');
                    halves
                        .entry((place(tile).to_owned(), half))
                        .or_default()
                        .insert(format!("{name}.{}", &rest[2..]));
                }
            }
        }
    }
    for (tile, bit) in &decoded.leftovers {
        if is_pad_tile(tile) || tile.starts_with("HCLK_IOI") {
            tile_level
                .entry(place(tile).to_owned())
                .or_default()
                .insert(format!("?{:02}_{:02}", bit.row, bit.col));
        }
    }
    let mut out = Vec::new();
    for ((at, half), features) in &halves {
        let tile: Vec<String> = tile_level
            .get(at)
            .into_iter()
            .flatten()
            .map(|f| format!("T:{f}"))
            .collect();
        let line: Vec<&str> = features
            .iter()
            .map(String::as_str)
            .chain(tile.iter().map(String::as_str))
            .collect();
        out.push(format!("pad {at} {half} {}", line.join(" ")));
    }
    for (at, features) in &banks {
        let line: Vec<&str> = features.iter().map(String::as_str).collect();
        out.push(format!("bank {at} {}", line.join(" ")));
    }
    out.push(format!("stepdown {stepdown}"));
    out.sort();
    out
}

/// Each pad tile's and clock row's set bits, by frame and bit, leaving out a
/// `STEPDOWN` that is all a tile has, which `stepdown` in the answer counts.
fn raw_of(db: &XrayDatabase, frames: &xc7::FrameData) -> Vec<String> {
    let tiles = db.tiles(&DiskFiles).unwrap();
    let mut out = Vec::new();
    for tile in tiles
        .iter()
        .filter(|t| is_pad_tile(&t.name) || t.name.starts_with("HCLK_IOI"))
    {
        let Some((_, window)) = tile.bits.first() else {
            continue;
        };
        let mut set = Vec::new();
        // Frames 38 up: below those it is the interconnect's and the IO logic's
        // in the same window.
        for row in 38..window.frames {
            for col in 0..window.words * 32 {
                let bit = ConfigBit::new(row, col);
                let Some((address, word, index)) = window.locate(bit) else {
                    continue;
                };
                if frames.get(address, word, index).unwrap_or(false) {
                    set.push(format!("{row:02}_{col:02}"));
                }
            }
        }
        // `STEPDOWN` is a bit a half and on every pad of a bank, so a tile with
        // nothing else says nothing. Any pad in use has more than that.
        if set.is_empty() || (is_pad_tile(&tile.name) && set.len() <= 2) {
            continue;
        }
        out.push(format!("raw {} {}", place(&tile.name), set.join(" ")));
    }
    out
}

/// The answer of a `.bit`: its decoding, and its raw bits.
fn decode(root: &str, bit: &Path) -> Vec<String> {
    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let read = xc7::read_bit(&std::fs::read(bit).unwrap()).unwrap();
    let frames = read.image(part.layout.clone()).unwrap();
    let decoded: Decoded = db.decode(&DiskFiles, &frames).unwrap();
    let mut out = answer_of(&decoded);
    out.extend(raw_of(&db, &frames));
    out.sort();
    out
}

/// One design in `vivado.txt`: refused by a rule, or these lines.
struct Answer {
    refused: Option<String>,
    lines: Vec<String>,
}

fn answers() -> Option<BTreeMap<String, Answer>> {
    let Ok(text) = std::fs::read_to_string(fixtures().join("vivado.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut signatures: BTreeMap<&str, &str> = BTreeMap::new();
    let mut out: BTreeMap<String, Answer> = BTreeMap::new();
    let mut at = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("sig ") {
            let (n, features) = rest.split_once(' ').unwrap_or((rest, ""));
            signatures.insert(n, features);
        } else if let Some(rest) = line.strip_prefix("design ") {
            let mut words = rest.split_whitespace();
            at = words.next().unwrap_or("").to_owned();
            let refused =
                (words.next() == Some("refused")).then(|| words.collect::<Vec<_>>().join(" "));
            out.insert(
                at.clone(),
                Answer {
                    refused,
                    lines: Vec::new(),
                },
            );
        } else if let Some(rest) = line.strip_prefix("  ") {
            let answer = out.get_mut(&at).unwrap();
            if let Some(pad) = rest.strip_prefix("pad ") {
                // `pad <place> <half> <ball> <signature>`
                let w: Vec<&str> = pad.split(' ').collect();
                answer
                    .lines
                    .push(format!("pad {} {} {}", w[0], w[1], signatures[w[3]]));
            } else if let Some(raw) = rest.strip_prefix("raw ") {
                let (place, n) = raw.split_once(' ').unwrap_or((raw, ""));
                answer.lines.push(format!("raw {place} {}", signatures[n]));
            } else {
                answer.lines.push(rest.to_owned());
            }
        }
    }
    Some(out)
}

/// The ball, from `package_pins.csv`, as `(place, half) -> ball`.
fn balls(root: &str) -> BTreeMap<(String, char), String> {
    let path = format!("{root}/artix7/xc7a100tftg256-1/package_pins.csv");
    let text = std::fs::read_to_string(path).unwrap();
    let mut out = BTreeMap::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        let Some((ball, tile, function)) = f
            .first()
            .zip(f.get(3))
            .zip(f.get(4))
            .map(|((b, t), func)| (*b, *t, *func))
        else {
            continue;
        };
        let half = if function.contains("IO_0_") || function.contains("IO_25_") {
            // A `_SING` tile's lone pad, whichever name the database gives it.
            if function.contains("IO_0_") { '1' } else { '0' }
        } else if function
            .split('_')
            .any(|w| w.ends_with('P') && w.starts_with('L'))
        {
            '0'
        } else {
            '1'
        };
        out.insert((place(tile).to_owned(), half), ball.to_owned());
    }
    out
}

/// When `RETICLE_IO_BITS` and `UPDATE_EXPECT` are set, `vivado.txt` is made from
/// `run_all.sh`'s output. The rule that refused a design is the first error of
/// its log that names one.
#[test]
fn regenerate_when_asked() {
    let (Some(out), true, Some(root)) = (
        std::env::var_os("RETICLE_IO_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
        chipdb(),
    ) else {
        return;
    };
    let out = PathBuf::from(out);
    let balls = balls(&root);
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_suffix(".v").map(str::to_owned)
        })
        .collect();
    names.sort();
    let mut signatures: BTreeMap<String, usize> = BTreeMap::new();
    let mut body = String::new();
    for name in &names {
        let bit = out.join(name).join(format!("{name}.bit"));
        if !bit.is_file() {
            let log = std::fs::read_to_string(out.join(format!("{name}.log"))).unwrap_or_default();
            let rule = log
                .lines()
                .filter(|l| l.starts_with("ERROR: ["))
                .find_map(|l| {
                    let rest = l.strip_prefix("ERROR: [")?;
                    let rule = rest.split(']').next()?;
                    (!rule.starts_with("Vivado_Tcl") && !rule.starts_with("Vivado 12-1345"))
                        .then(|| rule.strip_prefix("DRC ").unwrap_or(rule).to_owned())
                })
                .unwrap_or_else(|| "?".to_owned());
            body.push_str(&format!("design {name} refused {rule}\n"));
            continue;
        }
        body.push_str(&format!("design {name} built\n"));
        for line in decode(&root, &bit) {
            if let Some(raw) = line.strip_prefix("raw ") {
                let (at, bits) = raw.split_once(' ').unwrap_or((raw, ""));
                let n = signatures.len();
                let n = *signatures.entry(bits.to_owned()).or_insert(n);
                body.push_str(&format!("  raw {at} {n}\n"));
            } else if let Some(pad) = line.strip_prefix("pad ") {
                let mut w = pad.splitn(3, ' ');
                let (at, half, features) = (
                    w.next().unwrap_or(""),
                    w.next().unwrap_or(""),
                    w.next().unwrap_or(""),
                );
                let n = signatures.len();
                let n = *signatures.entry(features.to_owned()).or_insert(n);
                let ball = balls
                    .get(&(at.to_owned(), half.chars().next().unwrap_or('?')))
                    .map_or("-", String::as_str);
                body.push_str(&format!("  pad {at} {half} {ball} {n}\n"));
            } else {
                body.push_str(&format!("  {line}\n"));
            }
        }
    }
    let mut table: Vec<(usize, &String)> = signatures.iter().map(|(s, n)| (*n, s)).collect();
    table.sort();
    let mut text = String::from(
        "# Vivado 2026.1 on testdata/fpga/vivado/io, xc7a100tftg256-1, by\n\
         # tools/vivado-parity/run_all.sh, and decoded by tests/fpga_xray_pads.rs.\n",
    );
    for (n, s) in table {
        text.push_str(&format!("sig {n} {s}\n"));
    }
    text.push_str(&body);
    std::fs::write(fixtures().join("vivado.txt"), text).unwrap();
}

/// A pad's line of the sweep, by ball: what its setting asks.
struct Asked {
    design: String,
    ball: String,
    standard: String,
    kind: String,
    setting: String,
    /// The other half of a pair.
    second: bool,
}

fn index() -> Vec<Asked> {
    let text = std::fs::read_to_string(fixtures().join("index.txt")).unwrap_or_default();
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let w: Vec<&str> = l.split(' ').collect();
            Some(Asked {
                design: (*w.first()?).to_owned(),
                second: w.get(1)?.ends_with('n'),
                ball: (*w.get(2)?).to_owned(),
                standard: (*w.get(3)?).to_owned(),
                kind: (*w.get(4)?).to_owned(),
                setting: (*w.get(5)?).to_owned(),
            })
        })
        .collect()
}

fn request_of(asked: &Asked) -> PadRequest {
    let mut request = PadRequest {
        standard: asked.standard.clone(),
        way: match asked.kind.as_str() {
            "in" | "din" => Way::In,
            "inout" | "dinout" => Way::InOut,
            _ => Way::Out,
        },
        ..PadRequest::default()
    };
    for item in asked.setting.split(',') {
        let Some((key, value)) = item.split_once('=') else {
            continue;
        };
        match key {
            "DRIVE" => request.drive = value.parse().ok(),
            "SLEW" => request.fast = value == "FAST",
            "PULLTYPE" => request.pull = Some(value.to_owned()),
            "IBUF_LOW_PWR" => request.low_power = value != "FALSE",
            "IN_TERM" => request.in_term = Some(value.to_owned()).filter(|t| t != "NONE"),
            "DIFF_TERM" => request.diff_term = value == "TRUE",
            other => panic!("nothing reads `{other}`"),
        }
    }
    request
}

/// The features of a pad's half in a line, with what names no bit taken out,
/// since a decoding cannot see those.
fn seen(features: Vec<String>) -> BTreeSet<String> {
    features
        .into_iter()
        .filter(|f| {
            !f.starts_with("OLOGIC.")
                && !f.starts_with("ILOGIC.")
                && !f.starts_with("IDELAY.")
                && f != "PULLTYPE.PULLDOWN"
                && !f.ends_with("LVCMOS33_LVTTL.SLEW.FAST")
                && f != "IN_TERM.NONE"
        })
        .collect()
}

/// Every pad of the whole sweep, in `recipe`'s words and Vivado's. Needs no
/// database, so it runs anywhere; see the module's documentation.
#[test]
fn every_recipe_is_what_vivado_wrote() {
    let Ok(text) = std::fs::read_to_string(fixtures().join("vivado.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    };
    let mut signatures: BTreeMap<&str, &str> = BTreeMap::new();
    let mut pads: BTreeMap<(String, String), &str> = BTreeMap::new();
    let mut at = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("sig ") {
            let (n, features) = rest.split_once(' ').unwrap_or((rest, ""));
            signatures.insert(n, features);
        } else if let Some(rest) = line.strip_prefix("design ") {
            at = rest.split(' ').next().unwrap_or("").to_owned();
        } else if let Some(pad) = line.strip_prefix("  pad ") {
            let w: Vec<&str> = pad.split(' ').collect();
            pads.insert((at.clone(), w[2].to_owned()), signatures[w[3]]);
        }
    }
    let mut checked = 0usize;
    let mut wrong = Vec::new();
    for asked in index() {
        let Some(features) = pads.get(&(asked.design.clone(), asked.ball.clone())) else {
            // The other ball of a true differential input with its own setting
            // has an answer too; a pad with none has no bits at all.
            wrong.push(format!(
                "{} {}: Vivado has nothing there",
                asked.design, asked.ball
            ));
            continue;
        };
        let vivado = seen(features.split_whitespace().map(str::to_owned).collect());
        let recipe = pad_features(&request_of(&asked)).unwrap();
        let half = if asked.second && recipe.2 {
            recipe.1
        } else {
            recipe.0
        };
        let tile = recipe.3.iter().map(|t| format!("T:{t}"));
        let ours = seen(half.into_iter().chain(tile).collect());
        checked += 1;
        if ours != vivado {
            wrong.push(format!(
                "{} {} ({} {} {}): Vivado {:?}, recipe {:?}",
                asked.design,
                asked.ball,
                asked.standard,
                asked.kind,
                asked.setting,
                vivado.difference(&ours).collect::<Vec<_>>(),
                ours.difference(&vivado).collect::<Vec<_>>()
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {checked}:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    assert!(checked > 1000, "only {checked}");
}

/// Vivado's design rule check on every pair in one bank, by `check_pads`.
#[test]
fn the_bank_rules_are_vivados_on_every_pair() {
    let Ok(text) = std::fs::read_to_string(fixtures().join("pairs.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    };
    let mut map = SourceMap::new();
    let file = map.add("x.xdc", "").unwrap();
    let span = Span::new(file, 0, 0);
    let items: Vec<(&str, &str)> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.contains(':'))
        .filter_map(|l| l.split_once(' '))
        .collect();
    let pad = |item: (&str, &str), port: &str, ball: &str| {
        let (name, kind) = item;
        Pad {
            port: port.to_owned(),
            ball: ball.to_owned(),
            bank: "35".to_owned(),
            vref_ball: false,
            standard: name.to_owned(),
            way: match kind {
                "in" | "din" | "dinT" => Way::In,
                "out" | "dout" => Way::Out,
                _ => Way::InOut,
            },
            pair: standard(name).unwrap().differential,
            in_term: None,
            diff_term: kind == "dinT",
            ibufdisable: None,
            span,
        }
    };
    let mut checked = 0usize;
    let mut wrong = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && l.contains(": "))
    {
        let (first, row) = line.split_once(": ").unwrap();
        let first: usize = first.parse().unwrap();
        for (second, code) in row.chars().enumerate() {
            let want: &[&str] = match code {
                '.' => &[],
                'C' => &["BIVC-1"],
                'R' => &["BIVR-1"],
                'B' => &["BIVC-1", "BIVR-1"],
                other => panic!("`{other}`"),
            };
            let pads = [pad(items[first], "a", "G2"), pad(items[second], "b", "H2")];
            let got: Vec<&str> = check_pads(&pads, &[]).iter().map(|r| r.rule).collect();
            checked += 1;
            if got != want {
                wrong.push(format!(
                    "{:?} with {:?}: Vivado {want:?}, here {got:?}",
                    items[first], items[second]
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {checked}:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    assert_eq!(checked, items.len() * items.len());
    assert!(checked > 10_000);
}

/// One design through the flow, from its `.xdc`: the bitstream decoded, or the
/// log of a refusal.
fn build(name: &str, root: &str) -> Result<Vec<String>, String> {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("io").join(name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let dir = fixtures();
    let bit = out.join(format!("{name}.bit"));
    let built = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--quiet", "--offline"])
        .arg("--chipdb")
        .arg(root)
        .args(["--top", name, "--constraints"])
        .arg(dir.join(format!("{name}.xdc")))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join(format!("{name}.v")))
        .output()
        .expect("failed to run the reticle binary");
    if !bit.is_file() {
        return Err(String::from_utf8_lossy(&built.stderr).into_owned());
    }
    Ok(decode(root, &bit))
}

/// One design against Vivado's answer for it.
fn compare(name: &str, answer: &Answer, root: &str) -> Result<(), String> {
    let got = build(name, root);
    match (&answer.refused, got) {
        (Some(rule), Ok(_)) => Err(format!(
            "{name}: Vivado refuses it ({rule}), and this built"
        )),
        (Some(_), Err(_)) => Ok(()),
        (None, Err(log)) => Err(format!("{name}: Vivado built it, and this says: {log}")),
        (None, Ok(lines)) => {
            let ours: BTreeSet<&String> = lines.iter().collect();
            let known: Vec<String> = answer
                .lines
                .iter()
                .map(|line| {
                    KNOWN
                        .iter()
                        .filter(|(design, _)| *design == name)
                        .fold(line.clone(), |line, (_, feature)| {
                            line.replace(&format!(" {feature}"), "")
                        })
                })
                .collect();
            let theirs: BTreeSet<&String> = known.iter().collect();
            if ours == theirs {
                Ok(())
            } else {
                Err(format!(
                    "{name}: Vivado only {:#?}\nhere only {:#?}",
                    theirs.difference(&ours).collect::<Vec<_>>(),
                    ours.difference(&theirs).collect::<Vec<_>>()
                ))
            }
        }
    }
}

/// A few designs in the gate: each kind of standard in each direction, the bank
/// features, and a `_SING` tile. See the module's documentation for the rest.
#[test]
fn what_reticle_writes_is_what_vivado_wrote() {
    let (Some(root), Some(answers)) = (chipdb(), answers()) else {
        return;
    };
    let names = [
        "i_sstl15",
        "i_pci33_3",
        "o_pci33_3",
        "o_hstl_ii_18",
        "o_lvcmos18",
        "b_lvttl",
        "di_lvds_25",
        "di_tmds_33",
        "do_rsds_25",
        "do_mini_lvds_25",
        "do_blvds_25",
        "bias_lvds_ppds",
        "bias_mini_ppds",
        "sing_hstl",
        "left_rsds",
        "vref_ball_free",
        "unread_sstl",
        "bank14_stepdown",
        // A tristate tied high, released: what a folded `z` would drive.
        "buf_obuft_t1",
        "buf_iobuf_t1",
        "buf_obuftds_t1",
        "buf_ibufds_intermdisable_lvds_25",
        "buf_keeper_inout",
        // The second half's own receiver read, alone and beside the first,
        // and a pair both ways whose `TS` Vivado leaves unrouted.
        "buf_ibufds_diff_out_ob_lvds_25",
        "buf_ibufds_diff_out_tmds_33",
        "buf_iobufds_diff_out_diff_sstl15",
    ];
    let wrong = in_parallel(&names, 8, |name| compare(name, &answers[*name], &root));
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Runs `job` over `items`, `jobs` at a time, and gathers the errors. Each build
/// is a process of its own, so this is only waiting.
fn in_parallel<T: Sync>(
    items: &[T],
    jobs: usize,
    job: impl Fn(&T) -> Result<(), String> + Sync,
) -> Vec<String> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let wrong = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                while let Some(item) =
                    items.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                {
                    if let Err(e) = job(item) {
                        wrong.lock().unwrap().push(e);
                    }
                }
            });
        }
    });
    wrong.into_inner().unwrap()
}

/// Every design Vivado built, to the same decoding; and every one it refused,
/// refused. Takes some time, so it is ignored.
#[test]
#[ignore = "builds every design to a bitstream, and needs the Project X-Ray database"]
fn every_design_builds_to_vivados_bits() {
    let (Some(root), Some(answers)) = (chipdb(), answers()) else {
        return;
    };
    let wanted = std::env::var("RETICLE_IO_ONLY").unwrap_or_default();
    let queue: Vec<(&String, &Answer)> = answers
        .iter()
        .filter(|(name, _)| name.contains(&wanted))
        .filter(|(name, _)| !PENDING.iter().any(|p| name.starts_with(p)))
        .filter(|(name, _)| !LOOSER.contains(&name.as_str()))
        .filter(|(name, _)| !STRICTER.iter().any(|(design, _)| design == name))
        // `NSTD-1`: Vivado asks for a stated standard, and this defaults to
        // `LVCMOS33` as it has always done, which the document says.
        .filter(|(name, _)| *name != "no_standard")
        .collect();
    let jobs = std::env::var("RETICLE_IO_JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .unwrap_or(8usize);
    let wrong = in_parallel(&queue, jobs, |(name, answer)| compare(name, answer, &root));
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// What needs a buffer this flow does not build yet.
/// `t_`, `dt_`, `db_`: sweeps of a tristate whose enable is a port, compared
/// apart in `buf_*`. `buf_iobuf_params`: the cell library's `IOBUF` takes no
/// parameters, and is refused them. The `DIFF_OUT` forms were here until they
/// were built, and all 44 agree.
const PENDING: [&str; 6] = [
    "t_",
    "dt_",
    "db_",
    "bad_bidir_pair",
    "bias_lvds_io",
    "buf_iobuf_params",
];

/// What Vivado writes and Reticle does not, known and why, as a feature taken
/// out of Vivado's answer for that design and nothing else. A buffer's tristate
/// tied low: Vivado routes a ground to `T1` and sets `ZINV_T1`; Reticle leaves
/// `T1` unrouted, which reads a one, and its inverter on, as a plain output has.
/// Both drive the pad. This would not catch a design whose `T1` Vivado inverts
/// for another reason, which is why it names designs and not a feature.
const KNOWN: [(&str, &str); 4] = [
    ("buf_obuft_t0", "OLOGIC.ZINV_T1"),
    ("buf_obuftds_t0", "OLOGIC.ZINV_T1"),
    ("buf_iobuf_t0", "OLOGIC.ZINV_T1"),
    ("buf_iobufds_t0", "OLOGIC.ZINV_T1"),
];

/// What Vivado refuses and Reticle builds, each with why. A `PULLUP` or
/// `PULLDOWN` alone on an `inout`: Vivado refuses these (`Place 30-389`, no IO
/// buffer) and builds `pullup_probe` and `pulldown_probe` of
/// `testdata/fpga/vivado/primitives/`, the same thing beside a clocked design,
/// as an output with the pull and its data unconnected. Reticle builds both as
/// that, and the probes' bits agree.
const LOOSER: [&str; 2] = ["buf_pullup_inout", "buf_pulldown_inout"];

/// What Vivado takes and Reticle refuses, on purpose, each with its reason.
const STRICTER: [(&str, &str); 7] = [
    // A critical warning and the pad as it was, which a build should not ask to
    // be read in a log.
    ("bad_drive", "does not offer a drive strength"),
    ("bad_term", "not a valid value for input termination"),
    ("low_power_on_output", "is not a valid property"),
    // A warning there.
    ("bad_slew", "does not offer the slew rate"),
    // Vivado moves the `_P` port to the `_N` ball's pair and says nothing.
    ("not_a_pair", "are not a pair"),
    // Vivado takes the first ball's standard and says nothing.
    ("pair_mixed", "two IO standards"),
    // A critical warning, and then the first balls it can.
    ("pair_backwards", "`_N` ball"),
];

/// Each of Vivado's refusals, by name where this flow has the rule. Would catch a
/// rule moved to a later stage that lets something through, not a message's
/// wording, beyond the name. Each is a real build up to the refusal.
#[test]
fn reticle_refuses_what_vivado_refuses() {
    let (Some(root), Some(answers)) = (chipdb(), answers()) else {
        return;
    };
    let mut wrong = Vec::new();
    for (name, answer) in &answers {
        if PENDING.iter().any(|p| name.starts_with(p)) || LOOSER.contains(&name.as_str()) {
            continue;
        }
        let Some(rule) = &answer.refused else {
            continue;
        };
        // `UCIO-1`, `NSTD-1`: Reticle's defaults are stated apart from this.
        if rule.starts_with("UCIO") || rule.starts_with("NSTD") {
            continue;
        }
        if STRICTER.iter().any(|(design, _)| design == name) {
            continue;
        }
        // The second ball names a pair, and the first is why it fails.
        match build(name, &root) {
            Ok(_) => wrong.push(format!("{name}: {rule} refuses it, and this built")),
            // The bank's supply has been the constraints' own check on every
            // family since before this, and says so in its own words.
            Err(log)
                if !log.contains(rule.as_str())
                    && !(rule == "BIVC-1" && log.contains("would need")) =>
            {
                wrong.push(format!("{name}: Vivado says {rule}, and this says {log}"));
            }
            Err(_) => {}
        }
    }
    for (name, message) in STRICTER {
        match build(name, &root) {
            Ok(_) => wrong.push(format!("{name}: this should refuse it")),
            Err(log) if !log.contains(message) => {
                wrong.push(format!("{name}: expected `{message}`, and this says {log}"))
            }
            Err(_) => {}
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A `.rcf`'s `set_io` says what an `.xdc` says: the same bitstream's decoding.
/// Would catch an option read in one language and not the other.
#[test]
fn set_io_says_what_set_property_says() {
    let (Some(root), Some(answers)) = (chipdb(), answers()) else {
        return;
    };
    for name in ["rcf_sstl15", "rcf_lvds"] {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("io-rcf")
            .join(name);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let dir = fixtures();
        let bit = out.join(format!("{name}.bit"));
        let built = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args(["fpga", "--device", DEVICE, "--quiet", "--offline"])
            .arg("--chipdb")
            .arg(&root)
            .args(["--top", name, "--constraints"])
            .arg(dir.join(format!("{name}.rcf")))
            .arg("--output-dir")
            .arg(&out)
            .arg("--bitstream")
            .arg(&bit)
            .arg(dir.join(format!("{name}.v")))
            .output()
            .expect("failed to run the reticle binary");
        assert!(
            bit.is_file(),
            "{name}: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        let ours: BTreeSet<String> = decode(&root, &bit).into_iter().collect();
        let theirs: BTreeSet<String> = answers[name].lines.iter().cloned().collect();
        assert_eq!(ours, theirs, "{name}");
    }
}
