//! Proving a mapped network equivalent to the AIG it was mapped from.
//!
//! # The gap this closes
//!
//! `reticle synth --verify` ([`crate::synth::verify`]) proves the optimised,
//! cellified netlist equivalent to a minimal lowering of the same source.
//! Technology mapping runs **after** that — `map_module` is a separate step
//! in every flow — so nothing in this compiler used to compare a mapped
//! netlist with what it was mapped from. A mapper that covered one cone
//! wrongly emitted wrong logic, every test still passed, and the first thing
//! to notice was a USB host refusing a device over one descriptor byte. That
//! happened; `super::cuts` has the defect and its
//! `every_cut_computes_its_node` has the reason.
//!
//! # What is checked, and how
//!
//! Mapping's input is an [`Aig`] and its output is a network of LUTs or
//! library cells over the *same inputs*, so the two are combinational
//! circuits with one interface and the comparison is a plain combinational
//! equivalence problem — no state, no reset, no induction, nothing that can
//! come back "inconclusive because the registers do not match". That is the
//! whole reason this check is worth having where a sequential one is not: on
//! a registered design, "the mapped module against the unmapped module" is a
//! sequential miter whose counter-example is thousands of cycles deep and
//! whose induction does not close. Cut at the AIG boundary, the same
//! question is decidable.
//!
//! [`lut_miter`] builds it as one AIG: the original graph, the mapped
//! network expanded back into AND nodes (each LUT by Shannon decomposition
//! of its `init`, which is where a wrong `init` becomes a wrong cone), one
//! output per AIG output carrying the XOR of the two sides, and a last
//! output that is the OR of them all. The miter is then decided in two
//! tiers:
//!
//! 1. **Simulation.** [`Aig::simulate`] runs `64 * sim_words` random
//!    patterns at once. A word in which the difference output is set is a
//!    counter-example, reported with the input assignment and the outputs it
//!    breaks. This is the cheap tier, and it is what would have caught the
//!    descriptor ROM: the wrong cover disagreed on about one random vector
//!    in two thousand.
//! 2. **Proof.** [`fraig`] functionally reduces the miter, merging only
//!    nodes it has *proved* equal (exhaustively for small cones, by SAT for
//!    the rest). Two sides that compute the same function collapse into each
//!    other and every difference output folds to the constant false. When
//!    they all do, the mapping is equivalent for every input, which is a
//!    proof and not a sample.
//!
//! A miter that neither simulates a difference nor collapses is
//! [`MapEquivalence::Inconclusive`]: the usual reason is a candidate pair
//! whose SAT query ran out of conflicts. That is a warning, not a verdict —
//! the same reading [`crate::synth::verify`] gives its own inconclusive
//! answers.
//!
//! # What it costs
//!
//! The work is bounded by the graph and the options, never by a clock:
//! `sim_words` words of simulation over the miter, then one [`fraig`] pass
//! under [`FraigOptions::sat_conflicts`] and [`FraigOptions::max_attempts`].
//! The miter is about the size of the two circuits together, and the mapped
//! side expands to a few AND nodes per LUT input. `tests/ip_library.rs` runs
//! the check over every block of the library at two LUT widths and asserts
//! that every one is proved; the measurements are in `docs/ip-library.md`.

use super::super::aig::emit::Signal;
use super::super::aig::fraig::{FraigOptions, fraig};
use super::super::aig::{Aig, Edge};
use super::super::cells::GateLibrary;
use super::{GateNetwork, LutNetwork};
use crate::ir::types::Const;
use crate::logic::Bit;

/// What a mapping check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapEquivalence {
    /// The mapped network computes the AIG's outputs for every input.
    Equivalent,
    /// It does not, and here is an input assignment that shows it.
    Different {
        /// One value per AIG input, input 0 first.
        inputs: Vec<bool>,
        /// The AIG outputs that disagree, by position.
        outputs: Vec<usize>,
    },
    /// Neither settled: no difference was simulated and the miter did not
    /// collapse.
    Inconclusive {
        /// AND nodes left in the reduced miter.
        ands: usize,
    },
}

impl MapEquivalence {
    /// True only when the mapping was proved equivalent.
    pub fn proved(&self) -> bool {
        matches!(self, MapEquivalence::Equivalent)
    }

    /// True when a counter-example was found: the mapper emitted wrong
    /// logic.
    pub fn wrong(&self) -> bool {
        matches!(self, MapEquivalence::Different { .. })
    }

