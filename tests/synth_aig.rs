//! End-to-end tests of the AIG optimiser.
//!
//! The AIG is checked against the IR it was built from: random modules are
//! generated with `ModuleBuilder`, their `Assign` expressions are evaluated
//! directly on [`Logic`] for every (or many) input vectors, and the result
//! is compared with simulation of the AIG before and after optimisation.
//! Golden files under `testdata/synth/aig/` pin down the written-back form
//! of a few fixed designs; run with `UPDATE_EXPECT=1` to rewrite them.

#![cfg(feature = "synth")]

mod common;

use common::{CellEval, RandomModule, Rng, eval_assigns, golden};

use reticle::ir::builder::ModuleBuilder;
use reticle::ir::validate::validate_module;
use reticle::ir::{Design, Module, Type};
use reticle::logic::Logic;
use reticle::synth::aig::{self, Aig, AigOptions, Edge};

/// Builds an AIG for a module and checks it computes the module's outputs.
fn check_aig(module: &Module, vectors: usize, seed: u64) -> Aig {
    let (aig, mapping) = aig::from_module(module);
    let mut rng = Rng::new(seed);
    let inputs: Vec<_> = mapping.inputs.clone();
    for _ in 0..vectors {
        // One random value per input net bit, collected into net values.
        let mut bits: Vec<bool> = Vec::with_capacity(inputs.len());
        let mut word = rng.next_u64();
        for (i, _) in inputs.iter().enumerate() {
            if i % 64 == 0 && i > 0 {
                word = rng.next_u64();
            }
            bits.push((word >> (i % 64)) & 1 == 1);
        }
        let values = common::net_values(module, &inputs, &bits);
        let expected = eval_assigns(module, &values);
        let actual = aig.eval(&bits);
        for (i, out) in mapping.outputs.iter().enumerate() {
            let Some(want) = expected.get(&out.net) else {
                continue;
            };
            // Skip the bits the reference leaves unknown; see
            // `assert_known_bits_match`.
            let bit = want.bit(out.bit);
            if !bit.is_known() {
                continue;
            }
            assert_eq!(
                actual[i],
                bit == reticle::logic::Bit::One,
                "net {} bit {} of module `{}`",
                module.nets[out.net].name,
                out.bit,
                module.name
            );
        }
    }
    aig
}

#[test]
fn random_modules_blast_and_optimise_correctly() {
    let mut rng = Rng::new(0xA16);
    for case in 0..40 {
        let module = RandomModule::new(&mut rng).build(case);
        assert!(
            validate_module(&module).is_empty(),
            "generated module {} does not validate",
            module.name
        );
        let aig = check_aig(&module, 24, 1000 + case);

        // Optimising must not change what it computes.
        let mut optimised = aig.clone();
        let (before, after) = aig::optimize(&mut optimised, &AigOptions::default());
        assert_eq!(before.inputs, after.inputs);
        assert_eq!(before.outputs, after.outputs);
        assert!(
            after.nodes <= before.nodes,
            "case {case}: {} -> {} nodes",
            before.nodes,
            after.nodes
        );
        let mut vrng = Rng::new(7000 + case);
        for _ in 0..64 {
            let bits = common::random_bits(&mut vrng, aig.inputs().len());
            assert_eq!(
                aig.eval(&bits),
                optimised.eval(&bits),
                "case {case}: optimisation changed the function"
            );
        }
    }
}

