//! A `FIFO18E1` and a `FIFO36E1` by name, held to what Vivado wrote for the
//! same designs.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/fifo/` has one small design per setting of either
//! cell (`tools/vivado-parity/gen_fifo.py`): every width in every mode, on one
//! clock and on two, falling through, flag offsets, initial and reset values,
//! every inverter, the output register's reset and enable from logic, error
//! correction, three on a `BRAM_L` tile, and seven whose name starts `fx_`
//! that ask what Vivado refuses. `vivado.txt` is what Vivado 2026.1 made of
//! each (`FIXTURE=fifo tools/vivado-parity/lutram.sh`): its cells, the site pins
//! each net of the cell reached, each block RAM tile of its bitstream decoded
//! through this repository's database, the pips that bring a clock or a
//! constant to the tile's clock and control pins, and the design rules it
//! refused a bitstream by. `xsim.txt` is what each does as Vivado built it,
//! under Vivado's simulator on its own models. `ranges.txt` is Vivado's
//! refusal of a flag offset past any depth at every width and mode, and
//! `values.txt` which bit of `INIT` each register bit of the tile is, solved
//! from twenty builds of random values per width. This test regenerates
//! `vivado.txt` when `RETICLE_FIFO_BITS` names `lutram.sh`'s output and
//! `UPDATE_EXPECT` is set.
//!
//! # What these would and would not catch
//!
//! [`each_fifo_tile_holds_what_vivados_does`] would catch a width, a register,
//! an inverter, `FIFO_MODE`, `EN_SYN`, `FIRST_WORD_FALL_THROUGH`, the error
//! correction, a flag offset off by the one Vivado adds or takes away, the width
//! bits the database does not name, and an initial or reset value on the wrong
//! bit, on the designs here. [`vivados_pins_are_the_fabrics`] would catch a
//! cell pin on another site pin, a data input on the port its width does not
//! put it on, and an output register's clock not routed.
//! [`a_fifos_clocks_and_held_pins_are_vivados`] counts the pips bringing a
//! global clock, or a one or a zero nothing drives, to the tile's clock and
//! control pins, which would see a held reset or enable lost or added, and a
//! clock routed off the network; it does not follow routes. What a FIFO does
//! is the vendor's model's word, which
//! [`a_routed_netlist_answers_as_vivados_build_does`] asks under Vivado's
//! simulator when `RETICLE_VIVADO` names an installation. None of it has been
//! near a part.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use reticle::fpga::xc7;
use reticle::fpga::xray::{
    Decoded, FifoShape, XrayDatabase, XrayOptions, fifo_check, fifo_input_roles,
    fifo_offset_ranges, fifo_register_bit,
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

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/fifo")
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

/// A small, stable hash of the contents, the same on every platform.
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

/// One line per block RAM tile: every feature of its sites and the tile's own
/// (the FIFO's `EN_SYN`, flag offsets and width), the contents counted and
/// hashed, the tile's own routing left out.
fn block_ram_tiles(features: &[(String, String)]) -> Vec<String> {
    let mut tiles: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (tile, feature) in features {
        if !tile.starts_with("BRAM_")
            || feature.starts_with("BRAM_")
            || feature.starts_with("CASCOUT_")
        {
            continue;
        }
        tiles.entry(tile).or_default().insert(feature);
    }
    let mut out = Vec::new();
    for here in tiles.into_values() {
        let contents: Vec<&str> = here
            .iter()
            .copied()
            .filter(|f| f.contains(".INIT_") || f.contains(".INITP_"))
            .collect();
        let site: Vec<&str> = here
            .iter()
            .copied()
            .filter(|f| !f.contains(".INIT_") && !f.contains(".INITP_"))
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

/// What reaches a block RAM tile's clock and control pins, over the
/// interconnect tiles beside it: a clock pin's pips from a global clock, a
/// clock pin's from a bounce wire nothing drives (a one), a control pin's
/// likewise, and an input's from a ground fan, as `gclk=N one=M ctrl_one=K
/// zero=Z`. A bounce wire is driven when a pip of its tile feeds the wire it
/// bounces (`BYP_ALT4` for `BYP_BOUNCE4`), and then it carries a signal and
/// is not counted.
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
    let beside = |tile: &str| at(tile, "INT").is_some_and(|here| rows.contains(&here));
    let mut driven: BTreeSet<(&str, String)> = BTreeSet::new();
    let mut grounded: BTreeSet<(&str, String)> = BTreeSet::new();
    for (tile, feature) in features {
        let feature = feature.replace("_L", "");
        if let Some((to, from)) = feature.split_once('.') {
            driven.insert((tile.as_str(), to.to_owned()));
            if from == "GND_WIRE" {
                grounded.insert((tile.as_str(), to.to_owned()));
            }
        }
    }
    let bare = |from: &str, kind: &str| -> bool {
        from.strip_prefix(kind)
            .is_some_and(|n| n.len() == 1 && n.chars().all(|c| c.is_ascii_digit()))
    };
    let (mut gclk, mut one, mut ctrl_one, mut zero) = (0, 0, 0, 0);
    for (tile, feature) in features {
        if !beside(tile) {
            continue;
        }
        let feature = feature.replace("_L", "");
        let Some((to, from)) = feature.split_once('.') else {
            continue;
        };
        let undriven =
            || !driven.contains(&(tile.as_str(), from.replace("BOUNCE", "ALT").to_owned()));
        if to.starts_with("CLK") && from.starts_with("GCLK") {
            gclk += 1;
        } else if to.starts_with("CLK") && bare(from, "FAN_BOUNCE") && undriven() {
            one += 1;
        } else if to.starts_with("CTRL") && bare(from, "BYP_BOUNCE") && undriven() {
            ctrl_one += 1;
        } else if from.starts_with("GFAN") && grounded.contains(&(tile.as_str(), from.to_owned())) {
            zero += 1;
        }
    }
    format!("gclk={gclk} one={one} ctrl_one={ctrl_one} zero={zero}")
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
    /// Cell counts after routing, of the FIFOs and block RAMs.
    routed: BTreeMap<String, u32>,
    /// `pin net sitepin...` of the named cell `dut`.
    pins: Vec<(String, String, Vec<String>)>,
    tiles: Vec<String>,
    /// [`clocks_and_ones`] of its bitstream.
    clocks: String,
    /// The design rules Vivado refused a bitstream by.
    refused: Vec<String>,
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
        } else if let Some(tile) = line.strip_prefix("  tile ") {
            answer.tiles.push(tile.to_owned());
        } else if let Some(clocks) = line.strip_prefix("  clocks ") {
            clocks.clone_into(&mut answer.clocks);
        } else if let Some(rule) = line.strip_prefix("  refused ") {
            answer.refused.push(rule.to_owned());
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

/// The designs this flow refuses, and a word of each refusal: the four Vivado
/// refuses, by its own design rule, and the three it builds as neither of
/// the modes the cell has.
const REFUSED: &[(&str, &str)] = &[
    ("fx_async_noreg", "enum_EN_SYN_enum_DO_REG_1"),
    ("fx_ecc_narrow", "enum_EN_ECC_READ_TRUE_enum_DATA_WIDTH_72"),
    ("fx_offset_big", "FIFO_AlmostRange_full"),
    ("fx_sync_fwft", "en_syn_and_fwft"),
    (
        "fx_f18_w36_mode",
        "`DATA_WIDTH` is 36 in `FIFO_MODE` \"FIFO18\"",
    ),
    (
        "fx_f18_w72",
        "`DATA_WIDTH` is 72 in `FIFO_MODE` \"FIFO18_36\"",
    ),
    (
        "fx_f36_w72_mode",
        "`DATA_WIDTH` is 72 in `FIFO_MODE` \"FIFO36\"",
    ),
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
        .join("fifo")
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

/// When `RETICLE_FIFO_BITS` and `UPDATE_EXPECT` are set, `vivado.txt` is made
/// again from `lutram.sh`'s output: never otherwise, so that nothing here runs
/// Vivado or needs to have.
fn regenerate(root: &str) {
    let (Some(bits), true) = (
        std::env::var_os("RETICLE_FIFO_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
    ) else {
        return;
    };
    let bits = PathBuf::from(bits);
    let mut out = String::from(
        "# What Vivado 2026.1 made of each design here, on xc7a100tftg256-1, from\n\
         # `FIXTURE=fifo tools/vivado-parity/lutram.sh`. `routed` is its count of\n\
         # FIFOs and block RAMs after routing, `pin` each pin of the cell `dut`,\n\
         # its net and the site pins that net reaches at the cell's site\n\
         # (lutram_probe.tcl), `tile` each block RAM tile of its bitstream\n\
         # decoded through this repository's database, `clocks` what reaches the\n\
         # tile's clock and control pins, and `refused` each design rule it\n\
         # refused a bitstream by: tests/fpga_xray_fifo_vivado.rs says how to\n\
         # read each.\n",
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
                    .filter(|p| {
                        p.len() == 2 && (p[0].starts_with("FIFO") || p[0].starts_with("RAMB"))
                    })
                    .map(|p| format!("{}={}", p[0], p[1]))
                    .collect();
                out.push_str(&format!("  routed {}\n", pairs.join(" ")));
            } else if let Some(rest) = line.strip_prefix("sitepin dut ") {
                let mut words = rest.split_whitespace();
                let Some(pin) = words.next() else { continue };
                let net = nets.get(pin).map_or("-", String::as_str);
                let sites: Vec<&str> = words.collect();
                out.push_str(&format!("  pin {pin} {net} {}\n", sites.join(" ")));
            }
        }
        // `ERROR: [DRC AVAL-10] enum_EN_SYN_enum_DO_REG_1: dut: ...` in the
        // run's log: the rule's own name, not its number, which only counts
        // the errors of one run.
        let log = std::fs::read_to_string(bits.join(format!("{name}.log"))).unwrap_or_default();
        let mut rules = BTreeSet::new();
        for line in log.lines() {
            if let Some(rest) = line.strip_prefix("ERROR: [DRC AVAL-")
                && let Some((_, rest)) = rest.split_once("] ")
                && let Some((rule, _)) = rest.split_once(':')
            {
                rules.insert(rule.to_owned());
            }
        }
        for rule in rules {
            out.push_str(&format!("  refused {rule}\n"));
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

/// **The block RAM tiles are Vivado's**, site feature for site feature, the
/// tile's own FIFO features and the contents, for every design both build. See
/// the module docs for what a line says and what this would catch.
#[test]
fn each_fifo_tile_holds_what_vivados_does() {
    let Some(root) = chipdb() else { return };
    regenerate(&root);
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    let mut compared = 0;
    for (name, answer) in &answers {
        let Some(decoded) = &built[name].decoded else {
            continue;
        };
        let ours = block_ram_tiles(&decoded.features);
        compared += answer.tiles.len();
        if ours != answer.tiles {
            wrong.push(format!(
                "{name}:\n  ours   {ours:?}\n  Vivado {:?}",
                answer.tiles
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(compared >= 38, "only {compared} tiles compared");
}

/// The one design whose clock and control pins are reached otherwise than
/// Vivado reaches them, and how. `f36_w72_sync`'s output register has no clock
/// (`DO_REG` 0), and Vivado holds both halves' register clocks at one from its
/// `VCC` net, routed from a slice (`CLK0.ER1END1`, from `LOGIC_OUTS_L12`) where
/// every other design here, and Reticle, holds it with a bounce wire nothing
/// drives (`CLK0.FAN_BOUNCE5`): the same one, by another road.
const CLOCKED_OTHERWISE: &[(&str, &str)] = &[("f36_w72_sync", "gclk=4 one=2 ctrl_one=2 zero=2")];

/// **A FIFO's clocks are Vivado's, and so are the ones and zeros it holds its
/// pins with**, counted as [`clocks_and_ones`] counts them, for every design
/// both build but [`CLOCKED_OTHERWISE`]'s, which is held to what it does
/// instead. Would catch an output register's clock not routed or routed when
/// Vivado gives it a one, a clock off the global network (which is how the
/// first build of these came out: the global buffers did not know a FIFO's
/// clock pins), a reset held through its inverter lost or added, the upper
/// half's enables, and the error injections' grounds.
#[test]
fn a_fifos_clocks_and_held_pins_are_vivados() {
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    let mut compared = 0;
    for (name, answer) in &answers {
        let Some(decoded) = &built[name].decoded else {
            continue;
        };
        let ours = clocks_and_ones(&decoded.features);
        let want = CLOCKED_OTHERWISE
            .iter()
            .find(|(n, _)| n == name)
            .map_or(answer.clocks.as_str(), |(_, ours)| ours);
        compared += 1;
        if ours != want {
            wrong.push(format!("{name}: ours {ours}, Vivado {}", answer.clocks));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(compared >= 38, "only {compared} compared");
}

/// **Each design gets the FIFOs Vivado gave it**, and no block RAM: a cell by
/// name stays itself. Would catch a FIFO made into something else.
#[test]
fn each_design_gets_the_fifo_vivado_gave_it() {
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    let mut wrong = Vec::new();
    for (name, answer) in &answers {
        if built[name].decoded.is_none() {
            continue;
        }
        let mut ours: BTreeMap<String, u32> = BTreeMap::new();
        for line in built[name].log.lines() {
            if let Some((n, cell)) = line.trim().split_once(" x ")
                && let Ok(n) = n.parse::<u32>()
                && (cell.starts_with("FIFO") || cell.starts_with("RAMB"))
            {
                ours.insert(cell.to_owned(), n);
            }
        }
        if ours != answer.routed {
            wrong.push(format!("{name}: ours {ours:?}, Vivado {:?}", answer.routed));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **What Vivado refuses, Reticle refuses by the same rule**, and the three
/// shapes Vivado builds as neither mode Reticle refuses for that. Would catch
/// a refusal turning into a build, and a rule cited that is not Vivado's.
#[test]
fn what_vivado_refuses_is_refused_by_its_rule() {
    let Some(built) = built() else { return };
    let answers = answers().unwrap();
    for (name, reason) in REFUSED {
        let log = &built[*name].log;
        assert!(log.contains(reason), "{name}: {log}");
        for rule in &answers[*name].refused {
            // The second ECC rule is the write side's, which Reticle names
            // after the read side's has refused the cell.
            if rule.contains("EN_ECC_WRITE") {
                continue;
            }
            assert!(log.contains(rule.as_str()), "{name}: {rule}: {log}");
        }
    }
    let refused_by_vivado = answers.values().filter(|a| !a.refused.is_empty()).count();
    assert_eq!(refused_by_vivado, 4, "Vivado refuses four of these");
}

/// The roles of a FIFO's site that a cell pin reaches, at this shape, as the
/// fabric names them; see `fifo::fifo18_pins`.
fn roles_of(pin: &str, shape: FifoShape) -> Vec<String> {
    let (base, bit) = match pin.split_once('[') {
        Some((base, rest)) => (base, rest.trim_end_matches(']').parse::<u32>().ok()),
        None => (pin, None),
    };
    let indexed = |role: &str| match bit {
        Some(bit) => format!("{role}{bit}"),
        None => role.to_owned(),
    };
    match base {
        "RDCLK" if shape.registered => vec!["p0_clk".to_owned(), "p0_regclk".to_owned()],
        "RDCLK" => vec!["p0_clk".to_owned()],
        "WRCLK" => vec!["p1_clk".to_owned()],
        "RDEN" => vec!["p0_en".to_owned()],
        "WREN" => vec!["p1_en".to_owned()],
        "RST" => vec!["p0_rst".to_owned()],
        "RSTREG" => vec!["p0_rstreg".to_owned()],
        "REGCE" => vec!["p0_regce".to_owned()],
        "DI" => fifo_input_roles(shape, &indexed("fifo_di")),
        "DIP" => fifo_input_roles(shape, &indexed("fifo_dip")),
        "DO" => vec![indexed("p0_dout")],
        "DOP" => vec![indexed("p0_doutp")],
        "INJECTDBITERR" | "INJECTSBITERR" | "DBITERR" | "SBITERR" | "ECCPARITY" => {
            vec![indexed(&format!("p0_{}", base.to_lowercase()))]
        }
        other => vec![indexed(&format!("fifo_{}", other.to_lowercase()))],
    }
}

/// A design's parameters of its `dut`, read off the fixture.
fn params_of(name: &str) -> BTreeMap<String, AttrValue> {
    let text = std::fs::read_to_string(fixtures().join(format!("{name}.v"))).unwrap();
    let mut out = BTreeMap::new();
    for key in [
        "DATA_WIDTH",
        "DO_REG",
        "FIFO_MODE",
        "EN_SYN",
        "FIRST_WORD_FALL_THROUGH",
        "EN_ECC_READ",
        "EN_ECC_WRITE",
    ] {
        let at = format!(".{key}(");
        if let Some(start) = text.find(&at) {
            let rest = &text[start + at.len()..];
            let value = rest[..rest.find(')').unwrap_or(0)].trim();
            let value = match value.strip_prefix('"') {
                Some(s) => AttrValue::String(s.trim_end_matches('"').to_owned()),
                None => AttrValue::Int(value.parse().unwrap()),
            };
            out.insert(key.to_owned(), value);
        }
    }
    out
}

/// The wire each site pin of the site of `site_type` in a tile of `tile_type`
/// is on, by the database's own description of the site.
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

/// **Every pin of every FIFO reaches the site pins Vivado's reached**, net by
/// net: for each net on the cell, the wires of the site pins Vivado's report
/// has it on against the wires of the fabric's pins for the roles its cell
/// pins take at that shape. See the module docs.
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
        if answer.tiles.is_empty() || REFUSED.iter().any(|(n, _)| n == name) {
            continue;
        }
        let params = params_of(name);
        let wide = answer.routed.contains_key("FIFO36E1");
        let primitive = if wide { "FIFO36E1" } else { "FIFO18E1" };
        let shape = fifo_check(primitive, &|key: &str| params.get(key).cloned()).unwrap();
        let (bel, sites) = if wide {
            ("FIFO36", &whole)
        } else {
            ("FIFO18", &y0)
        };
        let bel = tile.bels.iter().find(|b| b.name == bel).unwrap();
        let wires_of = |role: &str| -> BTreeSet<String> {
            bel.pins
                .iter()
                .filter(|(r, _)| r == role)
                .map(|(_, w)| w.name.clone())
                .collect()
        };
        let mut theirs: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        let mut ours: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        for (pin, net, site_pins) in &answer.pins {
            // A constant, and `REGCE` tied high, which `opt_design` moves onto
            // a net of its own (`OPT_FIFO_REGCE_dut`) that reaches no site pin.
            if net.contains("const")
                || net.starts_with("GND")
                || net.starts_with("VCC")
                || net.starts_with("OPT_FIFO_REGCE")
            {
                continue;
            }
            theirs.entry(net).or_default().extend(
                site_pins
                    .iter()
                    .map(|s| sites.get(s).cloned().unwrap_or_else(|| format!("?{s}"))),
            );
            let mine = ours.entry(net).or_default();
            for role in roles_of(pin, shape) {
                mine.extend(wires_of(&role));
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
    assert!(checked >= 3000, "only {checked} nets checked");
}

/// **Every bit decodes, the arcs are the router's, and the mapping is proved**,
/// for every design this flow builds: the width's bits, which the database does
/// not name, among them.
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

/// **The flag offsets Reticle accepts are the ones Vivado accepts**, at every
/// width of both cells on one clock, on two, and falling through: every line of
/// `ranges.txt`, Vivado's own refusal of an offset past any depth. Needs no
/// database. Would catch a range off by one at either end, and a `FIFO18E1`
/// counted at its own depth, which Vivado does not.
#[test]
fn the_flag_offset_ranges_are_vivados() {
    let Ok(text) = std::fs::read_to_string(fixtures().join("ranges.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    };
    let mut seen = 0;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let [_, width, mode, "empty", e0, e1, "full", f0, f1] = w.as_slice() else {
            panic!("`{line}`");
        };
        let n = |s: &str| s.parse::<i64>().unwrap();
        let (empty, full) = fifo_offset_ranges(n(width), *mode == "sync", *mode == "fwft");
        assert_eq!(empty, (n(e0), n(e1)), "{line}");
        assert_eq!(full, (n(f0), n(f1)), "{line}");
        seen += 1;
    }
    assert_eq!(seen, 27, "every width of both cells in three modes");
}

/// **Every register bit of the tile is the bit of `INIT` Vivado puts there**,
/// for every width of both cells, both halves and both ports: every line of
/// `values.txt`, solved from twenty builds of random values per width. Needs no
/// database. Would catch a word split between the halves another way, a
/// parity bit on a data bit's place, and port B's one bit lost.
#[test]
fn the_initial_values_are_vivados() {
    let Ok(text) = std::fs::read_to_string(fixtures().join("values.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    };
    let mut seen = 0;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let [cell, width, at, source] = w.as_slice() else {
            panic!("`{line}`");
        };
        let (half, rest) = at.split_once('.').unwrap();
        let (port, bit) = rest.split_once('[').unwrap();
        let bit: u32 = bit.trim_end_matches(']').parse().unwrap();
        let width: i64 = width.parse().unwrap();
        let mode = match (*cell, width) {
            ("FIFO18E1", 36) => "FIFO18_36",
            ("FIFO18E1", _) => "FIFO18",
            (_, 72) => "FIFO36_72",
            _ => "FIFO36",
        };
        let params = |key: &str| match key {
            "DATA_WIDTH" => Some(AttrValue::Int(width)),
            "FIFO_MODE" => Some(AttrValue::String(mode.to_owned())),
            "EN_SYN" => Some(AttrValue::String("TRUE".to_owned())),
            _ => None,
        };
        let shape = fifo_check(cell, &params).unwrap();
        let ours = fifo_register_bit(shape, half == "RAMB18_Y1", usize::from(port == "B"), bit);
        let want = match *source {
            "=0" => None,
            k => Some(k.parse::<u32>().unwrap_or_else(|_| panic!("`{line}`"))),
        };
        assert_eq!(ours, want, "{line}");
        seen += 1;
    }
    assert_eq!(seen, 504, "every bit of every register of every width");
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

/// What Vivado's simulator printed after `R `.
fn answer_of(out: &str) -> Option<String> {
    let at = out
        .find("\nR ")
        .map(|i| i + 3)
        .or_else(|| out.strip_prefix("R ").map(|_| 2))?;
    Some(out[at..].lines().next()?.trim().to_lowercase())
}

/// **The netlist each bitstream was made from answers as Vivado's build
/// does**, on Vivado's models under Vivado's simulator, when `RETICLE_VIVADO`
/// names one: 2,000 cycles of each design's `so`, which folds in every output
/// of the FIFO.
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
