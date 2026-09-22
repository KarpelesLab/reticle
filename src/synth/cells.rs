//! Gate libraries for standard-cell mapping.
//!
//! A [`GateLibrary`] is a set of [`Gate`]s, each a named single-output
//! combinational cell described by
//!
//! - a **function**, as a truth table over its input pins (pin `i` is
//!   variable `i`, so bit `p` of the table is the output for the pin
//!   pattern `p` with pin 0 as the least significant bit);
//! - an **area** in arbitrary but consistent units; and
//! - a **delay per pin**, so the mapper can add a pin-to-output delay
//!   along a path.
//!
//! [`GateLibrary::generic`] is the small technology-independent library
//! the mapper uses when no real one is supplied: inverter, the two- and
//! three-input NAND/NOR, the buffered AND/OR, XOR/XNOR, a multiplexer and
//! the four common and-or-invert / or-and-invert cells. Its areas follow
//! the usual relative transistor counts of a static CMOS library (an
//! inverter is 1, a NAND2 is 2 and so on), which is enough for the mapper
//! to make sensible choices and is what an open PDK's numbers reduce to
//! after normalisation.
//!
//! [`GateLibrary::builder`] builds a library from scratch, which is how
//! the Liberty reader of phase 6 will construct one: it parses `.lib`
//! cells, evaluates each `function` attribute into a truth table and adds
//! it here, so nothing downstream needs to know where the library came
//! from.
//!
//! Gates are matched against cut functions by [`super::techmap::gate_map`]
//! through Boolean matching up to permutation and input negation (the "PN"
//! of NPN): a cut whose function equals a gate's function after permuting
//! the pins and complementing some of them can be implemented by that
//! gate, with an inverter on each complemented input. [`GateLibrary`]
//! precomputes that correspondence for every function of at most four
//! variables, preferring the match that needs the fewest inverters and
//! then the smallest area, so [`GateLibrary::match_function`] is a single
//! lookup.

use std::collections::HashMap;

use super::aig::truth::TruthTable;
use crate::ir::CellKind;

/// One standard cell: a name, a function, an area and per-pin delays.
#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    /// The cell name as the technology library spells it.
    pub name: String,
    /// The input pin names, in the order the truth table's variables use.
    pub pins: Vec<String>,
    /// The output pin name.
    pub output: String,
    /// The function over the pins.
    pub function: TruthTable,
    /// Area in library units.
    pub area: f64,
    /// Delay from each input pin to the output, in library units.
    pub delays: Vec<f64>,
    /// The IR primitive this cell is exactly, when one matches, so the
    /// mapper can emit a primitive instead of a black box.
    pub primitive: Option<CellKind>,
}

impl Gate {
    /// Number of input pins.
    pub fn arity(&self) -> usize {
        self.pins.len()
    }

    /// The delay from pin `i` to the output.
    pub fn delay(&self, pin: usize) -> f64 {
        self.delays.get(pin).copied().unwrap_or(0.0)
    }

    /// The largest pin-to-output delay.
    pub fn max_delay(&self) -> f64 {
        self.delays.iter().copied().fold(0.0, f64::max)
    }
}

/// A set of gates available to the standard-cell mapper.
#[derive(Clone, Debug, Default)]
pub struct GateLibrary {
    name: String,
    gates: Vec<Gate>,
    /// Index of the cheapest gate for each function of at most 4 inputs,
    /// keyed by the function extended to 4 variables.
    index: HashMap<u16, GateMatch>,
}

/// How a cut function is realised by a library gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateMatch {
    /// Position of the gate in [`GateLibrary::gates`].
    pub gate: usize,
    /// For each variable of the function, the gate pin it feeds. Variables
    /// the function does not depend on have no entry, so this is as long
    /// as the gate has pins.
    pub wiring: Vec<usize>,
    /// Bit `v` set: variable `v` must be complemented before it reaches
    /// its pin, which costs an inverter.
    pub invert: u32,
}

impl GateMatch {
    /// Number of inverters this match needs on its inputs.
    pub fn inverters(&self) -> u32 {
        self.invert.count_ones()
    }
}

