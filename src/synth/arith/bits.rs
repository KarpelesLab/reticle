//! Bit-level construction: the gate emitter every architecture builds on.
//!
//! The architectures in this module family are described at the level of
//! single bits — a full adder, a prefix cell, a 2:1 multiplexer — so they
//! all share one small emitter. [`GateBuilder`] wraps a [`ModuleBuilder`]
//! and turns a request for a gate into a cell driving a fresh one-bit
//! net, returning the [`ExprId`] that reads that net. Words are
//! `Vec<ExprId>` with the **least significant bit first**, which is the
//! order every function in this module family takes and returns.
//!
//! Three properties matter and are worth stating:
//!
//! - **Constant folding.** `and(x, 0)` is `0`, `or(x, 1)` is `1`,
//!   `xor(x, 0)` is `x`, `mux(1, t, e)` is `t`, and so on. This is not an
//!   optimisation for its own sake: a prefix adder's carry-in column has
//!   a propagate of zero, and a partial product of a constant operand
//!   vanishes, so without folding every architecture would carry a tail
//!   of gates that compute nothing. A folded gate costs no cell, so the
//!   measurements in `docs/arithmetic.md` are of real logic.
//! - **Determinism.** Names are `<prefix>_n<k>` for nets and
//!   `<prefix>_g<k>` for cells, with `k` running from zero in creation
//!   order, and a name that collides with one already in the module is
//!   suffixed until it does not. The same input therefore always
//!   produces the same netlist, byte for byte.
//! - **Almost no sharing.** Two identical gates are emitted twice.
//!   [`crate::synth::opt::Merge`] exists to share them and runs after
//!   this pass in any sensible pipeline; doing it here would make one
//!   architecture's cell count depend on what another built earlier.
//!   Inverters are the exception, because the folding rules above
//!   *create* them — `mux(s, 0, e)` becomes `!s & e` — and one per bit
//!   of a shifted word would cost more than the multiplexers saved.

use std::collections::{HashMap, HashSet};

use crate::ir::builder::ModuleBuilder;
use crate::ir::{Bit, CellKind, Const, ExprId, ExprKind, Name, NetId, Type};

/// A width or count as a `usize`.
///
/// Every `u32` fits in a `usize` on the targets Reticle builds for (64-bit
/// hosts and `wasm32`), so this cannot fail; it exists so the conversion
/// is spelled once and checked, rather than cast silently at each use.
pub fn to_usize(value: u32) -> usize {
    usize::try_from(value).expect("a u32 fits in a usize")
}

/// What a memoised inverter is keyed on: two reads of one net are the
/// same signal and share an inverter, two unrelated nodes do not.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Signal {
    Net(NetId),
    Node(ExprId),
}

/// Emits gates into a module.
///
/// See the module docs for the naming scheme and the folding rules.
pub struct GateBuilder<'a> {
    builder: &'a mut ModuleBuilder,
    prefix: String,
    taken: HashSet<String>,
    nets: u64,
    gates: u64,
    cells: u64,
    zero: Option<ExprId>,
    one: Option<ExprId>,
    inverters: HashMap<Signal, ExprId>,
}

