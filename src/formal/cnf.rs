//! Tseitin encoding of gate-level logic into CNF.
//!
//! The Tseitin transformation introduces one fresh variable per gate and
//! adds clauses tying it to the gate's inputs, so the resulting CNF is
//! linear in the circuit size and equisatisfiable with it. [`CnfBuilder`]
//! does that for the gates a bit-blaster needs (`and`, `or`, `xor`, `ite`,
//! equality, adders, multipliers) and applies two standard optimisations:
//!
//! - **Constant folding**: variable 0 is reserved as the constant *true*,
//!   and every gate with a constant or repeated input is simplified away
//!   instead of encoded.
//! - **Structural hashing**: gates are normalised (commutative inputs
//!   sorted, negations pushed out of `xor` and `ite`) and looked up in a
//!   table, so the same sub-circuit built twice yields the same literal.
//!   This is the CNF-level counterpart of an AIG's strashing.
//!
//! The encoding uses the full (two-sided) Tseitin clauses for every gate,
//! since the bit-blaster does not know which polarity a gate will be needed
//! in, and adds the redundant "propagation" clauses for `ite` and `xor`
//! that let unit propagation see through them in every direction.
//!
//! ```
//! use reticle::formal::cnf::CnfBuilder;
//! use reticle::formal::sat::SolveResult;
//!
//! let mut b = CnfBuilder::new();
//! let a = b.new_var();
//! let c = b.new_var();
//! let sum = b.add(&[a], &[c]); // 1-bit adder: [sum, carry]
//! b.add_unit(sum[1]);          // demand the carry
//! let mut s = b.into_solver();
//! assert_eq!(s.solve(), SolveResult::Sat);
//! assert_eq!(s.value(a.var()), Some(true));
//! assert_eq!(s.value(c.var()), Some(true));
//! ```

use std::collections::HashMap;

use super::sat::{Lit, Solver, Var};

/// A gate in normalised form: the key of the structural hash table, and
/// the definition of the variable the gate introduced, which
/// [`CnfBuilder::gate`] exposes so that a circuit can be read back out of
/// the clauses (as [`super::sweep`] does to sweep it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Gate {
    /// `a & b` with `a < b`.
    And(Lit, Lit),
    /// `a ^ b` with both positive and `a < b`.
    Xor(Lit, Lit),
    /// `c ? t : e` with `c` and `t` positive: condition, then, else.
    Ite(Lit, Lit, Lit),
}

/// Builds a CNF from gates, with constant folding and structural hashing.
///
/// Variable 0 is the constant true; [`CnfBuilder::new`] adds its unit
/// clause. Gates return a literal for their output; when the output is a
/// constant or an input, no clauses are added.
#[derive(Clone, Debug)]
pub struct CnfBuilder {
    num_vars: u32,
    lits: Vec<Lit>,
    /// `bounds[i]..bounds[i + 1]` is clause `i` in `lits`.
    bounds: Vec<usize>,
    gates: HashMap<Gate, Lit>,
    /// The gate defining each variable, indexed by variable; `None` for
    /// the constant and for free variables.
    defs: Vec<Option<Gate>>,
    /// Indices of the clauses added through [`CnfBuilder::add_clause`]
    /// (and its wrappers) rather than by a gate's encoding.
    constraints: Vec<usize>,
}

impl Default for CnfBuilder {
    fn default() -> CnfBuilder {
        CnfBuilder::new()
    }
}

impl CnfBuilder {
    /// An empty builder holding only the constant-true variable.
    pub fn new() -> CnfBuilder {
        let mut b = CnfBuilder {
            num_vars: 1,
            lits: Vec::new(),
            bounds: vec![0],
            gates: HashMap::new(),
            defs: vec![None],
            constraints: Vec::new(),
        };
        b.add_unit(b.true_lit());
        b
    }

    /// The constant true literal.
    pub fn true_lit(&self) -> Lit {
        Lit::pos(Var::new(0))
    }

    /// The constant false literal.
    pub fn false_lit(&self) -> Lit {
        Lit::neg(Var::new(0))
    }

    /// Number of variables, including the constant.
    pub fn num_vars(&self) -> u32 {
        self.num_vars
    }