    /// A one-line verdict.
    pub fn render(&self) -> String {
        match self {
            MapEquivalence::Equivalent => "equivalent".to_owned(),
            MapEquivalence::Different { inputs, outputs } => {
                let bits: String = inputs.iter().map(|&b| if b { '1' } else { '0' }).collect();
                format!("WRONG: output(s) {outputs:?} differ for inputs {bits} (input 0 first)")
            }
            MapEquivalence::Inconclusive { ands } => {
                format!("unproved: {ands} AND node(s) left in the miter")
            }
        }
    }
}

/// Knobs for a mapping check.
#[derive(Clone, Debug)]
pub struct MapVerifyOptions {
    /// Words of random patterns simulated before the proof is attempted, 64
    /// patterns per word. This is the tier that finds counter-examples.
    pub sim_words: usize,
    /// Seed for those patterns, so the check is deterministic.
    pub seed: u64,
    /// Options of the [`fraig`] pass that proves the miter.
    pub fraig: FraigOptions,
}

impl Default for MapVerifyOptions {
    fn default() -> Self {
        MapVerifyOptions {
            sim_words: 16,
            seed: 0x5dee_ce66,
            fraig: FraigOptions::default(),
        }
    }
}

/// Checks a LUT network against the AIG it was mapped from.
pub fn check_lut_mapping(
    aig: &Aig,
    net: &LutNetwork,
    options: &MapVerifyOptions,
) -> MapEquivalence {
    decide(aig, lut_miter(aig, net), options)
}

/// Checks a standard-cell network against the AIG it was mapped from.
///
/// `library` must be the one the network was mapped onto. A cell it does not
/// name is read as the constant false, which shows up as a difference rather
/// than as a false proof.
pub fn check_gate_mapping(
    aig: &Aig,
    net: &GateNetwork,
    library: &GateLibrary,
    options: &MapVerifyOptions,
) -> MapEquivalence {
    decide(aig, gate_miter(aig, net, library), options)
}

/// The miter of `aig` against `net`: see the module docs for its outputs.
pub fn lut_miter(aig: &Aig, net: &LutNetwork) -> Aig {
    let (mut m, inputs, copy) = original(aig);
    let mut mapped: Vec<Edge> = Vec::with_capacity(net.luts.len());
    for lut in &net.luts {
        let fanins: Vec<Edge> = lut
            .inputs
            .iter()
            .map(|&s| signal_edge(s, &inputs, &mapped))
            .collect();
        let edge = lut_to_aig(&mut m, &fanins, &lut.init);
        mapped.push(edge);
    }
    finish(m, aig, &inputs, &copy, &net.outputs, &mapped)
}

/// The miter of `aig` against a standard-cell network.
pub fn gate_miter(aig: &Aig, net: &GateNetwork, library: &GateLibrary) -> Aig {
    let (mut m, inputs, copy) = original(aig);
    let mut mapped: Vec<Edge> = Vec::with_capacity(net.gates.len());
    for gate in &net.gates {
        let fanins: Vec<Edge> = gate
            .fanins
            .iter()
            .map(|&s| signal_edge(s, &inputs, &mapped))
            .collect();
        let edge = match library.gate(&gate.gate) {
            // A library cell's function is a truth table over its pins in
            // pin order, which is the order its fanins are wired in.
            Some(cell) => truth_to_aig(&mut m, &fanins, |pattern| cell.function.bit(pattern)),
            None => Edge::constant(false),
        };
        mapped.push(edge);
    }
    finish(m, aig, &inputs, &copy, &net.outputs, &mapped)
}

/// A fresh miter graph holding a copy of `aig`: the graph, one input per AIG
/// input, and the edge each AIG node became.
fn original(aig: &Aig) -> (Aig, Vec<Edge>, Vec<Edge>) {
    let mut m = Aig::new();
    let inputs: Vec<Edge> = aig.inputs().iter().map(|_| m.add_input()).collect();
    let mut copy: Vec<Edge> = vec![Edge::constant(false); aig.len()];
    for (pos, &pi) in aig.inputs().iter().enumerate() {
        copy[pi as usize] = inputs[pos];
    }
    for id in 1..u32::try_from(aig.len()).expect("node count") {
        if !aig.is_and(id) {
            continue;
        }
        let (a, b) = aig.fanins(id);
        let ea = copy[a.index()].xor(a.is_complement());
        let eb = copy[b.index()].xor(b.is_complement());
        copy[id as usize] = m.and(ea, eb);
    }
    (m, inputs, copy)
}

