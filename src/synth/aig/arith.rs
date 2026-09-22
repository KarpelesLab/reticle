//! Word-level builders over the AIG: bit vectors as `Vec<Edge>` (least
//! significant bit first) and the arithmetic, comparison, shift and
//! selection structures the bit-blaster lowers IR operators to.
//!
//! The architectures are the plain ones, since the rewriting passes clean
//! up afterwards and technology mapping decides what a LUT absorbs:
//! ripple-carry adders, array multipliers (a sum of shifted partial
//! products), restoring dividers, ripple comparators and barrel shifters
//! built as mux trees. Every result is truncated to the width the IR
//! prescribes for the operator.
//!
//! Semantics follow IEEE 1364-2005 §5 for two-state values: arithmetic is
//! modulo `2^width`, comparisons and division are signed when the caller
//! says so, shifts by an amount at or beyond the width shift everything
//! out, and an out-of-range variable bit select reads as 0 (where the
//! standard says `x`).

use super::{Aig, Edge};
use crate::logic::{Bit, Logic};

impl Aig {
    /// The edges for a constant, one per bit, LSB first; `x` and `z` bits
    /// become 0.
    pub fn const_bits(&self, value: &Logic) -> Vec<Edge> {
        (0..value.width())
            .map(|i| Edge::constant(value.bit(i) == Bit::One))
            .collect()
    }

    /// An unsigned constant of `width` bits.
    pub fn const_u64(&self, value: u64, width: u32) -> Vec<Edge> {
        (0..width)
            .map(|i| Edge::constant(i < 64 && (value >> i) & 1 == 1))
            .collect()
    }

    /// Bitwise complement.
    pub fn not_bits(&self, a: &[Edge]) -> Vec<Edge> {
        a.iter().map(|&e| !e).collect()
    }

    /// Bitwise AND of equal-width vectors.
    pub fn and_bits(&mut self, a: &[Edge], b: &[Edge]) -> Vec<Edge> {
        a.iter().zip(b).map(|(&x, &y)| self.and(x, y)).collect()
    }

    /// Bitwise OR of equal-width vectors.
    pub fn or_bits(&mut self, a: &[Edge], b: &[Edge]) -> Vec<Edge> {
        a.iter().zip(b).map(|(&x, &y)| self.or(x, y)).collect()
    }

    /// Bitwise XOR of equal-width vectors.
    pub fn xor_bits(&mut self, a: &[Edge], b: &[Edge]) -> Vec<Edge> {
        a.iter().zip(b).map(|(&x, &y)| self.xor(x, y)).collect()
    }

    /// `s ? t : e` bit by bit.
    pub fn mux_bits(&mut self, s: Edge, t: &[Edge], e: &[Edge]) -> Vec<Edge> {
        t.iter().zip(e).map(|(&x, &y)| self.mux(s, x, y)).collect()
    }

    /// AND of all bits (`true` for an empty vector).
    pub fn reduce_and(&mut self, a: &[Edge]) -> Edge {
        self.and_n(a)
    }

    /// OR of all bits.
    pub fn reduce_or(&mut self, a: &[Edge]) -> Edge {
        self.or_n(a)
    }

    /// XOR of all bits.
    pub fn reduce_xor(&mut self, a: &[Edge]) -> Edge {
        self.xor_n(a)
    }

    /// Truncates or extends to `width`, with copies of the MSB when
    /// `sign_extend` (zeros otherwise).
    pub fn resize_bits(&self, a: &[Edge], width: u32, sign_extend: bool) -> Vec<Edge> {
        let width = width as usize;
        let fill = if sign_extend {
            a.last().copied().unwrap_or(Edge::FALSE)
        } else {
            Edge::FALSE
        };
        let mut out: Vec<Edge> = a.iter().take(width).copied().collect();
        out.resize(width, fill);
        out
    }

    /// One full adder: `(sum, carry)`.
    pub fn full_adder(&mut self, a: Edge, b: Edge, cin: Edge) -> (Edge, Edge) {
        let p = self.xor(a, b);
        let sum = self.xor(p, cin);
        // carry = p ? cin : a  (majority of a, b, cin).
        let carry = self.mux(p, cin, a);
        (sum, carry)
    }

