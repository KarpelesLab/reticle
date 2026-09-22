//! Golden tests for whole IP projects.
//!
//! Each case under `testdata/ip/projects/` is a `reticle.proj` plus
//! whatever HDL the user of that project writes; the packages it depends
//! on live under `testdata/ip/packages/`. A case is taken all the way
//! through resolution and elaboration and compared against:
//!
//! | File | What it holds |
//! |------|----------------|
//! | `<case>.diag` | everything resolution and elaboration reported |
//! | `<case>.lock` | the `reticle.lock` the resolution would write |
//! | `<case>.build` | the build report: sources used, black boxes made |
//! | `<case>.rtl` | the elaborated design in the IR text format |
//!
//! The four cases are the four things that have to work:
//!
//! - `two_deps` — a project with two dependencies, one of which needs
//!   the other, and a third package neither names directly.
//! - `conflict` — two requirements no available version satisfies.
//! - `encrypted` — one vendor core with no readable source at all, and
//!   one with a behavioural model, so both black-box paths are taken.
//! - `crossbar` — an AXI4-Lite crossbar generated into the design, wired
//!   to two instances of a resolved IP package with `ip::bus::connect`,
//!   and (with the `sim` feature) driven with real transactions.
//!
//! The library performs no I/O, so the filesystem appears here exactly
//! once: a closure handed to [`PathProvider`]. That closure is the whole
//! of what the CLI would own.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations after an intended
//! change, and read the diff before committing it.

#![cfg(feature = "ip")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::ip::bus::{BusEndpoint, connect, wire};
use reticle::ip::{self, AddressRange, BusRole, Crossbar, PathProvider, Project, Resolved};
use reticle::ir::builder::ModuleBuilder;
use reticle::ir::validate::validate;
use reticle::ir::{Design, ModuleRef, Name, PortDir, Type};
use reticle::source::{SourceMap, Span};

/// Every project under `testdata/ip/projects/`.
const CASES: [&str; 5] = ["two_deps", "conflict", "encrypted", "crossbar", "mixed"];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/ip")
}

/// The one place this test suite touches the filesystem.
fn read(path: &str) -> Option<String> {
    fs::read_to_string(dir().join(path)).ok()
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
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&path, actual)
                .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        }
        return;
    }
    let first = expected
        .lines()
        .zip(actual.lines())
        .enumerate()
        .find(|(_, (e, a))| e != a)
        .map_or_else(
            || {
                format!(
                    "line count differs: {} vs {}",
                    expected.lines().count(),
                    actual.lines().count()
                )
            },
            |(i, (e, a))| format!("line {}: expected {e:?}, got {a:?}", i + 1),
        );
    failures.push(format!(
        "{name}: {first}\n--- expected ---\n{expected}--- actual ---\n{actual}"
    ));
}

/// Everything one case produced.
struct Built {
    project: Project,
    resolved: Resolved,
    report: String,
    diagnostics: String,
    design: Option<Design>,
}

/// Resolves and elaborates one project, exactly as `reticle build`
/// would.
fn build(case: &str) -> Built {
    let root = format!("projects/{case}");
    let manifest_path = format!("{root}/reticle.proj");
    let text = read(&manifest_path).unwrap_or_else(|| panic!("no {manifest_path}"));

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let project = ip::load_project(&mut map, manifest_path.clone(), &text, &mut diags)
        .unwrap_or_else(|| panic!("{manifest_path} does not parse"));

    let mut provider = PathProvider::new(root, read);
    let mut resolved = ip::resolve(map, &project, &mut provider, &mut diags);
    let elaboration = ip::elaborate(&project, &mut resolved, &mut diags);

    diags.sort();
    let diagnostics = diags.render(resolved.source_map());
    let mut report = elaboration.report();
    if let Some(design) = &elaboration.design {
        let problems = validate(design);
        assert!(
            problems.is_empty(),
            "{case}: the elaborated design does not validate: {:?}",
            problems.iter().next()
        );
        let count = design.modules.len();
        report.push_str("rtl\n");
        report.push_str(&format!(
            "  {count} module{}\n",
            if count == 1 { "" } else { "s" }
        ));
    }
    let rtl = elaboration
        .design
        .as_ref()
        .map(Design::to_text)
        .unwrap_or_default();

    let mut failures = Vec::new();
    expect(
        &format!("{case}.lock"),
        &resolved.lock.to_text(),
        &mut failures,
    );
    expect(&format!("{case}.build"), &report, &mut failures);
    expect(&format!("{case}.diag"), &diagnostics, &mut failures);
    if case != "crossbar" {
        // The crossbar case writes its golden after the generator has
        // added the interconnect, which is the design that matters.
        expect(&format!("{case}.rtl"), &rtl, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));

    Built {
        project,
        resolved,
        report,
        diagnostics,
        design: elaboration.design,
    }
}

