//! Execution of a compiled program.
//!
//! [`State`] is the mutable half of a compiled run: the register file, the
//! memories and the scratch buffers. [`State::run`] walks an op list and
//! applies each op in order; [`State::commit`] applies the memory writes
//! the clock edge queued and copies the next-state block over the
//! current-state block in one `copy_within`, which is what makes a
//! cycle-based simulator correct without an event queue.
//!
//! Every op has a one-word fast path, because that is the case that
//! decides the speed: the vast majority of nets in a real design are 64
//! bits or fewer, so the op reduces to a couple of machine instructions
//! and a mask. Wider values go through the kernels in [`super::words`] via
//! pre-grown scratch buffers, so a warm engine allocates nothing except
//! when a `$display` actually produces text.
//!
//! # Where two-state differs
//!
//! The 4-state simulator yields `x` in a handful of places that have no
//! two-state answer. Eligibility ([`super::check`]) rules out the ones
//! that can be decided statically; the rest are decided here, and the
//! choice is always "the bits that are not there read as zero":
//!
//! * a bit or element select outside its operand reads zero, not `x`;
//! * division or remainder by zero yields zero, not `x`;
//! * a `Pmux` with more than one select bit set takes the lowest one,
//!   where the event simulator yields `x`.

use std::cmp::Ordering;

use crate::ir::{BinaryOp, ReportSeverity, Span, UnaryOp};
use crate::logic::Logic;

use super::prog::{MemLayout, Op, Program, Slot};
use super::words;

/// A side effect one op produced, drained by the caller once the edge has
/// been evaluated and before the state is committed.
#[derive(Clone, Debug)]
pub(crate) enum Effect {
    /// Append the text of message `.0` to the output.
    Display(u32),
    /// Report message `.1` with the given severity at the given span.
    Assert(u32, ReportSeverity, Span),
    /// `$finish`.
    Finish,
    /// `$stop`.
    Stop,
}

/// Scratch buffers, grown once so no op allocates while the engine runs.
#[derive(Clone, Debug, Default)]
struct Scratch {
    a: Vec<u64>,
    b: Vec<u64>,
    d: Vec<u64>,
    e: Vec<u64>,
    f: Vec<u64>,
    g: Vec<u64>,
}

impl Scratch {
    fn with_capacity(words: usize) -> Scratch {
        let one = || Vec::with_capacity(words + 2);
        Scratch {
            a: one(),
            b: one(),
            d: one(),
            e: one(),
            f: one(),
            g: one(),
        }
    }
}

/// A memory write queued by the clock edge.
#[derive(Clone, Copy, Debug)]
struct Pending {
    mem: u32,
    index: usize,
    data: Slot,
}

/// The mutable state of a compiled run.
#[derive(Clone, Debug)]
pub(crate) struct State {
    /// The register file: constants, nets, temporaries and state.
    pub(crate) regs: Vec<u64>,
    /// Memory contents, elements packed word-aligned.
    pub(crate) mems: Vec<Vec<u64>>,
    /// Side effects produced by the last edge.
    pub(crate) effects: Vec<Effect>,
    pending: Vec<Pending>,
    scratch: Scratch,
}

/// The mask of a value that fits one word.
fn mask_word(width: u32) -> u64 {
    words::word_mask(0, width)
}

/// Sign-extends a one-word value of `width` bits to the full word.
fn sext_word(v: u64, width: u32, signed: bool) -> u64 {
    if signed && width > 0 && width < 64 && (v >> (width - 1)) & 1 == 1 {
        v | !mask_word(width)
    } else {
        v
    }
}

/// Reads a slot into `buf`.
fn read(regs: &[u64], s: Slot, buf: &mut Vec<u64>) {
    buf.clear();
    buf.extend_from_slice(&regs[s.range()]);
}

/// Resizes `buf` to hold `s` and clears it.
fn blank(buf: &mut Vec<u64>, s: Slot) {
    buf.clear();
    buf.resize(s.words as usize, 0);
}

/// Writes `src` into a slot.
fn store(regs: &mut [u64], s: Slot, src: &[u64]) {
    regs[s.range()].copy_from_slice(&src[..s.words as usize]);
}

/// The value of an index operand as a signed integer, saturating; this is
/// the two-state form of [`crate::sim::eval`]'s `index_value`.
fn index_of(w: &[u64], width: u32, signed: bool, scratch: &mut Vec<u64>) -> i64 {
    if words::sign_of(w, width, signed) {
        scratch.clear();
        scratch.resize(w.len(), 0);
        words::neg_into(scratch, w, width);
        let mag = words::to_u64_saturating(scratch);
        return i64::try_from(mag).map_or(i64::MIN, |m| -m);
    }
    i64::try_from(words::to_u64_saturating(w)).unwrap_or(i64::MAX)
}