    /// Ripple-carry addition of equal-width vectors with a carry in;
    /// returns the sum and the carry out.
    pub fn add_carry(&mut self, a: &[Edge], b: &[Edge], cin: Edge) -> (Vec<Edge>, Edge) {
        let mut carry = cin;
        let mut sum = Vec::with_capacity(a.len());
        for (&x, &y) in a.iter().zip(b) {
            let (s, c) = self.full_adder(x, y, carry);
            sum.push(s);
            carry = c;
        }
        (sum, carry)
    }

    /// `a + b` modulo the width.
    pub fn add_bits(&mut self, a: &[Edge], b: &[Edge]) -> Vec<Edge> {
        self.add_carry(a, b, Edge::FALSE).0
    }

    /// `a - b` modulo the width.
    pub fn sub_bits(&mut self, a: &[Edge], b: &[Edge]) -> Vec<Edge> {
        let nb = self.not_bits(b);
        self.add_carry(a, &nb, Edge::TRUE).0
    }

    /// Two's complement negation.
    pub fn neg_bits(&mut self, a: &[Edge]) -> Vec<Edge> {
        let zero = vec![Edge::FALSE; a.len()];
        self.sub_bits(&zero, a)
    }

    /// `a * b` modulo the width: an array multiplier summing the partial
    /// products `a & b[i]` shifted by `i`. Signed and unsigned products
    /// agree modulo `2^width`, so no sign handling is needed.
    pub fn mul_bits(&mut self, a: &[Edge], b: &[Edge]) -> Vec<Edge> {
        let n = a.len();
        let mut acc = vec![Edge::FALSE; n];
        for (i, &bi) in b.iter().enumerate() {
            if bi == Edge::FALSE {
                continue;
            }
            // Partial product shifted by i, truncated to n bits.
            let mut pp = vec![Edge::FALSE; i];
            for &aj in a.iter().take(n - i) {
                pp.push(self.and(aj, bi));
            }
            acc = self.add_bits(&acc, &pp);
        }
        acc
    }

    /// Unsigned restoring division: `(quotient, remainder)`. Division by
    /// zero yields an all-ones quotient and the dividend as remainder.
    pub fn udivmod_bits(&mut self, a: &[Edge], b: &[Edge]) -> (Vec<Edge>, Vec<Edge>) {
        let n = a.len();
        let mut rem = vec![Edge::FALSE; n];
        let mut quot = vec![Edge::FALSE; n];
        // The divisor extended by one bit so the trial subtraction of the
        // shifted remainder (up to 2^(n+1) - 2) cannot overflow.
        let mut b_ext = b.to_vec();
        b_ext.push(Edge::FALSE);
        for i in (0..n).rev() {
            let mut shifted = vec![a[i]];
            shifted.extend_from_slice(&rem);
            let nb = self.not_bits(&b_ext);
            let (diff, no_borrow) = self.add_carry(&shifted, &nb, Edge::TRUE);
            quot[i] = no_borrow;
            rem = self.mux_bits(no_borrow, &diff[..n], &shifted[..n]);
        }
        (quot, rem)
    }

    /// Division and remainder, signed (truncating, remainder with the sign
    /// of the dividend) or unsigned.
    pub fn divmod_bits(&mut self, a: &[Edge], b: &[Edge], signed: bool) -> (Vec<Edge>, Vec<Edge>) {
        if !signed || a.is_empty() {
            return self.udivmod_bits(a, b);
        }
        let sa = a[a.len() - 1];
        let sb = b[b.len() - 1];
        let na = self.neg_bits(a);
        let nb = self.neg_bits(b);
        let abs_a = self.mux_bits(sa, &na, a);
        let abs_b = self.mux_bits(sb, &nb, b);
        let (q, r) = self.udivmod_bits(&abs_a, &abs_b);
        let q_neg = self.xor(sa, sb);
        let nq = self.neg_bits(&q);
        let nr = self.neg_bits(&r);
        let quot = self.mux_bits(q_neg, &nq, &q);
        let rem = self.mux_bits(sa, &nr, &r);
        (quot, rem)
    }

    /// `a == b`.
    pub fn eq_bits(&mut self, a: &[Edge], b: &[Edge]) -> Edge {
        let bits: Vec<Edge> = a.iter().zip(b).map(|(&x, &y)| self.xnor(x, y)).collect();
        self.and_n(&bits)
    }

