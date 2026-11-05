//! The configuration primitives, held to what Vivado wrote for the same designs.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/config/` has one small design per question, written
//! here, each asking one parameter of a `BSCANE2`, `CAPTUREE2`, `DNA_PORT`,
//! `EFUSE_USR`, `FRAME_ECCE2`, `ICAPE2`, `STARTUPE2` or `USR_ACCESSE2`, one
//! wiring of its pins, or one `write_bitstream` option beside one (`<name>.opts`,
//! which is what `--bitstream-option` is given). `vivado.txt` has what Vivado
//! 2026.1 made of each, from `tools/vivado-parity/config.sh` and
//! `config_fixture.py`:
//!
//! - a `site` line for each configuration tile the bitstream sets anything in,
//!   with its features, read back through this repository's database;
//! - a `ground` line for each feature of the interconnect column the tile's
//!   pins enter from on a road from a `GND_WIRE`: which inputs were tied to
//!   zero, and how;
//! - the stream's packets other than the frames and the CRCs, word for word:
//!   `COR0`, `CTL0` and its masks, `AXSS`, and the order of all of it;
//! - a `refused` line in place of both, with the rule Vivado gave.
//!
//! # What these would and would not catch
//!
//! [`each_design_is_vivados`] would catch a chain on the wrong site, a width
//! not set or set on the wrong bits, one of a `STARTUPE2`'s three-bit groups
//! set by the wrong pins, a `COR0` or `CTL0` bit the design asks for and this
//! does not write (a `CAPTUREE2`'s `SINGLE`, a `STARTUPE2`'s user clock, a
//! `FRAME_ECCE2`'s `FARSRC` and `GLUTMASK_B`, the bottom `ICAPE2`), and a
//! refusal Vivado makes that this does not, and an input tied where Vivado
//! leaves it or left where Vivado ties it (a `STARTUPE2`'s open `GSR` reads one
//! untied, and holds the part in reset). It compares the configuration tiles,
//! the ties and the stream, not the rest of the interconnect: where this routes
//! a signal is its own router's, and the bits of that are held by the flow's
//! own check that the arcs selected are the router's. It would not catch a bit that means
//! something other than its name: **nothing here has been on a board**, so
//! all it says is that the bits are Vivado's.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const DEVICE: &str = "xc7a100t-ftg256";

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures() -> PathBuf {
    manifest().join("testdata/fpga/vivado/config")
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

/// What `vivado.txt` says of one design.
#[derive(Default)]
struct Answer {
    sites: Vec<String>,
    ground: Vec<String>,
    stream: Vec<String>,
    /// How many frames' ECC `--decode` finds other than its rule, on Vivado's
    /// frames: `None` for a design refused.
    ecc: Option<String>,
    refused: Option<String>,
}

fn answers() -> Option<BTreeMap<String, Answer>> {
    let Ok(text) = std::fs::read_to_string(fixtures().join("vivado.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let split = |s: &str| s.split(" | ").map(str::to_owned).collect::<Vec<_>>();
    let mut base = Vec::new();
    let mut out: BTreeMap<String, Answer> = BTreeMap::new();
    let mut at = String::new();
    for line in text.lines() {
        if let Some(stream) = line.strip_prefix("base-stream ") {
            base = split(stream);
        } else if let Some(name) = line.strip_prefix("design ") {
            at = name.to_owned();
            out.entry(at.clone()).or_default();
        } else if let Some(site) = line.strip_prefix("  site ") {
            out.get_mut(&at).unwrap().sites.push(site.to_owned());
        } else if let Some(tie) = line.strip_prefix("  ground ") {
            out.get_mut(&at).unwrap().ground.push(tie.to_owned());
        } else if let Some(ecc) = line.strip_prefix("  ecc ") {
            out.get_mut(&at).unwrap().ecc = Some(ecc.to_owned());
        } else if let Some(rule) = line.strip_prefix("  refused ") {
            out.get_mut(&at).unwrap().refused = Some(rule.to_owned());
        } else if let Some(stream) = line.strip_prefix("  stream ") {
            out.get_mut(&at).unwrap().stream = if stream == "base" {
                base.clone()
            } else {
                split(stream)
            };
        }
    }
    Some(out)
}

/// A register's name, as `tools/vivado-parity/bit_packets.py` writes it.
fn register(address: u32) -> String {
    let name = match address {
        0 => "CRC",
        1 => "FAR",
        2 => "FDRI",
        3 => "FDRO",
        4 => "CMD",
        5 => "CTL0",
        6 => "MASK",
        7 => "STAT",
        8 => "LOUT",
        9 => "COR0",
        10 => "MFWR",
        11 => "CBC",
        12 => "IDCODE",
        13 => "AXSS",
        14 => "COR1",
        16 => "WBSTAR",
        17 => "TIMER",
        19 => "RSVD19",
        22 => "BOOTSTS",
        24 => "CTL1",
        31 => "BSPI",
        other => return format!("REG{other}"),
    };
    name.to_owned()
}

/// A command's name, the same way.
fn command(value: u32) -> &'static str {
    match value {
        0 => "NULL",
        1 => "WCFG",
        2 => "MFW",
        3 => "LFRM",
        4 => "RCFG",
        5 => "START",
        7 => "RCRC",
        8 => "AGHIGH",
        9 => "SWITCH",
        10 => "GRESTORE",
        11 => "SHUTDOWN",
        13 => "DESYNC",
        15 => "IPROG",
        _ => "?",
    }
}

/// The stream's packets in `bit_packets.py`'s words, without the frames and
/// the CRCs, as `config_fixture.py` keeps Vivado's.
fn packets(bytes: &[u8]) -> Vec<String> {
    let data = reticle::fpga::xc7::bit_payload(bytes).expect("a .bit");
    let sync = data
        .windows(4)
        .position(|w| w == [0xAA, 0x99, 0x55, 0x66])
        .expect("a sync word");
    let word = |at: usize| u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    let mut runs: Vec<(u32, usize)> = Vec::new();
    for at in (0..sync).step_by(4) {
        match runs.last_mut() {
            Some((w, n)) if *w == word(at) => *n += 1,
            _ => runs.push((word(at), 1)),
        }
    }
    let lead: Vec<String> = runs
        .iter()
        .map(|(w, n)| {
            if *n > 1 {
                format!("{w:08x}*{n}")
            } else {
                format!("{w:08x}")
            }
        })
        .collect();
    let mut out = vec![format!("leading {} / aa995566", lead.join(" / "))];
    let words: Vec<u32> = (sync + 4..data.len() - 3).step_by(4).map(word).collect();
    let (mut i, mut nops, mut last) = (0usize, 0usize, 0u32);
    let flush = |out: &mut Vec<String>, nops: &mut usize| {
        if *nops > 0 {
            out.push(format!("nop {nops}"));
        }
        *nops = 0;
    };
    while i < words.len() {
        let h = words[i];
        i += 1;
        let (reg, n) = match h >> 29 {
            1 => {
                last = (h >> 13) & 0x3FFF;
                (last, usize::try_from(h & 0x7FF).unwrap_or(0))
            }
            2 => (last, usize::try_from(h & 0x07FF_FFFF).unwrap_or(0)),
            _ => {
                flush(&mut out, &mut nops);
                out.push(format!("word {h:08x} (not a packet header)"));
                continue;
            }
        };
        let body = &words[i..(i + n).min(words.len())];
        i += n;
        match (h >> 27) & 3 {
            0 => nops += 1,
            1 => {
                flush(&mut out, &mut nops);
                out.push(format!("read {} {n}", register(reg)));
            }
            _ => {
                flush(&mut out, &mut nops);
                if matches!(reg, 0 | 2 | 3) {
                    continue;
                }
                if reg == 4 && n == 1 {
                    out.push(format!("CMD {:08x} {}", body[0], command(body[0])));
                } else {
                    let values: Vec<String> = body.iter().map(|w| format!("{w:08x}")).collect();
                    out.push(
                        format!("{} {}", register(reg), values.join(" "))
                            .trim_end()
                            .to_owned(),
                    );
                }
            }
        }
    }
    flush(&mut out, &mut nops);
    out
}

/// The features of the interconnect column the configuration tile's pins
/// enter from (`INT_L_X18`) on a road from a `GND_WIRE`, as
/// `config_fixture.py`'s `grounded` reads them: a ground fan's hop, what a
/// grounded fan drives, and a clock line's bounce from a grounded `FAN_ALT`.
fn grounded(features: &[(String, String)]) -> Vec<String> {
    let mut out: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    let has = |tile: &str, name: &str| features.iter().any(|(t, n)| t == tile && n == name);
    for (tile, name) in features {
        let Some((_, from)) = name.split_once('.') else {
            continue;
        };
        if from == "GND_WIRE" || from.starts_with("GFAN") && has(tile, &format!("{from}.GND_WIRE"))
        {
            out.insert((tile.clone(), name.clone()));
        }
    }
    for (tile, name) in features {
        let Some((to, from)) = name.split_once('.') else {
            continue;
        };
        if let Some(k) = from.strip_prefix("FAN_BOUNCE")
            && to.starts_with("CLK_L")
            && out
                .iter()
                .any(|(t, n)| t == tile && n.starts_with(&format!("FAN_ALT{k}.")))
        {
            out.insert((tile.clone(), name.clone()));
        }
    }
    out.into_iter().map(|(t, n)| format!("{t} {n}")).collect()
}

/// How many frames' ECC `--decode`'s summary calls wrong, or `?`.
fn ecc_wrong(decoded: &str) -> String {
    decoded
        .lines()
        .filter_map(|l| l.split_once(" frame(s) with a wrong ECC"))
        .filter_map(|(head, _)| head.rsplit(' ').next())
        .next()
        .unwrap_or("?")
        .to_owned()
}

/// One design through the flow.
struct Built {
    ok: bool,
    log: String,
    sites: Vec<String>,
    ground: Vec<String>,
    stream: Vec<String>,
    /// Set bits the database has no name for.
    unnamed: Vec<String>,
    /// `ecc <n> wrong`, as `vivado.txt` has it of Vivado's.
    ecc: String,
}

fn build(root: &str, name: &str) -> Built {
    let dir = fixtures();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("config")
        .join(name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join(format!("{name}.bit"));
    let mut run = Command::new(env!("CARGO_BIN_EXE_reticle"));
    run.args(["fpga", "--device", DEVICE, "--chipdb", root, "--top", name])
        .arg("--constraints")
        .arg(dir.join(format!("{name}.rcf")))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join(format!("{name}.v")));
    if let Ok(opts) = std::fs::read_to_string(dir.join(format!("{name}.opts"))) {
        for line in opts.lines() {
            run.args(["--bitstream-option", line]);
        }
    }
    let run = run.output().expect("the reticle binary runs");
    let log = String::from_utf8_lossy(&run.stderr).into_owned();
    if !run.status.success() {
        return Built {
            ok: false,
            log,
            sites: Vec::new(),
            ground: Vec::new(),
            stream: Vec::new(),
            unnamed: Vec::new(),
            ecc: String::new(),
        };
    }
    let decoded = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--chipdb", root, "--decode"])
        .arg(&bit)
        .output()
        .expect("the reticle binary runs");
    let text = String::from_utf8_lossy(&decoded.stdout);
    let mut sites: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unnamed = Vec::new();
    let mut column = Vec::new();
    for line in text.lines() {
        if let Some(bit) = line.strip_prefix("? ") {
            unnamed.push(bit.to_owned());
            continue;
        }
        let Some((tile, feature)) = line.split_once(' ') else {
            continue;
        };
        let kind = tile.rsplit_once("_X").map_or(tile, |(kind, _)| kind);
        if matches!(kind, "CFG_CENTER_MID" | "CFG_CENTER_TOP") {
            sites
                .entry(kind.to_owned())
                .or_default()
                .push(feature.to_owned());
        } else if tile.starts_with("INT_L_X18Y") {
            column.push((tile.to_owned(), feature.to_owned()));
        }
    }
    let sites = sites
        .into_iter()
        .map(|(kind, mut features)| {
            features.sort();
            format!("{kind}: {}", features.join(" "))
        })
        .collect();
    Built {
        ok: true,
        log,
        sites,
        ground: grounded(&column),
        stream: packets(&std::fs::read(&bit).unwrap()),
        unnamed,
        ecc: format!(
            "{} wrong",
            ecc_wrong(&String::from_utf8_lossy(&decoded.stderr))
        ),
    }
}

fn build_all(names: Vec<String>) -> Vec<(String, Built)> {
    let Some(root) = chipdb() else {
        return Vec::new();
    };
    std::thread::scope(|scope| {
        let root = &root;
        let handles: Vec<_> = names
            .chunks(names.len().div_ceil(4).max(1))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|name| (name.clone(), build(root, name)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    })
}

/// Each design's answer against Vivado's, and every disagreement said.
fn compare(built: &[(String, Built)], answers: &BTreeMap<String, Answer>) {
    let mut wrong = Vec::new();
    for (name, one) in built {
        let theirs = &answers[name];
        match (&theirs.refused, one.ok) {
            (Some(rule), true) => wrong.push(format!(
                "{name}: Vivado refuses it ({rule}), and this built it"
            )),
            (Some(rule), false) => {
                // The rule's number is in what this says, or the placer's
                // own words for the same thing: two cells and one site.
                let said = rule
                    .split(' ')
                    .filter(|w| w.contains('-'))
                    .any(|w| one.log.contains(w))
                    || one.log.contains("site(s) and the part has 1");
                if !said {
                    wrong.push(format!(
                        "{name}: Vivado refuses it ({rule}), and this says\n{}",
                        one.log
                    ));
                }
            }
            (None, false) => wrong.push(format!("{name}: Vivado built it, and:\n{}", one.log)),
            (None, true) => {
                if one.sites != theirs.sites {
                    wrong.push(format!(
                        "{name}:\n  ours   {:?}\n  Vivado {:?}",
                        one.sites, theirs.sites
                    ));
                }
                if one.ground != theirs.ground {
                    wrong.push(format!(
                        "{name}: the ties differ:\n  ours   {:?}\n  Vivado {:?}",
                        one.ground, theirs.ground
                    ));
                }
                if one.stream != theirs.stream {
                    let differ: Vec<String> = one
                        .stream
                        .iter()
                        .zip(&theirs.stream)
                        .filter(|(a, b)| a != b)
                        .map(|(a, b)| format!("{a} where Vivado has {b}"))
                        .collect();
                    wrong.push(format!(
                        "{name}: the stream differs, {} packets against {}: {differ:?}",
                        one.stream.len(),
                        theirs.stream.len()
                    ));
                }
                // Vivado's frames all have the ECC `--decode`'s rule gives
                // them, a shift register's or a RAM's contents left out; and
                // so must this flow's, which a frame with contents written
                // with the whole frame's code is not.
                if theirs.ecc.as_deref() != Some(one.ecc.as_str()) {
                    wrong.push(format!(
                        "{name}: frame ECC {} where Vivado's is {:?}",
                        one.ecc, theirs.ecc
                    ));
                }
                if !one.unnamed.is_empty() {
                    wrong.push(format!("{name}: unnamed bits {:?}", one.unnamed));
                }
                // A pin with no road to it routes nothing and writes a stream
                // and a tile that are still Vivado's: `USR_ACCESSE2`'s first
                // build did. So the routing is part of the answer.
                if one.log.contains("NOT FULLY ROUTED") {
                    wrong.push(format!("{name}: not fully routed:\n{}", one.log));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The designs each run of the gate builds: one or two of each primitive, its
/// stream's every kind of difference, and refusals of each kind.
/// [`every_design_here`] builds the rest.
const IN_THE_GATE: &[&str] = &[
    "base",
    "bscan_chain3",
    "bscan_disable",
    "bscan_two",
    "capture_plain",
    "capture_oneshot_false",
    "capture_open",
    "dna_plain",
    "dna_dout_open",
    "efuse_plain",
    "ecc_far",
    "ecc_srl",
    "ecc_srl_logic",
    "ecc_ram_logic",
    "noecc_srl16_logic",
    "icap_x8",
    "icap_loc0_x16",
    "icap_two_select_bottom",
    "startup_all",
    "startup_clk_jtagclk",
    "startup_gsr_1",
    "startup_gts",
    "startup_usrdonets",
    "usr_value",
    "all_eight",
    "bscan_loc1",
    "bscan_open",
    "no_bscan_same",
    "no_icap_select_top_loc0",
    "no_startup_userclk",
    "no_startup_two",
];

// `NOT_YET` was here: `ecc_srl`, a `FRAME_ECCE2` beside a shift register, which
// this refused, reading Vivado's ECC of zero on four frames as what a
// `FRAME_ECCE2` does. It is what Vivado writes with or without one, the code
// with the shift register's contents left out, and nothing else was in those
// frames; this flow wrote the whole frame's code, with or without one. It
// builds now, and every design's ECC is compared.

/// The gate's designs, built once for what asks about them.
fn gate() -> Option<&'static Vec<(String, Built)>> {
    static BUILT: OnceLock<Option<Vec<(String, Built)>>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            chipdb()?;
            answers()?;
            Some(build_all(
                IN_THE_GATE.iter().map(|s| (*s).to_owned()).collect(),
            ))
        })
        .as_ref()
}

/// **The configuration tiles and the stream are Vivado's**, on the designs each
/// run of the gate builds. See the module docs for what this would catch.
#[test]
fn each_design_is_vivados() {
    let Some(built) = gate() else { return };
    compare(built, &answers().unwrap());
}

/// **The same on every design here**, 128 of them. Takes a bitstream per
/// design, which is why it is not in the gate.
#[test]
#[ignore = "builds every design here to a bitstream, and needs the Project X-Ray database"]
fn every_design_here() {
    let Some(answers) = answers() else { return };
    let built = build_all(answers.keys().cloned().collect());
    compare(&built, &answers);
}

/// **Each design in the gate says it was checked.** `reticle fpga` refuses to
/// finish when the arcs its bits select are not the ones its router chose; this
/// reads the verdict, so that the check being dropped from the flow is noticed.
/// It is what found the configuration sites' two-part features read as pips:
/// `STARTUP.GSR_OR_GTS` was a selected arc nothing chose, until `NOT_ARCS` said
/// otherwise. Would not catch the check being wrong.
#[test]
fn the_arcs_selected_are_the_routers() {
    let Some(built) = gate() else { return };
    let mut seen = 0;
    for (name, one) in built.iter().filter(|(_, one)| one.ok) {
        seen += 1;
        assert!(
            one.log
                .contains("the interconnect arcs those bits select are exactly the ones"),
            "{name}: {}",
            one.log
        );
    }
    assert!(seen > 15, "only {seen} designs built");
}

/// **The fixture is what its generator writes.** Every design `vivado.txt`
/// answers for has its source here and the other way round, so a design added
/// to `gen_config.py` without a Vivado run, or one dropped, fails this. Would
/// not catch a design changed after its run.
#[test]
fn every_design_has_an_answer() {
    let Some(answers) = answers() else { return };
    let mut designs: Vec<String> = std::fs::read_dir(fixtures())
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.path()
                .file_name()?
                .to_str()?
                .strip_suffix(".v")
                .map(str::to_owned)
        })
        .collect();
    designs.sort();
    let answered: Vec<String> = answers.keys().cloned().collect();
    assert_eq!(designs, answered);
    assert_eq!(answered.len(), 128);
    assert_eq!(answers.values().filter(|a| a.refused.is_some()).count(), 11);
}