#[test]
fn each_pass_preserves_the_function() {
    let mut rng = Rng::new(0x9A55);
    type Pass = (&'static str, fn(&mut Aig));
    let passes: [Pass; 6] = [
        ("strash", |a: &mut Aig| *a = a.rebuild()),
        ("balance", |a: &mut Aig| {
            aig::balance::balance(a);
        }),
        ("rewrite", |a: &mut Aig| {
            aig::rewrite::rewrite(a, false);
        }),
        ("rewrite -z", |a: &mut Aig| {
            aig::rewrite::rewrite(a, true);
        }),
        ("refactor", |a: &mut Aig| {
            aig::refactor::refactor(a, 10);
        }),
        ("fraig", |a: &mut Aig| {
            aig::fraig::fraig(a, &aig::fraig::FraigOptions::default());
        }),
    ];
    for case in 0..12 {
        let module = RandomModule::new(&mut rng).build(case);
        let (reference, _) = aig::from_module(&module);
        if reference.inputs().is_empty() || reference.outputs().is_empty() {
            continue;
        }
        for (name, pass) in passes {
            let mut aig = reference.clone();
            pass(&mut aig);
            assert_eq!(aig.inputs().len(), reference.inputs().len());
            assert_eq!(aig.outputs().len(), reference.outputs().len());
            let mut vrng = Rng::new(31 + case);
            for _ in 0..48 {
                let bits = common::random_bits(&mut vrng, reference.inputs().len());
                assert_eq!(
                    aig.eval(&bits),
                    reference.eval(&bits),
                    "pass `{name}` changed case {case}"
                );
            }
        }
    }
}

#[test]
fn structural_hashing_invariants_hold() {
    let mut rng = Rng::new(5);
    let mut g = Aig::new();
    let ins: Vec<Edge> = (0..6).map(|_| g.add_input()).collect();
    let mut pool = ins.clone();
    for _ in 0..200 {
        let a = pool[rng.below(pool.len())];
        let b = pool[rng.below(pool.len())];
        let a = if rng.next_u64() & 1 == 1 { !a } else { a };
        let b = if rng.next_u64() & 1 == 1 { !b } else { b };
        let e = g.and(a, b);
        // Building the same pair again must give the same edge, in either
        // operand order.
        assert_eq!(g.and(a, b), e);
        assert_eq!(g.and(b, a), e);
        // A node's fanins always have smaller indices: the graph stays
        // topologically ordered.
        if g.is_and(e.node()) {
            let (f0, f1) = g.fanins(e.node());
            assert!(f0.node() < e.node() && f1.node() < e.node());
            assert!(f0 < f1, "fanins are ordered");
            assert_eq!(
                g.level(e.node()),
                g.level(f0.node()).max(g.level(f1.node())) + 1
            );
        }
        pool.push(e);
    }
    for &e in &pool {
        g.add_output(e);
    }
    // Reference counts and levels recomputed from scratch must agree with
    // the ones maintained incrementally.
    let levels: Vec<u32> = (0..g.len())
        .map(|i| g.level(u32::try_from(i).unwrap()))
        .collect();
    let refs: Vec<u32> = (0..g.len())
        .map(|i| g.refs(u32::try_from(i).unwrap()))
        .collect();
    g.recompute_levels();
    g.recount_refs();
    for i in 0..g.len() {
        let i32_ = u32::try_from(i).unwrap();
        assert_eq!(g.level(i32_), levels[i], "level of node {i}");
        assert_eq!(g.refs(i32_), refs[i], "refs of node {i}");
    }
    // Rebuilding never grows the graph and never changes the function.
    let rebuilt = g.rebuild();
    assert!(rebuilt.num_ands() <= g.num_ands());
    let mut vrng = Rng::new(11);
    for _ in 0..64 {
        let bits = common::random_bits(&mut vrng, 6);
        assert_eq!(g.eval(&bits), rebuilt.eval(&bits));
    }
}

#[test]
fn optimisation_shrinks_redundant_logic() {
    // Four copies of the same function, written four different ways, plus
    // a chain that balancing should flatten. The optimiser must collapse
    // them into one.
    let mut b = ModuleBuilder::new("redundant", common::span());
    let a = b.input("a", Type::bits(4));
    let c = b.input("c", Type::bits(4));
    let y0 = b.output("y0", Type::bits(4));
    let y1 = b.output("y1", Type::bits(4));
    let y2 = b.output("y2", Type::bits(4));
    let (an, cn) = (b.net(a), b.net(c));
    // y0 = a ^ c
    let x = b.xor(an, cn);
    b.assign(y0, x);
    // y1 = (a | c) & ~(a & c), the same function
    let or = b.or(an, cn);
    let and = b.and(an, cn);
    let nand = b.not(and);
    let same = b.and(or, nand);
    b.assign(y1, same);
    // y2 = (a & ~c) | (~a & c), the same function again
    let nc = b.not(cn);
    let na = b.not(an);
    let l = b.and(an, nc);
    let r = b.and(na, cn);
    let third = b.or(l, r);
    b.assign(y2, third);
    let module = b.finish();

    let (aig, _) = aig::from_module(&module);
    let mut optimised = aig.clone();
    let (before, after) = aig::optimize(&mut optimised, &AigOptions::default());
    // One 4-bit xor is three AND nodes per bit; the three spellings of it
    // must end up sharing that one implementation.
    assert_eq!(
        after.nodes, 12,
        "expected the three copies to collapse: {before} -> {after}"
    );
    assert!(after.nodes * 2 <= before.nodes);
    // All three outputs become the same edge.
    let outs = optimised.outputs();
    assert_eq!(outs[0..4], outs[4..8], "y0 and y1 differ");
    assert_eq!(outs[0..4], outs[8..12], "y0 and y2 differ");
    for _ in 0..16 {
        let mut vrng = Rng::new(3);
        let bits = common::random_bits(&mut vrng, aig.inputs().len());
        assert_eq!(aig.eval(&bits), optimised.eval(&bits));
    }
}

#[test]
fn round_trips_through_the_cell_form() {
    for (name, module) in common::fixed_designs() {
        let (mut aig, mapping) = aig::from_module(&module);
        // Feature-independent, so the golden file is too.
        aig::optimize(&mut aig, &common::portable_aig_options());
        let mut mapped = module.clone();
        aig::to_module(&aig, &mapping, &mut mapped);
        let diags = validate_module(&mapped);
        assert!(
            diags.is_empty(),
            "`{name}` does not validate after write-back"
        );
        // Every cell is an AND or a NOT, and the module still computes
        // what it did.
        for cell in mapped.cells.values() {
            assert!(
                matches!(
                    cell.kind,
                    reticle::ir::CellKind::And | reticle::ir::CellKind::Not
                ) || !cell.kind.is_combinational(),
                "`{name}` kept a {} cell",
                cell.kind.keyword()
            );
        }
        let mut rng = Rng::new(0xD00D);
        let evaluator = CellEval::new(&mapped);
        for _ in 0..32 {
            let values = common::random_input_values(&module, &mut rng);
            let expected = eval_assigns(&module, &values);
            let actual = evaluator.eval(&values);
            for (net, want) in &expected {
                if let Some(got) = actual.get(net) {
                    common::assert_known_bits_match(
                        want,
                        got,
                        &format!("`{name}`: net {}", module.nets[*net].name),
                    );
                }
            }
        }

        // The written-back form is a golden file.
        let mut design = Design::new();
        let id = design.add_module(mapped);
        design.top = Some(id);
        golden(&format!("aig/{name}.rtl"), &design.to_text());
    }
}

/// A design whose logic is entirely constant folds away to nothing.
#[test]
fn constant_logic_folds_away() {
    let mut b = ModuleBuilder::new("constants", common::span());
    let y = b.output("y", Type::bits(8));
    let z = b.output("z", Type::bit());
    let k = b.const_u64(8, 0xA5);
    let l = b.const_u64(8, 0x5A);
    let sum = b.add(k, l);
    b.assign(y, sum);
    let eq = b.eq(k, l);
    b.assign(z, eq);
    let module = b.finish();
    let (aig, mapping) = aig::from_module(&module);
    assert_eq!(aig.num_ands(), 0, "constants should need no nodes");
    assert!(aig.outputs().iter().all(|e| e.is_const()));
    let mut mapped = module.clone();
    aig::to_module(&aig, &mapping, &mut mapped);
    assert!(validate_module(&mapped).is_empty());
    assert_eq!(mapped.cells.len(), 0);
    // `0xA5 + 0x5A == 0xFF` and the two constants are different.
    let evaluator = CellEval::new(&mapped);
    let values = evaluator.eval(&std::collections::BTreeMap::new());
    let y_net = mapped.net_by_name("y").unwrap();
    assert_eq!(values.get(&y_net), Some(&Logic::from_u64(0xFF, 8)));
    let z_net = mapped.net_by_name("z").unwrap();
    assert_eq!(values.get(&z_net), Some(&Logic::from_u64(0, 1)));
}