#[test]
fn a_project_with_two_dependencies_builds() {
    let built = build("two_deps");
    assert!(built.resolved.is_complete(), "{}", built.diagnostics);
    assert_eq!(built.project.top.as_deref(), Some("top"));
    assert_eq!(built.project.device.as_deref(), Some("ice40-hx1k-tq144"));

    // Leaves first: `cdc_sync` is under `fifo_sync`, which is under
    // `uart_lite`.
    let order: Vec<&str> = built
        .resolved
        .packages
        .iter()
        .map(reticle::ip::Package::name)
        .collect();
    assert_eq!(order, ["cdc_sync", "fifo_sync", "uart_lite"]);

    // The lock file pins what was chosen, with where each came from.
    let lock = &built.resolved.lock;
    assert_eq!(
        lock.package("fifo_sync").unwrap().version.to_string(),
        "1.0.4"
    );
    assert_eq!(
        lock.package("uart_lite").unwrap().version.to_string(),
        "1.2.0"
    );
    assert!(lock.differences(lock).is_empty());

    // Nothing was black boxed: every package had readable sources.
    assert!(built.report.contains("rtl/uart_lite.v"));
    assert!(!built.report.contains("black boxes"));
}

/// A project whose top is Verilog and whose dependency is VHDL. Both
/// frontends reach the IR, so the design must hold modules from each and
/// the instance must bind across the language boundary.
#[test]
fn a_mixed_language_project_builds() {
    let built = build("mixed");
    assert!(built.resolved.is_complete(), "{}", built.diagnostics);

    // Nothing was skipped: VHDL used to be analysed and dropped.
    assert!(
        !built.report.contains("VHDL"),
        "VHDL was skipped:\n{}",
        built.report
    );
    assert!(
        !built.diagnostics.contains("P0403"),
        "{}",
        built.diagnostics
    );

    let design = built.design.as_ref().expect("no design was elaborated");
    // The Verilog top and the VHDL entity are both there.
    assert!(design.module_by_name("top").is_some(), "no Verilog top");
    assert!(
        design.module_by_name("gray_counter").is_some(),
        "the VHDL entity did not reach the design"
    );
    // And the cross-language instance is bound, not left dangling.
    let top = design.module_by_name("top").unwrap();
    let bound = design.modules[top]
        .instances
        .iter()
        .all(|(_, inst)| matches!(inst.module, ModuleRef::Resolved(_)));
    assert!(bound, "the VHDL instance was left unresolved");
}

#[test]
fn a_version_conflict_names_both_requirements() {
    let built = build("conflict");
    assert!(!built.resolved.is_complete());
    let text = &built.diagnostics;
    assert!(
        text.contains("error[P0102]: no version of `fifo_sync` satisfies every requirement"),
        "{text}"
    );
    assert!(text.contains("`conflicted` requires ^1.0.0"), "{text}");
    assert!(
        text.contains("`conflicted > uart_strict` requires ^2.0.0"),
        "{text}"
    );
    assert!(text.contains("available: 1.0.4"), "{text}");
    // Nothing is locked, because nothing was chosen.
    assert!(built.resolved.lock.package("fifo_sync").is_none());
}