    /// Number of clauses so far.
    pub fn num_clauses(&self) -> usize {
        self.bounds.len() - 1
    }

    /// Adds a fresh variable and returns its positive literal.
    pub fn new_var(&mut self) -> Lit {
        let v = Var::new(self.num_vars);
        self.num_vars += 1;
        self.defs.push(None);
        Lit::pos(v)
    }

    /// Adds `n` fresh variables (a bit vector, LSB first).
    pub fn new_vars(&mut self, n: usize) -> Vec<Lit> {
        (0..n).map(|_| self.new_var()).collect()
    }

    /// Adds a clause verbatim. It is a *constraint*: part of the formula,
    /// but not the definition of any gate (see
    /// [`CnfBuilder::constraints`]).
    pub fn add_clause(&mut self, lits: &[Lit]) {
        self.constraints.push(self.num_clauses());
        self.push_clause(lits);
    }

    /// Adds one clause of a gate's definition.
    fn push_clause(&mut self, lits: &[Lit]) {
        self.lits.extend_from_slice(lits);
        self.bounds.push(self.lits.len());
    }

    /// Records the gate `key` as the definition of its fresh output `y`.
    fn define(&mut self, key: Gate, y: Lit) {
        self.gates.insert(key, y);
        self.defs[y.var().index() as usize] = Some(key);
    }

    /// The gate whose output is `v`, or `None` for the constant and for a
    /// free variable (an input, from the circuit's point of view).
    ///
    /// Every gate output is a fresh variable created after its operands,
    /// so reading the definitions in variable order is a topological
    /// walk of the circuit.
    pub fn gate(&self, v: Var) -> Option<Gate> {
        self.defs.get(v.index() as usize).copied().flatten()
    }

    /// The clauses that are not part of any gate's definition: the unit
    /// fixing the constant, and everything added through
    /// [`CnfBuilder::add_clause`], [`CnfBuilder::add_unit`] and
    /// [`CnfBuilder::add_eq`]. The gate clauses only define outputs in
    /// terms of inputs and are satisfiable for every input assignment,
    /// so these are what actually constrain the formula.
    pub fn constraints(&self) -> impl Iterator<Item = &[Lit]> {
        self.constraints
            .iter()
            .map(|&i| &self.lits[self.bounds[i]..self.bounds[i + 1]])
    }

    /// Asserts `l`.
    pub fn add_unit(&mut self, l: Lit) {
        self.add_clause(&[l]);
    }

    /// Asserts `a == b`.
    pub fn add_eq(&mut self, a: Lit, b: Lit) {
        self.add_clause(&[!a, b]);
        self.add_clause(&[a, !b]);
    }

    /// Asserts two bit vectors of equal width are equal.
    pub fn add_eq_vec(&mut self, a: &[Lit], b: &[Lit]) {
        assert_eq!(a.len(), b.len(), "width mismatch");
        for (&x, &y) in a.iter().zip(b) {
            self.add_eq(x, y);
        }
    }

    /// The clauses added so far.
    pub fn clauses(&self) -> impl ExactSizeIterator<Item = &[Lit]> {
        self.bounds.windows(2).map(|w| &self.lits[w[0]..w[1]])
    }

    /// Adds every variable and clause to `solver`. Returns `false` if the
    /// solver became unsatisfiable at the top level.
    pub fn load_into(&self, solver: &mut Solver) -> bool {
        while solver.num_vars() < self.num_vars as usize {
            solver.new_var();
        }
        let mut ok = true;
        for clause in self.clauses() {
            ok = solver.add_clause(clause);
            if !ok {
                break;
            }
        }
        ok
    }

    /// A fresh solver loaded with the clauses.
    pub fn into_solver(self) -> Solver {
        let mut solver = Solver::new();
        self.load_into(&mut solver);
        solver
    }

    fn is_const(&self, l: Lit) -> bool {
        l.var().index() == 0
    }

    // ----- gates ----------------------------------------------------------

    /// `!a`. Free: literals carry their own sign.
    pub fn not(&self, a: Lit) -> Lit {
        !a
    }

