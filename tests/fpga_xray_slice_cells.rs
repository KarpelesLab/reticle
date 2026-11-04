//! A latch and a shift register in a slice, against what Vivado 2026.1 wrote
//! for the same cell.
//!
//! `testdata/fpga/vivado/slice_cells/` has one small design per form of each
//! cell, and `vivado.features` what Vivado's bitstream for each decodes to
//! through Reticle's own decoder (`tools/vivado-parity/` made it; Vivado is not
//! needed to run this). Each test builds a design with `reticle fpga`, decodes
//! the result the same way, and compares:
//!
//! - **the site's configuration, exactly**: what the slice says of the cell,
//!   with the lane it happens to be on written `L`, since the two tools place
//!   differently;
//! - **the interconnect by its ends**: which of the slice's pins are driven at
//!   all. Which wires reached them is the router's business, and a pin nothing
//!   drives reads one, so whether it is driven is part of the function;
//! - **what it computes**: the netlist the bitstream was made from, simulated
//!   beside the source over 600 cycles of pseudo-random input.
//!
//! # What it would and would not catch
//!
//! It would catch the bit that was wrong by reading alone: a latch open while
//! its gate is high sets `CLKINV`, the opposite of what the bit means to a
//! flip-flop, and a mapping that took the flip-flop's table placed, routed and
//! decoded with every latch holding when it should follow. It would catch a
//! clear lost to a lookup table on the gate, a lane's pin on the wrong wire, and
//! a latch sharing a slice with a flip-flop.
//!
//! Of a shift register it would catch a stored bit in the wrong place in `INIT`
//! (each is written twice, and an address is `A[i]` on the table's input `i + 2`),
//! a data pin on `<L>I` where the sixteen-bit form reads `<L>X`, a zero on an
//! address left unrouted so that it reads one, `WEMUX.CE` lost, and a chain
//! shortened or lengthened by one when it was taken apart.
//!
//! It would not catch a wrong reading of what Vivado's choice *means* in a case
//! neither tool exercises here, and it says nothing of the part. **Nothing here
//! has been on a board.** The simulation is of the netlist and the library's
//! model: a bitstream that encodes that netlist wrongly is the decode's to catch.
//!
//! Skips without a Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "sim", feature = "cli"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures() -> PathBuf {
    manifest().join("testdata/fpga/vivado/slice_cells")
}

/// The database root, or `None` with a line saying what is missing. The same
/// search as `tests/fpga_xray.rs`.
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

/// One tile's worth of decoded lines, as either tool's bitstream gives them.
#[derive(Default, Debug)]
struct Image {
    /// `(tile, feature)`: a slice's and the interconnect's.
    features: Vec<(String, String)>,
    /// Where each cell landed, `(primitive, bel)`, for Vivado's only.
    placed: Vec<(String, String)>,
    /// How many of each primitive the netlist has.
    cells: BTreeMap<String, usize>,
    /// Whether a bitstream exists at all.
    built: bool,
}

/// `vivado.features`, by design.
fn vivado() -> Option<BTreeMap<String, Image>> {
    let Ok(text) = std::fs::read_to_string(fixtures().join("vivado.features")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut out: BTreeMap<String, Image> = BTreeMap::new();
    let mut current = String::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["design", name] => {
                current = (*name).to_owned();
                out.insert(
                    current.clone(),
                    Image {
                        built: true,
                        ..Image::default()
                    },
                );
            }
            ["no-bitstream"] => {
                out.get_mut(&current).unwrap().built = false;
            }
            ["feature" | "pip", tile, feature] => {
                out.get_mut(&current)
                    .unwrap()
                    .features
                    .push(((*tile).to_owned(), (*feature).to_owned()));
            }
            ["table", tile, lut, value] => {
                // Back into the features a decode of the same bits gives.
                let value = u64::from_str_radix(value, 16).unwrap();
                for bit in (0..64).filter(|bit| (value >> bit) & 1 == 1) {
                    out.get_mut(&current)
                        .unwrap()
                        .features
                        .push(((*tile).to_owned(), format!("{lut}.INIT[{bit:02}]")));
                }
            }
            ["cell", primitive, count] => {
                out.get_mut(&current)
                    .unwrap()
                    .cells
                    .insert((*primitive).to_owned(), count.parse().unwrap());
            }
            ["placed", primitive, _, bel, ..] => {
                out.get_mut(&current)
                    .unwrap()
                    .placed
                    .push(((*primitive).to_owned(), (*bel).to_owned()));
            }
            _ => {}
        }
    }
    Some(out)
}

/// A design through `reticle fpga` and its `--decode`; `Err` with the
/// compiler's words if it would not build.
struct Built {
    image: Image,
    routed: PathBuf,
    stderr: String,
}

