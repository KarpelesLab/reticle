//! The compiled program: a flat list of operations over a register file.
//!
//! Lowering produces a [`Program`]. It has two op lists and no control
//! flow at all:
//!
//! * `comb` is the combinational region, in topological order. Running it
//!   from the current register values and the current inputs settles every
//!   combinational net exactly once — no event queue, no fixed-point loop,
//!   because the order was computed at compile time.
//! * `seq` is what happens at the clock edge: the next value of every
//!   register, every memory write and every surviving side effect
//!   (`$display`, an immediate assertion, `$finish`). It only *computes*;
//!   nothing it produces is visible until the commit.
//!
//! The register file is one `Vec<u64>`. Every value — a constant, a net, a
//! temporary, a register's current and next state — owns a fixed, disjoint
//! run of words in it, described by a [`Slot`]. Because the slots are
//! disjoint and each op writes its own, the program is in static single
//! assignment form, which is what makes constant folding and common
//! subexpression elimination during lowering sound.
//!
//! State is laid out so the commit is one `copy_within`: the current values
//! of every register occupy `[state_cur, state_cur + state_len)` and their
//! next values `[state_next, state_next + state_len)`, element for element.

use crate::ir::{BinaryOp, ReportSeverity, Span, UnaryOp};
use crate::sim::elab::InstId;

use super::words;

/// Where a value lives in the register file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Slot {
    /// Index of the first word.
    pub(crate) off: u32,
    /// Number of words.
    pub(crate) words: u32,
    /// Number of significant bits; the rest of the top word is zero.
    pub(crate) width: u32,
    /// Whether the value is signed, which decides extension and ordering.
    pub(crate) signed: bool,
}

impl Slot {
    /// The word range the slot occupies.
    pub(crate) fn range(self) -> std::ops::Range<usize> {
        let off = self.off as usize;
        off..off + self.words as usize
    }

    /// True for the common case of a value that fits one machine word.
    pub(crate) fn narrow(self) -> bool {
        self.words == 1
    }
}

/// One argument of a surviving `$display` or assertion message.
#[derive(Clone, Debug)]
pub(crate) enum MsgArg {
    /// A string literal from the source.
    Str(String),
    /// A value read out of the register file.
    Val(Slot),
}

/// A formatted side effect: `$display`, `$write` or an assertion message.
#[derive(Clone, Debug)]
pub(crate) struct Message {
    /// The instance the text is formatted in (`%m`, `$time` scaling).
    pub(crate) inst: InstId,
    /// The arguments, in call order.
    pub(crate) args: Vec<MsgArg>,
    /// The radix a bare value prints in (`'d'`, `'b'`, `'o'`, `'h'`).
    pub(crate) radix: char,
    /// True to append a newline (`$display` rather than `$write`).
    pub(crate) newline: bool,
}

/// One operation of the compiled program.
///
/// Every variant names its destination slot first; sources never overlap
/// it.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    /// `dst = a`, truncated or extended to `dst`'s width.
    Resize {
        /// Destination.
        dst: Slot,
        /// Source.
        a: Slot,
    },
    /// `dst = op a`.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// Destination.
        dst: Slot,
        /// Operand.
        a: Slot,
    },
    /// `dst = a op b`, with `a` and `b` already sized by the lowerer.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Destination.
        dst: Slot,
        /// Left operand.
        a: Slot,
        /// Right operand.
        b: Slot,
    },
    /// `dst = s ? b : a`, the IR's `Mux` cell and `Ternary` expression.
    Mux {
        /// Destination.
        dst: Slot,
        /// Select; non-zero picks `b`.
        s: Slot,
        /// Value for a zero select.
        a: Slot,
        /// Value for a non-zero select.
        b: Slot,
    },
    /// One-hot multiplexer: bit `i` of `s` selects slice `i` of `b`, a zero
    /// select picks `a`.
    Pmux {
        /// Destination.
        dst: Slot,
        /// One-hot select.
        s: Slot,
        /// Default.
        a: Slot,
        /// The concatenated candidates.
        b: Slot,
    },
    /// `dst` = bit `a` of a truth table.
    Lut {
        /// Destination, one bit.
        dst: Slot,
        /// The index.
        a: Slot,
        /// Index into [`Program::luts`].
        init: u32,
    },
    /// `dst` = the parts concatenated, the first most significant.
    Concat {
        /// Destination.
        dst: Slot,
        /// The parts.
        parts: Box<[Slot]>,
    },
    /// `dst` = `count` copies of `a`.
    Replicate {
        /// Destination.
        dst: Slot,
        /// The repeated value.
        a: Slot,
        /// Number of copies.
        count: u32,
    },
    /// `dst` = `dst.width` bits of `a` starting at bit `lo`.
    Extract {
        /// Destination.
        dst: Slot,
        /// Source.
        a: Slot,
        /// Starting bit, which may lie outside `a`.
        lo: i64,
    },
    /// `dst` = `dst.width` bits of `a` at a computed bit offset
    /// (`a[off +: w]` and `a[off -: w]`).
    DynExtract {
        /// Destination.
        dst: Slot,
        /// Source.
        a: Slot,
        /// The offset in bits.
        off: Slot,
        /// False when the slice extends downwards (`-:`).
        up: bool,
    },
    /// `dst` = element `index` of `a`; an index outside `count` reads
    /// zero.
    DynIndex {
        /// Destination.
        dst: Slot,
        /// Source.
        a: Slot,
        /// The element index.
        index: Slot,
        /// Bits per element.
        elem: u32,
        /// Number of elements.
        count: u64,
    },
    /// `dst` = `base` with bits `[lo, lo + part.width)` replaced.
    Insert {
        /// Destination.
        dst: Slot,
        /// The value written around.
        base: Slot,
        /// The replacement.
        part: Slot,
        /// Where the replacement starts.
        lo: i64,
    },
    /// `dst` = `base` with element `index` replaced.
    DynInsert {
        /// Destination.
        dst: Slot,
        /// The value written around.
        base: Slot,
        /// The replacement.
        part: Slot,
        /// The element index.
        index: Slot,
        /// Bits per element.
        elem: u32,
        /// Number of elements; an index at or past it writes nothing.
        count: u64,
    },
    /// `dst` = memory element `addr`; out of range reads zero.
    MemRead {
        /// Destination.
        dst: Slot,
        /// Index into [`super::engine::State::mems`].
        mem: u32,
        /// The element address.
        addr: Slot,
    },
    /// Queues a memory write for the commit.
    MemWrite {
        /// Index into [`super::engine::State::mems`].
        mem: u32,
        /// The element address.
        addr: Slot,
        /// The value.
        data: Slot,
        /// Write enable; `None` writes unconditionally.
        en: Option<Slot>,
    },
    /// Appends formatted text to the output.
    Display {
        /// Guard; the op runs when it is non-zero or absent.
        guard: Option<Slot>,
        /// Index into [`Program::messages`].
        msg: u32,
    },
    /// Reports when `cond` is zero.
    Assert {
        /// Guard; the op runs when it is non-zero or absent.
        guard: Option<Slot>,
        /// The condition that must hold.
        cond: Slot,
        /// Index into [`Program::messages`].
        msg: u32,
        /// How serious a failure is.
        severity: ReportSeverity,
        /// Where the assertion is written.
        span: Span,
    },
    /// Ends the run (`$finish`).
    Finish {
        /// Guard; the op runs when it is non-zero or absent.
        guard: Option<Slot>,
    },
    /// Pauses the run (`$stop`).
    Stop {
        /// Guard; the op runs when it is non-zero or absent.
        guard: Option<Slot>,
    },
}