#[test]
fn encrypted_ip_still_elaborates() {
    let built = build("encrypted");
    assert!(built.resolved.is_complete(), "{}", built.diagnostics);

    // One core has nothing readable and became a black box; the other
    // ran from its behavioural model, and the report says which.
    assert!(
        built
            .report
            .contains("black box `ddr_phy` from vendor_ddr 2.1.0 (encrypted sources)"),
        "{}",
        built.report
    );
    assert!(
        built
            .report
            .contains("(the behavioural model `sim/vendor_pll_model.v`)"),
        "{}",
        built.report
    );
    // The stub's `interface` line became a whole AXI4-Lite port set.
    assert!(
        built.report.contains("interface s_axi: 19 ports"),
        "{}",
        built.report
    );
}

/// Builds the crossbar system: the generated interconnect, two copies of
/// the resolved `axi_regs` subordinate, and a top that brings every
/// manager interface out to a port.
fn crossbar_system(resolved: &Resolved, project: &Project, mut design: Design) -> Design {
    let span = project.span;
    let slave = design
        .module_by_name("axil_regs")
        .expect("the resolved package provides `axil_regs`");

    // The subordinate must be the AXI4-Lite subordinate its manifest
    // claims, or wiring it to a crossbar is meaningless.
    let axi = reticle::ip::bus::builtin("axi4lite").unwrap();
    let declared = resolved
        .package("axi_regs")
        .and_then(|p| p.manifest.interface("s_axi"))
        .expect("the package declares `s_axi`");
    reticle::ip::bus::match_ports(design.module(slave), axi, declared.role, &declared.prefix())
        .expect("the RTL implements the interface its manifest declares");

    let crossbar = Crossbar::new(
        "axil_xbar",
        2,
        vec![
            AddressRange::new(0x0000, 0x1000),
            AddressRange::new(0x1000, 0x1000),
        ],
        span,
    );
    assert!(crossbar.problems().is_empty(), "{:?}", crossbar.problems());
    let xbar_module = crossbar.build();
    let xbar_ports: Vec<(Name, PortDir, u32)> = xbar_module
        .ports
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                p.dir,
                xbar_module.nets[p.net].ty.width().unwrap_or(1),
            )
        })
        .collect();
    let xbar = design.add_module(xbar_module);

    let mut b = ModuleBuilder::new("top", span);
    let clk = b.input("clk", Type::bit());
    let rst_n = b.input("rst_n", Type::bit());
    let clk_expr = b.net(clk);
    let rst_expr = b.net(rst_n);
    let mut connections = vec![(Name::new("clk"), clk_expr), (Name::new("rst_n"), rst_expr)];
    for (name, dir, width) in &xbar_ports {
        if !name.as_str().starts_with('s') {
            continue;
        }
        let net = match dir {
            PortDir::In => b.input(name.clone(), Type::bits(*width)),
            _ => b.output(name.clone(), Type::bits(*width)),
        };
        let expr = b.net(net);
        connections.push((name.clone(), expr));
    }
    let u_xbar = b.instance("u_xbar", ModuleRef::Resolved(xbar), connections);
    let mut slaves = Vec::new();
    for j in 0..2 {
        let clk_expr = b.net(clk);
        let rst_expr = b.net(rst_n);
        slaves.push(b.instance(
            format!("u_regs{j}"),
            ModuleRef::Resolved(slave),
            vec![(Name::new("clk"), clk_expr), (Name::new("rst_n"), rst_expr)],
        ));
    }
    let top = design.add_module(b.finish());
    design.top = Some(top);

    // One call per subordinate instead of nineteen port lines each.
    let mut links = Vec::new();
    for (j, instance) in slaves.iter().enumerate() {
        let joined = connect(
            &design,
            top,
            &BusEndpoint::new(u_xbar, format!("m{j}_")),
            &BusEndpoint::new(*instance, declared.prefix()),
            axi,
        )
        .unwrap_or_else(|problems| panic!("wiring subordinate {j}: {problems:?}"));
        assert_eq!(joined.len(), axi.signals.len());
        links.push((*instance, joined));
    }
    let module = design.module(top).clone();
    let mut b = ModuleBuilder::from_module(module, span);
    for (instance, joined) in &links {
        wire(&mut b, u_xbar, *instance, joined);
    }
    *design.module_mut(top) = b.finish();

    let problems = validate(&design);
    assert!(problems.is_empty(), "{:?}", problems.iter().next());
    design
}