/// A two-state value rebuilt as a [`Logic`], for the rare operators that
/// borrow the shared kernels rather than duplicating them.
fn as_logic(w: &[u64], s: Slot) -> Logic {
    Logic::from_planes(s.width, s.signed, w.to_vec(), vec![0u64; w.len()])
}

impl State {
    /// The state a program starts in.
    pub(crate) fn new(prog: &Program) -> State {
        State {
            regs: prog.init.clone(),
            mems: prog.mems.iter().map(|m| m.init.clone()).collect(),
            effects: Vec::new(),
            pending: Vec::new(),
            scratch: Scratch::with_capacity(prog.widest),
        }
    }

    /// Reads a slot as a fresh two-state [`Logic`].
    pub(crate) fn value(&self, s: Slot) -> Logic {
        as_logic(&self.regs[s.range()], s)
    }

    /// Writes a [`Logic`] into a slot, dropping any unknown bits.
    pub(crate) fn set_value(&mut self, s: Slot, value: &Logic) {
        // Through the scratch buffers rather than `Logic::resize`, because
        // driving an input is on the hot path of every testbench and this
        // way it allocates nothing.
        let Scratch { a: sa, d: sd, .. } = &mut self.scratch;
        sa.clear();
        sa.extend(
            value
                .value_words()
                .iter()
                .zip(value.unknown_words())
                .map(|(v, u)| v & !u),
        );
        blank(sd, s);
        words::resize_into(sd, s.width, sa, value.width(), value.is_signed());
        store(&mut self.regs, s, sd);
    }

    /// One element of a memory as a [`Logic`], or `None` out of range.
    pub(crate) fn mem_value(&self, mem: usize, layout: &MemLayout, index: usize) -> Option<Logic> {
        let lo = index.checked_mul(layout.elem_words)?;
        let hi = lo.checked_add(layout.elem_words)?;
        let data = self.mems.get(mem)?;
        if index >= layout.len || hi > data.len() {
            return None;
        }
        Some(Logic::from_planes(
            layout.elem_width,
            layout.signed,
            data[lo..hi].to_vec(),
            vec![0u64; layout.elem_words],
        ))
    }

    /// Writes one element of a memory; false when out of range.
    pub(crate) fn set_mem_value(
        &mut self,
        mem: usize,
        layout: &MemLayout,
        index: usize,
        value: &Logic,
    ) -> bool {
        if index >= layout.len {
            return false;
        }
        let src = super::prog::known_words(&value.resize(layout.elem_width));
        let lo = index * layout.elem_words;
        let Some(data) = self.mems.get_mut(mem) else {
            return false;
        };
        for i in 0..layout.elem_words {
            data[lo + i] = src.get(i).copied().unwrap_or(0);
        }
        true
    }

    /// Runs one op list from start to end.
    pub(crate) fn run(&mut self, prog: &Program, ops: &[Op]) {
        for op in ops {
            self.exec(prog, op);
        }
    }

    /// Applies the queued memory writes, then makes the next state
    /// current.
    pub(crate) fn commit(&mut self, prog: &Program) {
        // Indexed rather than drained, so the queue keeps its capacity and
        // a steady-state cycle allocates nothing.
        for i in 0..self.pending.len() {
            let p = self.pending[i];
            let layout = &prog.mems[p.mem as usize];
            if layout.elem_words == 0 {
                continue;
            }
            let lo = p.index * layout.elem_words;
            let src = p.data.range();
            for w in 0..layout.elem_words {
                let word = self.regs.get(src.start + w).copied().unwrap_or(0);
                self.mems[p.mem as usize][lo + w] = word;
            }
            let last = lo + layout.elem_words - 1;
            self.mems[p.mem as usize][last] &=
                words::word_mask(layout.elem_words - 1, layout.elem_width);
        }
        self.pending.clear();
        if prog.state_len > 0 {
            self.regs.copy_within(
                prog.state_next..prog.state_next + prog.state_len,
                prog.state_cur,
            );
        }
    }

    /// True when the guard is absent or non-zero.
    fn guarded(&self, guard: Option<Slot>) -> bool {
        match guard {
            None => true,
            Some(g) => !words::is_zero(&self.regs[g.range()]),
        }
    }

