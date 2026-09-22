//! End-to-end tests of technology mapping.
//!
//! Each fixed design is written out as `testdata/synth/techmap/<name>.rtl`
//! and mapped three ways — 4-LUTs, 6-LUTs and the generic gate library —
//! with the results pinned down as `<name>.lut4.rtl`, `<name>.lut6.rtl`
//! and `<name>.gates.rtl`. `UPDATE_EXPECT=1` rewrites all of them.
//!
//! Correctness is checked independently of the golden files: the mapped
//! cell form is simulated with a small evaluator and compared, over many
//! random input vectors, against evaluating the original module's
//! expressions on `Logic`. Random modules are cross-checked the same way.
//!
//! The cell counts of the larger designs are collected into
//! `testdata/synth/techmap/summary.txt` (and printed with `--nocapture`),
//! so they can be compared with Yosys or ABC by hand.

#![cfg(feature = "synth")]

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use common::{CellEval, RandomModule, Rng, eval_assigns, golden};

use reticle::ir::validate::validate_module;
use reticle::ir::{CellKind, Design, Module};
use reticle::source::SourceMap;
use reticle::synth::cells::GateLibrary;
use reticle::synth::techmap::{self, MapOptions, MapStats, Target};

/// Renders a module as a one-module design.
fn to_text(module: &Module) -> String {
    let mut design = Design::new();
    let id = design.add_module(module.clone());
    design.top = Some(id);
    design.to_text()
}

/// Parses a one-module design back.
fn from_text(name: &str, text: &str) -> Module {
    let mut map = SourceMap::new();
    let file = map.add(name.to_owned(), text.to_owned()).unwrap();
    let design = Design::parse_text(text, file)
        .unwrap_or_else(|d| panic!("`{name}` does not parse:\n{}", d.render(&map)));
    design.modules.values().next().expect("one module").clone()
}

/// Checks that `mapped` computes what `original` does, over random inputs.
fn check_equivalent(
    name: &str,
    original: &Module,
    mapped: &Module,
    library: Option<&GateLibrary>,
    vectors: usize,
    seed: u64,
) {
    let evaluator = match library {
        Some(lib) => CellEval::with_library(mapped, lib),
        None => CellEval::new(mapped),
    };
    let mut rng = Rng::new(seed);
    for vector in 0..vectors {
        let values = common::random_input_values(original, &mut rng);
        let expected = eval_assigns(original, &values);
        // The mapped module keeps the port nets, but its net ids have been
        // renumbered, so inputs are matched by port name.
        let mut mapped_inputs = BTreeMap::new();
        for port in &original.ports {
            if port.dir != reticle::ir::PortDir::In {
                continue;
            }
            if let (Some(value), Some(target)) = (
                values.get(&port.net),
                mapped.port(port.name.as_str()).map(|p| p.net),
            ) {
                mapped_inputs.insert(target, value.clone());
            }
        }
        let actual = evaluator.eval(&mapped_inputs);
        for port in &original.ports {
            if port.dir != reticle::ir::PortDir::Out {
                continue;
            }
            let (Some(want), Some(target)) = (
                expected.get(&port.net),
                mapped.port(port.name.as_str()).map(|p| p.net),
            ) else {
                continue;
            };
            let got = actual
                .get(&target)
                .unwrap_or_else(|| panic!("`{name}`: port {} has no value", port.name));
            common::assert_known_bits_match(
                want,
                got,
                &format!("`{name}` vector {vector}, port {}", port.name),
            );
        }
    }
}

/// Maps `module` and returns the mapped module with its statistics.
///
/// The AIG script is the feature-independent one, so the golden files and
/// the reported counts are the same whether or not the SAT solver is
/// compiled in (see `common::portable_aig_options`).
fn map(module: &Module, target: Target<'_>) -> (Module, MapStats) {
    let mut mapped = module.clone();
    let options = MapOptions {
        target,
        aig_opt: common::portable_aig_options(),
        ..MapOptions::default()
    };
    let stats = techmap::map_module(&mut mapped, &options);
    (mapped, stats)
}