/// Resolves the `crossbar` project and builds the generated system.
fn crossbar_design() -> Design {
    let root = "projects/crossbar";
    let manifest_path = format!("{root}/reticle.proj");
    let text = read(&manifest_path).expect("the project manifest");
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let project = ip::load_project(&mut map, manifest_path, &text, &mut diags).expect("parses");
    let mut provider = PathProvider::new(root, read);
    let mut resolved = ip::resolve(map, &project, &mut provider, &mut diags);
    let design = ip::elaborate_project(&project, &mut resolved, &mut diags).expect("elaborates");
    assert!(
        !diags.has_errors(),
        "{}",
        diags.render(resolved.source_map())
    );
    crossbar_system(&resolved, &project, design)
}

#[test]
fn a_generated_crossbar_joins_the_resolved_ip() {
    let built = build("crossbar");
    assert!(built.resolved.is_complete(), "{}", built.diagnostics);
    let design = crossbar_design();

    let top = design.top_module().expect("a top");
    assert_eq!(top.name, "top");
    assert_eq!(top.instances.len(), 3);
    // Every manager interface reached the outside.
    let axi = reticle::ip::bus::builtin("axi4lite").unwrap();
    for i in 0..2 {
        reticle::ip::bus::match_ports(top, axi, BusRole::Subordinate, &format!("s{i}_"))
            .unwrap_or_else(|problems| panic!("manager {i}: {problems:?}"));
    }

    let mut failures = Vec::new();
    expect("crossbar.rtl", &design.to_text(), &mut failures);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));

    // And it survives a trip through the IR text format.
    let text = design.to_text();
    let mut map = SourceMap::new();
    let file = map.add("crossbar.rtl", &text).unwrap();
    let again = Design::parse_text(&text, file).expect("the golden parses");
    assert_eq!(again.to_text(), text);
    let _ = Span::new(file, 0, 0);
}