    pub(crate) fn exec(&mut self, prog: &Program, op: &Op) {
        match op {
            Op::Resize { dst, a } => self.resize(*dst, *a),
            Op::Unary { op, dst, a } => self.unary(*op, *dst, *a),
            Op::Binary { op, dst, a, b } => self.binary(*op, *dst, *a, *b),
            Op::Mux { dst, s, a, b } => {
                let pick = if words::is_zero(&self.regs[s.range()]) {
                    *a
                } else {
                    *b
                };
                self.resize(*dst, pick);
            }
            Op::Pmux { dst, s, a, b } => self.pmux(*dst, *s, *a, *b),
            Op::Lut { dst, a, init } => {
                let i = words::to_u64_saturating(&self.regs[a.range()]);
                let table = &prog.luts[*init as usize];
                let bit = u32::try_from(i).is_ok_and(|i| words::get_bit(table, i));
                self.regs[dst.off as usize] = u64::from(bit);
            }
            Op::Concat { dst, parts } => self.concat(*dst, parts),
            Op::Replicate { dst, a, count } => self.replicate(*dst, *a, *count),
            Op::Extract { dst, a, lo } => self.extract(*dst, *a, *lo),
            Op::DynExtract { dst, a, off, up } => {
                let Scratch { f, .. } = &mut self.scratch;
                let o = index_of(&self.regs[off.range()], off.width, off.signed, f);
                let lo = if *up {
                    o
                } else {
                    o.saturating_sub(i64::from(dst.width)).saturating_add(1)
                };
                self.extract(*dst, *a, lo);
            }
            Op::DynIndex {
                dst,
                a,
                index,
                elem,
                count,
            } => {
                let Scratch { f, .. } = &mut self.scratch;
                let i = index_of(&self.regs[index.range()], index.width, index.signed, f);
                let ok = i >= 0 && u64::try_from(i).is_ok_and(|i| i < *count);
                if ok {
                    self.extract(*dst, *a, i.saturating_mul(i64::from(*elem)));
                } else {
                    self.regs[dst.range()].fill(0);
                }
            }
            Op::Insert {
                dst,
                base,
                part,
                lo,
            } => self.insert(*dst, *base, *part, *lo),
            Op::DynInsert {
                dst,
                base,
                part,
                index,
                elem,
                count,
            } => {
                let Scratch { f, .. } = &mut self.scratch;
                let i = index_of(&self.regs[index.range()], index.width, index.signed, f);
                let ok = i >= 0 && u64::try_from(i).is_ok_and(|i| i < *count);
                let lo = if ok {
                    i.saturating_mul(i64::from(*elem))
                } else {
                    i64::from(dst.width)
                };
                self.insert(*dst, *base, *part, lo);
            }
            Op::MemRead { dst, mem, addr } => {
                let layout = &prog.mems[*mem as usize];
                let index = words::to_u64_saturating(&self.regs[addr.range()]);
                let index = usize::try_from(index).unwrap_or(usize::MAX);
                if index < layout.len {
                    let lo = index * layout.elem_words;
                    let Scratch { a: sa, .. } = &mut self.scratch;
                    sa.clear();
                    sa.extend_from_slice(&self.mems[*mem as usize][lo..lo + layout.elem_words]);
                    let Scratch { a: sa, d: sd, .. } = &mut self.scratch;
                    blank(sd, *dst);
                    words::resize_into(sd, dst.width, sa, layout.elem_width, layout.signed);
                    store(&mut self.regs, *dst, sd);
                } else {
                    self.regs[dst.range()].fill(0);
                }
            }
            Op::MemWrite {
                mem,
                addr,
                data,
                en,
            } => {
                if !self.guarded(*en) {
                    return;
                }
                let layout = &prog.mems[*mem as usize];
                let index = words::to_u64_saturating(&self.regs[addr.range()]);
                let index = usize::try_from(index).unwrap_or(usize::MAX);
                if index < layout.len {
                    self.pending.push(Pending {
                        mem: *mem,
                        index,
                        data: *data,
                    });
                }
            }
            Op::Display { guard, msg } => {
                if self.guarded(*guard) {
                    self.effects.push(Effect::Display(*msg));
                }
            }
            Op::Assert {
                guard,
                cond,
                msg,
                severity,
                span,
            } => {
                if self.guarded(*guard) && words::is_zero(&self.regs[cond.range()]) {
                    self.effects.push(Effect::Assert(*msg, *severity, *span));
                }
            }
            Op::Finish { guard } => {
                if self.guarded(*guard) {
                    self.effects.push(Effect::Finish);
                }
            }
            Op::Stop { guard } => {
                if self.guarded(*guard) {
                    self.effects.push(Effect::Stop);
                }
            }
        }
    }

    fn resize(&mut self, dst: Slot, a: Slot) {
        if dst.narrow() && a.narrow() {
            let v = sext_word(self.regs[a.off as usize], a.width, a.signed);
            self.regs[dst.off as usize] = v & mask_word(dst.width);
            return;
        }
        let Scratch { a: sa, d: sd, .. } = &mut self.scratch;
        read(&self.regs, a, sa);
        blank(sd, dst);
        words::resize_into(sd, dst.width, sa, a.width, a.signed);
        store(&mut self.regs, dst, sd);
    }

