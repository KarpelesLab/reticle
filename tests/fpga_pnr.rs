//! Golden tests for Reticle's own placer, router and bitstream writer.
//!
//! Each case under `testdata/fpga/` is an iCE40 design in the IR text
//! format plus its constraints, taken all the way through
//! `fpga::implement`: synthesis and primitive mapping (which
//! `fpga_flow.rs` already covers), then placement, routing and the
//! bitstream.
//!
//! | File | What it holds |
//! |------|----------------|
//! | `<name>.place` | the site each instance was placed on |
//! | `<name>.route` | the per-iteration overuse and the pips per signal |
//! | `<name>.bits` | the set bits of the bitstream, tile by tile |
//! | `blinky_ice40.asc` | one full IceStorm-style bitstream |
//!
//! Only one full `.asc` is kept: a whole part is 184 kB of mostly
//! zeroes, and the `.bits` summary is the part a reviewer reads. The
//! `.asc` writer and reader are checked against each other for every
//! case regardless.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations after an intended
//! change, and read the diff before committing it.
//!
//! **What these files are not.** The architecture the flow places and
//! routes on is synthetic — see `fpga::arch::synthetic`, which says so at
//! length. A site name here is a site of a plausible iCE40-shaped fabric,
//! not of an iCE40, and the bitstream will not program a part. What these
//! tests prove is that the algorithms are right and the flow is
//! reproducible: the placement respects the constraints, the routing
//! reaches every sink from its driver, and the bitstream round-trips.
//!
//! One test runs `icepack` and `icebox_vlog` over the result when they
//! happen to be installed. It is not ignored: with no tool present it
//! says so and returns, and with one present it reports what the tool
//! said without failing the suite, since a synthetic architecture is not
//! one those tools have any reason to accept.

#![cfg(feature = "fpga")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::fpga::arch::{Arch, NodeId};
use reticle::fpga::{self, Bitstream, Constraints, FpgaOptions, Implementation, PnrOptions};
use reticle::ir::{Design, ModuleId};
use reticle::source::SourceMap;

/// The designs, all on the one part that has an architecture.
const CASES: [&str; 3] = ["blinky_ice40", "carry_ice40", "ram_ice40"];
/// The part they target.
const DEVICE: &str = "ice40-hx1k-tq144";

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga")
}

fn read(name: &str) -> String {
    let path = dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Compares `actual` with the file `name`, rewriting it under
/// `UPDATE_EXPECT`. An absent file counts as empty.
fn expect(name: &str, actual: &str, failures: &mut Vec<String>) {
    let path = dir().join(name);
    let expected = fs::read_to_string(&path).unwrap_or_default();
    if expected == actual {
        return;
    }
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        if actual.is_empty() {
            let _ = fs::remove_file(&path);
        } else {
            fs::write(&path, actual)
                .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        }
        return;
    }
    let diff = expected
        .lines()
        .zip(actual.lines())
        .enumerate()
        .find(|(_, (e, a))| e != a)
        .map_or_else(
            || {
                format!(
                    "expected {} lines, got {}",
                    expected.lines().count(),
                    actual.lines().count()
                )
            },
            |(i, (e, a))| format!("line {}:\n  expected: {e}\n  actual:   {a}", i + 1),
        );
    failures.push(format!(
        "{name}: mismatch (set UPDATE_EXPECT=1 to rewrite)\n{diff}"
    ));
}