#[test]
fn fixed_designs_map_three_ways() {
    let library = GateLibrary::generic();
    let mut summary = String::new();
    summary.push_str(
        "Cell counts after `map_module` on the reference designs.\n\
         `aig` is the optimised AIG (AND nodes / levels); the mapped\n\
         columns are cells and levels of cells. Compare with Yosys\n\
         (`synth; abc -lut 4`) or ABC (`if -K 4`) by hand.\n\
         The AIG script here leaves FRAIGing out so the numbers do not\n\
         depend on whether the `formal` feature is compiled in.\n\n",
    );
    let _ = writeln!(
        summary,
        "{:<14} {:>12} {:>14} {:>14} {:>16}",
        "design", "aig", "lut4", "lut6", "gates"
    );

    for (name, module) in common::fixed_designs() {
        assert!(
            validate_module(&module).is_empty(),
            "`{name}` does not validate"
        );
        // The source design is a golden file of its own, and must survive
        // a round trip through the text format.
        let text = to_text(&module);
        golden(&format!("techmap/{name}.rtl"), &text);
        let reparsed = from_text(&name, &text);
        assert_eq!(to_text(&reparsed), text, "`{name}` does not round-trip");

        let (lut4, s4) = map(&reparsed, Target::Lut(4));
        let (lut6, s6) = map(&reparsed, Target::Lut(6));
        let (gates, sg) = map(&reparsed, Target::Gates(&library));

        for (suffix, mapped, lib) in [
            ("lut4", &lut4, None),
            ("lut6", &lut6, None),
            ("gates", &gates, Some(&library)),
        ] {
            let diags = validate_module(mapped);
            assert!(
                diags.is_empty(),
                "`{name}.{suffix}` does not validate:\n{:?}",
                diags
            );
            check_equivalent(
                &format!("{name}.{suffix}"),
                &module,
                mapped,
                lib,
                48,
                0x5EED,
            );
            golden(&format!("techmap/{name}.{suffix}.rtl"), &to_text(mapped));
        }

        // Only the expected cell kinds survive.
        for (mapped, k) in [(&lut4, 4u32), (&lut6, 6)] {
            for cell in mapped.cells.values() {
                match &cell.kind {
                    CellKind::Lut { k: got, init } => {
                        assert!(*got <= k, "`{name}`: a {got}-LUT in a {k}-LUT mapping");
                        assert_eq!(init.width(), 1 << got);
                    }
                    other => panic!("`{name}`: unexpected {} cell", other.keyword()),
                }
            }
        }
        for cell in gates.cells.values() {
            let named = cell.attrs.get("lib_cell").and_then(|v| v.as_str());
            let gate = named.unwrap_or_else(|| panic!("`{name}`: cell without `lib_cell`"));
            assert!(
                library.gate(gate).is_some(),
                "`{name}`: `{gate}` is not in the library"
            );
        }

        let _ = writeln!(
            summary,
            "{:<14} {:>7} / {:>2} {:>9} / {:>2} {:>9} / {:>2} {:>7} / {:>2} {:>4.0}",
            name,
            s4.after.nodes,
            s4.after.levels,
            s4.cells,
            s4.depth,
            s6.cells,
            s6.depth,
            sg.cells,
            sg.depth,
            sg.area,
        );
        println!(
            "{name:<14} aig {:>5} nodes / {:>2} lev | lut4 {:>5} / {:>2} | lut6 {:>5} / {:>2} | gates {:>5} / {:>2}, area {:.0}",
            s4.after.nodes,
            s4.after.levels,
            s4.cells,
            s4.depth,
            s6.cells,
            s6.depth,
            sg.cells,
            sg.depth,
            sg.area,
        );

        // Bigger LUTs never need more of them, and mapping never invents
        // more cells than the AIG had nodes.
        assert!(
            s6.cells <= s4.cells,
            "`{name}`: 6-LUTs ({}) should not exceed 4-LUTs ({})",
            s6.cells,
            s4.cells
        );
        assert!(s4.cells <= s4.after.nodes + module.ports.len());
    }

    golden("techmap/summary.txt", &summary);
}