    fn extract(&mut self, dst: Slot, a: Slot, lo: i64) {
        if dst.narrow() && a.narrow() && (0..64).contains(&lo) {
            let v = self.regs[a.off as usize];
            let shifted = if lo >= 64 { 0 } else { v >> lo };
            self.regs[dst.off as usize] = shifted & mask_word(dst.width);
            return;
        }
        let Scratch {
            a: sa,
            d: sd,
            e: se,
            ..
        } = &mut self.scratch;
        read(&self.regs, a, sa);
        blank(sd, dst);
        words::extract_into(sd, dst.width, sa, a.width, lo, se);
        store(&mut self.regs, dst, sd);
    }

    fn insert(&mut self, dst: Slot, base: Slot, part: Slot, lo: i64) {
        let Scratch { a: sa, d: sd, .. } = &mut self.scratch;
        read(&self.regs, base, sa);
        blank(sd, dst);
        words::resize_into(sd, dst.width, sa, base.width, base.signed);
        read(&self.regs, part, sa);
        words::insert_into(sd, dst.width, sa, part.width, lo);
        words::mask_top(sd, dst.width);
        store(&mut self.regs, dst, sd);
    }

    fn concat(&mut self, dst: Slot, parts: &[Slot]) {
        let Scratch { a: sa, d: sd, .. } = &mut self.scratch;
        blank(sd, dst);
        let mut pos = 0i64;
        for p in parts.iter().rev() {
            read(&self.regs, *p, sa);
            words::insert_into(sd, dst.width, sa, p.width, pos);
            pos += i64::from(p.width);
        }
        words::mask_top(sd, dst.width);
        store(&mut self.regs, dst, sd);
    }

    fn replicate(&mut self, dst: Slot, a: Slot, count: u32) {
        let Scratch { a: sa, d: sd, .. } = &mut self.scratch;
        read(&self.regs, a, sa);
        blank(sd, dst);
        let mut pos = 0i64;
        for _ in 0..count {
            words::insert_into(sd, dst.width, sa, a.width, pos);
            pos += i64::from(a.width);
        }
        words::mask_top(sd, dst.width);
        store(&mut self.regs, dst, sd);
    }

    fn pmux(&mut self, dst: Slot, s: Slot, a: Slot, b: Slot) {
        let sel = (0..s.width).find(|i| words::get_bit(&self.regs[s.range()], *i));
        match sel {
            None => self.resize(dst, a),
            Some(i) => {
                let lo = i64::from(i).saturating_mul(i64::from(dst.width));
                self.extract(dst, b, lo);
            }
        }
    }

    fn unary(&mut self, op: UnaryOp, dst: Slot, a: Slot) {
        if dst.narrow() && a.narrow() {
            let v = self.regs[a.off as usize];
            let m = mask_word(a.width);
            let r = match op {
                UnaryOp::Not => !v & mask_word(dst.width),
                UnaryOp::Neg => v.wrapping_neg() & mask_word(dst.width),
                UnaryOp::ReduceAnd => u64::from(v & m == m),
                UnaryOp::ReduceNand => u64::from(v & m != m),
                UnaryOp::ReduceOr => u64::from(v != 0),
                UnaryOp::ReduceNor | UnaryOp::LogicNot => u64::from(v == 0),
                UnaryOp::ReduceXor => u64::from(v.count_ones() % 2 == 1),
                UnaryOp::ReduceXnor => u64::from(v.count_ones().is_multiple_of(2)),
            };
            self.regs[dst.off as usize] = r;
            return;
        }
        let Scratch { a: sa, d: sd, .. } = &mut self.scratch;
        read(&self.regs, a, sa);
        match op {
            UnaryOp::Not => {
                blank(sd, dst);
                for (o, v) in sd.iter_mut().zip(sa.iter()) {
                    *o = !*v;
                }
                words::mask_top(sd, dst.width);
                store(&mut self.regs, dst, sd);
            }
            UnaryOp::Neg => {
                blank(sd, dst);
                words::neg_into(sd, sa, dst.width);
                store(&mut self.regs, dst, sd);
            }
            _ => {
                let r = match op {
                    UnaryOp::ReduceAnd => words::all_ones(sa, a.width),
                    UnaryOp::ReduceNand => !words::all_ones(sa, a.width),
                    UnaryOp::ReduceOr => !words::is_zero(sa),
                    UnaryOp::ReduceNor | UnaryOp::LogicNot => words::is_zero(sa),
                    UnaryOp::ReduceXor => words::parity(sa),
                    _ => !words::parity(sa),
                };
                self.regs[dst.off as usize] = u64::from(r);
            }
        }
    }