/// Takes one case from its `.rtl` and `.rcf` to a bitstream.
fn run_case(name: &str) -> (Design, ModuleId, Implementation) {
    let device = fpga::target(DEVICE).expect("the built-in iCE40 part");
    let mut sources = SourceMap::new();
    let rtl = read(&format!("{name}.rtl"));
    let file = sources.add(format!("{name}.rtl"), rtl.clone()).unwrap();
    let mut design = Design::parse_text(&rtl, file)
        .unwrap_or_else(|diags| panic!("{name}: parse failed\n{}", diags.render(&sources)));
    let top = design
        .top
        .unwrap_or_else(|| panic!("{name}: no top module"));

    let rcf = read(&format!("{name}.rcf"));
    let rcf_file = sources.add(format!("{name}.rcf"), rcf.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::parse(&rcf, rcf_file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);

    let done = fpga::implement(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &PnrOptions::new(),
        &mut diags,
    )
    .unwrap_or_else(|e| panic!("{name}: the flow failed: {e}\n{}", diags.render(&sources)));
    assert!(!diags.has_errors(), "{name}:\n{}", diags.render(&sources));
    (design, top, done)
}

/// The routing golden: the convergence, then the pips per signal.
fn routing_text(done: &Implementation) -> String {
    let mut out = done.pnr.routing_report.to_text();
    out.push_str("signals:\n");
    let mut lines: Vec<String> = done
        .pnr
        .routing
        .routes()
        .map(|route| {
            format!(
                "  {} {} pip(s)\n",
                done.pnr.netlist.signals[route.signal].name,
                route.pips.len()
            )
        })
        .collect();
    lines.sort();
    out.push_str(&lines.concat());
    out
}

#[test]
fn golden_place_and_route() {
    let mut failures = Vec::new();
    for name in CASES {
        let (_, _, done) = run_case(name);
        let placement = done
            .pnr
            .placement
            .to_text(&done.pnr.netlist, &done.pnr.graph);
        expect(&format!("{name}.place"), &placement, &mut failures);
        expect(
            &format!("{name}.route"),
            &routing_text(&done),
            &mut failures,
        );
        let bitstream = done.bitstream().expect("a bitstream was asked for");
        expect(
            &format!("{name}.bits"),
            &bitstream.to_summary(),
            &mut failures,
        );
        if name == "blinky_ice40" {
            expect(
                &format!("{name}.asc"),
                &bitstream.write_asc(),
                &mut failures,
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The property that matters: every sink of every signal is reachable
/// from that signal's driver through the pips the router gave it.
///
/// The walk is written out here rather than delegated to
/// `Routing::verify`, so that the check does not share an implementation
/// with what it is checking. It also confirms that every pin of the
/// netlist was accounted for: a routed signal must have exactly as many
/// sinks as the netlist gives it.
#[test]
fn routing_implements_the_netlist() {
    for name in CASES {
        let (_, _, done) = run_case(name);
        let pnr = &done.pnr;
        let graph = &pnr.graph;
        let mut checked = 0usize;
        for signal in pnr.netlist.routable() {
            let route = pnr.routing.route(signal).unwrap_or_else(|| {
                panic!(
                    "{name}: `{}` has no route",
                    pnr.netlist.signals[signal].name
                )
            });
            // Which pip drives each node, for this signal alone.
            let mut driver: BTreeMap<NodeId, NodeId> = BTreeMap::new();
            for pip in &route.pips {
                let pip = graph.pip(*pip);
                assert!(
                    driver.insert(pip.to, pip.from).is_none(),
                    "{name}: `{}` drives one wire from two pips",
                    pnr.netlist.signals[signal].name
                );
            }
            // The driver's own pin really is the source of the route.
            let driving_pin = pnr.netlist.signals[signal].driver.expect("routable");
            let pin = &pnr.netlist.pins[driving_pin];
            let site = pnr.placement.site_of(pin.instance).expect("placed");
            assert_eq!(
                graph.sites[site].pin(&pin.role),
                Some(route.source),
                "{name}: the route of `{}` does not start at its driver",
                pnr.netlist.signals[signal].name
            );

            for sink in &pnr.netlist.signals[signal].sinks {
                let pin = &pnr.netlist.pins[*sink];
                let site = pnr.placement.site_of(pin.instance).expect("placed");
                let mut node = graph.sites[site]
                    .pin(&pin.role)
                    .expect("the architecture gives the pin a wire");
                let mut steps = 0;
                while node != route.source {
                    node = *driver.get(&node).unwrap_or_else(|| {
                        panic!(
                            "{name}: walking `{}` back from {}.{} reached {}, which nothing drives",
                            pnr.netlist.signals[signal].name,
                            pnr.netlist.instances[pin.instance].name,
                            pin.port,
                            graph.wire(node).full_name()
                        )
                    });
                    steps += 1;
                    assert!(
                        steps <= route.pips.len(),
                        "{name}: the route of `{}` loops",
                        pnr.netlist.signals[signal].name
                    );
                }
                checked += 1;
            }
        }
        assert!(checked > 0, "{name}: nothing was routed");
        // And the library's own check agrees.
        assert!(pnr.verify().is_empty(), "{name}: {:?}", pnr.verify());
        assert_eq!(pnr.routing_report.signals, pnr.netlist.routable().len());
        // The last iteration left nothing oversubscribed.
        let last = pnr.routing_report.iterations.last().expect("an iteration");
        assert_eq!(last.overused_nodes, 0, "{name}");
    }
}

/// The bitstream survives both formats, and the text one carries every
/// bit the generator set.
#[test]
fn the_bitstream_round_trips() {
    for name in CASES {
        let (_, _, done) = run_case(name);
        let bitstream = done.bitstream().expect("a bitstream");
        assert!(bitstream.ones() > 0, "{name}: an empty bitstream");

        let asc = bitstream.write_asc();
        let back = Bitstream::parse_asc(&asc)
            .unwrap_or_else(|e| panic!("{name}: the .asc does not read back: {e}"));
        assert_eq!(&back, bitstream, "{name}: the .asc lost something");
        assert_eq!(back.write_asc(), asc, "{name}: the .asc is not stable");

        let bin = bitstream.write_bin();
        let back = Bitstream::read_bin(&bin)
            .unwrap_or_else(|e| panic!("{name}: the .bin does not read back: {e}"));
        assert_eq!(&back, bitstream, "{name}: the .bin lost something");
        assert!(
            bin.len() < asc.len() / 4,
            "{name}: the binary form should be the compact one"
        );

        // Every pip the router used contributed its bits.
        for route in done.pnr.routing.routes() {
            for id in &route.pips {
                let tile = done.pnr.graph.pip(*id).tile;
                for bit in done.pnr.graph.pip_bits(*id) {
                    assert_eq!(
                        bitstream.get(tile, *bit),
                        Some(true),
                        "{name}: a pip's bit is not set"
                    );
                }
            }
        }
    }
}

/// A part with no routing architecture is told so, and pointed at the
/// export path instead of being half placed.
#[test]
fn a_part_without_an_architecture_is_reported() {
    let mut sources = SourceMap::new();
    let rtl = read("blinky_ecp5.rtl");
    let file = sources.add("blinky_ecp5.rtl", rtl.clone()).unwrap();
    let mut design = Design::parse_text(&rtl, file).unwrap();
    let top = design.top.unwrap();
    let device = fpga::target("ecp5-45f-CABGA381").unwrap();
    let mut diags = Diagnostics::new();
    let err = fpga::implement(
        &mut design,
        top,
        device,
        &Constraints::new(),
        &FpgaOptions::default(),
        &PnrOptions::new(),
        &mut diags,
    )
    .unwrap_err();
    assert_eq!(
        err,
        fpga::FlowError::NoArchitecture {
            device: "ecp5-45f-CABGA381".to_owned()
        }
    );
    assert!(err.to_string().contains("export it to nextpnr"), "{err}");
}

/// A hand-written architecture file is read, expanded and written back
/// unchanged, which is the path a database derived from Project IceStorm
/// would take.
#[test]
fn a_hand_written_architecture_loads() {
    let text = read("tiny.arch");
    let mut sources = SourceMap::new();
    let file = sources.add("tiny.arch", text.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let archs = Arch::parse(&text, file, &mut diags);
    assert_eq!(diags.render(&sources), "");
    assert_eq!(archs.len(), 1);
    let arch = &archs[0];
    assert_eq!(arch.name, "tiny");
    assert!(arch.serves("tiny-part"));
    assert_eq!((arch.width, arch.height), (3, 2));

    let graph = arch.build_graph();
    // One global plus eight wires in each of six tiles.
    assert_eq!(graph.nodes.len(), 1 + 6 * 8);
    assert_eq!(
        graph.site_counts(),
        vec![("ff".to_owned(), 6), ("lut".to_owned(), 6)]
    );
    // The span-2 line only resolves where it has somewhere to come from.
    assert_eq!(graph.dangling, 4);
    assert_eq!(arch.site_of_pin("A"), Some("X0Y0/lut"));

    // The writer and the reader agree, so a converter can generate the
    // format and diff its output.
    let written = arch.to_text();
    let file = sources.add("again.arch", written.clone()).unwrap();
    let again = Arch::parse(&written, file, &mut diags);
    assert_eq!(diags.render(&sources), "");
    assert_eq!(&again[0], arch);
    assert_eq!(again[0].to_text(), written);
}

/// True when `tool` is on the path.
fn installed(tool: &str) -> bool {
    std::process::Command::new("which")
        .arg(tool)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Runs the IceStorm tools over the generated `.asc` when they happen to
/// be installed.
///
/// The bitstream comes from a synthetic architecture, so `icepack` and
/// `icebox_vlog` have no reason to accept it; what this test is for is to
/// say what happened, which is what will matter the day a real database
/// is dropped in and the flow is tried for real. It therefore reports
/// rather than asserts, and with no tool installed it says so and
/// returns, the way the other environment-dependent tests in this
/// repository do.
#[test]
fn the_icestorm_tools_read_the_bitstream_if_they_are_installed() {
    use std::process::Command;

    let tools: Vec<&str> = ["icepack", "icebox_vlog"]
        .into_iter()
        .filter(|tool| installed(tool))
        .collect();
    if tools.is_empty() {
        println!(
            "neither icepack nor icebox_vlog is installed, so the bitstream was checked \
             against its own reader only (see `the_bitstream_round_trips`)"
        );
        return;
    }
    let (_, _, done) = run_case("blinky_ice40");
    let asc = done.bitstream().expect("a bitstream").write_asc();
    let dir = std::env::temp_dir().join("reticle-fpga-pnr");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("blinky.asc");
    fs::write(&path, &asc).unwrap();

    let mut refused = Vec::new();
    for tool in tools {
        let mut command = Command::new(tool);
        command.arg(&path);
        if tool == "icepack" {
            command.arg(dir.join("blinky.bin"));
        }
        let output = command
            .output()
            .unwrap_or_else(|e| panic!("cannot run {tool}: {e}"));
        println!(
            "{tool} {} -> {}\n{}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            refused.push(tool);
        }
    }
    if !refused.is_empty() {
        println!(
            "{} refused the bitstream, which is what a synthetic architecture deserves: \
             the tile contents do not match the database those tools carry. See \
             fpga::arch::synthetic.",
            refused.join(" and ")
        );
    }
}