    /// `a < b`, signed or unsigned.
    pub fn lt_bits(&mut self, a: &[Edge], b: &[Edge], signed: bool) -> Edge {
        let mut a = a.to_vec();
        let mut b = b.to_vec();
        if signed && !a.is_empty() {
            // Flipping the sign bits maps two's complement order onto
            // unsigned order.
            let n = a.len() - 1;
            a[n] = !a[n];
            b[n] = !b[n];
        }
        let mut lt = Edge::FALSE;
        for (&x, &y) in a.iter().zip(&b) {
            // lt = (!x & y) | (x == y) & lt_prev  ==  (x ^ y) ? (!x & y) : lt_prev
            let diff = self.xor(x, y);
            let below = self.and(!x, y);
            lt = self.mux(diff, below, lt);
        }
        lt
    }

    /// A mux tree selecting `values[index]`, with `values` padded by zeros
    /// to a power of two and an out-of-range index reading 0.
    pub fn select_bits(&mut self, values: &[Edge], index: &[Edge]) -> Edge {
        if values.is_empty() {
            return Edge::FALSE;
        }
        let mut layer: Vec<Edge> = values.to_vec();
        let mut used = 0usize;
        for &s in index {
            if layer.len() == 1 {
                break;
            }
            let mut next = Vec::with_capacity(layer.len().div_ceil(2));
            for pair in layer.chunks(2) {
                let hi = pair.get(1).copied().unwrap_or(Edge::FALSE);
                next.push(self.mux(s, hi, pair[0]));
            }
            layer = next;
            used += 1;
        }
        let overflow = self.or_n(&index[used.min(index.len())..]);
        self.and(!overflow, layer[0])
    }

    /// `a << amount`: a barrel shifter, zero filling.
    pub fn shl_bits(&mut self, a: &[Edge], amount: &[Edge]) -> Vec<Edge> {
        self.barrel(a, amount, true, Edge::FALSE)
    }

    /// `a >> amount`, zero filling.
    pub fn shr_bits(&mut self, a: &[Edge], amount: &[Edge]) -> Vec<Edge> {
        self.barrel(a, amount, false, Edge::FALSE)
    }

    /// `a >>> amount`, filling with the MSB.
    pub fn sshr_bits(&mut self, a: &[Edge], amount: &[Edge]) -> Vec<Edge> {
        let fill = a.last().copied().unwrap_or(Edge::FALSE);
        self.barrel(a, amount, false, fill)
    }

    fn barrel(&mut self, a: &[Edge], amount: &[Edge], left: bool, fill: Edge) -> Vec<Edge> {
        let n = a.len();
        let mut cur = a.to_vec();
        let mut overflow = Vec::new();
        for (j, &s) in amount.iter().enumerate() {
            if j >= 32 || (1usize << j) >= n {
                overflow.push(s);
                continue;
            }
            let k = 1usize << j;
            let shifted: Vec<Edge> = (0..n)
                .map(|i| {
                    if left {
                        if i >= k { cur[i - k] } else { fill }
                    } else if i + k < n {
                        cur[i + k]
                    } else {
                        fill
                    }
                })
                .collect();
            cur = self.mux_bits(s, &shifted, &cur);
        }
        if overflow.is_empty() {
            return cur;
        }
        let over = self.or_n(&overflow);
        let all_fill = vec![fill; n];
        self.mux_bits(over, &all_fill, &cur)
    }

    /// The function of a `k`-input lookup table applied to `inputs`
    /// (input 0 is the LSB of the pattern index): `init` bit `i` is the
    /// output for input pattern `i`.
    pub fn lut_bits(&mut self, inputs: &[Edge], init: &Logic) -> Edge {
        let table: Vec<bool> = (0..init.width()).map(|i| init.bit(i) == Bit::One).collect();
        assert_eq!(table.len(), 1usize << inputs.len(), "LUT table size");
        self.lut_rec(inputs, &table)
    }

    fn lut_rec(&mut self, inputs: &[Edge], table: &[bool]) -> Edge {
        match inputs.split_last() {
            None => Edge::constant(table[0]),
            Some((&top, rest)) => {
                let half = table.len() / 2;
                let f0 = self.lut_rec(rest, &table[..half]);
                let f1 = self.lut_rec(rest, &table[half..]);
                self.mux(top, f1, f0)
            }
        }
    }