impl<'a> GateBuilder<'a> {
    /// Starts emitting into `builder`, naming everything after `prefix`.
    ///
    /// The names already used by the module are collected once, so a
    /// generated name never collides with an existing net or cell.
    pub fn new(builder: &'a mut ModuleBuilder, prefix: &str) -> GateBuilder<'a> {
        let mut taken: HashSet<String> = HashSet::new();
        taken.extend(builder.module().nets.values().map(|n| n.name.to_string()));
        taken.extend(builder.module().cells.values().map(|c| c.name.to_string()));
        GateBuilder {
            builder,
            prefix: prefix.to_owned(),
            taken,
            nets: 0,
            gates: 0,
            cells: 0,
            zero: None,
            one: None,
            inverters: HashMap::new(),
        }
    }

    /// The module being built, for the callers that need to reach past
    /// the bit level (to slice a word, or to drive a result net).
    pub fn builder(&mut self) -> &mut ModuleBuilder {
        self.builder
    }

    /// How many cells have been emitted so far.
    pub fn cell_count(&self) -> u64 {
        self.cells
    }

    /// The constant zero bit.
    pub fn zero(&mut self) -> ExprId {
        match self.zero {
            Some(e) => e,
            None => {
                let e = self.builder.const_bit(false);
                self.zero = Some(e);
                e
            }
        }
    }

    /// The constant one bit.
    pub fn one(&mut self) -> ExprId {
        match self.one {
            Some(e) => e,
            None => {
                let e = self.builder.const_bit(true);
                self.one = Some(e);
                e
            }
        }
    }

    /// The constant bit `value`.
    pub fn constant(&mut self, value: bool) -> ExprId {
        if value { self.one() } else { self.zero() }
    }

    /// The value of `e` when it is a known one-bit constant.
    pub fn as_const(&self, e: ExprId) -> Option<bool> {
        let c = self.builder.module().expr(e).as_const()?;
        if c.width() != 1 {
            return None;
        }
        c.bit(0).to_bool()
    }

    /// True when `a` and `b` are certainly the same signal: the same
    /// node, or two reads of the same net.
    fn same(&self, a: ExprId, b: ExprId) -> bool {
        if a == b {
            return true;
        }
        let m = self.builder.module();
        match (&m.expr(a).kind, &m.expr(b).kind) {
            (ExprKind::Net(x), ExprKind::Net(y)) => x == y,
            _ => false,
        }
    }

    fn fresh(&mut self, kind: char, counter: u64) -> Name {
        let mut candidate = format!("{}_{kind}{counter}", self.prefix);
        while self.taken.contains(&candidate) {
            candidate.push('_');
        }
        self.taken.insert(candidate.clone());
        Name::new(candidate)
    }

    /// A fresh one-bit wire.
    fn new_bit(&mut self) -> NetId {
        let name = self.fresh('n', self.nets);
        self.nets += 1;
        self.builder.add_net(name, Type::bit())
    }

    /// Emits `kind` over `inputs`, driving a fresh one-bit net.
    fn emit(&mut self, kind: CellKind, inputs: Vec<(&'static str, ExprId)>) -> ExprId {
        let net = self.new_bit();
        let name = self.fresh('g', self.gates);
        self.gates += 1;
        self.cells += 1;
        let inputs: Vec<(Name, ExprId)> =
            inputs.into_iter().map(|(p, e)| (Name::new(p), e)).collect();
        self.builder
            .cell(name, kind, inputs, vec![(Name::new("y"), net)]);
        self.builder.net(net)
    }

    /// The key an inverter of `e` is memoised under.
    fn signal(&self, e: ExprId) -> Signal {
        match self.builder.module().expr(e).kind {
            ExprKind::Net(net) => Signal::Net(net),
            _ => Signal::Node(e),
        }
    }

    /// `!a`.
    ///
    /// Inverters are the one gate this builder shares. Folding
    /// `mux(s, 0, e)` into `!s & e` — which is what makes a barrel
    /// shifter's zero fill cheap — would otherwise build one inverter
    /// of `s` per bit of the word, and a stage would cost more than the
    /// multiplexers it replaced.
    pub fn not(&mut self, a: ExprId) -> ExprId {
        if let Some(v) = self.as_const(a) {
            return self.constant(!v);
        }
        let key = self.signal(a);
        if let Some(&e) = self.inverters.get(&key) {
            return e;
        }
        let e = self.emit(CellKind::Not, vec![("a", a)]);
        self.inverters.insert(key, e);
        // And the inverse, so a double negation folds away.
        let back = self.signal(e);
        self.inverters.insert(back, a);
        e
    }

    /// True when `b` is known to be the complement of `a`, which the
    /// memoised inverters record.
    fn complementary(&self, a: ExprId, b: ExprId) -> bool {
        let inverse = self.inverters.get(&self.signal(a));
        inverse.is_some_and(|&e| self.signal(e) == self.signal(b))
    }

    /// `a & b`.
    pub fn and(&mut self, a: ExprId, b: ExprId) -> ExprId {
        match (self.as_const(a), self.as_const(b)) {
            (Some(false), _) | (_, Some(false)) => return self.zero(),
            (Some(true), _) => return b,
            (_, Some(true)) => return a,
            _ => {}
        }
        if self.same(a, b) {
            return a;
        }
        if self.complementary(a, b) {
            return self.zero();
        }
        self.emit(CellKind::And, vec![("a", a), ("b", b)])
    }

    /// `a | b`.
    pub fn or(&mut self, a: ExprId, b: ExprId) -> ExprId {
        match (self.as_const(a), self.as_const(b)) {
            (Some(true), _) | (_, Some(true)) => return self.one(),
            (Some(false), _) => return b,
            (_, Some(false)) => return a,
            _ => {}
        }
        if self.same(a, b) {
            return a;
        }
        if self.complementary(a, b) {
            return self.one();
        }
        self.emit(CellKind::Or, vec![("a", a), ("b", b)])
    }

    /// `a ^ b`.
    pub fn xor(&mut self, a: ExprId, b: ExprId) -> ExprId {
        match (self.as_const(a), self.as_const(b)) {
            (Some(x), Some(y)) => return self.constant(x != y),
            (Some(false), _) => return b,
            (_, Some(false)) => return a,
            (Some(true), _) => return self.not(b),
            (_, Some(true)) => return self.not(a),
            _ => {}
        }
        if self.same(a, b) {
            return self.zero();
        }
        if self.complementary(a, b) {
            return self.one();
        }
        self.emit(CellKind::Xor, vec![("a", a), ("b", b)])
    }

    /// `!(a ^ b)`; two cells, since the primitive set has no `xnor`.
    pub fn xnor(&mut self, a: ExprId, b: ExprId) -> ExprId {
        let x = self.xor(a, b);
        self.not(x)
    }

    /// `s ? t : e`.
    pub fn mux(&mut self, s: ExprId, t: ExprId, e: ExprId) -> ExprId {
        match self.as_const(s) {
            Some(true) => return t,
            Some(false) => return e,
            None => {}
        }
        if self.same(t, e) {
            return t;
        }
        // `s ? t : s` is `s & t`, and `s ? s : e` is `s | e`. Both come
        // up in an adder with a constant operand, where the carry out
        // of a stage is `p ? cin : a` with `p` equal to `a`.
        if self.same(s, e) {
            return self.and(s, t);
        }
        if self.same(s, t) {
            return self.or(s, e);
        }
        match (self.as_const(t), self.as_const(e)) {
            // s ? 1 : e is s | e; s ? 0 : e is !s & e; and so on.
            (Some(true), _) => return self.or(s, e),
            (Some(false), _) => {
                let ns = self.not(s);
                return self.and(ns, e);
            }
            (_, Some(true)) => {
                let ns = self.not(s);
                return self.or(ns, t);
            }
            (_, Some(false)) => return self.and(s, t),
            _ => {}
        }
        // The `mux` cell picks `b` when `s` is one.
        self.emit(CellKind::Mux, vec![("a", e), ("b", t), ("s", s)])
    }

    /// The AND of every bit, as a balanced tree (`true` when empty).
    pub fn and_all(&mut self, bits: &[ExprId]) -> ExprId {
        self.tree(bits, true, GateBuilder::and)
    }

    /// The OR of every bit, as a balanced tree (`false` when empty).
    pub fn or_all(&mut self, bits: &[ExprId]) -> ExprId {
        self.tree(bits, false, GateBuilder::or)
    }

    /// Folds `bits` with `op` as a balanced tree, so the depth is
    /// logarithmic rather than linear in the number of terms.
    fn tree(
        &mut self,
        bits: &[ExprId],
        empty: bool,
        op: fn(&mut GateBuilder<'a>, ExprId, ExprId) -> ExprId,
    ) -> ExprId {
        if bits.is_empty() {
            return self.constant(empty);
        }
        let mut layer = bits.to_vec();
        while layer.len() > 1 {
            let mut next = Vec::with_capacity(layer.len().div_ceil(2));
            let (pairs, rest) = layer.as_chunks::<2>();
            for pair in pairs {
                let v = op(self, pair[0], pair[1]);
                next.push(v);
            }
            next.extend_from_slice(rest);
            layer = next;
        }
        layer[0]
    }

    /// The OR of `bits` folded left to right, one gate per term: the
    /// linear counterpart of [`GateBuilder::or_all`], for the
    /// architectures that are deliberately linear.
    pub fn or_chain(&mut self, bits: &[ExprId]) -> ExprId {
        let mut acc = match bits.first() {
            Some(&b) => b,
            None => return self.zero(),
        };
        for &b in &bits[1..] {
            acc = self.or(acc, b);
        }
        acc
    }

    /// Splits `word` (of `width` bits) into single bits, LSB first.
    ///
    /// A constant is split into constant bits, so every architecture
    /// folds against a constant operand without a special case. An `x`
    /// or `z` bit is a don't-care to synthesis and reads as zero, which
    /// is what the rest of the pipeline does with it.
    pub fn split(&mut self, word: ExprId, width: u32) -> Vec<ExprId> {
        if let Some(c) = self.builder.module().expr(word).as_const().cloned() {
            return (0..width)
                .map(|i| {
                    let v = c.get(i).and_then(Bit::to_bool).unwrap_or(false);
                    self.constant(v)
                })
                .collect();
        }
        (0..width).map(|i| self.builder.slice(word, i, i)).collect()
    }

    /// Joins single bits, LSB first, into one unsigned word.
    pub fn join(&mut self, bits: &[ExprId]) -> ExprId {
        if bits.len() == 1 && !self.builder.module().expr(bits[0]).ty.is_signed() {
            return bits[0];
        }
        let parts: Vec<ExprId> = bits.iter().rev().copied().collect();
        self.builder.concat(parts)
    }

    /// A word of `width` zero bits.
    pub fn zeros(&mut self, width: usize) -> Vec<ExprId> {
        let z = self.zero();
        vec![z; width]
    }

    /// The low `width` bits of `value`, LSB first.
    pub fn const_word(&mut self, value: &Const, width: usize) -> Vec<ExprId> {
        (0..width)
            .map(|i| {
                let bit = u32::try_from(i).ok().and_then(|i| value.get(i));
                let v = bit.and_then(Bit::to_bool).unwrap_or(false);
                self.constant(v)
            })
            .collect()
    }

    /// Truncates or extends a word to `width`, filling with `fill`.
    pub fn extend(&mut self, bits: &[ExprId], width: usize, fill: ExprId) -> Vec<ExprId> {
        let mut out: Vec<ExprId> = bits.iter().take(width).copied().collect();
        out.resize(width, fill);
        out
    }

    /// Truncates or extends a word to `width`, sign extending when
    /// `signed` and zero extending otherwise.
    pub fn resize(&mut self, bits: &[ExprId], width: usize, signed: bool) -> Vec<ExprId> {
        let fill = match bits.last() {
            Some(&msb) if signed => msb,
            _ => self.zero(),
        };
        self.extend(bits, width, fill)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Module;
    use crate::synth::arith::testkit::span;

    fn builder() -> ModuleBuilder {
        let mut b = ModuleBuilder::new("m", span());
        let _ = b.input("a", Type::bits(4));
        let _ = b.input("b", Type::bits(4));
        b
    }

    fn operands(b: &mut ModuleBuilder) -> (ExprId, ExprId) {
        let nets: Vec<NetId> = b.module().nets.ids().collect();
        (b.net(nets[0]), b.net(nets[1]))
    }

    /// Nothing that is known folds into a cell.
    #[test]
    fn constants_fold() {
        let mut b = builder();
        let (a, _) = operands(&mut b);
        let x = b.slice(a, 0, 0);
        let mut cx = GateBuilder::new(&mut b, "u");
        let (t, f) = (cx.one(), cx.zero());
        assert_eq!(cx.and(x, f), f);
        assert_eq!(cx.and(x, t), x);
        assert_eq!(cx.or(x, t), t);
        assert_eq!(cx.or(x, f), x);
        assert_eq!(cx.xor(x, f), x);
        let ff = cx.xor(t, t);
        assert_eq!(cx.as_const(ff), Some(false));
        assert_eq!(cx.and(x, x), x);
        assert_eq!(cx.or(x, x), x);
        let xx = cx.xor(x, x);
        assert_eq!(cx.as_const(xx), Some(false));
        assert_eq!(cx.mux(t, x, f), x);
        assert_eq!(cx.mux(f, f, x), x);
        assert_eq!(cx.mux(x, t, t), t);
        assert_eq!(cx.and_all(&[]), t);
        assert_eq!(cx.or_all(&[]), f);
        assert_eq!(cx.or_chain(&[]), f);
        assert_eq!(cx.cell_count(), 0, "not one gate computes anything");
    }

    /// An inverter of a signal is built once, and inverting it back is
    /// the signal itself.
    #[test]
    fn inverters_are_shared() {
        let mut b = builder();
        let (a, _) = operands(&mut b);
        let x = b.slice(a, 0, 0);
        let mut cx = GateBuilder::new(&mut b, "u");
        let n1 = cx.not(x);
        let n2 = cx.not(x);
        assert_eq!(n1, n2);
        assert_eq!(cx.cell_count(), 1);
        assert_eq!(cx.not(n1), x, "a double negation is the signal");
        assert_eq!(cx.cell_count(), 1);
        // And the complement is recognised by the other gates.
        let (a1, o1, x1) = (cx.and(x, n1), cx.or(x, n1), cx.xor(x, n1));
        assert_eq!(cx.as_const(a1), Some(false));
        assert_eq!(cx.as_const(o1), Some(true));
        assert_eq!(cx.as_const(x1), Some(true));
        assert_eq!(cx.cell_count(), 1);
    }

    /// `mux(s, 0, e)` is `!s & e`, which is what makes a zero fill
    /// cheap; the inverter is shared across the word.
    #[test]
    fn a_constant_arm_becomes_a_gate() {
        let mut b = builder();
        let (a, sel) = operands(&mut b);
        let s = b.slice(sel, 0, 0);
        let mut cx = GateBuilder::new(&mut b, "u");
        let bits = cx.split(a, 4);
        let zero = cx.zero();
        for bit in &bits {
            let _ = cx.mux(s, zero, *bit);
        }
        // Four ANDs and one shared inverter, rather than four
        // multiplexers or four inverters.
        assert_eq!(cx.cell_count(), 5);
    }

    /// Splitting a word and joining it back is the word, and a constant
    /// splits into constant bits.
    #[test]
    fn words_split_and_join() {
        let mut b = builder();
        let (a, _) = operands(&mut b);
        let k = b.const_u64(4, 0b1010);
        let mut cx = GateBuilder::new(&mut b, "u");
        let bits = cx.split(a, 4);
        assert_eq!(bits.len(), 4);
        let word = cx.join(&bits);
        assert_eq!(cx.builder().module().expr(word).ty.width(), Some(4));
        let ks = cx.split(k, 4);
        let values: Vec<Option<bool>> = ks.iter().map(|&e| cx.as_const(e)).collect();
        assert_eq!(
            values,
            [Some(false), Some(true), Some(false), Some(true)],
            "the least significant bit comes first"
        );
        // One bit joins to itself rather than to a one-part concat.
        let one = cx.join(&bits[..1]);
        assert_eq!(one, bits[0]);
        assert_eq!(cx.cell_count(), 0);
    }

    /// A balanced tree is shallower than a chain over the same terms,
    /// and costs the same.
    #[test]
    fn a_tree_is_shallower_than_a_chain() {
        use crate::synth::report::Report;

        let measure = |chain: bool| -> (usize, usize) {
            let mut b = ModuleBuilder::new("m", span());
            let a = b.input("a", Type::bits(16));
            let y = b.output("y", Type::bit());
            let av = b.net(a);
            let out = {
                let mut cx = GateBuilder::new(&mut b, "u");
                let bits = cx.split(av, 16);
                if chain {
                    cx.or_chain(&bits)
                } else {
                    cx.or_all(&bits)
                }
            };
            b.assign(y, out);
            let m: Module = b.finish();
            let report = Report::of_module(&m);
            (report.cells.iter().map(|(_, n)| n).sum(), report.depth)
        };
        let (chain_cells, chain_depth) = measure(true);
        let (tree_cells, tree_depth) = measure(false);
        assert_eq!(chain_cells, tree_cells);
        assert_eq!((chain_depth, tree_depth), (15, 4));
    }

    /// Generated names never collide with what the module already has.
    #[test]
    fn names_avoid_collisions() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bit());
        let _ = b.add_net("u_n0", Type::bit());
        let av = b.net(a);
        {
            let mut cx = GateBuilder::new(&mut b, "u");
            let _ = cx.not(av);
        }
        let m = b.finish();
        let names: Vec<&str> = m.nets.values().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"u_n0"));
        assert!(names.contains(&"u_n0_"), "{names:?}");
    }
}