    /// `a & b`.
    pub fn and(&mut self, a: Lit, b: Lit) -> Lit {
        let t = self.true_lit();
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        if a == b {
            return a;
        }
        if a == !b {
            return !t;
        }
        if self.is_const(a) {
            // `b` is not constant (constants sort first and a != b, !b).
            return if a == t { b } else { !t };
        }
        let key = Gate::And(a, b);
        if let Some(&y) = self.gates.get(&key) {
            return y;
        }
        let y = self.new_var();
        self.push_clause(&[!y, a]);
        self.push_clause(&[!y, b]);
        self.push_clause(&[y, !a, !b]);
        self.define(key, y);
        y
    }

    /// `a | b`, as `!(!a & !b)` so it shares with `and`.
    pub fn or(&mut self, a: Lit, b: Lit) -> Lit {
        !self.and(!a, !b)
    }

    /// `a ^ b`.
    pub fn xor(&mut self, a: Lit, b: Lit) -> Lit {
        let t = self.true_lit();
        // Pull the signs out: a ^ b == (a' ^ b') ^ (sa ^ sb).
        let flip = a.is_neg() ^ b.is_neg();
        let (a, b) = (Lit::pos(a.var()), Lit::pos(b.var()));
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        let y = if a == b {
            !t
        } else if a == t {
            // Constants sort first, so `a` is the constant if either is.
            !b
        } else {
            let key = Gate::Xor(a, b);
            match self.gates.get(&key) {
                Some(&y) => y,
                None => {
                    let y = self.new_var();
                    self.push_clause(&[!y, a, b]);
                    self.push_clause(&[!y, !a, !b]);
                    self.push_clause(&[y, !a, b]);
                    self.push_clause(&[y, a, !b]);
                    self.define(key, y);
                    y
                }
            }
        };
        if flip { !y } else { y }
    }

    /// `a == b` (xnor).
    pub fn eq(&mut self, a: Lit, b: Lit) -> Lit {
        !self.xor(a, b)
    }

    /// `c ? t : e`.
    pub fn ite(&mut self, c: Lit, t: Lit, e: Lit) -> Lit {
        let one = self.true_lit();
        // Normalise the condition positive.
        let (c, t, e) = if c.is_neg() { (!c, e, t) } else { (c, t, e) };
        if c == one {
            return t;
        }
        if t == e {
            return t;
        }
        if t == !e {
            return self.xor(c, e);
        }
        // Absorb the condition appearing in a branch.
        let t = if t == c {
            one
        } else if t == !c {
            !one
        } else {
            t
        };
        let e = if e == c {
            !one
        } else if e == !c {
            one
        } else {
            e
        };
        if self.is_const(t) || self.is_const(e) {
            return match (t == one, e == one) {
                (true, true) => one,
                (false, false) if self.is_const(t) && self.is_const(e) => !one,
                (true, false) if self.is_const(e) => c,
                (false, true) if self.is_const(t) => !c,
                (true, false) => self.or(c, e),
                (false, true) => self.or(!c, t),
                (false, false) if self.is_const(t) => self.and(!c, e),
                (false, false) => self.and(c, t),
            };
        }
        // Normalise the "then" branch positive: c ? !t : !e == !(c ? t : e).
        let flip = t.is_neg();
        let (t, e) = if flip { (!t, !e) } else { (t, e) };
        let key = Gate::Ite(c, t, e);
        let y = match self.gates.get(&key) {
            Some(&y) => y,
            None => {
                let y = self.new_var();
                self.push_clause(&[!y, !c, t]);
                self.push_clause(&[!y, c, e]);
                self.push_clause(&[y, !c, !t]);
                self.push_clause(&[y, c, !e]);
                // Redundant but propagation-strengthening.
                self.push_clause(&[!y, t, e]);
                self.push_clause(&[y, !t, !e]);
                self.define(key, y);
                y
            }
        };
        if flip { !y } else { y }
    }

    /// Conjunction of any number of literals (true when empty), built as a
    /// balanced tree.
    pub fn and_n(&mut self, lits: &[Lit]) -> Lit {
        match lits {
            [] => self.true_lit(),
            [a] => *a,
            _ => {
                let (l, r) = lits.split_at(lits.len() / 2);
                let l = self.and_n(l);
                let r = self.and_n(r);
                self.and(l, r)
            }
        }
    }