fn build(root: &str, design: &str) -> Result<Built, String> {
    // Tests run together and several build one design.
    static BUILDS: AtomicUsize = AtomicUsize::new(0);
    let build = BUILDS.fetch_add(1, Ordering::Relaxed);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("slice-cell-{design}-{build}"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join(format!("{design}.bit"));
    let routed = out.join(format!("{design}.routed.v"));
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a100t-ftg256",
            "--chipdb",
            root,
            "--top",
            design,
            "--report",
        ])
        .arg("--constraints")
        .arg(fixtures().join(format!("{design}.rcf")))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg("--routed-netlist")
        .arg(&routed)
        .arg(fixtures().join(format!("{design}.v")))
        .output()
        .expect("the reticle binary runs");
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    if !run.status.success() {
        return Err(stderr);
    }
    let report = String::from_utf8_lossy(&run.stdout).into_owned();
    let mut cells = BTreeMap::new();
    for line in report.lines().chain(stderr.lines()) {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let [count, "x", primitive] = words.as_slice()
            && let Ok(count) = count.parse::<usize>()
        {
            cells.insert((*primitive).to_owned(), count);
        }
    }
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", "xc7a100t-ftg256", "--chipdb", root])
        .arg("--decode")
        .arg(&bit)
        .output()
        .expect("the reticle binary runs");
    assert!(run.status.success());
    let stdout = String::from_utf8_lossy(&run.stdout);
    let decode_log = String::from_utf8_lossy(&run.stderr);
    let mut image = Image {
        built: true,
        cells,
        ..Image::default()
    };
    for line in stdout.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let [tile, feature] = words.as_slice() {
            image
                .features
                .push(((*tile).to_owned(), (*feature).to_owned()));
        }
    }
    // Never weakened: every set bit of a bitstream written here decodes.
    assert!(
        decode_log.contains(" 0 bit(s) unexplained"),
        "{design}: bits nothing names:\n{decode_log}"
    );
    assert!(
        stderr.contains("0 bit(s) unexplained") && stderr.contains("exactly the ones the router"),
        "{design}: the flow's own decode and arc checks did not report clean:\n{stderr}"
    );
    Ok(Built {
        image,
        routed,
        stderr,
    })
}

/// The slice wires of a site, by the prefix a tile's `ppips` file gives them.
fn wire_prefix(tile: &str, slice: &str) -> &'static str {
    match (tile.starts_with("CLBLM"), slice) {
        (true, "SLICEM_X0") => "M",
        (true, _) => "L",
        (false, "SLICEL_X0") => "LL",
        (false, _) => "L",
    }
}

/// Which of a slice's pins the interconnect drives, by the pin's name with its
/// lane kept: `CLK`, `SR`, `A3`.
///
/// An interconnect tile's sink (`IMUX12`, `CTRL1`,
/// `CLK0`) has one number, and `ppips_<tile type>.db` says which pin of which
/// slice has that number. Only the sinks that lead nowhere else are counted.
fn driven_pins(root: &str, image: &Image, tile: &str, slice: &str) -> BTreeSet<String> {
    let kind = tile.split("_X").next().unwrap_or(tile);
    let text =
        std::fs::read_to_string(format!("{root}/artix7/ppips_{}.db", kind.to_lowercase())).unwrap();
    let side = if kind.ends_with("_L") { "L" } else { "R" };
    let short = kind.split('_').next().unwrap_or(kind);
    let prefix = format!("{short}_{}_", wire_prefix(tile, slice));
    // `CLBLM_R.CLBLM_M_AX.CLBLM_BYP1 always`: `BYP1` is `AX`.
    let mut pin_of: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        let mut parts = name.split('.');
        let (Some(_), Some(to), Some(from)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if let (Some(pin), Some(wire)) = (
            to.strip_prefix(&prefix),
            from.strip_prefix(&format!("{short}_")),
        ) {
            pin_of.entry(wire.to_owned()).or_insert(pin.to_owned());
        }
    }
    let where_ = tile.rsplit_once("_X").map_or("", |(_, rest)| rest);
    let beside = format!("INT_{side}_X{where_}");
    let mut out = BTreeSet::new();
    for (t, feature) in &image.features {
        if *t != beside {
            continue;
        }
        let Some(sink) = feature.split('.').next() else {
            continue;
        };
        // Not `BYP_ALT` and `FAN_ALT`: those also lead to a bounce wire, and
        // both tools route other signals through them, so a pip into one does
        // not say the pin is driven. What such a pin carries is the mux bit's
        // to say where it has one, and the simulation's where it has not.
        let wire = sink
            .replace("IMUX_L", "IMUX")
            .replace("CTRL_L", "CTRL")
            .replace("CLK_L", "CLK");
        if let Some(pin) = pin_of.get(&wire) {
            out.insert(pin.clone());
        }
    }
    out
}

