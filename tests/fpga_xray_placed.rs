//! Probes whose own bitstream is Vivado's bitstream of the same placement and
//! routes, feature for feature, and what the implementation written for
//! Vivado says so that it is.
//!
//! # The oracle
//!
//! `run_placed.sh` gives Vivado 2026.1 Reticle's own placement and routes of a
//! primitive probe (`reticle fpga --vivado-placement`), and Vivado writes the
//! bitstream of them; `testdata/fpga/vivado/primitives/placed.txt` is that
//! bitstream decoded, for the probes whose classes of difference were closed
//! on 2026-11-19 (`docs/fpga-vivado-parity.md`, "The last classes of the
//! ledger"). Only Vivado's answer is kept.
//!
//! # What these would and would not catch
//!
//! [`each_probe_is_vivados_bitstream_of_its_routes`] builds each probe as
//! `run_placed.sh` builds it and asks that the decoded bitstream be the
//! fixture's, every feature. Each of these was taken out and failed it:
//!
//! - a clock manager's DRP pins tied low in the source, or by a `_BASE`,
//!   grounded from the interconnect's fans rather than from a lookup table of
//!   zeros (`xc7.dev`'s `absorbs`): the `MMCME2_BASE` and `PLLE2_BASE` probes;
//! - the order Vivado grounds a manager's pins in, and a bounce one tie has
//!   grounded reused by the next (`FAN_BOUNCE5` onto `IMUX_L1`);
//! - a `CLKIN2` whose bounce `CLKIN1` has taken, grounded from an empty
//!   slice's table (`ER1BEG1.LOGIC_OUTS12`): the `PLLE2_*` probes, and the
//!   `MMCME2_BASE` on the XC7A35T;
//! - the clock row's `HCLK_CMT_*_DRP_WRITE` for a manager whose `DWE` is a
//!   signal: the `_ADV` probes;
//! - `ENABLE_BUFFER` only where a leaf takes the track, not in every
//!   horizontal clock tile it crosses: the `BUFH` and `BUFHCE` probes;
//! - `ENABLE_REGIONAL` where a regional line leaves for the tile beside: the
//!   `BUFR` probe;
//! - a clock row's input from the interconnect, `CLK_HROW_CK_INT_*_ACTIVE`:
//!   the `BUFH` and `BUFHCE` probes on the XC7A35T;
//! - a `BUFMRCE` built on a global buffer having its `I1` tied as a
//!   `BUFGCE`'s is: the `BUFMRCE` probe;
//! - a FIFO's `REGCE`, taken off the cell with `EN_SYN` false, left on its
//!   undriven `IMUX` rather than grounded: the `FIFO18E1` and `FIFO36E1`
//!   probes.
//!
//! It is a placement's test as well: a change to the placer or the router that
//! moves one of these probes fails it without anything being wrong, and the
//! fixture is then made again (`run_placed.sh`, `decode_placed.sh`,
//! `placed_features.py`). It would not catch a bit Vivado sets on another
//! design, nor what anything does on a part: **nothing here has been on a
//! board**.
//!
//! [`the_implementation_written_for_vivado_says_what_the_bitstream_does`] holds
//! the export fixes, which change nothing of Reticle's own bits and
//! everything of Vivado's: an `ISERDESE2`'s `CLKDIVP`, an input nothing reads
//! (with its pull on the port), a clock manager's outputs nothing reads and a
//! bank's `INTERNAL_VREF`, each of which made the probe differ under Vivado;
//! and a FIFO's `REGCE` and `RSTREG` with `EN_SYN` false (the netlist's, so
//! Reticle's own routes change too), a `BUFR` on a global buffer written as the
//! `BUFG` it is there, and a `RAM128X1D` written at all, without which Vivado
//! wrote no bitstream of the probe (`run_placed.sh`). Each taken out fails it.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn primitives() -> PathBuf {
    manifest().join("testdata/fpga/vivado/primitives")
}

/// The package file whose presence says the database can build for a device.
fn package(device: &str) -> String {
    format!("artix7/{}-1/package_pins.csv", device.replace('-', ""))
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

/// `placed.txt`: each `(probe, device)`'s decoded features.
fn fixture() -> BTreeMap<(String, String), BTreeSet<String>> {
    let text = std::fs::read_to_string(primitives().join("placed.txt")).unwrap();
    let mut out: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut at = (String::new(), String::new());
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("design ") {
            let (name, device) = rest.split_once(' ').unwrap();
            at = (name.to_owned(), device.to_owned());
            out.entry(at.clone()).or_default();
        } else if let Some(feature) = line.strip_prefix("  ") {
            out.entry(at.clone())
                .or_default()
                .insert(feature.to_owned());
        }
    }
    out
}

/// One probe built as `run_placed.sh` builds it.
struct Built {
    log: String,
    decoded: BTreeSet<String>,
    /// The implementation written for Vivado: netlist, placement, constraints.
    verilog: String,
    placement: String,
    xdc: String,
}