    fn binary(&mut self, op: BinaryOp, dst: Slot, a: Slot, b: Slot) {
        if dst.narrow() && a.narrow() && b.narrow() && op != BinaryOp::Pow {
            self.binary_word(op, dst, a, b);
            return;
        }
        self.binary_wide(op, dst, a, b);
    }

    fn binary_word(&mut self, op: BinaryOp, dst: Slot, a: Slot, b: Slot) {
        let av = self.regs[a.off as usize];
        let bv = self.regs[b.off as usize];
        let signed = a.signed && b.signed;
        let m = mask_word(dst.width);
        let value = |v: u64| v & m;
        let bit = |c: bool| u64::from(c);
        let r = match op {
            BinaryOp::And => value(av & bv),
            BinaryOp::Or => value(av | bv),
            BinaryOp::Xor => value(av ^ bv),
            BinaryOp::Xnor => value(!(av ^ bv)),
            BinaryOp::Add => value(av.wrapping_add(bv)),
            BinaryOp::Sub => value(av.wrapping_sub(bv)),
            BinaryOp::Mul => value(av.wrapping_mul(bv)),
            BinaryOp::Div | BinaryOp::Mod => {
                if bv == 0 {
                    0
                } else if signed {
                    let x = sext_word(av, a.width, true);
                    let y = sext_word(bv, b.width, true);
                    // Sign reinterpretation is the point of the extension.
                    let (x, y) = (x as i64, y as i64);
                    let q = if op == BinaryOp::Div {
                        x.wrapping_div(y)
                    } else {
                        x.wrapping_rem(y)
                    };
                    value(q.cast_unsigned())
                } else if op == BinaryOp::Div {
                    value(av / bv)
                } else {
                    value(av % bv)
                }
            }
            BinaryOp::Shl => value(shift_word(av, bv, a.width, false, false)),
            BinaryOp::Shr => value(shift_word(av, bv, a.width, true, false)),
            BinaryOp::Sshr => value(shift_word(
                av,
                bv,
                a.width,
                true,
                words::sign_of(&[av], a.width, a.signed),
            )),
            BinaryOp::Eq | BinaryOp::CaseEq | BinaryOp::WildEq => bit(av == bv),
            BinaryOp::Ne | BinaryOp::CaseNe => bit(av != bv),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let ord = if signed {
                    sext_word(av, a.width, true)
                        .cast_signed()
                        .cmp(&sext_word(bv, b.width, true).cast_signed())
                } else {
                    av.cmp(&bv)
                };
                bit(relation(op, ord))
            }
            BinaryOp::LogicAnd => bit(av != 0 && bv != 0),
            BinaryOp::LogicOr => bit(av != 0 || bv != 0),
            BinaryOp::Pow => unreachable!("handled by the wide path"),
        };
        self.regs[dst.off as usize] = r;
    }

    fn binary_wide(&mut self, op: BinaryOp, dst: Slot, a: Slot, b: Slot) {
        let signed = a.signed && b.signed;
        let width = dst.width;
        let Scratch {
            a: sa,
            b: sb,
            d: sd,
            e: se,
            f: sf,
            g: sg,
        } = &mut self.scratch;
        read(&self.regs, a, sa);
        read(&self.regs, b, sb);
        blank(sd, dst);
        let mut predicate: Option<bool> = None;
        match op {
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor | BinaryOp::Xnor => {
                for (i, o) in sd.iter_mut().enumerate() {
                    let (x, y) = (sa[i], sb[i]);
                    *o = match op {
                        BinaryOp::And => x & y,
                        BinaryOp::Or => x | y,
                        BinaryOp::Xor => x ^ y,
                        _ => !(x ^ y),
                    };
                }
                words::mask_top(sd, width);
            }
            BinaryOp::Add => words::add_into(sd, sa, sb, width),
            BinaryOp::Sub => words::sub_into(sd, sa, sb, width),
            BinaryOp::Mul => words::mul_into(sd, sa, sb, width),
            BinaryOp::Div | BinaryOp::Mod => {
                se.clear();
                se.resize(sd.len(), 0);
                let ok = words::divrem_into(sd, se, sa, sb, width, signed, sf, sg);
                if !ok {
                    sd.fill(0);
                } else if op == BinaryOp::Mod {
                    sd.copy_from_slice(se);
                }
            }
            BinaryOp::Shl => {
                let n = words::to_u64_saturating(sb);
                words::shl_into(sd, sa, width, n);
            }
            BinaryOp::Shr | BinaryOp::Sshr => {
                let n = words::to_u64_saturating(sb);
                let fill = op == BinaryOp::Sshr && words::sign_of(sa, a.width, a.signed);
                words::shr_into(sd, sa, width, n, fill);
            }
            BinaryOp::Pow => {
                let l = as_logic(sa, a).pow(&as_logic(sb, b));
                let packed = super::prog::known_words(&l);
                for (i, o) in sd.iter_mut().enumerate() {
                    *o = packed.get(i).copied().unwrap_or(0);
                }
                words::mask_top(sd, width);
            }
            BinaryOp::Eq | BinaryOp::CaseEq | BinaryOp::WildEq => predicate = Some(sa == sb),
            BinaryOp::Ne | BinaryOp::CaseNe => predicate = Some(sa != sb),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                predicate = Some(relation(op, words::cmp(sa, sb, a.width, signed)));
            }
            BinaryOp::LogicAnd => predicate = Some(!words::is_zero(sa) && !words::is_zero(sb)),
            BinaryOp::LogicOr => predicate = Some(!words::is_zero(sa) || !words::is_zero(sb)),
        }
        match predicate {
            Some(p) => self.regs[dst.off as usize] = u64::from(p),
            None => store(&mut self.regs, dst, sd),
        }
    }
}