    /// `a ** e` per IEEE 1364-2005 table 5-6 for two-state values: the
    /// power modulo `2^width` for a non-negative exponent; for a negative
    /// exponent (only when `e_signed`), 1 for a base of 1, ±1 by the parity
    /// of the exponent for a base of -1 (when `signed`), and 0 otherwise.
    /// A zero base to a negative exponent is `x` in the standard and reads
    /// 0 here.
    pub fn pow_bits(&mut self, a: &[Edge], e: &[Edge], signed: bool, e_signed: bool) -> Vec<Edge> {
        let n = a.len();
        // Square-and-multiply modulo 2^n over the exponent bits.
        let mut result = self.const_u64(1, u32::try_from(n).expect("width"));
        let mut base = a.to_vec();
        for &bit in e {
            let mul = self.mul_bits(&result, &base);
            result = self.mux_bits(bit, &mul, &result);
            base = self.mul_bits(&base, &base);
        }
        if !e_signed || e.is_empty() {
            return result;
        }
        let neg = e[e.len() - 1];
        let one = self.const_u64(1, u32::try_from(n).expect("width"));
        let is_one = self.eq_bits(a, &one);
        let special = if signed {
            let minus_one = vec![Edge::TRUE; n];
            let is_minus_one = self.eq_bits(a, &minus_one);
            let odd = e[0];
            let pm = self.mux_bits(odd, &minus_one, &one);
            let zero = vec![Edge::FALSE; n];
            let m1 = self.mux_bits(is_minus_one, &pm, &zero);
            self.mux_bits(is_one, &one, &m1)
        } else {
            let zero = vec![Edge::FALSE; n];
            self.mux_bits(is_one, &one, &zero)
        };
        self.mux_bits(neg, &special, &result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(aig: &mut Aig, n: u32) -> Vec<Edge> {
        (0..n).map(|_| aig.add_input()).collect()
    }

    fn run(aig: &Aig, a: u64, b: u64, wa: u32, wb: u32) -> Vec<bool> {
        let mut ins = Vec::new();
        for i in 0..wa {
            ins.push((a >> i) & 1 == 1);
        }
        for i in 0..wb {
            ins.push((b >> i) & 1 == 1);
        }
        aig.eval(&ins)
    }

    fn word(bits: &[bool]) -> u64 {
        bits.iter()
            .enumerate()
            .fold(0, |acc, (i, &b)| acc | (u64::from(b) << i))
    }

    /// The low four bits of a signed value as an unsigned word.
    fn low4(v: i64) -> u64 {
        u64::try_from(v & 15).expect("non-negative")
    }

    /// A 4-bit pattern read as a signed value.
    fn signed4(x: u64) -> i64 {
        i64::try_from(x).expect("small") - if x >= 8 { 16 } else { 0 }
    }

    #[test]
    fn adders_multipliers_dividers() {
        let mut g = Aig::new();
        let a = inputs(&mut g, 4);
        let b = inputs(&mut g, 4);
        let sum = g.add_bits(&a, &b);
        let diff = g.sub_bits(&a, &b);
        let prod = g.mul_bits(&a, &b);
        let (q, r) = g.udivmod_bits(&a, &b);
        let (sq, sr) = g.divmod_bits(&a, &b, true);
        let neg = g.neg_bits(&a);
        for e in sum
            .iter()
            .chain(&diff)
            .chain(&prod)
            .chain(&q)
            .chain(&r)
            .chain(&sq)
            .chain(&sr)
            .chain(&neg)
        {
            g.add_output(*e);
        }
        for x in 0..16u64 {
            for y in 0..16u64 {
                let out = run(&g, x, y, 4, 4);
                assert_eq!(word(&out[0..4]), (x + y) & 15, "add {x} {y}");
                assert_eq!(word(&out[4..8]), x.wrapping_sub(y) & 15, "sub {x} {y}");
                assert_eq!(word(&out[8..12]), (x * y) & 15, "mul {x} {y}");
                if let (Some(q), Some(r)) = (x.checked_div(y), x.checked_rem(y)) {
                    assert_eq!(word(&out[12..16]), q, "div {x} {y}");
                    assert_eq!(word(&out[16..20]), r, "mod {x} {y}");
                    let sx = signed4(x);
                    let sy = signed4(y);
                    assert_eq!(word(&out[20..24]), low4(sx / sy), "sdiv {sx} {sy}");
                    assert_eq!(word(&out[24..28]), low4(sx % sy), "smod {sx} {sy}");
                }
                assert_eq!(word(&out[28..32]), x.wrapping_neg() & 15, "neg {x}");
            }
        }
    }

    #[test]
    fn comparisons_and_shifts() {
        let mut g = Aig::new();
        let a = inputs(&mut g, 4);
        let b = inputs(&mut g, 4);
        let eq = g.eq_bits(&a, &b);
        let lt = g.lt_bits(&a, &b, false);
        let slt = g.lt_bits(&a, &b, true);
        let shl = g.shl_bits(&a, &b[..3]);
        let shr = g.shr_bits(&a, &b[..3]);
        let sshr = g.sshr_bits(&a, &b[..3]);
        let sel = g.select_bits(&a, &b[..3]);
        for e in [eq, lt, slt] {
            g.add_output(e);
        }
        for e in shl.iter().chain(&shr).chain(&sshr) {
            g.add_output(*e);
        }
        g.add_output(sel);
        for x in 0..16u64 {
            for y in 0..16u64 {
                let out = run(&g, x, y, 4, 4);
                let sx = signed4(x);
                let sy = signed4(y);
                assert_eq!(out[0], x == y);
                assert_eq!(out[1], x < y);
                assert_eq!(out[2], sx < sy, "slt {sx} {sy}");
                let amt = u32::try_from(y & 7).expect("small");
                assert_eq!(word(&out[3..7]), (x << amt) & 15, "shl {x} {amt}");
                assert_eq!(word(&out[7..11]), x >> amt, "shr {x} {amt}");
                assert_eq!(word(&out[11..15]), low4(sx >> amt), "sshr {sx} {amt}");
                let expect = if amt < 4 { (x >> amt) & 1 == 1 } else { false };
                assert_eq!(out[15], expect, "select {x} {amt}");
            }
        }
    }

    #[test]
    fn luts_resize_and_pow() {
        let mut g = Aig::new();
        let a = inputs(&mut g, 3);
        let init = Logic::from_u64(0b1001_0110, 8); // xor3
        let lut = g.lut_bits(&a, &init);
        g.add_output(lut);
        let ext = g.resize_bits(&a, 5, true);
        assert_eq!(ext.len(), 5);
        assert_eq!(ext[4], a[2]);
        let z = g.resize_bits(&a, 5, false);
        assert_eq!(z[4], Edge::FALSE);
        assert_eq!(g.resize_bits(&a, 2, true).len(), 2);
        for x in 0..8u64 {
            let out = run(&g, x, 0, 3, 0);
            assert_eq!(out[0], (x.count_ones() & 1) == 1, "lut {x}");
        }
        let c = g.const_bits(&Logic::parse_verilog("4'b1x0z").unwrap());
        assert_eq!(c, [Edge::FALSE, Edge::FALSE, Edge::FALSE, Edge::TRUE]);
        assert_eq!(
            g.const_u64(5, 4),
            [Edge::TRUE, Edge::FALSE, Edge::TRUE, Edge::FALSE]
        );

        let mut g = Aig::new();
        let a = inputs(&mut g, 4);
        let e = inputs(&mut g, 4);
        let p = g.pow_bits(&a, &e, false, false);
        let sp = g.pow_bits(&a, &e, true, true);
        for x in p.iter().chain(&sp) {
            g.add_output(*x);
        }
        for x in 0..16u64 {
            for y in 0..16u64 {
                let out = run(&g, x, y, 4, 4);
                let mut expect = 1u64;
                for _ in 0..y {
                    expect = (expect * x) & 15;
                }
                assert_eq!(word(&out[0..4]), expect, "pow {x} {y}");
                let sx = signed4(x);
                let sy = signed4(y);
                let sexpect: i64 = if sy < 0 {
                    match sx {
                        1 => 1,
                        -1 => {
                            if sy % 2 == 0 {
                                1
                            } else {
                                -1
                            }
                        }
                        _ => 0,
                    }
                } else {
                    let mut r = 1i64;
                    for _ in 0..sy {
                        r = (r * sx) & 15;
                    }
                    r
                };
                assert_eq!(word(&out[4..8]), low4(sexpect), "spow {sx} {sy}");
            }
        }
    }
}