/// A memory of the compiled design, elements packed word-aligned.
#[derive(Clone, Debug)]
pub(crate) struct MemLayout {
    /// Words per element.
    pub(crate) elem_words: usize,
    /// Bits per element.
    pub(crate) elem_width: u32,
    /// Whether elements are signed.
    pub(crate) signed: bool,
    /// Number of elements.
    pub(crate) len: usize,
    /// The initial contents, `len * elem_words` words.
    pub(crate) init: Vec<u64>,
}

/// The straight-line program a compiled run executes.
#[derive(Clone, Debug)]
pub(crate) struct Program {
    /// The combinational region, in topological order.
    pub(crate) comb: Vec<Op>,
    /// The clock edge: next-state computation and side effects.
    pub(crate) seq: Vec<Op>,
    /// The register file at reset: constants and initial state.
    pub(crate) init: Vec<u64>,
    /// First word of the current-state block.
    pub(crate) state_cur: usize,
    /// First word of the next-state block.
    pub(crate) state_next: usize,
    /// Length of each state block in words.
    pub(crate) state_len: usize,
    /// Memories, in the order of the flattened design.
    pub(crate) mems: Vec<MemLayout>,
    /// Truth tables referenced by [`Op::Lut`].
    pub(crate) luts: Vec<Vec<u64>>,
    /// Message templates referenced by [`Op::Display`] and [`Op::Assert`].
    pub(crate) messages: Vec<Message>,
    /// The widest value in words, which sizes the engine's scratch buffers.
    pub(crate) widest: usize,
}

/// A count of what a compiled program contains, for reports and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProgramStats {
    /// Operations in the combinational region.
    pub comb_ops: usize,
    /// Operations run at the clock edge.
    pub seq_ops: usize,
    /// State elements (registers and registered memory reads).
    pub registers: usize,
    /// Bits of state.
    pub state_bits: u32,
    /// Machine words in the register file.
    pub words: usize,
    /// Memories.
    pub memories: usize,
}

impl std::fmt::Display for ProgramStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} comb ops, {} edge ops, {} registers ({} bits), {} memories, {} words",
            self.comb_ops, self.seq_ops, self.registers, self.state_bits, self.memories, self.words
        )
    }
}

/// Packs a [`crate::logic::Logic`] constant into two-state words, with
/// every unknown bit read as zero.
pub(crate) fn known_words(value: &crate::logic::Logic) -> Vec<u64> {
    let mut out: Vec<u64> = value
        .value_words()
        .iter()
        .zip(value.unknown_words())
        .map(|(v, u)| v & !u)
        .collect();
    words::mask_top(&mut out, value.width());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::Logic;

    #[test]
    fn slots_describe_word_ranges() {
        let s = Slot {
            off: 3,
            words: 2,
            width: 100,
            signed: true,
        };
        assert_eq!(s.range(), 3..5);
        assert!(!s.narrow());
        assert!(
            Slot {
                off: 0,
                words: 1,
                width: 8,
                signed: false
            }
            .narrow()
        );
    }

    #[test]
    fn constants_drop_unknown_bits() {
        assert_eq!(
            known_words(&Logic::parse_verilog("8'b1x1z0101").unwrap()),
            [0b1010_0101]
        );
        assert_eq!(known_words(&Logic::zero(0)), Vec::<u64>::new());
    }

    #[test]
    fn stats_render() {
        let s = ProgramStats {
            comb_ops: 4,
            seq_ops: 2,
            registers: 1,
            state_bits: 8,
            words: 12,
            memories: 0,
        };
        assert_eq!(
            s.to_string(),
            "4 comb ops, 2 edge ops, 1 registers (8 bits), 0 memories, 12 words"
        );
    }
}