    /// Disjunction of any number of literals (false when empty).
    pub fn or_n(&mut self, lits: &[Lit]) -> Lit {
        let negated: Vec<Lit> = lits.iter().map(|&l| !l).collect();
        !self.and_n(&negated)
    }

    /// Parity of any number of literals (false when empty).
    pub fn xor_n(&mut self, lits: &[Lit]) -> Lit {
        match lits {
            [] => self.false_lit(),
            [a] => *a,
            _ => {
                let (l, r) = lits.split_at(lits.len() / 2);
                let l = self.xor_n(l);
                let r = self.xor_n(r);
                self.xor(l, r)
            }
        }
    }

    /// `a == b` for bit vectors of equal width.
    pub fn eq_vec(&mut self, a: &[Lit], b: &[Lit]) -> Lit {
        assert_eq!(a.len(), b.len(), "width mismatch");
        let bits: Vec<Lit> = a.iter().zip(b).map(|(&x, &y)| self.eq(x, y)).collect();
        self.and_n(&bits)
    }

    /// A full adder: returns `(sum, carry)`.
    pub fn full_adder(&mut self, a: Lit, b: Lit, cin: Lit) -> (Lit, Lit) {
        let axb = self.xor(a, b);
        let sum = self.xor(axb, cin);
        // carry = (a & b) | (cin & (a ^ b)), which is ite(axb, cin, a).
        let carry = self.ite(axb, cin, a);
        (sum, carry)
    }

    /// Ripple-carry addition of two equal-width vectors (LSB first). The
    /// result has one more bit; the last bit is the carry out.
    pub fn add(&mut self, a: &[Lit], b: &[Lit]) -> Vec<Lit> {
        assert_eq!(a.len(), b.len(), "width mismatch");
        let mut out = Vec::with_capacity(a.len() + 1);
        let mut carry = self.false_lit();
        for (&x, &y) in a.iter().zip(b) {
            let (s, c) = self.full_adder(x, y, carry);
            out.push(s);
            carry = c;
        }
        out.push(carry);
        out
    }