#[test]
fn random_modules_map_correctly() {
    let library = GateLibrary::generic();
    let mut rng = Rng::new(0x7EC);
    // Each case is mapped three ways and simulated, and the generator can
    // produce multipliers and dividers whose AIGs run to hundreds of
    // nodes, so the count is kept modest to hold the debug test time down.
    for case in 0..12 {
        let module = RandomModule::new(&mut rng).build(case);
        let name = module.name.to_string();
        for (suffix, target, lib) in [
            ("lut4", Target::Lut(4), None),
            ("lut6", Target::Lut(6), None),
            ("gates", Target::Gates(&library), Some(&library)),
        ] {
            let (mapped, stats) = map(&module, target);
            assert!(
                validate_module(&mapped).is_empty(),
                "`{name}.{suffix}` does not validate"
            );
            assert_eq!(stats.cells, mapped.cells.len());
            check_equivalent(
                &format!("{name}.{suffix}"),
                &module,
                &mapped,
                lib,
                24,
                900 + case,
            );
        }
    }
}

/// Every LUT size from 2 to 8 maps correctly.
#[test]
fn every_lut_size_maps_correctly() {
    let module = common::adder(4);
    for k in 2..=8u32 {
        let (mapped, stats) = map(&module, Target::Lut(k));
        assert!(validate_module(&mapped).is_empty(), "k = {k}");
        assert!(stats.cells > 0);
        for cell in mapped.cells.values() {
            let CellKind::Lut { k: got, .. } = &cell.kind else {
                panic!("a {} cell in a LUT mapping", cell.kind.keyword());
            };
            assert!(*got <= k);
        }
        check_equivalent(&format!("adder4.lut{k}"), &module, &mapped, None, 64, 42);
    }
}

/// The documented `init` bit ordering: bit `i` of a LUT's table is the
/// output for the input pattern `i`, with input 0 the least significant
/// bit of the `a` port.
#[test]
fn lut_init_bit_order_is_documented() {
    use reticle::synth::aig::{Aig, Edge};

    // `y = a & !b`: pattern 1 (a = 1, b = 0) is the only one that is true,
    // so a two-input LUT must have `init` bit 1 set and nothing else.
    let mut g = Aig::new();
    let a = g.add_input();
    let b = g.add_input();
    let y = g.and(a, !b);
    g.add_output(y);
    let net = techmap::lut_map(&g, 2);
    assert_eq!(net.len(), 1);
    let lut = &net.luts[0];
    assert_eq!(lut.inputs.len(), 2);
    assert_eq!(lut.init.width(), 4);
    // Which AIG input feeds LUT input 0 decides which pattern is set.
    let expects_bit = match lut.inputs[0] {
        reticle::synth::aig::emit::Signal::Input(0) => 0b0010,
        reticle::synth::aig::emit::Signal::Input(1) => 0b0100,
        other => panic!("unexpected LUT input {other:?}"),
    };
    assert_eq!(
        lut.init.to_u64(),
        Some(expects_bit),
        "init {} for inputs {:?}",
        lut.init,
        lut.inputs
    );

    // Simulating the LUT network reproduces the AIG for every pattern.
    for pat in 0..4u32 {
        let ins = [pat & 1 == 1, pat & 2 == 2];
        let want = g.eval(&ins);
        let mut values: Vec<bool> = Vec::new();
        for lut in &net.luts {
            let mut pattern = 0u32;
            for (i, s) in lut.inputs.iter().enumerate() {
                let v = match s {
                    reticle::synth::aig::emit::Signal::Const(c) => *c,
                    reticle::synth::aig::emit::Signal::Input(n) => ins[*n as usize],
                    reticle::synth::aig::emit::Signal::Node(n) => values[*n as usize],
                };
                if v {
                    pattern |= 1 << i;
                }
            }
            values.push(lut.init.bit(pattern) == reticle::logic::Bit::One);
        }
        let got: Vec<bool> = net
            .outputs
            .iter()
            .map(|s| match s {
                reticle::synth::aig::emit::Signal::Const(c) => *c,
                reticle::synth::aig::emit::Signal::Input(n) => ins[*n as usize],
                reticle::synth::aig::emit::Signal::Node(n) => values[*n as usize],
            })
            .collect();
        assert_eq!(got, want, "pattern {pat}");
    }
    let _ = Edge::TRUE;
}

