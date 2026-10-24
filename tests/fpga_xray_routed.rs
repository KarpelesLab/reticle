//! The design a 7-series bitstream is made from computes what its RTL does.
//!
//! After routing, the flow pairs lookup tables and rewrites the truth table
//! of every table the router reached on other inputs than the mapper named
//! (`xray::lutswap`): on a small design most of them. The bitstream is
//! generated from that rewritten design, and every check the flow makes of
//! it — every bit decodes, every arc is one the router chose — is about
//! wires, not about what the tables compute. `--routed-netlist` writes that
//! design as Verilog; this test simulates it against the RTL, cycle by
//! cycle, with `examples/husky/sim`'s cell models.
//!
//! Would catch a table's inputs permuted one way and its truth table the
//! other, a pair's halves swapped or its `I5` misused, or a flip-flop or
//! carry configured as something else in the written netlist; would not
//! catch a bitstream that encodes that netlist wrongly, which the decode
//! checks are for, nor anything about timing. Skips without a Project
//! X-Ray database.

#![cfg(all(feature = "fpga", feature = "sim", feature = "cli"))]

use std::path::Path;
use std::process::Command;

/// The database root, or `None` (and why) when there is none.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a35tcpg236-1/package_pins.csv";
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
                "skipped: needs a Project X-Ray database with the CPG236 package; run \
                 `reticle fetch prjxray-db` or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

#[test]
fn a_routed_netlist_computes_what_its_rtl_does() {
    let Some(root) = chipdb() else { return };
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = here.join("testdata/fpga/routed");
    if !dir.join("dut.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("routed-netlist");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let routed = out.join("routed.v");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a35t-cpg236",
            "--chipdb",
            &root,
            "--top",
            "dut",
        ])
        .arg("--constraints")
        .arg(dir.join("dut.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(out.join("dut.bit"))
        .arg("--routed-netlist")
        .arg(&routed)
        .arg(dir.join("dut.v"))
        .output()
        .expect("the reticle binary runs");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    // The test is only worth its time if tables were moved.
    assert!(
        stderr.contains("take their inputs on other pins than mapped"),
        "{stderr}"
    );

    let text = std::fs::read_to_string(&routed).unwrap();
    let renamed = text.replacen("module dut ", "module dut_routed ", 1);
    assert_ne!(renamed, text, "the routed netlist's top is `dut`");
    let netlist = out.join("routed_renamed.v");
    std::fs::write(&netlist, renamed).unwrap();
    let sim = here.join("examples/husky/sim");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["sim", "--quiet", "--top", "tb"])
        .arg(dir.join("tb.v"))
        .arg(dir.join("dut.v"))
        .arg(&netlist)
        .arg(sim.join("xilinx_cells.v"))
        .arg(sim.join("xilinx_primitives.v"))
        .output()
        .expect("the reticle binary runs");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    assert!(stdout.contains("PASS:"), "{stdout}\n{stderr}");
}