/// Drives the generated crossbar with real AXI4-Lite transactions.
///
/// This is the test that says the generator works: the netlist could be
/// inspected all day and still be wrong.
#[cfg(feature = "sim")]
#[test]
fn the_generated_crossbar_carries_transactions() {
    use reticle::ir::{Delay, TimeUnit};
    use reticle::logic::Logic;
    use reticle::sim::{SimOptions, Simulator};

    let design = crossbar_design();
    let mut sim = Simulator::new(&design, SimOptions::default()).expect("elaborates");
    let half = sim.ticks(Delay::new(5, TimeUnit::Ns));
    sim.run_for(0);

    let handle = |sim: &Simulator, name: &str| {
        sim.net(&format!("top.{name}"))
            .unwrap_or_else(|| panic!("no net `{name}`"))
    };
    let put = |sim: &mut Simulator, name: &str, width: u32, value: u64| {
        let h = handle(sim, name);
        sim.set(h, Logic::from_u64(value, width));
    };

    // Everything low, then a reset.
    put(&mut sim, "clk", 1, 0);
    for i in 0..2 {
        for (signal, width) in [
            ("awaddr", 32u32),
            ("awprot", 3),
            ("awvalid", 1),
            ("wdata", 32),
            ("wstrb", 4),
            ("wvalid", 1),
            ("bready", 1),
            ("araddr", 32),
            ("arprot", 3),
            ("arvalid", 1),
            ("rready", 1),
        ] {
            put(&mut sim, &format!("s{i}_{signal}"), width, 0);
        }
    }
    put(&mut sim, "rst_n", 1, 0);
    let step = |sim: &mut Simulator| {
        let clk = handle(sim, "clk");
        sim.set(clk, Logic::from_u64(1, 1));
        sim.run_for(half);
        sim.set(clk, Logic::from_u64(0, 1));
        sim.run_for(half);
    };
    for _ in 0..3 {
        step(&mut sim);
    }
    put(&mut sim, "rst_n", 1, 1);
    step(&mut sim);

    let get = |sim: &Simulator, name: &str| sim.get(handle(sim, name)).to_u64().unwrap_or(u64::MAX);

    // A write, then a read back, driven one handshake at a time.
    let write = |sim: &mut Simulator, manager: usize, addr: u64, data: u64| -> u64 {
        put(sim, &format!("s{manager}_awaddr"), 32, addr);
        put(sim, &format!("s{manager}_awvalid"), 1, 1);
        put(sim, &format!("s{manager}_wdata"), 32, data);
        put(sim, &format!("s{manager}_wstrb"), 4, 0xf);
        put(sim, &format!("s{manager}_wvalid"), 1, 1);
        put(sim, &format!("s{manager}_bready"), 1, 1);
        for _ in 0..200 {
            let awready = get(sim, &format!("s{manager}_awready"));
            let wready = get(sim, &format!("s{manager}_wready"));
            let bvalid = get(sim, &format!("s{manager}_bvalid"));
            let bresp = get(sim, &format!("s{manager}_bresp"));
            step(sim);
            if awready == 1 {
                put(sim, &format!("s{manager}_awvalid"), 1, 0);
            }
            if wready == 1 {
                put(sim, &format!("s{manager}_wvalid"), 1, 0);
            }
            if bvalid == 1 {
                put(sim, &format!("s{manager}_bready"), 1, 0);
                return bresp;
            }
        }
        panic!("the write never completed");
    };
    let read = |sim: &mut Simulator, manager: usize, addr: u64| -> (u64, u64) {
        put(sim, &format!("s{manager}_araddr"), 32, addr);
        put(sim, &format!("s{manager}_arvalid"), 1, 1);
        put(sim, &format!("s{manager}_rready"), 1, 1);
        for _ in 0..200 {
            let arready = get(sim, &format!("s{manager}_arready"));
            let rvalid = get(sim, &format!("s{manager}_rvalid"));
            let rdata = get(sim, &format!("s{manager}_rdata"));
            let rresp = get(sim, &format!("s{manager}_rresp"));
            step(sim);
            if arready == 1 {
                put(sim, &format!("s{manager}_arvalid"), 1, 0);
            }
            if rvalid == 1 {
                put(sim, &format!("s{manager}_rready"), 1, 0);
                return (rdata, rresp);
            }
        }
        panic!("the read never completed");
    };

    // Manager 0 writes to both subordinates; manager 1 reads them back,
    // which only works if the decode and the routing are both right.
    assert_eq!(write(&mut sim, 0, 0x0004, 0xcafe_0001), 0);
    assert_eq!(write(&mut sim, 0, 0x1004, 0xcafe_0002), 0);
    assert_eq!(read(&mut sim, 1, 0x0004), (0xcafe_0001, 0));
    assert_eq!(read(&mut sim, 1, 0x1004), (0xcafe_0002, 0));

    // An address no subordinate claims is answered by the crossbar with
    // DECERR rather than hanging the bus.
    assert_eq!(read(&mut sim, 0, 0x8000).1, 0b11);
    assert_eq!(write(&mut sim, 0, 0x8000, 0xdead_beef), 0b11);
    // And the bus is still usable afterwards.
    assert_eq!(read(&mut sim, 1, 0x1004), (0xcafe_0002, 0));
}

/// Every case is listed, and every listed case has a project.
#[test]
fn the_case_list_matches_the_directory() {
    for case in CASES {
        let path = format!("projects/{case}/reticle.proj");
        assert!(read(&path).is_some(), "{path} is missing");
    }
    let mut found: Vec<String> = fs::read_dir(dir().join("projects"))
        .expect("testdata/ip/projects")
        .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
        .collect();
    found.sort();
    let mut listed: Vec<String> = CASES.iter().map(|c| (*c).to_owned()).collect();
    listed.sort();
    assert_eq!(found, listed);
}