impl GateLibrary {
    /// An empty library called `name`.
    pub fn new(name: impl Into<String>) -> GateLibrary {
        GateLibrary {
            name: name.into(),
            gates: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// A builder for a library called `name`.
    pub fn builder(name: impl Into<String>) -> GateLibraryBuilder {
        GateLibraryBuilder {
            library: GateLibrary::new(name),
        }
    }

    /// The library's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Every gate, in the order they were added.
    pub fn gates(&self) -> &[Gate] {
        &self.gates
    }

    /// The gate with the given name.
    pub fn gate(&self, name: &str) -> Option<&Gate> {
        self.gates.iter().find(|g| g.name == name)
    }

    /// The number of gates.
    pub fn len(&self) -> usize {
        self.gates.len()
    }

    /// True when the library has no gates.
    pub fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }

    /// The largest number of inputs any gate has.
    pub fn max_arity(&self) -> usize {
        self.gates.iter().map(Gate::arity).max().unwrap_or(0)
    }

    /// The inverter, if the library has one.
    pub fn inverter(&self) -> Option<&Gate> {
        self.gates
            .iter()
            .filter(|g| g.arity() == 1 && g.function.as_u64() & 0b11 == 0b01)
            .min_by(|a, b| a.area.total_cmp(&b.area))
    }

    /// Adds a gate and re-indexes the library.
    pub fn add(&mut self, gate: Gate) {
        self.gates.push(gate);
        self.reindex();
    }

    /// Rebuilds the match index.
    ///
    /// For every gate of at most four inputs, every permutation of its
    /// pins and every combination of complemented inputs, the function so
    /// realised (extended to four variables) is recorded, keeping the
    /// match that needs the fewest inverters and then the least area.
    fn reindex(&mut self) {
        self.index.clear();
        let inverter_area = self
            .gates
            .iter()
            .filter(|g| g.arity() == 1 && g.function.as_u64() & 0b11 == 0b01)
            .map(|g| g.area)
            .fold(f64::INFINITY, f64::min);
        for (i, gate) in self.gates.iter().enumerate() {
            if gate.arity() > 4 || gate.arity() == 0 {
                continue;
            }
            let mut perm: Vec<usize> = (0..gate.arity()).collect();
            loop {
                // `perm` maps gate pin `p` to the variable `perm[p]` that
                // feeds it; the inverse says which pin a variable feeds.
                let mut wiring = vec![0usize; perm.len()];
                for (p, &v) in perm.iter().enumerate() {
                    wiring[v] = p;
                }
                for negate in 0..(1u32 << gate.arity()) {
                    // `negate` is a mask over the gate's *pins*; translate
                    // it to a mask over the function's variables.
                    let mut var_mask = 0u32;
                    for (v, &pin) in wiring.iter().enumerate() {
                        if (negate >> pin) & 1 == 1 {
                            var_mask |= 1 << v;
                        }
                    }
                    let realised = gate
                        .function
                        .permute(&perm)
                        .negate_inputs(var_mask)
                        .extend(4);
                    let key = u16::try_from(realised.as_u64() & 0xFFFF).expect("16 bits");
                    let candidate_cost = gate.area
                        + f64::from(var_mask.count_ones())
                            * if inverter_area.is_finite() {
                                inverter_area
                            } else {
                                0.0
                            };
                    let better = match self.index.get(&key) {
                        None => true,
                        Some(existing) => {
                            let other = &self.gates[existing.gate];
                            let other_cost = other.area
                                + f64::from(existing.inverters())
                                    * if inverter_area.is_finite() {
                                        inverter_area
                                    } else {
                                        0.0
                                    };
                            // Fewest inverters first, then cost, then the
                            // smaller gate, so the choice is stable.
                            (var_mask.count_ones(), candidate_cost, gate.arity())
                                < (existing.inverters(), other_cost, other.arity())
                        }
                    };
                    if better {
                        self.index.insert(
                            key,
                            GateMatch {
                                gate: i,
                                wiring: wiring.clone(),
                                invert: var_mask,
                            },
                        );
                    }
                }
                if !super::aig::truth::next_permutation(&mut perm) {
                    break;
                }
            }
        }
    }

    /// The best gate realising `function` (a table over at most four
    /// variables), with the pin each variable feeds and which of them need
    /// an inverter.
    ///
    /// The function must depend on every one of its variables; the mapper
    /// restricts cut functions to their support before matching.
    pub fn match_function(&self, function: &TruthTable) -> Option<(&Gate, GateMatch)> {
        if function.vars() > 4 {
            return None;
        }
        let key = u16::try_from(function.extend(4).as_u64() & 0xFFFF).expect("16 bits");
        let m = self.index.get(&key)?;
        Some((&self.gates[m.gate], m.clone()))
    }

    /// The cost of implementing `function`: the gate's area plus an
    /// inverter for each complemented input.
    pub fn match_cost(&self, function: &TruthTable) -> Option<f64> {
        let (gate, m) = self.match_function(function)?;
        let inv = self.inverter().map_or(0.0, |g| g.area);
        Some(gate.area + f64::from(m.inverters()) * inv)
    }

    /// The small technology-independent library: `INV`, `NAND2`, `NOR2`,
    /// `AND2`, `OR2`, `XOR2`, `XNOR2`, `MUX2`, `AOI21`, `OAI21`, `AOI22`,
    /// `OAI22`, `NAND3`, `NOR3`, with relative static-CMOS areas.
    pub fn generic() -> GateLibrary {
        // Variables over four inputs; a gate's own table uses as many as
        // it has pins.
        let v = |n: usize, i: usize| TruthTable::var(n, i);
        let mut b = GateLibrary::builder("generic");

        // INV: !a.
        b.gate("INV", &["A"], v(1, 0).not(), 1.0, &[1.0])
            .primitive(CellKind::Not);
        // Buffer-free two-input gates.
        b.gate(
            "NAND2",
            &["A", "B"],
            v(2, 0).and(&v(2, 1)).not(),
            2.0,
            &[1.4, 1.4],
        );
        b.gate(
            "NOR2",
            &["A", "B"],
            v(2, 0).or(&v(2, 1)).not(),
            2.0,
            &[1.6, 1.6],
        );
        b.gate("AND2", &["A", "B"], v(2, 0).and(&v(2, 1)), 3.0, &[2.0, 2.0])
            .primitive(CellKind::And);
        b.gate("OR2", &["A", "B"], v(2, 0).or(&v(2, 1)), 3.0, &[2.2, 2.2])
            .primitive(CellKind::Or);
        b.gate("XOR2", &["A", "B"], v(2, 0).xor(&v(2, 1)), 5.0, &[2.6, 2.6])
            .primitive(CellKind::Xor);
        b.gate(
            "XNOR2",
            &["A", "B"],
            v(2, 0).xor(&v(2, 1)).not(),
            5.0,
            &[2.6, 2.6],
        );
        // Three-input gates.
        b.gate(
            "NAND3",
            &["A", "B", "C"],
            v(3, 0).and(&v(3, 1)).and(&v(3, 2)).not(),
            3.0,
            &[1.8, 1.8, 1.8],
        );
        b.gate(
            "NOR3",
            &["A", "B", "C"],
            v(3, 0).or(&v(3, 1)).or(&v(3, 2)).not(),
            3.0,
            &[2.2, 2.2, 2.2],
        );
        // `S ? B : A`, matching the IR's `Mux` port order (`s = 1` picks
        // `b`), with the select as the last pin.
        let mux = v(3, 1).and(&v(3, 2)).or(&v(3, 0).and(&v(3, 2).not()));
        b.gate("MUX2", &["A", "B", "S"], mux, 5.0, &[2.4, 2.4, 2.8])
            .primitive(CellKind::Mux);
        // And-or-invert and or-and-invert.
        b.gate(
            "AOI21",
            &["A", "B", "C"],
            v(3, 0).and(&v(3, 1)).or(&v(3, 2)).not(),
            3.0,
            &[2.0, 2.0, 1.8],
        );
        b.gate(
            "OAI21",
            &["A", "B", "C"],
            v(3, 0).or(&v(3, 1)).and(&v(3, 2)).not(),
            3.0,
            &[2.0, 2.0, 1.8],
        );
        b.gate(
            "AOI22",
            &["A", "B", "C", "D"],
            v(4, 0).and(&v(4, 1)).or(&v(4, 2).and(&v(4, 3))).not(),
            4.0,
            &[2.2, 2.2, 2.2, 2.2],
        );
        b.gate(
            "OAI22",
            &["A", "B", "C", "D"],
            v(4, 0).or(&v(4, 1)).and(&v(4, 2).or(&v(4, 3))).not(),
            4.0,
            &[2.2, 2.2, 2.2, 2.2],
        );
        b.finish()
    }
}

/// Builds a [`GateLibrary`] gate by gate.
pub struct GateLibraryBuilder {
    library: GateLibrary,
}

impl GateLibraryBuilder {
    /// Adds a gate with the given pins, function, area and per-pin delays,
    /// and returns a handle for optional extras.
    pub fn gate(
        &mut self,
        name: impl Into<String>,
        pins: &[&str],
        function: TruthTable,
        area: f64,
        delays: &[f64],
    ) -> GateHandle<'_> {
        assert_eq!(
            function.vars(),
            pins.len(),
            "function arity must match pins"
        );
        assert_eq!(delays.len(), pins.len(), "one delay per pin");
        self.library.gates.push(Gate {
            name: name.into(),
            pins: pins.iter().map(|p| (*p).to_owned()).collect(),
            output: "Y".to_owned(),
            function,
            area,
            delays: delays.to_vec(),
            primitive: None,
        });
        GateHandle {
            gate: self.library.gates.last_mut().expect("just pushed"),
        }
    }