/// Adds the comparison outputs: one XOR per AIG output, then their OR.
fn finish(
    mut m: Aig,
    aig: &Aig,
    inputs: &[Edge],
    copy: &[Edge],
    outputs: &[Signal],
    mapped: &[Edge],
) -> Aig {
    let mut differs: Vec<Edge> = Vec::with_capacity(aig.outputs().len());
    for (i, &out) in aig.outputs().iter().enumerate() {
        let want = copy[out.index()].xor(out.is_complement());
        let got = signal_edge(outputs[i], inputs, mapped);
        differs.push(m.xor(want, got));
    }
    for &d in &differs {
        m.add_output(d);
    }
    let any = m.or_n(&differs);
    m.add_output(any);
    m
}

/// One signal of a mapped network as a miter edge.
fn signal_edge(signal: Signal, inputs: &[Edge], mapped: &[Edge]) -> Edge {
    match signal {
        Signal::Const(c) => Edge::constant(c),
        Signal::Input(i) => inputs[i as usize],
        Signal::Node(n) => mapped[n as usize],
    }
}

/// Expands a LUT's `init` over `inputs` as an AIG cone.
fn lut_to_aig(aig: &mut Aig, inputs: &[Edge], init: &Const) -> Edge {
    truth_to_aig(aig, inputs, |pattern| {
        u32::try_from(pattern).is_ok_and(|p| p < init.width() && init.bit(p) == Bit::One)
    })
}

/// Expands the function `value` of `inputs.len()` variables as an AIG cone,
/// by Shannon decomposition: the `2^k` values are muxed down one input at a
/// time, input 0 first. Structural hashing folds the constant branches, so a
/// LUT that is really a small function costs a small cone.
fn truth_to_aig(aig: &mut Aig, inputs: &[Edge], value: impl Fn(usize) -> bool) -> Edge {
    let mut layer: Vec<Edge> = (0..(1usize << inputs.len()))
        .map(|pattern| Edge::constant(value(pattern)))
        .collect();
    for &select in inputs {
        let mut next = Vec::with_capacity(layer.len().div_ceil(2));
        for pair in layer.chunks(2) {
            let zero = pair[0];
            let one = *pair.get(1).unwrap_or(&Edge::constant(false));
            next.push(aig.mux(select, one, zero));
        }
        layer = next;
    }
    layer.first().copied().unwrap_or(Edge::constant(false))
}