/// Which of a slice's pins the interconnect has a pip into a `BYP_ALT` or
/// `FAN_ALT` for, which [`driven_pins`] leaves out because either tool may be
/// passing through to a bounce wire. True for a pin that is driven and possibly
/// for one that is not.
fn driven_hops(root: &str, image: &Image, tile: &str, slice: &str) -> BTreeSet<String> {
    let kind = tile.split("_X").next().unwrap_or(tile);
    let text =
        std::fs::read_to_string(format!("{root}/artix7/ppips_{}.db", kind.to_lowercase())).unwrap();
    let side = if kind.ends_with("_L") { "L" } else { "R" };
    let short = kind.split('_').next().unwrap_or(kind);
    let prefix = format!("{short}_{}_", wire_prefix(tile, slice));
    let mut pin_of: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        let mut parts = name.split('.');
        let (Some(_), Some(to), Some(from)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if let (Some(pin), Some(wire)) = (
            to.strip_prefix(&prefix),
            from.strip_prefix(&format!("{short}_")),
        ) {
            pin_of.entry(wire.to_owned()).or_insert(pin.to_owned());
        }
    }
    let where_ = tile.rsplit_once("_X").map_or("", |(_, rest)| rest);
    let beside = format!("INT_{side}_X{where_}");
    let mut out = BTreeSet::new();
    for (t, feature) in &image.features {
        if *t != beside {
            continue;
        }
        let Some(sink) = feature.split('.').next() else {
            continue;
        };
        let wire = sink.replace("BYP_ALT", "BYP").replace("FAN_ALT", "FAN");
        if let Some(pin) = pin_of.get(&wire) {
            out.insert(pin.clone());
        }
    }
    out
}

/// A lane's letter out of a feature or a pin, as `L`.
fn lane(text: &str, letter: char) -> String {
    text.replace(letter, "L")
}

/// **What each latch of an image is**, sorted: for every slice with `LATCH`
/// set, the slice-wide bits, each element's own, and the pins driven for them.
fn latches(root: &str, image: &Image) -> Vec<String> {
    let mut slices: BTreeMap<(&str, &str), BTreeSet<&str>> = BTreeMap::new();
    for (tile, feature) in &image.features {
        if let Some((slice, rest)) = feature.split_once('.')
            && slice.starts_with("SLICE")
        {
            slices.entry((tile, slice)).or_default().insert(rest);
        }
    }
    let mut out = Vec::new();
    for ((tile, slice), features) in &slices {
        if !features.contains("LATCH") {
            continue;
        }
        let wide: Vec<&str> = ["CEUSEDMUX", "CLKINV", "FFSYNC", "LATCH", "SRUSEDMUX"]
            .into_iter()
            .filter(|f| features.contains(f))
            .collect();
        let pins = driven_pins(root, image, tile, slice);
        let mut control: Vec<&str> = ["CLK", "SR"]
            .into_iter()
            .filter(|p| pins.contains(*p))
            .collect();
        control.sort_unstable();
        let mut elements = Vec::new();
        for letter in ['A', 'B', 'C', 'D'] {
            let mux = format!("{letter}FFMUX.");
            let source = features.iter().find_map(|f| f.strip_prefix(mux.as_str()));
            let Some(source) = source else { continue };
            let mut element = vec![lane(source, letter)];
            for bit in ["ZINI", "ZRST"] {
                if features.contains(format!("{letter}FF.{bit}").as_str()) {
                    element.push(bit.to_owned());
                }
            }
            elements.push(element.join(" "));
        }
        elements.sort();
        out.push(format!(
            "{} [{}] {}",
            wide.join(" "),
            control.join(" "),
            elements.join("; ")
        ));
    }
    out.sort();
    out
}

/// The cells a design here may name that `examples/husky/sim` has no model of.
const SIMULATED: [&str; 25] = [
    "LD",
    "LD_1",
    "LDC",
    "LDC_1",
    "LDCE",
    "LDCE_1",
    "LDE",
    "LDE_1",
    "LDP",
    "LDP_1",
    "LDPE",
    "LDPE_1",
    "SRL16",
    "SRL16E",
    "SRL16E_1",
    "SRL16_1",
    "SRL32E",
    "SRLC16",
    "SRLC16E",
    "SRLC16E_1",
    "SRLC16_1",
    "SRLC32E",
    "CFGLUT5",
    "MUXF7",
    "MUXF8",
];