    /// The finished library, with its match index built.
    pub fn finish(mut self) -> GateLibrary {
        self.library.reindex();
        self.library
    }
}

/// A handle to the gate just added, for setting its optional fields.
pub struct GateHandle<'a> {
    gate: &'a mut Gate,
}

impl GateHandle<'_> {
    /// Records that this cell is exactly an IR primitive, so the mapper
    /// emits that primitive rather than a black box.
    pub fn primitive(self, kind: CellKind) -> Self {
        self.gate.primitive = Some(kind);
        self
    }

    /// Renames the output pin (the default is `Y`).
    pub fn output(self, name: impl Into<String>) -> Self {
        self.gate.output = name.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_library_has_the_expected_cells() {
        let lib = GateLibrary::generic();
        assert_eq!(lib.name(), "generic");
        assert_eq!(lib.len(), 14);
        assert!(!lib.is_empty());
        assert_eq!(lib.max_arity(), 4);
        for name in [
            "INV", "NAND2", "NOR2", "AND2", "OR2", "XOR2", "XNOR2", "MUX2", "AOI21", "OAI21",
            "AOI22", "OAI22", "NAND3", "NOR3",
        ] {
            assert!(lib.gate(name).is_some(), "missing {name}");
        }
        assert_eq!(lib.gate("nope"), None);
        assert_eq!(lib.inverter().map(|g| g.name.as_str()), Some("INV"));
        let nand2 = lib.gate("NAND2").unwrap();
        assert_eq!(nand2.arity(), 2);
        assert_eq!(nand2.function.as_u64(), 0x7);
        assert_eq!(nand2.delay(0), 1.4);
        assert_eq!(nand2.delay(9), 0.0);
        assert_eq!(nand2.max_delay(), 1.4);
        assert_eq!(lib.gate("AND2").unwrap().primitive, Some(CellKind::And));
        assert_eq!(lib.gate("AOI21").unwrap().primitive, None);
        // MUX2 computes `s ? b : a`.
        let mux = &lib.gate("MUX2").unwrap().function;
        for pat in 0..8usize {
            let (a, b, s) = (pat & 1 == 1, pat & 2 == 2, pat & 4 == 4);
            assert_eq!(mux.bit(pat), if s { b } else { a }, "mux {pat}");
        }
    }

    #[test]
    fn matching_finds_the_cheapest_gate_and_the_wiring() {
        let lib = GateLibrary::generic();
        let v = |i| TruthTable::var(2, i);
        // AND of two variables: AND2, pins in order, no inverters.
        let (g, m) = lib.match_function(&v(0).and(&v(1))).unwrap();
        assert_eq!(g.name, "AND2");
        assert_eq!(m.wiring, [0, 1]);
        assert_eq!(m.inverters(), 0);
        // NAND is a cell of its own, so it needs no inverter either.
        let (g, m) = lib.match_function(&v(0).and(&v(1)).not()).unwrap();
        assert_eq!(g.name, "NAND2");
        assert_eq!(m.inverters(), 0);
        // The inverter itself.
        let (g, _) = lib.match_function(&TruthTable::var(1, 0).not()).unwrap();
        assert_eq!(g.name, "INV");
        // `a & !b` is no cell of the generic library, so it is matched
        // with one input inverted.
        let f = v(0).and(&v(1).not());
        let (_, m) = lib.match_function(&f).unwrap();
        assert_eq!(m.inverters(), 1);
        // Its cost includes that inverter.
        let (gate, _) = lib.match_function(&f).unwrap();
        assert_eq!(
            lib.match_cost(&f),
            Some(gate.area + lib.inverter().unwrap().area)
        );
        // AOI21 with the pins in a different order still matches, and the
        // wiring says which pin each variable feeds.
        let w = |i| TruthTable::var(3, i);
        let f = w(2).or(&w(0).and(&w(1))).not();
        let (g, m) = lib.match_function(&f).unwrap();
        assert_eq!(g.name, "AOI21");
        assert_eq!(m.inverters(), 0);
        // Re-deriving the function through the wiring reproduces `f`.
        let pins: Vec<TruthTable> = (0..3)
            .map(|pin| {
                let var = m.wiring.iter().position(|&p| p == pin).expect("pin driven");
                w(var)
            })
            .collect();
        let rebuilt = pins[0].and(&pins[1]).or(&pins[2]).not();
        assert_eq!(rebuilt, f);
        // Five variables are beyond any gate.
        assert!(lib.match_function(&TruthTable::var(5, 0)).is_none());
        assert!(lib.match_cost(&TruthTable::var(5, 0)).is_none());
    }

    /// Every match the library reports really computes the function it
    /// was asked for.
    ///
    /// A small library does not cover every function of three or four
    /// variables — `(a ^ b) & !c` is not a generic cell — and the mapper
    /// handles that by falling back to a cut it can implement. What must
    /// hold is that *every two-variable function* matches, since that is
    /// the fallback (one AIG node is a two-input function), and that no
    /// reported match is wrong.
    #[test]
    fn every_reported_match_is_correct() {
        let lib = GateLibrary::generic();
        for vars in 1..=4usize {
            for tt in 0..(1u64 << (1 << vars)) {
                let f = TruthTable::from_u64(vars, tt);
                if f.support() != (1u32 << vars) - 1 {
                    continue; // matched at its own arity instead
                }
                let Some((gate, m)) = lib.match_function(&f) else {
                    assert!(vars > 2, "{vars} vars, tt {tt:#x} has no match");
                    continue;
                };
                // Feed the gate's pins with the (possibly inverted)
                // variables the match names and check the result.
                let pins: Vec<TruthTable> = (0..gate.arity())
                    .map(|pin| {
                        let var = m.wiring.iter().position(|&p| p == pin).expect("pin driven");
                        let t = TruthTable::var(vars, var);
                        if (m.invert >> var) & 1 == 1 {
                            t.not()
                        } else {
                            t
                        }
                    })
                    .collect();
                let realised = apply(&gate.function, &pins, vars);
                assert_eq!(realised, f, "{} on {vars} vars, tt {tt:#x}", gate.name);
                assert!(lib.match_cost(&f).is_some());
            }
        }
    }

    /// Composes `gate` with the given tables on its pins.
    fn apply(gate: &TruthTable, pins: &[TruthTable], vars: usize) -> TruthTable {
        let mut out = TruthTable::constant(vars, false);
        for pattern in 0..out.len() {
            // The pin pattern this input pattern produces.
            let mut pin_pattern = 0usize;
            for (pin, table) in pins.iter().enumerate() {
                if table.bit(pattern) {
                    pin_pattern |= 1 << pin;
                }
            }
            if gate.bit(pin_pattern) {
                out = out.or(&minterm(vars, pattern));
            }
        }
        out
    }

    /// The table that is true only for `pattern`.
    fn minterm(vars: usize, pattern: usize) -> TruthTable {
        let mut t = TruthTable::constant(vars, true);
        for v in 0..vars {
            let var = TruthTable::var(vars, v);
            t = t.and(&if (pattern >> v) & 1 == 1 {
                var
            } else {
                var.not()
            });
        }
        t
    }

    #[test]
    fn a_custom_library_can_be_built() {
        let mut b = GateLibrary::builder("tiny");
        b.gate("not_a", &["I"], TruthTable::var(1, 0).not(), 0.5, &[0.3])
            .primitive(CellKind::Not)
            .output("ZN");
        let mut lib = b.finish();
        assert_eq!(lib.name(), "tiny");
        assert_eq!(lib.gates().len(), 1);
        assert_eq!(lib.gates()[0].output, "ZN");
        assert_eq!(lib.inverter().map(|g| g.name.as_str()), Some("not_a"));
        let v = |i| TruthTable::var(2, i);
        assert!(lib.match_function(&v(0).and(&v(1))).is_none());
        assert!(lib.match_cost(&v(0).and(&v(1))).is_none());
        lib.add(Gate {
            name: "and2".to_owned(),
            pins: vec!["A".to_owned(), "B".to_owned()],
            output: "Y".to_owned(),
            function: v(0).and(&v(1)),
            area: 2.0,
            delays: vec![1.0, 1.0],
            primitive: Some(CellKind::And),
        });
        assert!(lib.match_function(&v(0).and(&v(1))).is_some());
        assert_eq!(GateLibrary::new("e").len(), 0);
    }
}