/// One-word shift by an amount that may be far wider than the value.
fn shift_word(v: u64, amount: u64, width: u32, right: bool, fill: bool) -> u64 {
    let mask = mask_word(width);
    if amount >= u64::from(width) {
        return if fill { mask } else { 0 };
    }
    // The amount is below the width, which is at most 64.
    let n = u32::try_from(amount).expect("checked against the width");
    if !right {
        return (v << n) & mask;
    }
    let shifted = (v & mask) >> n;
    if fill {
        // The vacated bits are the top `n` of the width, which is not the
        // top `n` of the word: filling by widening `v` would only reach
        // bit 63 and would reach nothing at all at a width of 64.
        shifted | (mask & !(mask >> n))
    } else {
        shifted
    }
}

/// Whether an ordering satisfies a relational operator.
fn relation(op: BinaryOp, ord: Ordering) -> bool {
    match op {
        BinaryOp::Lt => ord == Ordering::Less,
        BinaryOp::Le => ord != Ordering::Greater,
        BinaryOp::Gt => ord == Ordering::Greater,
        _ => ord != Ordering::Less,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::eval::bits_binary;

    fn slot(off: u32, width: u32, signed: bool) -> Slot {
        Slot {
            off,
            words: u32::try_from(words::words_for(width)).unwrap(),
            width,
            signed,
        }
    }

    fn empty_program(words: usize) -> Program {
        Program {
            comb: Vec::new(),
            seq: Vec::new(),
            init: vec![0; words],
            state_cur: 0,
            state_next: 0,
            state_len: 0,
            mems: Vec::new(),
            luts: Vec::new(),
            messages: Vec::new(),
            widest: 8,
        }
    }

    /// Runs one binary op both ways and compares with the shared 4-state
    /// kernel the event simulator uses.
    fn check_binary(op: BinaryOp, a: &Logic, b: &Logic, dst_width: u32, signed: bool) {
        let prog = empty_program(64);
        let mut st = State::new(&prog);
        let sa = slot(0, a.width(), a.is_signed());
        let sb = slot(16, b.width(), b.is_signed());
        let sd = slot(32, dst_width, signed);
        st.set_value(sa, a);
        st.set_value(sb, b);
        st.exec(
            &prog,
            &Op::Binary {
                op,
                dst: sd,
                a: sa,
                b: sb,
            },
        );
        let want = bits_binary(op, a.clone(), b.clone());
        if want.has_unknown() {
            return;
        }
        assert_eq!(
            st.value(sd).value_words(),
            want.resize(dst_width).value_words(),
            "{op:?} on {a} and {b}"
        );
    }

    #[test]
    fn binary_ops_match_the_event_kernels() {
        let mut seed = 0xDEAD_BEEFu64;
        let mut next = move || {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        for width in [1u32, 4, 8, 32, 64, 96] {
            for signed in [false, true] {
                for _ in 0..25 {
                    let mk = |v: u64| {
                        let mut w = vec![v; words::words_for(width)];
                        words::mask_top(&mut w, width);
                        Logic::from_planes(width, signed, w.clone(), vec![0; w.len()])
                    };
                    let a = mk(next());
                    let b = mk(next());
                    for op in BinaryOp::ALL {
                        let dw = if op.is_predicate() { 1 } else { width };
                        let dsigned = !op.is_predicate() && signed;
                        check_binary(op, &a, &b, dw, dsigned);
                    }
                }
            }
        }
    }

    /// A random operand's low word is almost always at or above the
    /// width, so the fuzzing above only ever shifts everything out. Every
    /// shift amount below the width is where the one-word fast path can
    /// differ from the kernels, and where `>>>` used to stop sign
    /// filling once the amount passed `64 - width`.
    #[test]
    fn every_shift_amount_matches_the_event_kernels() {
        let mut seed = 0x5EED_1234u64;
        let mut next = move || {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        for width in [1u32, 4, 8, 31, 32, 33, 40, 48, 60, 63, 64, 65, 96] {
            for signed in [false, true] {
                for _ in 0..8 {
                    let mut w = vec![next(); words::words_for(width)];
                    words::mask_top(&mut w, width);
                    let a = Logic::from_planes(width, signed, w.clone(), vec![0; w.len()]);
                    for amount in 0..=u64::from(width) + 1 {
                        let b = Logic::from_u64(amount, 16).with_signed(signed);
                        for op in [BinaryOp::Shl, BinaryOp::Shr, BinaryOp::Sshr] {
                            check_binary(op, &a, &b, width, signed);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn unary_ops_match_the_event_kernels() {
        let prog = empty_program(32);
        let mut st = State::new(&prog);
        for width in [1u32, 5, 64, 70] {
            for signed in [false, true] {
                let mut w = vec![0u64; words::words_for(width)];
                for (i, slot) in w.iter_mut().enumerate() {
                    *slot = 0xA5A5_1234_DEAD_0F0Fu64.rotate_left(u32::try_from(i).unwrap() * 7);
                }
                words::mask_top(&mut w, width);
                let a = Logic::from_planes(width, signed, w.clone(), vec![0; w.len()]);
                let sa = slot(0, width, signed);
                st.set_value(sa, &a);
                for op in UnaryOp::ALL {
                    let dw = if op.is_reduction() { 1 } else { width };
                    let sd = slot(8, dw, signed && !op.is_reduction());
                    st.exec(&prog, &Op::Unary { op, dst: sd, a: sa });
                    let want = crate::sim::eval::unary(op, a.clone().into()).to_logic();
                    assert_eq!(st.value(sd).value_words(), want.value_words(), "{op:?}");
                }
            }
        }
    }

    #[test]
    fn structural_ops() {
        let prog = empty_program(32);
        let mut st = State::new(&prog);
        let a = slot(0, 8, false);
        let b = slot(1, 4, false);
        let d = slot(2, 12, false);
        st.set_value(a, &Logic::parse_verilog("8'hAB").unwrap());
        st.set_value(b, &Logic::parse_verilog("4'hC").unwrap());
        st.exec(
            &prog,
            &Op::Concat {
                dst: d,
                parts: vec![a, b].into(),
            },
        );
        assert_eq!(st.value(d).to_u64(), Some(0xABC));
        let r = slot(3, 12, false);
        st.exec(
            &prog,
            &Op::Replicate {
                dst: r,
                a: b,
                count: 3,
            },
        );
        assert_eq!(st.value(r).to_u64(), Some(0xCCC));
        let e = slot(4, 4, false);
        st.exec(&prog, &Op::Extract { dst: e, a, lo: 4 });
        assert_eq!(st.value(e).to_u64(), Some(0xA));
        st.exec(&prog, &Op::Extract { dst: e, a, lo: 6 });
        assert_eq!(st.value(e).to_u64(), Some(0x2));
        let ins = slot(5, 8, false);
        st.exec(
            &prog,
            &Op::Insert {
                dst: ins,
                base: a,
                part: b,
                lo: 0,
            },
        );
        assert_eq!(st.value(ins).to_u64(), Some(0xAC));
        let idx = slot(6, 4, false);
        st.set_value(idx, &Logic::from_u64(1, 4));
        st.exec(
            &prog,
            &Op::DynIndex {
                dst: e,
                a,
                index: idx,
                elem: 4,
                count: 2,
            },
        );
        assert_eq!(st.value(e).to_u64(), Some(0xA));
        st.set_value(idx, &Logic::from_u64(9, 4));
        st.exec(
            &prog,
            &Op::DynIndex {
                dst: e,
                a,
                index: idx,
                elem: 4,
                count: 2,
            },
        );
        assert_eq!(st.value(e).to_u64(), Some(0));
        st.exec(
            &prog,
            &Op::DynExtract {
                dst: e,
                a,
                off: idx,
                up: true,
            },
        );
        assert_eq!(st.value(e).to_u64(), Some(0));
    }

    #[test]
    fn pmux_and_lut() {
        let mut prog = empty_program(32);
        prog.luts.push(vec![0b1000_1110]);
        let mut st = State::new(&prog);
        let s = slot(0, 3, false);
        let a = slot(1, 4, false);
        let b = slot(2, 12, false);
        let d = slot(5, 4, false);
        st.set_value(a, &Logic::from_u64(0xF, 4));
        st.set_value(b, &Logic::from_u64(0x321, 12));
        st.set_value(s, &Logic::from_u64(0b000, 3));
        st.exec(&prog, &Op::Pmux { dst: d, s, a, b });
        assert_eq!(st.value(d).to_u64(), Some(0xF));
        st.set_value(s, &Logic::from_u64(0b010, 3));
        st.exec(&prog, &Op::Pmux { dst: d, s, a, b });
        assert_eq!(st.value(d).to_u64(), Some(2));
        st.set_value(s, &Logic::from_u64(0b110, 3));
        st.exec(&prog, &Op::Pmux { dst: d, s, a, b });
        assert_eq!(st.value(d).to_u64(), Some(2), "multi-hot takes the lowest");
        let lut_in = slot(6, 3, false);
        let lut_out = slot(7, 1, false);
        for i in 0..8u64 {
            st.set_value(lut_in, &Logic::from_u64(i, 3));
            st.exec(
                &prog,
                &Op::Lut {
                    dst: lut_out,
                    a: lut_in,
                    init: 0,
                },
            );
            let want = (0b1000_1110u64 >> i) & 1;
            assert_eq!(st.value(lut_out).to_u64(), Some(want));
        }
    }

    #[test]
    fn division_by_zero_is_zero() {
        let prog = empty_program(32);
        let mut st = State::new(&prog);
        for width in [8u32, 96] {
            let a = slot(0, width, false);
            let b = slot(4, width, false);
            let d = slot(8, width, false);
            st.set_value(a, &Logic::from_u64(7, width));
            st.set_value(b, &Logic::zero(width));
            for op in [BinaryOp::Div, BinaryOp::Mod] {
                st.exec(&prog, &Op::Binary { op, dst: d, a, b });
                assert_eq!(st.value(d).to_u64(), Some(0));
            }
        }
    }

    #[test]
    fn memories_and_commit() {
        let mut prog = empty_program(16);
        prog.mems.push(MemLayout {
            elem_words: 1,
            elem_width: 8,
            signed: false,
            len: 4,
            init: vec![0; 4],
        });
        prog.state_cur = 10;
        prog.state_next = 12;
        prog.state_len = 1;
        let mut st = State::new(&prog);
        let addr = slot(0, 2, false);
        let data = slot(1, 8, false);
        let out = slot(2, 8, false);
        st.set_value(addr, &Logic::from_u64(2, 2));
        st.set_value(data, &Logic::from_u64(0x5A, 8));
        st.exec(
            &prog,
            &Op::MemWrite {
                mem: 0,
                addr,
                data,
                en: None,
            },
        );
        st.exec(
            &prog,
            &Op::MemRead {
                dst: out,
                mem: 0,
                addr,
            },
        );
        assert_eq!(st.value(out).to_u64(), Some(0), "writes are deferred");
        st.regs[12] = 0x99;
        st.commit(&prog);
        st.exec(
            &prog,
            &Op::MemRead {
                dst: out,
                mem: 0,
                addr,
            },
        );
        assert_eq!(st.value(out).to_u64(), Some(0x5A));
        assert_eq!(st.regs[10], 0x99);
        let layout = prog.mems[0].clone();
        assert_eq!(st.mem_value(0, &layout, 2).unwrap().to_u64(), Some(0x5A));
        assert!(st.mem_value(0, &layout, 9).is_none());
        assert!(st.set_mem_value(0, &layout, 1, &Logic::from_u64(3, 8)));
        assert!(!st.set_mem_value(0, &layout, 8, &Logic::from_u64(3, 8)));
        assert_eq!(st.mem_value(0, &layout, 1).unwrap().to_u64(), Some(3));
    }

    #[test]
    fn guards_and_effects() {
        let prog = empty_program(8);
        let mut st = State::new(&prog);
        let g = slot(0, 1, false);
        let c = slot(1, 1, false);
        st.exec(&prog, &Op::Finish { guard: Some(g) });
        assert!(st.effects.is_empty());
        st.regs[0] = 1;
        st.exec(&prog, &Op::Finish { guard: Some(g) });
        st.exec(&prog, &Op::Stop { guard: None });
        st.exec(
            &prog,
            &Op::Display {
                guard: Some(g),
                msg: 0,
            },
        );
        let mut map = crate::source::SourceMap::new();
        let file = map.add("t", "x").unwrap();
        st.exec(
            &prog,
            &Op::Assert {
                guard: Some(g),
                cond: c,
                msg: 0,
                severity: ReportSeverity::Error,
                span: Span::new(file, 0, 0),
            },
        );
        assert_eq!(st.effects.len(), 4);
    }
}