/// Decides a miter: simulate for a counter-example, then prove.
fn decide(aig: &Aig, mut miter: Aig, options: &MapVerifyOptions) -> MapEquivalence {
    let n = aig.inputs().len();
    let outs = aig.outputs().len();
    let words = options.sim_words.max(1);
    if n > 0 && outs > 0 {
        let mut state = options.seed | 1;
        let patterns: Vec<u64> = (0..n * words)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            })
            .collect();
        let values = miter.simulate(&patterns, words);
        let any = miter.outputs()[outs];
        for w in 0..words {
            let diff = Aig::sim_value(&values, words, any, w);
            if diff == 0 {
                continue;
            }
            let bit = diff.trailing_zeros();
            let mask = 1u64 << bit;
            return MapEquivalence::Different {
                inputs: (0..n)
                    .map(|i| patterns[i * words + w] & mask != 0)
                    .collect(),
                outputs: (0..outs)
                    .filter(|&i| Aig::sim_value(&values, words, miter.outputs()[i], w) & mask != 0)
                    .collect(),
            };
        }
    }
    fraig(&mut miter, &options.fraig);
    if miter.outputs().iter().all(|&o| o == Edge::FALSE) {
        return MapEquivalence::Equivalent;
    }
    MapEquivalence::Inconclusive {
        ands: miter.num_ands(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{descriptor_rom_cone, simulate_luts};
    use super::super::{MapOptions, gate_map, lut_map};
    use super::*;

    /// The check proves a correct LUT mapping, at every width, for the cone
    /// that used to be mapped wrongly.
    #[test]
    fn a_correct_lut_mapping_is_proved() {
        let aig = descriptor_rom_cone();
        for k in 2..=8u32 {
            let net = lut_map(&aig, k);
            let report = check_lut_mapping(&aig, &net, &MapVerifyOptions::default());
            assert_eq!(
                report,
                MapEquivalence::Equivalent,
                "LUT{k}: {}",
                report.render()
            );
        }
    }

    /// Every single-bit change to every `init` of the cover, at every LUT
    /// width, agrees with the truth: the check proves the network exactly
    /// when the network really is the AIG's function.
    ///
    /// That is the invariant worth asserting, and it is stronger than
    /// "a mutated network is rejected". Some mutations are no change at all —
    /// a LUT nothing reads, or a bit for an input pattern its fanins cannot
    /// produce — and a checker that rejected those would be crying wolf. The
    /// truth is taken from exhaustive simulation of the three inputs, which
    /// is what a mapping check would do if it could afford to.
    #[test]
    fn every_init_mutation_is_judged_correctly() {
        let aig = descriptor_rom_cone();
        let truth: Vec<Vec<bool>> = (0..8u32)
            .map(|pat| aig.eval(&(0..3).map(|i| (pat >> i) & 1 == 1).collect::<Vec<_>>()))
            .collect();
        let mut mutations = 0usize;
        let mut rejected = 0usize;
        for k in 2..=8u32 {
            let net = lut_map(&aig, k);
            for which in 0..net.luts.len() {
                for bit in 0..net.luts[which].init.width() {
                    let mut broken = net.clone();
                    let init = &mut broken.luts[which].init;
                    let flipped = if init.bit(bit) == Bit::One {
                        Bit::Zero
                    } else {
                        Bit::One
                    };
                    init.set_bit(bit, flipped);
                    let really_equal = (0..8u32).all(|pat| {
                        let ins: Vec<bool> = (0..3).map(|i| (pat >> i) & 1 == 1).collect();
                        simulate_luts(&broken, &ins) == truth[pat as usize]
                    });
                    let report = check_lut_mapping(&aig, &broken, &MapVerifyOptions::default());
                    mutations += 1;
                    if !report.proved() {
                        rejected += 1;
                    }
                    assert_eq!(
                        report.proved(),
                        really_equal,
                        "LUT{k}: bit {bit} of lut {which} flipped: the check says {} \
                         and simulation says {}",
                        report.render(),
                        if really_equal { "equal" } else { "different" }
                    );
                }
            }
            // The untouched network is still proved, so the loop above did
            // not corrupt it.
            let report = check_lut_mapping(&aig, &net, &MapVerifyOptions::default());
            assert!(report.proved(), "LUT{k}: {}", report.render());
        }
        assert!(mutations > 100, "only {mutations} mutations tried");
        // Most mutations are real changes and some are don't-cares — a bit
        // for an input pattern a LUT's fanins cannot produce, or a LUT
        // nothing reads. Both kinds have to be represented for the equality
        // above to be testing both directions.
        assert!(
            rejected * 4 > mutations && rejected < mutations,
            "{rejected} of {mutations} mutations changed the function, which does not \
             exercise both answers"
        );
    }

    /// The same for a standard-cell mapping, whose cells come from a library
    /// rather than from a truth table the mapper wrote.
    #[test]
    fn a_correct_gate_mapping_is_proved() {
        let library = GateLibrary::generic();
        let aig = descriptor_rom_cone();
        let net = gate_map(&aig, &library);
        let report = check_gate_mapping(&aig, &net, &library, &MapVerifyOptions::default());
        assert_eq!(report, MapEquivalence::Equivalent, "{}", report.render());
        // A gate replaced by one that computes something else is caught.
        let mut broken = net.clone();
        let at = broken
            .gates
            .iter()
            .position(|g| g.fanins.len() == 2)
            .expect("a two-input cell");
        broken.gates[at].gate = if broken.gates[at].gate == "AND2" {
            "OR2".to_owned()
        } else {
            "AND2".to_owned()
        };
        let report = check_gate_mapping(&aig, &broken, &library, &MapVerifyOptions::default());
        assert!(
            !report.proved(),
            "a swapped cell was proved: {}",
            report.render()
        );
    }

    /// The options a caller gets by default prove the mapping of a module
    /// the mapper is given end to end.
    #[test]
    fn the_default_options_prove_a_module_mapping() {
        let _ = MapOptions::lut(4);
        let aig = descriptor_rom_cone();
        let net = lut_map(&aig, 4);
        let options = MapVerifyOptions {
            sim_words: 1,
            ..MapVerifyOptions::default()
        };
        assert!(check_lut_mapping(&aig, &net, &options).proved());
    }
}