    /// Shift-and-add multiplication of unsigned vectors (LSB first). The
    /// result is `a.len() + b.len()` bits wide, so it never overflows.
    pub fn mul(&mut self, a: &[Lit], b: &[Lit]) -> Vec<Lit> {
        let width = a.len() + b.len();
        let f = self.false_lit();
        let mut acc = vec![f; width];
        for (i, &bi) in b.iter().enumerate() {
            // Partial product a << i, gated by b[i], zero-extended.
            let mut pp = vec![f; width];
            for (j, &aj) in a.iter().enumerate() {
                if i + j < width {
                    pp[i + j] = self.and(aj, bi);
                }
            }
            let sum = self.add(&acc, &pp);
            acc = sum[..width].to_vec();
        }
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formal::sat::SolveResult;

    /// Asserts, for every input assignment, that the gate output can only
    /// take the value `f` says.
    fn check_truth_table(inputs: &[Lit], out: Lit, b: &CnfBuilder, f: impl Fn(&[bool]) -> bool) {
        let mut s = b.clone().into_solver();
        let n = inputs.len();
        for bits in 0..(1u32 << n) {
            let values: Vec<bool> = (0..n).map(|i| bits >> i & 1 == 1).collect();
            let mut assumptions: Vec<Lit> = inputs
                .iter()
                .zip(&values)
                .map(|(&l, &v)| if v { l } else { !l })
                .collect();
            let expect = f(&values);
            assumptions.push(if expect { out } else { !out });
            assert_eq!(
                s.solve_with_assumptions(&assumptions),
                SolveResult::Sat,
                "inputs {values:?} must allow output {expect}"
            );
            let last = assumptions.len() - 1;
            assumptions[last] = !assumptions[last];
            assert_eq!(
                s.solve_with_assumptions(&assumptions),
                SolveResult::Unsat,
                "inputs {values:?} must force output {expect}"
            );
        }
    }

    #[test]
    fn binary_gates_match_truth_tables() {
        let mut b = CnfBuilder::new();
        let x = b.new_var();
        let y = b.new_var();
        let and = b.and(x, y);
        let or = b.or(x, y);
        let xor = b.xor(x, y);
        let eq = b.eq(x, y);
        let nand = b.not(and);
        check_truth_table(&[x, y], and, &b, |v| v[0] && v[1]);
        check_truth_table(&[x, y], or, &b, |v| v[0] || v[1]);
        check_truth_table(&[x, y], xor, &b, |v| v[0] ^ v[1]);
        check_truth_table(&[x, y], eq, &b, |v| v[0] == v[1]);
        check_truth_table(&[x, y], nand, &b, |v| !(v[0] && v[1]));
        // Mixed polarities go through the normalisation paths.
        let a2 = b.and(!x, y);
        let x2 = b.xor(!x, !y);
        let x3 = b.xor(x, !y);
        check_truth_table(&[x, y], a2, &b, |v| !v[0] && v[1]);
        check_truth_table(&[x, y], x2, &b, |v| v[0] ^ v[1]);
        check_truth_table(&[x, y], x3, &b, |v| !(v[0] ^ v[1]));
    }

    #[test]
    fn ite_matches_truth_table() {
        let mut b = CnfBuilder::new();
        let c = b.new_var();
        let t = b.new_var();
        let e = b.new_var();
        let y = b.ite(c, t, e);
        check_truth_table(&[c, t, e], y, &b, |v| if v[0] { v[1] } else { v[2] });
        let y2 = b.ite(!c, !t, e);
        check_truth_table(&[c, t, e], y2, &b, |v| if !v[0] { !v[1] } else { v[2] });
        let y3 = b.ite(c, t, !e);
        check_truth_table(&[c, t, e], y3, &b, |v| if v[0] { v[1] } else { !v[2] });
        // Condition appearing in a branch.
        let y4 = b.ite(c, c, e);
        check_truth_table(&[c, t, e], y4, &b, |v| if v[0] { true } else { v[2] });
        let y5 = b.ite(c, t, c);
        check_truth_table(&[c, t, e], y5, &b, |v| if v[0] { v[1] } else { false });
        let y6 = b.ite(c, !c, e);
        check_truth_table(&[c, t, e], y6, &b, |v| if v[0] { false } else { v[2] });
        let y7 = b.ite(c, t, !c);
        check_truth_table(&[c, t, e], y7, &b, |v| if v[0] { v[1] } else { true });
    }

    #[test]
    fn constant_folding_adds_no_clauses() {
        let mut b = CnfBuilder::new();
        let t = b.true_lit();
        let f = b.false_lit();
        let x = b.new_var();
        let before = (b.num_clauses(), b.num_vars());
        assert_eq!(b.and(x, t), x);
        assert_eq!(b.and(t, x), x);
        assert_eq!(b.and(x, f), f);
        assert_eq!(b.and(x, x), x);
        assert_eq!(b.and(x, !x), f);
        assert_eq!(b.or(x, f), x);
        assert_eq!(b.or(x, t), t);
        assert_eq!(b.or(x, !x), t);
        assert_eq!(b.xor(x, f), x);
        assert_eq!(b.xor(x, t), !x);
        assert_eq!(b.xor(x, x), f);
        assert_eq!(b.xor(x, !x), t);
        assert_eq!(b.eq(x, x), t);
        assert_eq!(b.ite(t, x, f), x);
        assert_eq!(b.ite(f, x, t), t);
        assert_eq!(b.ite(x, t, f), x);
        assert_eq!(b.ite(x, f, t), !x);
        assert_eq!(b.ite(x, x, x), x);
        assert_eq!(b.and_n(&[]), t);
        assert_eq!(b.or_n(&[]), f);
        assert_eq!(b.xor_n(&[]), f);
        assert_eq!(b.xor_n(&[x, x]), f);
        assert_eq!((b.num_clauses(), b.num_vars()), before);
        // Folding into a smaller gate.
        let y = b.new_var();
        let g = b.ite(x, y, f);
        assert_eq!(g, b.and(x, y));
        let g = b.ite(x, t, y);
        assert_eq!(g, b.or(x, y));
        let g = b.ite(x, y, !y);
        assert_eq!(g, b.eq(x, y));
    }

    #[test]
    fn structural_hashing_shares_gates() {
        let mut b = CnfBuilder::new();
        let x = b.new_var();
        let y = b.new_var();
        let z = b.new_var();
        let g1 = b.and(x, y);
        let g2 = b.and(y, x);
        assert_eq!(g1, g2);
        let n = b.num_clauses();
        let o1 = b.or(x, y);
        let o2 = b.or(y, x);
        assert_eq!(o1, o2);
        assert_eq!(b.num_clauses(), n + 3);
        let x1 = b.xor(x, !y);
        let x2 = b.xor(!x, y);
        let x3 = b.xor(y, x);
        assert_eq!(x1, x2);
        assert_eq!(x1, !x3);
        assert_eq!(b.num_vars(), x3.var().index() + 1, "one xor gate in total");
        let i1 = b.ite(x, y, z);
        let i2 = b.ite(!x, z, y);
        let i3 = b.ite(x, !y, !z);
        assert_eq!(i1, i2);
        assert_eq!(i1, !i3);
    }

    #[test]
    fn nary_gates() {
        let mut b = CnfBuilder::new();
        let ins = b.new_vars(5);
        let all = b.and_n(&ins);
        let any = b.or_n(&ins);
        let par = b.xor_n(&ins);
        check_truth_table(&ins, all, &b, |v| v.iter().all(|&x| x));
        check_truth_table(&ins, any, &b, |v| v.iter().any(|&x| x));
        check_truth_table(&ins, par, &b, |v| v.iter().filter(|&&x| x).count() % 2 == 1);
    }

    #[test]
    fn full_adder_truth_table() {
        let mut b = CnfBuilder::new();
        let x = b.new_var();
        let y = b.new_var();
        let c = b.new_var();
        let (s, co) = b.full_adder(x, y, c);
        let count = |v: &[bool]| v.iter().filter(|&&x| x).count();
        check_truth_table(&[x, y, c], s, &b, |v| count(v) % 2 == 1);
        check_truth_table(&[x, y, c], co, &b, |v| count(v) >= 2);
    }

    /// Reads a bit vector out of the model.
    fn read(s: &Solver, bits: &[Lit]) -> u64 {
        bits.iter()
            .enumerate()
            .filter(|(_, l)| s.value(l.var()) == Some(!l.is_neg()))
            .map(|(i, _)| 1u64 << i)
            .sum()
    }

    /// Forces a bit vector to a value.
    fn set(b: &mut CnfBuilder, bits: &[Lit], value: u64) {
        for (i, &l) in bits.iter().enumerate() {
            b.add_unit(if value >> i & 1 == 1 { l } else { !l });
        }
    }

    #[test]
    fn adder_exhaustive_4bit() {
        let mut b = CnfBuilder::new();
        let x = b.new_vars(4);
        let y = b.new_vars(4);
        let sum = b.add(&x, &y);
        assert_eq!(sum.len(), 5);
        let mut s = b.into_solver();
        for i in 0..16u64 {
            for j in 0..16u64 {
                let mut assumptions = Vec::new();
                for k in 0..4 {
                    assumptions.push(if i >> k & 1 == 1 { x[k] } else { !x[k] });
                    assumptions.push(if j >> k & 1 == 1 { y[k] } else { !y[k] });
                }
                assert_eq!(s.solve_with_assumptions(&assumptions), SolveResult::Sat);
                assert_eq!(read(&s, &sum), i + j, "{i} + {j}");
            }
        }
    }

    #[test]
    fn multiplier_factoring() {
        // 6 x 6 -> 12 bits: factor 2021 = 43 * 47.
        let mut b = CnfBuilder::new();
        let x = b.new_vars(6);
        let y = b.new_vars(6);
        let p = b.mul(&x, &y);
        assert_eq!(p.len(), 12);
        let mut b2 = b.clone();
        set(&mut b2, &p, 2021);
        let mut s = b2.into_solver();
        assert_eq!(s.solve(), SolveResult::Sat);
        let (a, c) = (read(&s, &x), read(&s, &y));
        assert_eq!(a * c, 2021);
        assert!(a > 1 && c > 1);

        // 2039 is prime: with both factors > 1 there is no solution.
        let mut b3 = b.clone();
        set(&mut b3, &p, 2039);
        let not_one_x: Vec<Lit> = x[1..].to_vec();
        let not_one_y: Vec<Lit> = y[1..].to_vec();
        let mut cl = not_one_x.clone();
        cl.push(!x[0]);
        b3.add_clause(&cl); // x != 1
        let mut cl = not_one_y.clone();
        cl.push(!y[0]);
        b3.add_clause(&cl); // y != 1
        let mut s = b3.into_solver();
        assert_eq!(s.solve(), SolveResult::Unsat);

        // Random products check against native arithmetic.
        let mut s = b.into_solver();
        for (i, j) in [(0u64, 0u64), (63, 63), (17, 29), (1, 45), (32, 2)] {
            let mut assumptions = Vec::new();
            for k in 0..6 {
                assumptions.push(if i >> k & 1 == 1 { x[k] } else { !x[k] });
                assumptions.push(if j >> k & 1 == 1 { y[k] } else { !y[k] });
            }
            assert_eq!(s.solve_with_assumptions(&assumptions), SolveResult::Sat);
            assert_eq!(read(&s, &p), i * j, "{i} * {j}");
        }
    }

    #[test]
    fn parity_chains() {
        // Two independent encodings of the same 40-bit parity must agree.
        let mut b = CnfBuilder::new();
        let bits = b.new_vars(40);
        let chain = |b: &mut CnfBuilder, bits: &[Lit]| {
            let mut acc = bits[0];
            for &x in &bits[1..] {
                // Fresh variables defeat structural hashing on purpose.
                let y = b.new_var();
                let g = b.xor(acc, x);
                b.add_eq(y, g);
                acc = y;
            }
            acc
        };
        let p1 = chain(&mut b, &bits);
        let p2 = chain(&mut b, &bits);
        let mut sat = b.clone();
        sat.add_unit(p1);
        let mut s = sat.into_solver();
        assert_eq!(s.solve(), SolveResult::Sat);
        let ones = bits
            .iter()
            .filter(|l| s.value(l.var()) == Some(true))
            .count();
        assert_eq!(ones % 2, 1);
        let mut unsat = b;
        unsat.add_unit(p1);
        unsat.add_unit(!p2);
        assert_eq!(unsat.into_solver().solve(), SolveResult::Unsat);
    }

    #[test]
    fn gates_can_be_read_back() {
        let mut b = CnfBuilder::new();
        let x = b.new_var();
        let y = b.new_var();
        let z = b.new_var();
        let g = b.and(x, !y);
        let h = b.xor(!x, z);
        let m = b.ite(!x, y, z);
        assert_eq!(b.gate(x.var()), None);
        assert_eq!(b.gate(b.true_lit().var()), None);
        assert_eq!(b.gate(g.var()), Some(Gate::And(x, !y)));
        // Signs are pulled out of an xor and a negative condition swaps
        // the branches.
        assert_eq!(b.gate(h.var()), Some(Gate::Xor(x, z)));
        assert!(h.is_neg());
        assert_eq!(b.gate(m.var()), Some(Gate::Ite(x, z, y)));
        // Only the constant's unit so far; then an explicit constraint.
        assert_eq!(b.constraints().collect::<Vec<_>>(), [&[b.true_lit()][..]]);
        b.add_eq(g, h);
        let constraints: Vec<&[Lit]> = b.constraints().collect();
        assert_eq!(constraints.len(), 3);
        assert_eq!(constraints[1], &[!g, h]);
        assert_eq!(b.clauses().count(), 1 + 3 + 4 + 6 + 2);
    }

    #[test]
    fn load_into_existing_solver() {
        let mut b = CnfBuilder::new();
        let x = b.new_var();
        let y = b.new_var();
        let g = b.and(x, y);
        b.add_unit(g);
        let mut s = Solver::new();
        assert!(b.load_into(&mut s));
        assert_eq!(u32::try_from(s.num_vars()).unwrap(), b.num_vars());
        assert_eq!(s.solve(), SolveResult::Sat);
        assert_eq!(s.value(x.var()), Some(true));
        assert_eq!(s.value(y.var()), Some(true));
        b.add_unit(!x);
        assert!(!b.load_into(&mut s));
        assert_eq!(s.solve(), SolveResult::Unsat);
        assert_eq!(b.clauses().count(), b.num_clauses());
    }
}