/// The source and the routed netlist side by side over 600 cycles of a
/// pseudo-random `si`: the two `so`s agree from cycle 200, wherever the source's
/// is known.
///
/// Not from the start: the source's registers power up unknown in simulation and
/// the netlist's at their `INIT`, and a register that holds what it held, as a
/// latch does until it next opens, carries that difference forward. Two hundred
/// cycles clear the longest chain here.
fn computes_the_same(design: &str, built: &Built) -> Result<(), String> {
    let out = built.routed.parent().unwrap().to_path_buf();
    let text = std::fs::read_to_string(&built.routed).unwrap();
    let from = format!("module {design} ");
    if !text.contains(&from) {
        return Err(format!("the routed netlist's top is not `{design}`"));
    }
    let netlist = out.join("routed_renamed.v");
    std::fs::write(
        &netlist,
        text.replacen(&from, &format!("module {design}_routed "), 1),
    )
    .unwrap();
    let tb = out.join("tb.v");
    std::fs::write(
        &tb,
        format!(
            "module tb;\n\
             reg clk = 0; reg si = 0; wire a, b; integer i, wrong = 0, known = 0;\n\
             reg [31:0] lfsr = 32'hACE1ACE1;\n\
             {design} rtl (.clk(clk), .si(si), .so(a));\n\
             {design}_routed routed (.clk(clk), .si(si), .so(b));\n\
             initial begin\n\
               for (i = 0; i < 600; i = i + 1) begin\n\
                 lfsr = {{lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]}};\n\
                 si = lfsr[0];\n\
                 #5 clk = 1; #1;\n\
                 if (i >= 200 && (a === 1'b0 || a === 1'b1)) begin\n\
                   known = known + 1;\n\
                   if (a !== b) wrong = wrong + 1;\n\
                 end\n\
                 #4 clk = 0; #5;\n\
                 if (i >= 200 && (a === 1'b0 || a === 1'b1)) begin\n\
                   known = known + 1;\n\
                   if (a !== b) wrong = wrong + 1;\n\
                 end\n\
               end\n\
               if (wrong == 0 && known > 300) $display(\"PASS: %0d compared\", known);\n\
               else $display(\"FAIL: %0d wrong of %0d compared\", wrong, known);\n\
               $finish;\n\
             end\n\
             endmodule\n"
        ),
    )
    .unwrap();
    // The cells the Husky's models do not have, from the library the vendor's
    // own simulator checked (`tests/fpga_xray_cells.rs`).
    let names: Vec<String> = SIMULATED.iter().map(|n| (*n).to_owned()).collect();
    let cells = out.join("cells.v");
    std::fs::write(&cells, reticle::fpga::cell_source("xc7", &names).unwrap()).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["sim", "--quiet", "--top", "tb"])
        .arg(&tb)
        .arg(fixtures().join(format!("{design}.v")))
        .arg(&netlist)
        .arg(&cells)
        .arg(manifest().join("examples/husky/sim/xilinx_cells.v"))
        .arg(manifest().join("examples/husky/sim/xilinx_primitives.v"))
        .output()
        .expect("the reticle binary runs");
    let stdout = String::from_utf8_lossy(&run.stdout);
    if stdout.contains("PASS:") {
        Ok(())
    } else {
        Err(format!(
            "{stdout}{}",
            String::from_utf8_lossy(&run.stderr)
                .lines()
                .take(12)
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

/// The latch designs, and what each one is for.
const LATCH_DESIGNS: [&str; 26] = [
    "infer_latch",
    "infer_latch_clr",
    "infer_latch_en",
    "infer_latch_init",
    "infer_latch_low",
    "infer_latch_pre",
    "infer_latch_wide",
    "ld",
    "ld_1",
    "ld_init1",
    "ldc",
    "ldc_1",
    "ldce",
    "ldce_1",
    "ldce_g_inv",
    "ldce_gclk",
    "ldce_init1",
    "ldce_two",
    "lde",
    "lde_1",
    "ldp",
    "ldp_1",
    "ldpe",
    "ldpe_1",
    "ldpe_g_inv",
    "ldpe_init0",
];

/// One latch design against Vivado, with the words a person needs if it is not.
fn compare_latch(root: &str, vivado: &BTreeMap<String, Image>, design: &str) -> Result<(), String> {
    let built = build(root, design).map_err(|e| format!("{design} would not build: {e}"))?;
    let theirs = latches(root, &vivado[design]);
    let ours = latches(root, &built.image);
    // Vivado fills a slice before opening another and this placer need not:
    // what must agree is what each slice says, and how many elements there are
    // of each kind, not how they are grouped.
    let spread = |lines: &[String]| -> Vec<String> {
        let mut out = Vec::new();
        for line in lines {
            let (head, elements) = line.rsplit_once("] ").unwrap();
            for element in elements.split("; ") {
                out.push(format!("{head}] {element}"));
            }
        }
        out.sort();
        out
    };
    if spread(&theirs) != spread(&ours) {
        return Err(format!(
            "{design}:\n  Vivado  {theirs:#?}\n  Reticle {ours:#?}"
        ));
    }
    // `infer_latch_en`'s gate is a lookup table, as it is in Vivado's, so in a
    // simulation with no delays it closes one step after its data has moved,
    // and the latch takes the new value where the source takes the old. That
    // is the hazard a gate made of logic has on a part as well, and no fault of
    // the netlist's, so what it holds cannot be compared here.
    if design == "infer_latch_en" {
        return Ok(());
    }
    computes_the_same(design, &built).map_err(|e| format!("{design}: {e}"))
}

/// **Four latches against Vivado, in the gate.** The forms a mistake would show
/// in soonest: the default, an inverted gate, a preset that powers up zero, and
/// an inferred clear.
#[test]
fn a_latch_is_what_vivado_wrote() {
    let (Some(root), Some(vivado)) = (chipdb(), vivado()) else {
        return;
    };
    let wrong: Vec<String> = ["ldce", "ld_1", "ldpe_init0", "infer_latch_clr"]
        .iter()
        .filter_map(|design| compare_latch(&root, &vivado, design).err())
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **All of them.** A build each, which is why it is ignored; `cargo test --
/// --ignored`.
#[test]
#[ignore = "builds every latch design to a bitstream, and needs the Project X-Ray database"]
fn every_latch_is_what_vivado_wrote() {
    let (Some(root), Some(vivado)) = (chipdb(), vivado()) else {
        return;
    };
    let wrong: Vec<String> = LATCH_DESIGNS
        .iter()
        .filter_map(|design| compare_latch(&root, &vivado, design).err())
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **What Vivado refuses, this refuses, and says why.** An inverted clear has no
/// bit: Vivado's placer gives up on it ("Placer could not place all instances",
/// and no bitstream in the fixture). Would catch the parameter ignored, which
/// would have built a latch that clears on the wrong level.
#[test]
fn an_inverted_clear_is_refused_as_vivado_refuses_it() {
    let (Some(root), Some(vivado)) = (chipdb(), vivado()) else {
        return;
    };
    assert!(!vivado["ldce_clr_inv"].built);
    let Err(words) = build(&root, "ldce_clr_inv") else {
        panic!("an inverted clear built");
    };
    assert!(words.contains("IS_CLR_INVERTED"), "{words}");
}

/// **A latch sets a bit for its whole slice, so nothing else may be a flip-flop
/// there**, on a design where a gate is the same global clock as the flip-flops'
/// beside it, so that they share `CLK`. It says what the finished slice holds. It
/// would **not** catch `place::slice_mode` not knowing a latch: mutated so, this
/// build's placement still does not happen to put them together, and it passed.
/// `a_latch_shares_a_slice_only_with_a_latch_of_its_own_polarity`, in `place.rs`,
/// is what catches that. Vivado refuses the same pair by name (`Cannot set LOC
/// property`), in `docs/fpga-vivado-parity.md`.
#[test]
fn no_slice_holds_a_latch_and_a_flip_flop() {
    let Some(root) = chipdb() else { return };
    if !fixtures().join("ldce_gclk.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let built = build(&root, "ldce_gclk").unwrap();
    let mut slices: BTreeMap<(&str, &str), BTreeSet<&str>> = BTreeMap::new();
    for (tile, feature) in &built.image.features {
        if let Some((slice, rest)) = feature.split_once('.')
            && slice.starts_with("SLICE")
        {
            slices.entry((tile, slice)).or_default().insert(rest);
        }
    }
    let latched = slices.values().filter(|f| f.contains("LATCH")).count();
    assert_eq!(latched, 1, "{slices:#?}");
    for features in slices.values().filter(|f| f.contains("LATCH")) {
        let elements = features.iter().filter(|f| f.contains("FFMUX.")).count();
        assert_eq!(elements, 1, "the latch alone: {features:#?}");
        assert!(!features.contains("FFSYNC"), "{features:#?}");
    }
    assert!(built.stderr.contains("routed"), "{}", built.stderr);
}

/// **What each shift register of an image is**, sorted: for every lane with
/// `SRL` set, whether it is the sixteen-bit form, what it holds, and which of
/// its address pins the interconnect drives, with the slice's `WEMUX.CE` and
/// `CLKINV`.
///
/// `exact` adds where its data comes from and whether its last bit leaves the
/// slice. Those are what differ where Vivado has chosen differently and this
/// flow's result is the same function: a chain inside a slice, where this has
/// no cascade yet, and a sixteen-bit form it registers on the five-input half,
/// where the contents are the lower half of the table and not the upper.
fn shift_registers(root: &str, image: &Image, exact: bool) -> Vec<String> {
    let mut slices: BTreeMap<(&str, &str), BTreeSet<&str>> = BTreeMap::new();
    let mut tables: BTreeMap<(&str, &str, char), u64> = BTreeMap::new();
    for (tile, feature) in &image.features {
        let Some((slice, rest)) = feature.split_once('.') else {
            continue;
        };
        if !slice.starts_with("SLICE") {
            continue;
        }
        slices.entry((tile, slice)).or_default().insert(rest);
        if let Some(index) = rest.get(1..).and_then(|r| r.strip_prefix("LUT.INIT[")) {
            let letter = rest.chars().next().unwrap();
            let bit: u32 = index.trim_end_matches(']').parse().unwrap();
            *tables.entry((tile, slice, letter)).or_default() |= 1u64 << bit;
        }
    }
    let mut out = Vec::new();
    for ((tile, slice), features) in &slices {
        let pins = driven_pins(root, image, tile, slice);
        for letter in ['A', 'B', 'C', 'D'] {
            if !features.contains(format!("{letter}LUT.SRL").as_str()) {
                continue;
            }
            let small = features.contains(format!("{letter}LUT.SMALL").as_str());
            let table = tables.get(&(*tile, *slice, letter)).copied().unwrap_or(0);
            // Every other bit, since each is written twice; and of a sixteen-bit
            // form, whichever half holds it.
            let source = if small {
                (table >> 32) | (table & 0xFFFF_FFFF)
            } else {
                table
            };
            let mut held = 0u64;
            for k in 0..if small { 16 } else { 32 } {
                held |= ((source >> (2 * k)) & 1) << k;
            }
            // The rest of a chain behind a cascade is an `SRLC32E` read below 16,
            // its fifth address bit tied low, where Vivado's is the sixteen-bit
            // form on the five-input half. The same bits, the same depth.
            let tied_low = !exact && !small && pins.contains(&format!("{letter}6"));
            let mut line = vec![format!(
                "{} {held:08X}",
                if small || tied_low { "16" } else { "32" }
            )];
            // Which half holds it is the whole of what the table's `A6` selects:
            // an upper half read through an `A6` that reads one, or nothing.
            if exact && small && table != 0 {
                line.push(if table >> 32 != 0 { "upper" } else { "lower" }.to_owned());
            }
            for bit in ["WEMUX.CE", "CLKINV"] {
                if features.contains(bit) {
                    line.push(bit.to_owned());
                }
            }
            let address: Vec<String> = (1..=if tied_low { 5 } else { 6 })
                .filter(|n| pins.contains(&format!("{letter}{n}")))
                .map(|n| format!("L{n}"))
                .collect();
            line.push(format!("[{}]", address.join(" ")));
            if exact {
                let mux = format!("{letter}LUT.DI1MUX.{letter}I");
                line.push(
                    if small {
                        if features.contains(mux.as_str()) {
                            "from-LI"
                        } else {
                            "from-LX"
                        }
                    } else if letter == 'D' || features.contains(mux.as_str()) {
                        "from-LI"
                    } else {
                        "from-next-lane"
                    }
                    .to_owned(),
                );
                if letter == 'A' && features.contains("DOUTMUX.MC31") {
                    line.push("MC31-out".to_owned());
                }
                // The pins whose sinks also lead to a bounce wire, here where
                // the two tools' choices are the same, so that one left
                // undriven shows: a shift register reading the wrong one of
                // `<L>I` and `<L>X` sets no other bit that would say so.
                let hops = driven_hops(root, image, tile, slice);
                let data = if small {
                    format!("{letter}X")
                } else if letter == 'D' {
                    "DI".to_owned()
                } else {
                    format!("{letter}I")
                };
                if line.last().is_some_and(|l| l != "from-next-lane") && hops.contains(&data) {
                    line.push("data-driven".to_owned());
                }
                if hops.contains("CE") {
                    line.push("enable-driven".to_owned());
                }
            }
            out.push(line.join(" "));
        }
    }
    out.sort();
    out
}

/// One shift register design against Vivado.
fn compare_srl(
    root: &str,
    vivado: &BTreeMap<String, Image>,
    design: &str,
    exact: bool,
) -> Result<(), String> {
    let built = build(root, design).map_err(|e| format!("{design} would not build: {e}"))?;
    let theirs = shift_registers(root, &vivado[design], exact);
    let ours = shift_registers(root, &built.image, exact);
    if theirs != ours {
        return Err(format!(
            "{design}:\n  Vivado  {theirs:#?}\n  Reticle {ours:#?}"
        ));
    }
    // Vivado writes an `FDRE` on a falling edge as one with `IS_C_INVERTED`, and
    // this flow as an `FDRE_1`, so the two are counted together. A tail of up to
    // sixteen bits behind a thirty-two-bit stage is an `SRLC32E` here, so the
    // shift registers are too.
    let flip_flops = |image: &Image| -> usize {
        image
            .cells
            .iter()
            .filter(|(name, _)| matches!(name.as_str(), "FDRE" | "FDRE_1"))
            .map(|(_, n)| *n)
            .sum()
    };
    if flip_flops(&vivado[design]) != flip_flops(&built.image) {
        return Err(format!(
            "{design}: {} FDRE against Vivado's {}",
            flip_flops(&built.image),
            flip_flops(&vivado[design])
        ));
    }
    let srls = |image: &Image| -> usize {
        image
            .cells
            .iter()
            .filter(|(name, _)| name.starts_with("SRL"))
            .map(|(_, n)| *n)
            .sum()
    };
    if srls(&vivado[design]) != srls(&built.image) {
        return Err(format!(
            "{design}: {} shift registers against Vivado's {}",
            srls(&built.image),
            srls(&vivado[design])
        ));
    }
    computes_the_same(design, &built).map_err(|e| format!("{design}: {e}"))
}

/// The forms this flow writes as Vivado does: their bits, their ends, and their
/// data pins.
const EXACT_SRL: [&str; 15] = [
    "srlc32e_two_q31",
    "srlc32e_cascade2",
    "srlc32e_cascade4",
    "srlc32e_cascade5",
    "srl16e_1",
    "srl16e_clk_inv",
    "srl16e_two",
    "srlc16",
    "srlc16e",
    "srlc16e_q15",
    "srlc32e",
    "srlc32e_ce1",
    "srlc32e_clk_inv",
    "srlc32e_only_q31",
    "srlc32e_q31",
];

/// The same cells, where Vivado has chosen the five-input half for a sixteen-bit
/// form it can register, and chains. What they hold, what drives their addresses,
/// how many there are and how many flip-flops are left is still compared.
const CLOSE_SRL: [&str; 40] = [
    "chain300",
    "copies",
    "chain8_wide",
    "srl16e_fixed0",
    "srl16e_fixed15",
    "srlc32e_fixed31",
    "chain100",
    "chain16",
    "chain17",
    "chain18",
    "chain2",
    "chain3",
    "chain32",
    "chain33",
    "chain34",
    "chain35",
    "chain4",
    "chain5",
    "chain64",
    "chain66",
    "chain8",
    "chain8_arst",
    "chain8_async_reg",
    "chain8_dont_touch",
    "chain8_en",
    "chain8_init",
    "chain8_keep",
    "chain8_neg",
    "chain8_no_extract",
    "chain8_rst",
    "chain8_set",
    "chain8_tap",
    "chain8_tap_each",
    "pure2",
    "pure3",
    "pure4",
    "srl16",
    "srl16e",
    "srl16e_ce1",
    "srl16e_init",
];

/// **The shift registers that matter most against Vivado, in the gate**: both
/// forms, an enable tied high, its last bit read, inferred chains, chains that
/// must stay flip-flops, and flip-flops that are copies and are not.
#[test]
fn a_shift_register_is_what_vivado_wrote() {
    let (Some(root), Some(vivado)) = (chipdb(), vivado()) else {
        return;
    };
    let mut wrong = Vec::new();
    for design in [
        "srlc32e",
        "srl16e_1",
        "srlc32e_ce1",
        "srlc32e_q31",
        "srlc16e",
        "srlc32e_cascade5",
        "srlc32e_two_q31",
    ] {
        wrong.extend(compare_srl(&root, &vivado, design, true).err());
    }
    for design in [
        "chain8",
        "chain8_rst",
        "pure3",
        "chain35",
        "chain300",
        "chain64",
        "chain8_en",
        "chain8_keep",
        "chain8_async_reg",
        "chain8_init",
        "copies",
        "srl16e_fixed0",
    ] {
        wrong.extend(compare_srl(&root, &vivado, design, false).err());
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **All of them.** A build each; `cargo test -- --ignored`.
#[test]
#[ignore = "builds every shift register design to a bitstream, and needs the Project X-Ray database"]
fn every_shift_register_is_what_vivado_wrote() {
    let (Some(root), Some(vivado)) = (chipdb(), vivado()) else {
        return;
    };
    let mut wrong = Vec::new();
    for design in EXACT_SRL {
        wrong.extend(compare_srl(&root, &vivado, design, true).err());
    }
    for design in CLOSE_SRL {
        wrong.extend(compare_srl(&root, &vivado, design, false).err());
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **A long chain stays inside a slice wherever it can**: of a chain's shift
/// registers, one takes its data from the interconnect for every slice it needs
/// and the rest from the lane above. `chain300` is ten of them in three slices, so
/// it has seven that do. Would catch a chain that does not cascade, which is the
/// same function with a route a stage: nothing else here says it is one; and a
/// run of more than four, which no site can hold.
#[test]
fn a_long_chain_cascades_inside_a_slice() {
    let Some(root) = chipdb() else { return };
    if !fixtures().join("chain300.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let built = build(&root, "chain300").unwrap();
    let lanes = shift_registers(&root, &built.image, true);
    let cascading = lanes
        .iter()
        .filter(|line| line.contains("from-next-lane"))
        .count();
    assert_eq!(lanes.len(), 10, "{lanes:#?}");
    assert_eq!(cascading, 7, "{lanes:#?}");
}

/// **Where this differs from Vivado on purpose, pinned**, so that closing one has
/// to say so here:
///
/// - `srlc32e_fixed5`: Vivado narrows an `SRLC32E` read at a fixed address under
///   16 to an `SRL16E`. It stays what it was named here.
/// - `chain8_srl_reg`: `srl_style = "register"` is honoured here, and Vivado 2026.1
///   made it a shift register all the same.
/// - `mem_shift*`: a chain read at an address that moves is flip-flops and a
///   multiplexer here, and an `SRLC32E` with its `A` wired in Vivado's.
#[test]
#[ignore = "builds several designs to a bitstream, and needs the Project X-Ray database"]
fn where_a_shift_register_differs_from_vivado() {
    let (Some(root), Some(vivado)) = (chipdb(), vivado()) else {
        return;
    };
    let count = |image: &Image, prefix: &str| -> usize {
        image
            .cells
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(_, n)| *n)
            .sum()
    };
    let built = build(&root, "srlc32e_fixed5").unwrap();
    computes_the_same("srlc32e_fixed5", &built).unwrap();
    assert_eq!(count(&vivado["srlc32e_fixed5"], "SRLC32E"), 0);
    assert_eq!(count(&built.image, "SRLC32E"), 1);
    let built = build(&root, "chain8_srl_reg").unwrap();
    computes_the_same("chain8_srl_reg", &built).unwrap();
    assert_eq!(count(&vivado["chain8_srl_reg"], "SRL"), 1);
    assert_eq!(count(&built.image, "SRL"), 0);
    for design in ["mem_shift", "mem_shift16"] {
        let built = build(&root, design).unwrap();
        computes_the_same(design, &built).unwrap();
        assert_eq!(count(&vivado[design], "SRL"), 1);
        assert_eq!(count(&built.image, "SRL"), 0);
    }
}

/// **An enable tied high is left unrouted, and nothing else may use its wire.** It
/// reads one because nothing drives it, as in Vivado's, so a signal routed through
/// `FAN_ALT7` on its way to a bounce wire would write itself into the shift
/// register's enable with every bit decoding. Would catch a route there on this
/// design; the wire is closed to every signal
/// (`XrayFabric::block_ram_ground_fans`) so that none is, and a mutation that
/// reopens it passes here unless the router happens to want it.
#[test]
fn an_enable_tied_high_is_not_driven() {
    let (Some(root), Some(vivado)) = (chipdb(), vivado()) else {
        return;
    };
    let any_fan7 = |image: &Image| {
        let mut tiles = BTreeSet::new();
        for (tile, feature) in &image.features {
            if feature.ends_with("LUT.SRL") {
                let (_, rest) = tile.rsplit_once("_X").unwrap();
                tiles.insert(format!(
                    "INT_{}_X{rest}",
                    if tile.contains("_L_") { "L" } else { "R" }
                ));
            }
        }
        image
            .features
            .iter()
            .any(|(tile, feature)| tiles.contains(tile) && feature.starts_with("FAN_ALT7."))
    };
    assert!(
        !any_fan7(&vivado["srlc32e_ce1"]),
        "Vivado's reads one undriven"
    );
    let built = build(&root, "srlc32e_ce1").unwrap();
    assert!(!any_fan7(&built.image), "a route went through the enable");
}

/// **`--srl-min 0` is the netlist from before**: a chain stays flip-flops, and a
/// design with nothing to find is the same bitstream either way. Would catch the
/// option ignored, or an extraction with something to say about a design with no
/// chain; would not catch a netlist changed by something else in this file, which
/// is what building `qor/` against the commit before is for.
#[test]
fn a_design_with_no_chain_is_unchanged() {
    let Some(root) = chipdb() else { return };
    if !fixtures().join("ld.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let bits = |design: &str, extra: &[&str]| -> (Vec<u8>, String) {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("srl-min-{design}-{}", extra.len()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let bit = out.join("out.bit");
        let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args([
                "fpga",
                "--device",
                "xc7a100t-ftg256",
                "--chipdb",
                &root,
                "--top",
                design,
                "--report",
            ])
            .args(extra)
            .arg("--constraints")
            .arg(fixtures().join(format!("{design}.rcf")))
            .arg("--output-dir")
            .arg(&out)
            .arg("--bitstream")
            .arg(&bit)
            .arg(fixtures().join(format!("{design}.v")))
            .output()
            .expect("the reticle binary runs");
        assert!(run.status.success());
        (
            std::fs::read(&bit).unwrap(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&run.stdout),
                String::from_utf8_lossy(&run.stderr)
            ),
        )
    };
    let (with, _) = bits("no_chain", &[]);
    let (without, _) = bits("no_chain", &["--srl-min", "0"]);
    assert!(with == without, "a design with no chain moved");
    let (_, text) = bits("chain8", &[]);
    assert!(text.contains("x SRL16E"), "{text}");
    let (_, text) = bits("chain8", &["--srl-min", "0"]);
    assert!(!text.contains("x SRL16E"), "{text}");
}