/// Mapping keeps flip-flops, memories, instances and black boxes, and the
/// module's ports keep their nets.
#[test]
fn the_sequential_boundary_survives() {
    use reticle::ir::builder::ModuleBuilder;
    use reticle::ir::{ModuleRef, Name, Type};

    let mut b = ModuleBuilder::new("seq", common::span());
    let clk = b.input("clk", Type::bit());
    let d = b.input("d", Type::bits(4));
    let q = b.add_reg("q", Type::bits(4));
    let y = b.output("y", Type::bits(4));
    let bb = b.output("bb_out", Type::bit());
    let mem = b.memory("mem", Type::bits(4), 16);
    let rd = b.output("rd", Type::bits(4));

    let (dn, qn, clkn) = (b.net(d), b.net(q), b.net(clk));
    // Combinational logic feeding the flop and the output.
    let mixed = b.xor(dn, qn);
    let next = b.and(mixed, dn);
    b.cell(
        "ff",
        CellKind::Dff {
            clk_pos: true,
            has_enable: false,
            reset: None,
        },
        vec![(Name::new("clk"), clkn), (Name::new("d"), next)],
        vec![(Name::new("q"), q)],
    );
    b.assign(y, mixed);
    // A memory read port and a black box, both boundaries.
    let addr = b.slice(qn, 3, 0);
    b.cell(
        "rd0",
        CellKind::MemRdPort {
            mem,
            clocked: false,
        },
        vec![(Name::new("addr"), addr)],
        vec![(Name::new("data"), rd)],
    );
    let red = b.reduce_xor(dn);
    b.cell(
        "bb0",
        CellKind::Blackbox(Name::new("VENDOR_THING")),
        vec![(Name::new("I"), red)],
        vec![(Name::new("O"), bb)],
    );
    b.instance(
        "u0",
        ModuleRef::Unresolved(Name::new("leaf")),
        vec![(Name::new("a"), dn)],
    );
    let module = b.finish();
    assert!(validate_module(&module).is_empty());

    let (mapped, stats) = map(&module, Target::Lut(4));
    assert!(validate_module(&mapped).is_empty());
    assert!(stats.cells > 0);
    // The flop, the memory port, the black box and the instance are all
    // still there, and so is the memory itself.
    assert!(
        mapped
            .cells
            .values()
            .any(|c| matches!(c.kind, CellKind::Dff { .. }))
    );
    assert!(
        mapped
            .cells
            .values()
            .any(|c| matches!(c.kind, CellKind::MemRdPort { .. }))
    );
    assert!(
        mapped
            .cells
            .values()
            .any(|c| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == "VENDOR_THING"))
    );
    assert_eq!(mapped.instances.len(), 1);
    assert_eq!(mapped.memories.len(), 1);
    // The ports are unchanged.
    let ports: Vec<&str> = mapped.ports.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(ports, ["clk", "d", "y", "bb_out", "rd"]);
    // The combinational cloud became LUTs.
    assert!(
        mapped
            .cells
            .values()
            .any(|c| matches!(c.kind, CellKind::Lut { .. }))
    );
    // The reduction feeding the black box is mapped too, so no generic
    // arithmetic or reduction cell is left behind.
    assert!(
        !mapped
            .cells
            .values()
            .any(|c| matches!(c.kind, CellKind::ReduceXor | CellKind::Xor | CellKind::And)),
        "generic logic survived mapping"
    );
}