fn build(root: &str, name: &str, device: &str) -> Built {
    let dir = primitives();
    let rcf = if device == "xc7a35t-cpg236" {
        dir.join(device).join(format!("{name}.rcf"))
    } else {
        dir.join(format!("{name}.rcf"))
    };
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("placed")
        .join(device)
        .join(name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join(format!("{name}.bit"));
    let stem = out.join("x.tcl");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", device, "--chipdb", root, "--top", name])
        .args(["--timing", "--timing-driven", "--report"])
        .arg("--constraints")
        .arg(&rcf)
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg("--vivado-placement")
        .arg(&stem)
        .arg(dir.join(format!("{name}.v")))
        .output()
        .expect("the reticle binary runs");
    let log = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(run.status.success(), "{name} on {device}: {log}");
    let decoded = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", device, "--chipdb", root, "--decode"])
        .arg(&bit)
        .output()
        .expect("the reticle binary runs");
    let read = |p: PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    Built {
        log,
        decoded: String::from_utf8_lossy(&decoded.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect(),
        verilog: read(out.join("x.v")),
        placement: read(stem.clone()),
        xdc: read(out.join("x.xdc")),
    }
}

/// Every probe of the fixture, built once for both tests.
fn built() -> Option<&'static BTreeMap<(String, String), Built>> {
    static BUILT: OnceLock<Option<BTreeMap<(String, String), Built>>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let mut out = BTreeMap::new();
            for (name, device) in fixture().into_keys() {
                let root = chipdb(&package(&device))?;
                let one = build(&root, &name, &device);
                out.insert((name, device), one);
            }
            Some(out)
        })
        .as_ref()
}

/// **Reticle's own bitstream of each probe is Vivado's bitstream of the same
/// placement and routes**, feature for feature. See the module documentation
/// for what it holds.
#[test]
fn each_probe_is_vivados_bitstream_of_its_routes() {
    let Some(built) = built() else { return };
    let theirs = fixture();
    let mut wrong = Vec::new();
    for (key, one) in built {
        let want = &theirs[key];
        let only_ours: Vec<_> = one.decoded.difference(want).collect();
        let only_theirs: Vec<_> = want.difference(&one.decoded).collect();
        if !only_ours.is_empty() || !only_theirs.is_empty() {
            wrong.push(format!(
                "{} on {}: Reticle only {only_ours:?}; Vivado only {only_theirs:?}",
                key.0, key.1
            ));
        }
        assert!(
            one.log
                .contains("the interconnect arcs those bits select are exactly the ones"),
            "{} on {}: {}",
            key.0,
            key.1,
            one.log
        );
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **The implementation written for Vivado states what Reticle's bitstream
/// does**, where Vivado would otherwise write something else of the same
/// placement and routes. See the module documentation.
#[test]
fn the_implementation_written_for_vivado_says_what_the_bitstream_does() {
    let Some(built) = built() else { return };
    let get = |name: &str| &built[&(name.to_owned(), "xc7a100t-ftg256".to_owned())];

    // `CLKDIVP` on `CLKDIV`'s clock, which the flow takes off the cell.
    let iserdes = get("iserdese2_probe");
    let line = iserdes
        .verilog
        .lines()
        .find(|l| l.contains("ISERDESE2"))
        .unwrap();
    assert!(line.contains(".CLKDIVP("), "{line}");

    // An input nothing reads is no cell; its pull is the port's.
    let keeper = get("keeper_probe");
    assert!(!keeper.verilog.contains("(.I(o_pad)"), "{}", keeper.verilog);
    assert!(!keeper.verilog.contains("KEEPER "), "{}", keeper.verilog);
    assert!(
        keeper
            .placement
            .contains("set_property PULLTYPE KEEPER [get_ports {o_pad}]"),
        "{}",
        keeper.placement
    );

    // A clock manager's outputs nothing reads are open.
    for name in ["mmcme2_base_probe", "plle2_base_probe"] {
        let one = get(name);
        let line = one.verilog.lines().find(|l| l.contains("E2_ADV")).unwrap();
        assert!(line.contains(".CLKOUT0("), "{name}: {line}");
        assert!(!line.contains(".CLKOUT1("), "{name}: {line}");
        // And its DRP pins are the constant, not a table's net.
        assert!(line.contains(".DEN(1'b0)"), "{name}: {line}");
    }

    // A bank whose `SSTL18_I` input takes the internal reference says so.
    for name in ["ibuf_ibufdisable_probe", "ibuf_intermdisable_probe"] {
        let one = get(name);
        assert!(
            one.xdc
                .contains("set_property INTERNAL_VREF 0.900 [get_iobanks 35]"),
            "{name}: {}",
            one.xdc
        );
    }
    // And one with no referenced input says nothing of it.
    assert!(!keeper.xdc.contains("INTERNAL_VREF"), "{}", keeper.xdc);

    // A `BUFR` built on a global buffer is the `BUFG` it is there, with no
    // `CE` or `CLR`, which Vivado would refuse on a `BUFGCTRL` (12-1409).
    let bufmrce = get("bufmrce_probe");
    assert!(!bufmrce.verilog.contains("BUFR"), "{}", bufmrce.verilog);
    assert!(
        bufmrce
            .verilog
            .lines()
            .any(|l| l.trim_start().starts_with("BUFG c") && !l.contains(".CE(")),
        "{}",
        bufmrce.verilog
    );

    // A FIFO with `EN_SYN` false has no `REGCE` and a `RSTREG` tied low, as
    // Vivado's own flow leaves them; connected, Vivado refuses it (`REQP-35`,
    // `REQP-36`).
    for name in ["fifo18e1_probe", "fifo36e1_probe"] {
        let one = get(name);
        let line = one
            .verilog
            .lines()
            .find(|l| l.contains("FIFO") && l.contains("E1 "))
            .unwrap();
        assert!(!line.contains(".REGCE("), "{name}: {line}");
        assert!(line.contains(".RSTREG(1'b0)"), "{name}: {line}");
    }

    // A `RAM128X1D` is written, and placed by its site as Vivado expands it.
    let ram = get("ram128x1d_probe");
    assert!(ram.verilog.contains("RAM128X1D "), "{}", ram.verilog);
    assert!(
        ram.placement
            .lines()
            .any(|l| l.contains("_dut SLICE_X") && !l.contains('/')),
        "{}",
        ram.placement
    );
}
