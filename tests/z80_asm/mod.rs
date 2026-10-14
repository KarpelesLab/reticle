//! A minimal Zilog Z80 assembler, for the tests that run machine code on
//! the `z80` core.
//!
//! The whole of it hangs off [`TABLE`], which is the documented
//! instruction set typed out in encoding order: for each of the 698
//! encodings, the mnemonic, the shape its operands are written in, the
//! prefix and opcode bytes, and the T states the part's documentation
//! prints for it.
//!
//! **Nothing here is taken from the core's decoder, and nothing here has
//! been checked against it.** The bytes and the T states were typed out
//! from the published reference — the Zilog *Z80 CPU User Manual* and the
//! opcode matrices that come with it — because a testbench that assembled
//! its programs with the implementation it is testing would agree with it
//! however wrong both were, and a table corrected against the core under
//! test is the same mistake with an extra step in front of it. Where this
//! file and the core disagree, this file is the reference and the core is
//! the thing on trial. The T states are the reference's, which is what a
//! test that counts a core's cycles has to compare against.
//!
//! **CHECKED against a second reference, 15 October 2026.** Typing a
//! table out is exactly the way to put the same wrong number in two
//! places: a core and a table that agree because one person remembered
//! the same thing twice would pass every test in this repository. So all
//! 698 entries were compared, mechanically and by opcode bytes, against
//! the opcode matrix published at `clrhome.org/table/`, which was
//! compiled by somebody else and is not derived from anything here.
//! **Every entry
//! agrees**, in the mnemonic, in both T-state counts where there are
//! two, in the encoding's length in bytes, and in the operand text
//! itself but for two notation choices: that reference writes a
//! relative branch's displacement `d` where this writes `label`, and
//! `rst 00h` where this writes `rst $00`. The comparison was a
//! throwaway script against that page's own markup, not anything in this
//! repository, which is why it is recorded here as a reading rather than
//! left as a test: nothing is committed that would re-run it.
//!
//! That reference holds 1443 cells to this table's 698, and the 745 it
//! has and this does not are all deliberate, which is worth counting out
//! because a table short of an instruction would look exactly like a
//! table that is right:
//!
//! | absent | why |
//! |---|---|
//! | 322 | `DD CB` forms that also copy the result into a register |
//! | 160 | `DD` / `FD` on an instruction that does not use `HL`, where the prefix is four T states and nothing else |
//! | 112 | duplicate `DD CB` `bit` encodings, same effect, different opcode |
//! | 92 | the `ixh` / `ixl` / `iyh` / `iyl` halves |
//! | 33 | Z180: `in0`, `out0`, `tst`, `mlt`, `slp`, `tstio`, `otim`, `otdm`, `otimr`, `otdmr` — not Z80 at all |
//! | 24 | `sll` |
//! | 2 | `ED 70` `in (c)` and `ED 71` `out (c),0` |
//!
//! 322 + 160 + 112 + 92 + 33 + 24 + 2 is 745, so nothing is left over:
//! every cell of that reference is either in this table or in a row
//! above. "The undocumented encodings are deliberately absent", below,
//! says why each row is a choice.
//!
//! What that check would not catch: an error common to both, since both
//! are readings of the same published instruction set, and anything at
//! all about the core — which is the point of keeping this file ignorant
//! of it.
//!
//! All five encoding spaces are here:
//!
//! | space | entries |
//! |---|---|
//! | unprefixed | 252 — every byte but the four prefixes |
//! | `cb` | 248 — the rotates, shifts and bit operations |
//! | `ed` | 58 — the sparse documented set |
//! | `dd` / `fd` | 78 — 39 IX forms and the same 39 with IY |
//! | `dd cb` / `fd cb` | 62 — bit operations on `(ix+d)` and `(iy+d)` |
//!
//! [`assemble`] is a two-pass assembler over that table, in the syntax
//! every Z80 assembler accepts, lower case: `ld a,(hl)`, `ld a,$12`,
//! `ld hl,$1234`, `ld (ix+5),b`, `jr nz,loop`, `jp (hl)`, `bit 3,(iy-2)`,
//! `ex af,af'`, `in a,($fe)`, `out ($fe),a`, `im 2`, `rst $38`, `ldir`,
//! `add hl,de`, `adc a,b`, `sbc hl,bc`. It is deliberately small: the
//! directives `.org`, `.equ`, `.db`, `.dw` and `.ds`, `;` comments,
//! labels, `<` and `>` for the low and high byte of a value, and no
//! expression beyond that — no arithmetic, no macros, no segments.
//!
//! An operand shape is the reference's own operand text, with `n` for an
//! eight-bit literal, `nn` for a sixteen-bit one, `d` for an index
//! displacement and `label` for a relative branch's target: `"a,(hl)"`,
//! `"hl,nn"`, `"(ix+d),b"`, `"nz,label"`, `""`. [`Field`] says which of
//! those an encoding carries and where it goes, and is as far as the
//! assembler has to look to know how long an instruction is.
//!
//! Seven rules resolve what the assembly language leaves ambiguous. The
//! first is the one that bites:
//!
//! 1. **The accumulator is never implicit.** `add`, `adc` and `sbc` take
//!    two operands and `sub`, `and`, `or`, `xor` and `cp` take one, which
//!    is the reference's own split: `add a,b` and `cp $10`, never
//!    `add b` or `cp a,$10`. Accepting both spellings would put two
//!    entries on one opcode, and `sub a` is genuinely ambiguous between
//!    the one-operand form and a two-operand one with its source left
//!    off, so there is exactly one way to write each.
//! 2. **An index operand always carries its displacement**: `(ix+0)`, not
//!    `(ix)`, because `(ix)` is the operand of `jp (ix)` and of nothing
//!    else, and that instruction has no displacement byte to hold a zero.
//! 3. `(bc)`, `(de)`, `(hl)`, `(sp)`, `(c)`, `(ix)` and `(iy)` are always
//!    the register indirection and never a parenthesised symbol of that
//!    name; registers, condition codes and flags are reserved, so a label
//!    may not be called `c` or `m`.
//! 4. **The relative branch field is spelled `label`, not the
//!    reference's `e`**, because `e` is also a register and `jr e` would
//!    otherwise read as an eight-bit register operand.
//! 5. A shape token that is a number — the bit number of `bit 3,(hl)`,
//!    the mode of `im 2`, the vector of `rst $38` — is matched by value,
//!    so `rst 56` and `rst $38` are one instruction. Such an operand has
//!    to be a literal or an `.equ` that is already defined, because the
//!    first pass needs its value to know which encoding it is looking at.
//! 6. **`ld hl,(nn)` and `ld (nn),hl` each have two documented
//!    encodings**, the three-byte `0x2a` / `0x22` and the four-byte
//!    `ed 6b` / `ed 63`. Both are in the table, and [`find`] and the
//!    assembler always reach the short one, which is what every assembler
//!    emits.
//! 7. **The displacement of a `dd cb` bit operation goes between the
//!    `cb` and the opcode byte** — `dd cb d 5e` for `bit 3,(ix+d)` — and
//!    after the opcode everywhere else. That is the published encoding,
//!    and it is the only place where a literal is not last.
//!
//! No operand's *width* is ambiguous, which is the one place the Z80 is
//! easier than the 6502: no mnemonic has both an `n` and an `nn` form in
//! the same position, so `ld a,$0012` is still eight bits and
//! `ld hl,$12` is still sixteen, and the 6502 assembler's
//! three-hexadecimal-digits rule has no counterpart here.
//!
//! The undocumented encodings are deliberately absent: `sll`
//! (`cb 30`–`cb 37`), the `ixh` / `ixl` / `iyh` / `iyl` halves of the
//! index registers, the duplicate `im` and `retn` encodings in the `ed`
//! space, and `in (c)` / `out (c),0`. A core may implement them; this
//! file will not assemble them, and a test that wants their bytes writes
//! them with `.db`.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// The literal field an encoding carries, and where in the instruction it
/// goes.
///
/// This is the Z80's answer to the 6502's thirteen addressing modes,
/// except that on the Z80 the *shape* of an operand is the text the
/// reference prints for it — [`TABLE`] holds 298 distinct shapes, so
/// writing them out as an enumeration as well would be writing the same
/// strings twice — and this is only the hole the assembler has to fill
/// in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Field {
    /// Nothing to fill in: every byte of the encoding is fixed. `nop`,
    /// `ld a,(hl)`, `ldir`, `ex af,af'`, `im 2`, `rst $38`.
    None,
    /// An eight-bit immediate `n`, after the opcode. `ld a,$12`.
    Byte,
    /// A sixteen-bit immediate or address `nn`, after the opcode, low
    /// byte first. `ld hl,$1234`, `jp $0038`, `ld ($1234),a`.
    Word,
    /// An eight-bit port number in `(n)`, after the opcode.
    /// `in a,($fe)`, `out ($fe),a`.
    Port,
    /// A signed displacement `d`, which is where rule 7 of the module
    /// documentation applies. `ld (ix+5),b`, `bit 3,(iy-2)`.
    Disp,
    /// A displacement and then an eight-bit immediate, in that order:
    /// `ld (ix+5),$12` is `dd 36 05 12`.
    DispByte,
    /// A relative branch's signed displacement from the address of the
    /// next instruction, which the source writes as the target it wants.
    /// `jr nz,loop`, `djnz loop`.
    Rel,
}

impl Field {
    /// Which field a shape carries, read off the shape's own tokens.
    pub(crate) fn of(shape: &str) -> Field {
        let has = |want: &str| tokens(shape).any(|token| token == want);
        if has("(ix+d)") || has("(iy+d)") {
            return if has("n") {
                Field::DispByte
            } else {
                Field::Disp
            };
        }
        if has("nn") || has("(nn)") {
            Field::Word
        } else if has("(n)") {
            Field::Port
        } else if has("n") {
            Field::Byte
        } else if has("label") {
            Field::Rel
        } else {
            Field::None
        }
    }

    /// How many bytes the assembler fills in, on top of the fixed ones.
    pub(crate) fn operand_bytes(self) -> u16 {
        match self {
            Field::None => 0,
            Field::Byte | Field::Port | Field::Disp | Field::Rel => 1,
            Field::Word | Field::DispByte => 2,
        }
    }
}

/// The comma-separated tokens of an operand shape, which are none at all
/// for an instruction that takes no operand.
pub(crate) fn tokens(shape: &str) -> impl Iterator<Item = &str> {
    shape.split(',').filter(|token| !token.is_empty())
}

/// One documented encoding.
#[derive(Clone, Copy)]
pub(crate) struct Insn {
    /// The mnemonic, lower case.
    pub(crate) name: &'static str,
    /// How its operands are written, as the module documentation
    /// describes: `""`, `"a,(hl)"`, `"hl,nn"`, `"(ix+d),b"`,
    /// `"nz,label"`.
    pub(crate) shape: &'static str,
    /// The fixed bytes: the prefix if it has one, then the opcode.
    pub(crate) code: &'static [u8],
    /// The T states the reference prints for it. Where it prints two,
    /// this is the longer: the conditional jump, call or return that is
    /// taken, and the block instruction that repeats.
    pub(crate) tstates: u8,
    /// The reference's second count, where it prints two: the
    /// conditional jump, call or return that is **not** taken, and the
    /// final, non-repeating iteration of a block instruction. `None` for
    /// an instruction that always takes the same time.
    pub(crate) tstates_short: Option<u8>,
}

impl Insn {
    /// The literal field this encoding carries.
    pub(crate) fn field(&self) -> Field {
        Field::of(self.shape)
    }

    /// The whole instruction's length in bytes.
    pub(crate) fn len(&self) -> u16 {
        u16::try_from(self.code.len()).expect("four bytes at most") + self.field().operand_bytes()
    }

    /// Whether the reference prints two T-state counts for it.
    pub(crate) fn is_conditional(&self) -> bool {
        self.tstates_short.is_some()
    }

    /// How it is written, for a message.
    pub(crate) fn syntax(&self) -> String {
        if self.shape.is_empty() {
            self.name.to_owned()
        } else {
            format!("{} {}", self.name, self.shape)
        }
    }
}

const fn i(name: &'static str, shape: &'static str, code: &'static [u8], tstates: u8) -> Insn {
    Insn {
        name,
        shape,
        code,
        tstates,
        tstates_short: None,
    }
}

const fn c(
    name: &'static str,
    shape: &'static str,
    code: &'static [u8],
    tstates: u8,
    short: u8,
) -> Insn {
    Insn {
        name,
        shape,
        code,
        tstates,
        tstates_short: Some(short),
    }
}

/// Every documented Z80 encoding, in the order the opcode matrices print
/// them: the unprefixed space, then `cb`, then `ed`, then `dd` and `fd`,
/// then `dd cb` and `fd cb`.
///
/// The two four-byte `ed` forms of `ld hl,(nn)` and `ld (nn),hl` are
/// here for completeness and are shadowed by their three-byte forms; see
/// rule 6 of the module documentation.
pub(crate) const TABLE: &[Insn] = &[
    // ---------------------------------------------------------------
    // The unprefixed space, 0x00 through 0xFF with the four prefix
    // bytes 0xCB, 0xDD, 0xED and 0xFD left out.
    // ---------------------------------------------------------------
    // 0x00
    i("nop", "", &[0x00], 4),
    i("ld", "bc,nn", &[0x01], 10),
    i("ld", "(bc),a", &[0x02], 7),
    i("inc", "bc", &[0x03], 6),
    i("inc", "b", &[0x04], 4),
    i("dec", "b", &[0x05], 4),
    i("ld", "b,n", &[0x06], 7),
    i("rlca", "", &[0x07], 4),
    i("ex", "af,af'", &[0x08], 4),
    i("add", "hl,bc", &[0x09], 11),
    i("ld", "a,(bc)", &[0x0A], 7),
    i("dec", "bc", &[0x0B], 6),
    i("inc", "c", &[0x0C], 4),
    i("dec", "c", &[0x0D], 4),
    i("ld", "c,n", &[0x0E], 7),
    i("rrca", "", &[0x0F], 4),
    // 0x10
    c("djnz", "label", &[0x10], 13, 8),
    i("ld", "de,nn", &[0x11], 10),
    i("ld", "(de),a", &[0x12], 7),
    i("inc", "de", &[0x13], 6),
    i("inc", "d", &[0x14], 4),
    i("dec", "d", &[0x15], 4),
    i("ld", "d,n", &[0x16], 7),
    i("rla", "", &[0x17], 4),
    i("jr", "label", &[0x18], 12),
    i("add", "hl,de", &[0x19], 11),
    i("ld", "a,(de)", &[0x1A], 7),
    i("dec", "de", &[0x1B], 6),
    i("inc", "e", &[0x1C], 4),
    i("dec", "e", &[0x1D], 4),
    i("ld", "e,n", &[0x1E], 7),
    i("rra", "", &[0x1F], 4),
    // 0x20
    c("jr", "nz,label", &[0x20], 12, 7),
    i("ld", "hl,nn", &[0x21], 10),
    i("ld", "(nn),hl", &[0x22], 16),
    i("inc", "hl", &[0x23], 6),
    i("inc", "h", &[0x24], 4),
    i("dec", "h", &[0x25], 4),
    i("ld", "h,n", &[0x26], 7),
    i("daa", "", &[0x27], 4),
    c("jr", "z,label", &[0x28], 12, 7),
    i("add", "hl,hl", &[0x29], 11),
    i("ld", "hl,(nn)", &[0x2A], 16),
    i("dec", "hl", &[0x2B], 6),
    i("inc", "l", &[0x2C], 4),
    i("dec", "l", &[0x2D], 4),
    i("ld", "l,n", &[0x2E], 7),
    i("cpl", "", &[0x2F], 4),
    // 0x30
    c("jr", "nc,label", &[0x30], 12, 7),
    i("ld", "sp,nn", &[0x31], 10),
    i("ld", "(nn),a", &[0x32], 13),
    i("inc", "sp", &[0x33], 6),
    i("inc", "(hl)", &[0x34], 11),
    i("dec", "(hl)", &[0x35], 11),
    i("ld", "(hl),n", &[0x36], 10),
    i("scf", "", &[0x37], 4),
    c("jr", "c,label", &[0x38], 12, 7),
    i("add", "hl,sp", &[0x39], 11),
    i("ld", "a,(nn)", &[0x3A], 13),
    i("dec", "sp", &[0x3B], 6),
    i("inc", "a", &[0x3C], 4),
    i("dec", "a", &[0x3D], 4),
    i("ld", "a,n", &[0x3E], 7),
    i("ccf", "", &[0x3F], 4),
    // 0x40: `ld r,r'`, the eight destinations by the eight sources,
    // with 0x76 holding `halt` where `ld (hl),(hl)` would be.
    i("ld", "b,b", &[0x40], 4),
    i("ld", "b,c", &[0x41], 4),
    i("ld", "b,d", &[0x42], 4),
    i("ld", "b,e", &[0x43], 4),
    i("ld", "b,h", &[0x44], 4),
    i("ld", "b,l", &[0x45], 4),
    i("ld", "b,(hl)", &[0x46], 7),
    i("ld", "b,a", &[0x47], 4),
    // 0x48
    i("ld", "c,b", &[0x48], 4),
    i("ld", "c,c", &[0x49], 4),
    i("ld", "c,d", &[0x4A], 4),
    i("ld", "c,e", &[0x4B], 4),
    i("ld", "c,h", &[0x4C], 4),
    i("ld", "c,l", &[0x4D], 4),
    i("ld", "c,(hl)", &[0x4E], 7),
    i("ld", "c,a", &[0x4F], 4),
    // 0x50
    i("ld", "d,b", &[0x50], 4),
    i("ld", "d,c", &[0x51], 4),
    i("ld", "d,d", &[0x52], 4),
    i("ld", "d,e", &[0x53], 4),
    i("ld", "d,h", &[0x54], 4),
    i("ld", "d,l", &[0x55], 4),
    i("ld", "d,(hl)", &[0x56], 7),
    i("ld", "d,a", &[0x57], 4),
    // 0x58
    i("ld", "e,b", &[0x58], 4),
    i("ld", "e,c", &[0x59], 4),
    i("ld", "e,d", &[0x5A], 4),
    i("ld", "e,e", &[0x5B], 4),
    i("ld", "e,h", &[0x5C], 4),
    i("ld", "e,l", &[0x5D], 4),
    i("ld", "e,(hl)", &[0x5E], 7),
    i("ld", "e,a", &[0x5F], 4),
    // 0x60
    i("ld", "h,b", &[0x60], 4),
    i("ld", "h,c", &[0x61], 4),
    i("ld", "h,d", &[0x62], 4),
    i("ld", "h,e", &[0x63], 4),
    i("ld", "h,h", &[0x64], 4),
    i("ld", "h,l", &[0x65], 4),
    i("ld", "h,(hl)", &[0x66], 7),
    i("ld", "h,a", &[0x67], 4),
    // 0x68
    i("ld", "l,b", &[0x68], 4),
    i("ld", "l,c", &[0x69], 4),
    i("ld", "l,d", &[0x6A], 4),
    i("ld", "l,e", &[0x6B], 4),
    i("ld", "l,h", &[0x6C], 4),
    i("ld", "l,l", &[0x6D], 4),
    i("ld", "l,(hl)", &[0x6E], 7),
    i("ld", "l,a", &[0x6F], 4),
    // 0x70
    i("ld", "(hl),b", &[0x70], 7),
    i("ld", "(hl),c", &[0x71], 7),
    i("ld", "(hl),d", &[0x72], 7),
    i("ld", "(hl),e", &[0x73], 7),
    i("ld", "(hl),h", &[0x74], 7),
    i("ld", "(hl),l", &[0x75], 7),
    i("halt", "", &[0x76], 4),
    i("ld", "(hl),a", &[0x77], 7),
    // 0x78
    i("ld", "a,b", &[0x78], 4),
    i("ld", "a,c", &[0x79], 4),
    i("ld", "a,d", &[0x7A], 4),
    i("ld", "a,e", &[0x7B], 4),
    i("ld", "a,h", &[0x7C], 4),
    i("ld", "a,l", &[0x7D], 4),
    i("ld", "a,(hl)", &[0x7E], 7),
    i("ld", "a,a", &[0x7F], 4),
    // 0x80: the eight-bit arithmetic and logic group, eight
    // operations by the same eight operands.
    // 0x80
    i("add", "a,b", &[0x80], 4),
    i("add", "a,c", &[0x81], 4),
    i("add", "a,d", &[0x82], 4),
    i("add", "a,e", &[0x83], 4),
    i("add", "a,h", &[0x84], 4),
    i("add", "a,l", &[0x85], 4),
    i("add", "a,(hl)", &[0x86], 7),
    i("add", "a,a", &[0x87], 4),
    // 0x88
    i("adc", "a,b", &[0x88], 4),
    i("adc", "a,c", &[0x89], 4),
    i("adc", "a,d", &[0x8A], 4),
    i("adc", "a,e", &[0x8B], 4),
    i("adc", "a,h", &[0x8C], 4),
    i("adc", "a,l", &[0x8D], 4),
    i("adc", "a,(hl)", &[0x8E], 7),
    i("adc", "a,a", &[0x8F], 4),
    // 0x90
    i("sub", "b", &[0x90], 4),
    i("sub", "c", &[0x91], 4),
    i("sub", "d", &[0x92], 4),
    i("sub", "e", &[0x93], 4),
    i("sub", "h", &[0x94], 4),
    i("sub", "l", &[0x95], 4),
    i("sub", "(hl)", &[0x96], 7),
    i("sub", "a", &[0x97], 4),
    // 0x98
    i("sbc", "a,b", &[0x98], 4),
    i("sbc", "a,c", &[0x99], 4),
    i("sbc", "a,d", &[0x9A], 4),
    i("sbc", "a,e", &[0x9B], 4),
    i("sbc", "a,h", &[0x9C], 4),
    i("sbc", "a,l", &[0x9D], 4),
    i("sbc", "a,(hl)", &[0x9E], 7),
    i("sbc", "a,a", &[0x9F], 4),
    // 0xA0
    i("and", "b", &[0xA0], 4),
    i("and", "c", &[0xA1], 4),
    i("and", "d", &[0xA2], 4),
    i("and", "e", &[0xA3], 4),
    i("and", "h", &[0xA4], 4),
    i("and", "l", &[0xA5], 4),
    i("and", "(hl)", &[0xA6], 7),
    i("and", "a", &[0xA7], 4),
    // 0xA8
    i("xor", "b", &[0xA8], 4),
    i("xor", "c", &[0xA9], 4),
    i("xor", "d", &[0xAA], 4),
    i("xor", "e", &[0xAB], 4),
    i("xor", "h", &[0xAC], 4),
    i("xor", "l", &[0xAD], 4),
    i("xor", "(hl)", &[0xAE], 7),
    i("xor", "a", &[0xAF], 4),
    // 0xB0
    i("or", "b", &[0xB0], 4),
    i("or", "c", &[0xB1], 4),
    i("or", "d", &[0xB2], 4),
    i("or", "e", &[0xB3], 4),
    i("or", "h", &[0xB4], 4),
    i("or", "l", &[0xB5], 4),
    i("or", "(hl)", &[0xB6], 7),
    i("or", "a", &[0xB7], 4),
    // 0xB8
    i("cp", "b", &[0xB8], 4),
    i("cp", "c", &[0xB9], 4),
    i("cp", "d", &[0xBA], 4),
    i("cp", "e", &[0xBB], 4),
    i("cp", "h", &[0xBC], 4),
    i("cp", "l", &[0xBD], 4),
    i("cp", "(hl)", &[0xBE], 7),
    i("cp", "a", &[0xBF], 4),
    // 0xC0
    c("ret", "nz", &[0xC0], 11, 5),
    i("pop", "bc", &[0xC1], 10),
    i("jp", "nz,nn", &[0xC2], 10),
    i("jp", "nn", &[0xC3], 10),
    c("call", "nz,nn", &[0xC4], 17, 10),
    i("push", "bc", &[0xC5], 11),
    i("add", "a,n", &[0xC6], 7),
    i("rst", "$00", &[0xC7], 11),
    c("ret", "z", &[0xC8], 11, 5),
    i("ret", "", &[0xC9], 10),
    i("jp", "z,nn", &[0xCA], 10),
    c("call", "z,nn", &[0xCC], 17, 10),
    i("call", "nn", &[0xCD], 17),
    i("adc", "a,n", &[0xCE], 7),
    i("rst", "$08", &[0xCF], 11),
    // 0xD0
    c("ret", "nc", &[0xD0], 11, 5),
    i("pop", "de", &[0xD1], 10),
    i("jp", "nc,nn", &[0xD2], 10),
    i("out", "(n),a", &[0xD3], 11),
    c("call", "nc,nn", &[0xD4], 17, 10),
    i("push", "de", &[0xD5], 11),
    i("sub", "n", &[0xD6], 7),
    i("rst", "$10", &[0xD7], 11),
    c("ret", "c", &[0xD8], 11, 5),
    i("exx", "", &[0xD9], 4),
    i("jp", "c,nn", &[0xDA], 10),
    i("in", "a,(n)", &[0xDB], 11),
    c("call", "c,nn", &[0xDC], 17, 10),
    i("sbc", "a,n", &[0xDE], 7),
    i("rst", "$18", &[0xDF], 11),
    // 0xE0
    c("ret", "po", &[0xE0], 11, 5),
    i("pop", "hl", &[0xE1], 10),
    i("jp", "po,nn", &[0xE2], 10),
    i("ex", "(sp),hl", &[0xE3], 19),
    c("call", "po,nn", &[0xE4], 17, 10),
    i("push", "hl", &[0xE5], 11),
    i("and", "n", &[0xE6], 7),
    i("rst", "$20", &[0xE7], 11),
    c("ret", "pe", &[0xE8], 11, 5),
    i("jp", "(hl)", &[0xE9], 4),
    i("jp", "pe,nn", &[0xEA], 10),
    i("ex", "de,hl", &[0xEB], 4),
    c("call", "pe,nn", &[0xEC], 17, 10),
    i("xor", "n", &[0xEE], 7),
    i("rst", "$28", &[0xEF], 11),
    // 0xF0
    c("ret", "p", &[0xF0], 11, 5),
    i("pop", "af", &[0xF1], 10),
    i("jp", "p,nn", &[0xF2], 10),
    i("di", "", &[0xF3], 4),
    c("call", "p,nn", &[0xF4], 17, 10),
    i("push", "af", &[0xF5], 11),
    i("or", "n", &[0xF6], 7),
    i("rst", "$30", &[0xF7], 11),
    c("ret", "m", &[0xF8], 11, 5),
    i("ld", "sp,hl", &[0xF9], 6),
    i("jp", "m,nn", &[0xFA], 10),
    i("ei", "", &[0xFB], 4),
    c("call", "m,nn", &[0xFC], 17, 10),
    i("cp", "n", &[0xFE], 7),
    i("rst", "$38", &[0xFF], 11),
    // ---------------------------------------------------------------
    // The 0xCB space: the rotates and shifts, then the three bit
    // operations over the eight bit numbers. 0xCB 0x30 through 0x37
    // -- `sll`, sometimes written `sl1` -- is not in the manual and
    // is not here, so this is 248 of the 256 bytes.
    // ---------------------------------------------------------------
    // 0xCB 0x00
    i("rlc", "b", &[0xCB, 0x00], 8),
    i("rlc", "c", &[0xCB, 0x01], 8),
    i("rlc", "d", &[0xCB, 0x02], 8),
    i("rlc", "e", &[0xCB, 0x03], 8),
    i("rlc", "h", &[0xCB, 0x04], 8),
    i("rlc", "l", &[0xCB, 0x05], 8),
    i("rlc", "(hl)", &[0xCB, 0x06], 15),
    i("rlc", "a", &[0xCB, 0x07], 8),
    // 0xCB 0x08
    i("rrc", "b", &[0xCB, 0x08], 8),
    i("rrc", "c", &[0xCB, 0x09], 8),
    i("rrc", "d", &[0xCB, 0x0A], 8),
    i("rrc", "e", &[0xCB, 0x0B], 8),
    i("rrc", "h", &[0xCB, 0x0C], 8),
    i("rrc", "l", &[0xCB, 0x0D], 8),
    i("rrc", "(hl)", &[0xCB, 0x0E], 15),
    i("rrc", "a", &[0xCB, 0x0F], 8),
    // 0xCB 0x10
    i("rl", "b", &[0xCB, 0x10], 8),
    i("rl", "c", &[0xCB, 0x11], 8),
    i("rl", "d", &[0xCB, 0x12], 8),
    i("rl", "e", &[0xCB, 0x13], 8),
    i("rl", "h", &[0xCB, 0x14], 8),
    i("rl", "l", &[0xCB, 0x15], 8),
    i("rl", "(hl)", &[0xCB, 0x16], 15),
    i("rl", "a", &[0xCB, 0x17], 8),
    // 0xCB 0x18
    i("rr", "b", &[0xCB, 0x18], 8),
    i("rr", "c", &[0xCB, 0x19], 8),
    i("rr", "d", &[0xCB, 0x1A], 8),
    i("rr", "e", &[0xCB, 0x1B], 8),
    i("rr", "h", &[0xCB, 0x1C], 8),
    i("rr", "l", &[0xCB, 0x1D], 8),
    i("rr", "(hl)", &[0xCB, 0x1E], 15),
    i("rr", "a", &[0xCB, 0x1F], 8),
    // 0xCB 0x20
    i("sla", "b", &[0xCB, 0x20], 8),
    i("sla", "c", &[0xCB, 0x21], 8),
    i("sla", "d", &[0xCB, 0x22], 8),
    i("sla", "e", &[0xCB, 0x23], 8),
    i("sla", "h", &[0xCB, 0x24], 8),
    i("sla", "l", &[0xCB, 0x25], 8),
    i("sla", "(hl)", &[0xCB, 0x26], 15),
    i("sla", "a", &[0xCB, 0x27], 8),
    // 0xCB 0x28
    i("sra", "b", &[0xCB, 0x28], 8),
    i("sra", "c", &[0xCB, 0x29], 8),
    i("sra", "d", &[0xCB, 0x2A], 8),
    i("sra", "e", &[0xCB, 0x2B], 8),
    i("sra", "h", &[0xCB, 0x2C], 8),
    i("sra", "l", &[0xCB, 0x2D], 8),
    i("sra", "(hl)", &[0xCB, 0x2E], 15),
    i("sra", "a", &[0xCB, 0x2F], 8),
    // 0xCB 0x38
    i("srl", "b", &[0xCB, 0x38], 8),
    i("srl", "c", &[0xCB, 0x39], 8),
    i("srl", "d", &[0xCB, 0x3A], 8),
    i("srl", "e", &[0xCB, 0x3B], 8),
    i("srl", "h", &[0xCB, 0x3C], 8),
    i("srl", "l", &[0xCB, 0x3D], 8),
    i("srl", "(hl)", &[0xCB, 0x3E], 15),
    i("srl", "a", &[0xCB, 0x3F], 8),
    // 0xCB 0x40
    i("bit", "0,b", &[0xCB, 0x40], 8),
    i("bit", "0,c", &[0xCB, 0x41], 8),
    i("bit", "0,d", &[0xCB, 0x42], 8),
    i("bit", "0,e", &[0xCB, 0x43], 8),
    i("bit", "0,h", &[0xCB, 0x44], 8),
    i("bit", "0,l", &[0xCB, 0x45], 8),
    i("bit", "0,(hl)", &[0xCB, 0x46], 12),
    i("bit", "0,a", &[0xCB, 0x47], 8),
    // 0xCB 0x48
    i("bit", "1,b", &[0xCB, 0x48], 8),
    i("bit", "1,c", &[0xCB, 0x49], 8),
    i("bit", "1,d", &[0xCB, 0x4A], 8),
    i("bit", "1,e", &[0xCB, 0x4B], 8),
    i("bit", "1,h", &[0xCB, 0x4C], 8),
    i("bit", "1,l", &[0xCB, 0x4D], 8),
    i("bit", "1,(hl)", &[0xCB, 0x4E], 12),
    i("bit", "1,a", &[0xCB, 0x4F], 8),
    // 0xCB 0x50
    i("bit", "2,b", &[0xCB, 0x50], 8),
    i("bit", "2,c", &[0xCB, 0x51], 8),
    i("bit", "2,d", &[0xCB, 0x52], 8),
    i("bit", "2,e", &[0xCB, 0x53], 8),
    i("bit", "2,h", &[0xCB, 0x54], 8),
    i("bit", "2,l", &[0xCB, 0x55], 8),
    i("bit", "2,(hl)", &[0xCB, 0x56], 12),
    i("bit", "2,a", &[0xCB, 0x57], 8),
    // 0xCB 0x58
    i("bit", "3,b", &[0xCB, 0x58], 8),
    i("bit", "3,c", &[0xCB, 0x59], 8),
    i("bit", "3,d", &[0xCB, 0x5A], 8),
    i("bit", "3,e", &[0xCB, 0x5B], 8),
    i("bit", "3,h", &[0xCB, 0x5C], 8),
    i("bit", "3,l", &[0xCB, 0x5D], 8),
    i("bit", "3,(hl)", &[0xCB, 0x5E], 12),
    i("bit", "3,a", &[0xCB, 0x5F], 8),
    // 0xCB 0x60
    i("bit", "4,b", &[0xCB, 0x60], 8),
    i("bit", "4,c", &[0xCB, 0x61], 8),
    i("bit", "4,d", &[0xCB, 0x62], 8),
    i("bit", "4,e", &[0xCB, 0x63], 8),
    i("bit", "4,h", &[0xCB, 0x64], 8),
    i("bit", "4,l", &[0xCB, 0x65], 8),
    i("bit", "4,(hl)", &[0xCB, 0x66], 12),
    i("bit", "4,a", &[0xCB, 0x67], 8),
    // 0xCB 0x68
    i("bit", "5,b", &[0xCB, 0x68], 8),
    i("bit", "5,c", &[0xCB, 0x69], 8),
    i("bit", "5,d", &[0xCB, 0x6A], 8),
    i("bit", "5,e", &[0xCB, 0x6B], 8),
    i("bit", "5,h", &[0xCB, 0x6C], 8),
    i("bit", "5,l", &[0xCB, 0x6D], 8),
    i("bit", "5,(hl)", &[0xCB, 0x6E], 12),
    i("bit", "5,a", &[0xCB, 0x6F], 8),
    // 0xCB 0x70
    i("bit", "6,b", &[0xCB, 0x70], 8),
    i("bit", "6,c", &[0xCB, 0x71], 8),
    i("bit", "6,d", &[0xCB, 0x72], 8),
    i("bit", "6,e", &[0xCB, 0x73], 8),
    i("bit", "6,h", &[0xCB, 0x74], 8),
    i("bit", "6,l", &[0xCB, 0x75], 8),
    i("bit", "6,(hl)", &[0xCB, 0x76], 12),
    i("bit", "6,a", &[0xCB, 0x77], 8),
    // 0xCB 0x78
    i("bit", "7,b", &[0xCB, 0x78], 8),
    i("bit", "7,c", &[0xCB, 0x79], 8),
    i("bit", "7,d", &[0xCB, 0x7A], 8),
    i("bit", "7,e", &[0xCB, 0x7B], 8),
    i("bit", "7,h", &[0xCB, 0x7C], 8),
    i("bit", "7,l", &[0xCB, 0x7D], 8),
    i("bit", "7,(hl)", &[0xCB, 0x7E], 12),
    i("bit", "7,a", &[0xCB, 0x7F], 8),
    // 0xCB 0x80
    i("res", "0,b", &[0xCB, 0x80], 8),
    i("res", "0,c", &[0xCB, 0x81], 8),
    i("res", "0,d", &[0xCB, 0x82], 8),
    i("res", "0,e", &[0xCB, 0x83], 8),
    i("res", "0,h", &[0xCB, 0x84], 8),
    i("res", "0,l", &[0xCB, 0x85], 8),
    i("res", "0,(hl)", &[0xCB, 0x86], 15),
    i("res", "0,a", &[0xCB, 0x87], 8),
    // 0xCB 0x88
    i("res", "1,b", &[0xCB, 0x88], 8),
    i("res", "1,c", &[0xCB, 0x89], 8),
    i("res", "1,d", &[0xCB, 0x8A], 8),
    i("res", "1,e", &[0xCB, 0x8B], 8),
    i("res", "1,h", &[0xCB, 0x8C], 8),
    i("res", "1,l", &[0xCB, 0x8D], 8),
    i("res", "1,(hl)", &[0xCB, 0x8E], 15),
    i("res", "1,a", &[0xCB, 0x8F], 8),
    // 0xCB 0x90
    i("res", "2,b", &[0xCB, 0x90], 8),
    i("res", "2,c", &[0xCB, 0x91], 8),
    i("res", "2,d", &[0xCB, 0x92], 8),
    i("res", "2,e", &[0xCB, 0x93], 8),
    i("res", "2,h", &[0xCB, 0x94], 8),
    i("res", "2,l", &[0xCB, 0x95], 8),
    i("res", "2,(hl)", &[0xCB, 0x96], 15),
    i("res", "2,a", &[0xCB, 0x97], 8),
    // 0xCB 0x98
    i("res", "3,b", &[0xCB, 0x98], 8),
    i("res", "3,c", &[0xCB, 0x99], 8),
    i("res", "3,d", &[0xCB, 0x9A], 8),
    i("res", "3,e", &[0xCB, 0x9B], 8),
    i("res", "3,h", &[0xCB, 0x9C], 8),
    i("res", "3,l", &[0xCB, 0x9D], 8),
    i("res", "3,(hl)", &[0xCB, 0x9E], 15),
    i("res", "3,a", &[0xCB, 0x9F], 8),
    // 0xCB 0xA0
    i("res", "4,b", &[0xCB, 0xA0], 8),
    i("res", "4,c", &[0xCB, 0xA1], 8),
    i("res", "4,d", &[0xCB, 0xA2], 8),
    i("res", "4,e", &[0xCB, 0xA3], 8),
    i("res", "4,h", &[0xCB, 0xA4], 8),
    i("res", "4,l", &[0xCB, 0xA5], 8),
    i("res", "4,(hl)", &[0xCB, 0xA6], 15),
    i("res", "4,a", &[0xCB, 0xA7], 8),
    // 0xCB 0xA8
    i("res", "5,b", &[0xCB, 0xA8], 8),
    i("res", "5,c", &[0xCB, 0xA9], 8),
    i("res", "5,d", &[0xCB, 0xAA], 8),
    i("res", "5,e", &[0xCB, 0xAB], 8),
    i("res", "5,h", &[0xCB, 0xAC], 8),
    i("res", "5,l", &[0xCB, 0xAD], 8),
    i("res", "5,(hl)", &[0xCB, 0xAE], 15),
    i("res", "5,a", &[0xCB, 0xAF], 8),
    // 0xCB 0xB0
    i("res", "6,b", &[0xCB, 0xB0], 8),
    i("res", "6,c", &[0xCB, 0xB1], 8),
    i("res", "6,d", &[0xCB, 0xB2], 8),
    i("res", "6,e", &[0xCB, 0xB3], 8),
    i("res", "6,h", &[0xCB, 0xB4], 8),
    i("res", "6,l", &[0xCB, 0xB5], 8),
    i("res", "6,(hl)", &[0xCB, 0xB6], 15),
    i("res", "6,a", &[0xCB, 0xB7], 8),
    // 0xCB 0xB8
    i("res", "7,b", &[0xCB, 0xB8], 8),
    i("res", "7,c", &[0xCB, 0xB9], 8),
    i("res", "7,d", &[0xCB, 0xBA], 8),
    i("res", "7,e", &[0xCB, 0xBB], 8),
    i("res", "7,h", &[0xCB, 0xBC], 8),
    i("res", "7,l", &[0xCB, 0xBD], 8),
    i("res", "7,(hl)", &[0xCB, 0xBE], 15),
    i("res", "7,a", &[0xCB, 0xBF], 8),
    // 0xCB 0xC0
    i("set", "0,b", &[0xCB, 0xC0], 8),
    i("set", "0,c", &[0xCB, 0xC1], 8),
    i("set", "0,d", &[0xCB, 0xC2], 8),
    i("set", "0,e", &[0xCB, 0xC3], 8),
    i("set", "0,h", &[0xCB, 0xC4], 8),
    i("set", "0,l", &[0xCB, 0xC5], 8),
    i("set", "0,(hl)", &[0xCB, 0xC6], 15),
    i("set", "0,a", &[0xCB, 0xC7], 8),
    // 0xCB 0xC8
    i("set", "1,b", &[0xCB, 0xC8], 8),
    i("set", "1,c", &[0xCB, 0xC9], 8),
    i("set", "1,d", &[0xCB, 0xCA], 8),
    i("set", "1,e", &[0xCB, 0xCB], 8),
    i("set", "1,h", &[0xCB, 0xCC], 8),
    i("set", "1,l", &[0xCB, 0xCD], 8),
    i("set", "1,(hl)", &[0xCB, 0xCE], 15),
    i("set", "1,a", &[0xCB, 0xCF], 8),
    // 0xCB 0xD0
    i("set", "2,b", &[0xCB, 0xD0], 8),
    i("set", "2,c", &[0xCB, 0xD1], 8),
    i("set", "2,d", &[0xCB, 0xD2], 8),
    i("set", "2,e", &[0xCB, 0xD3], 8),
    i("set", "2,h", &[0xCB, 0xD4], 8),
    i("set", "2,l", &[0xCB, 0xD5], 8),
    i("set", "2,(hl)", &[0xCB, 0xD6], 15),
    i("set", "2,a", &[0xCB, 0xD7], 8),
    // 0xCB 0xD8
    i("set", "3,b", &[0xCB, 0xD8], 8),
    i("set", "3,c", &[0xCB, 0xD9], 8),
    i("set", "3,d", &[0xCB, 0xDA], 8),
    i("set", "3,e", &[0xCB, 0xDB], 8),
    i("set", "3,h", &[0xCB, 0xDC], 8),
    i("set", "3,l", &[0xCB, 0xDD], 8),
    i("set", "3,(hl)", &[0xCB, 0xDE], 15),
    i("set", "3,a", &[0xCB, 0xDF], 8),
    // 0xCB 0xE0
    i("set", "4,b", &[0xCB, 0xE0], 8),
    i("set", "4,c", &[0xCB, 0xE1], 8),
    i("set", "4,d", &[0xCB, 0xE2], 8),
    i("set", "4,e", &[0xCB, 0xE3], 8),
    i("set", "4,h", &[0xCB, 0xE4], 8),
    i("set", "4,l", &[0xCB, 0xE5], 8),
    i("set", "4,(hl)", &[0xCB, 0xE6], 15),
    i("set", "4,a", &[0xCB, 0xE7], 8),
    // 0xCB 0xE8
    i("set", "5,b", &[0xCB, 0xE8], 8),
    i("set", "5,c", &[0xCB, 0xE9], 8),
    i("set", "5,d", &[0xCB, 0xEA], 8),
    i("set", "5,e", &[0xCB, 0xEB], 8),
    i("set", "5,h", &[0xCB, 0xEC], 8),
    i("set", "5,l", &[0xCB, 0xED], 8),
    i("set", "5,(hl)", &[0xCB, 0xEE], 15),
    i("set", "5,a", &[0xCB, 0xEF], 8),
    // 0xCB 0xF0
    i("set", "6,b", &[0xCB, 0xF0], 8),
    i("set", "6,c", &[0xCB, 0xF1], 8),
    i("set", "6,d", &[0xCB, 0xF2], 8),
    i("set", "6,e", &[0xCB, 0xF3], 8),
    i("set", "6,h", &[0xCB, 0xF4], 8),
    i("set", "6,l", &[0xCB, 0xF5], 8),
    i("set", "6,(hl)", &[0xCB, 0xF6], 15),
    i("set", "6,a", &[0xCB, 0xF7], 8),
    // 0xCB 0xF8
    i("set", "7,b", &[0xCB, 0xF8], 8),
    i("set", "7,c", &[0xCB, 0xF9], 8),
    i("set", "7,d", &[0xCB, 0xFA], 8),
    i("set", "7,e", &[0xCB, 0xFB], 8),
    i("set", "7,h", &[0xCB, 0xFC], 8),
    i("set", "7,l", &[0xCB, 0xFD], 8),
    i("set", "7,(hl)", &[0xCB, 0xFE], 15),
    i("set", "7,a", &[0xCB, 0xFF], 8),
    // ---------------------------------------------------------------
    // The 0xED space, which is sparse: the 58 encodings the manual
    // documents and nothing else.
    // ---------------------------------------------------------------
    // 0xED 0x40
    i("in", "b,(c)", &[0xED, 0x40], 12),
    i("out", "(c),b", &[0xED, 0x41], 12),
    i("sbc", "hl,bc", &[0xED, 0x42], 15),
    i("ld", "(nn),bc", &[0xED, 0x43], 20),
    i("neg", "", &[0xED, 0x44], 8),
    i("retn", "", &[0xED, 0x45], 14),
    i("im", "0", &[0xED, 0x46], 8),
    i("ld", "i,a", &[0xED, 0x47], 9),
    i("in", "c,(c)", &[0xED, 0x48], 12),
    i("out", "(c),c", &[0xED, 0x49], 12),
    i("adc", "hl,bc", &[0xED, 0x4A], 15),
    i("ld", "bc,(nn)", &[0xED, 0x4B], 20),
    i("reti", "", &[0xED, 0x4D], 14),
    i("ld", "r,a", &[0xED, 0x4F], 9),
    // 0xED 0x50
    i("in", "d,(c)", &[0xED, 0x50], 12),
    i("out", "(c),d", &[0xED, 0x51], 12),
    i("sbc", "hl,de", &[0xED, 0x52], 15),
    i("ld", "(nn),de", &[0xED, 0x53], 20),
    i("im", "1", &[0xED, 0x56], 8),
    i("ld", "a,i", &[0xED, 0x57], 9),
    i("in", "e,(c)", &[0xED, 0x58], 12),
    i("out", "(c),e", &[0xED, 0x59], 12),
    i("adc", "hl,de", &[0xED, 0x5A], 15),
    i("ld", "de,(nn)", &[0xED, 0x5B], 20),
    i("im", "2", &[0xED, 0x5E], 8),
    i("ld", "a,r", &[0xED, 0x5F], 9),
    // 0xED 0x60
    i("in", "h,(c)", &[0xED, 0x60], 12),
    i("out", "(c),h", &[0xED, 0x61], 12),
    i("sbc", "hl,hl", &[0xED, 0x62], 15),
    i("ld", "(nn),hl", &[0xED, 0x63], 20), // shadowed; see above
    i("rrd", "", &[0xED, 0x67], 18),
    i("in", "l,(c)", &[0xED, 0x68], 12),
    i("out", "(c),l", &[0xED, 0x69], 12),
    i("adc", "hl,hl", &[0xED, 0x6A], 15),
    i("ld", "hl,(nn)", &[0xED, 0x6B], 20), // shadowed; see above
    i("rld", "", &[0xED, 0x6F], 18),
    // 0xED 0x70
    i("sbc", "hl,sp", &[0xED, 0x72], 15),
    i("ld", "(nn),sp", &[0xED, 0x73], 20),
    i("in", "a,(c)", &[0xED, 0x78], 12),
    i("out", "(c),a", &[0xED, 0x79], 12),
    i("adc", "hl,sp", &[0xED, 0x7A], 15),
    i("ld", "sp,(nn)", &[0xED, 0x7B], 20),
    // 0xED 0xA0
    i("ldi", "", &[0xED, 0xA0], 16),
    i("cpi", "", &[0xED, 0xA1], 16),
    i("ini", "", &[0xED, 0xA2], 16),
    i("outi", "", &[0xED, 0xA3], 16),
    i("ldd", "", &[0xED, 0xA8], 16),
    i("cpd", "", &[0xED, 0xA9], 16),
    i("ind", "", &[0xED, 0xAA], 16),
    i("outd", "", &[0xED, 0xAB], 16),
    // 0xED 0xB0
    c("ldir", "", &[0xED, 0xB0], 21, 16),
    c("cpir", "", &[0xED, 0xB1], 21, 16),
    c("inir", "", &[0xED, 0xB2], 21, 16),
    c("otir", "", &[0xED, 0xB3], 21, 16),
    c("lddr", "", &[0xED, 0xB8], 21, 16),
    c("cpdr", "", &[0xED, 0xB9], 21, 16),
    c("indr", "", &[0xED, 0xBA], 21, 16),
    c("otdr", "", &[0xED, 0xBB], 21, 16),
    // ---------------------------------------------------------------
    // The 0xDD and 0xFD spaces: every 0xDD encoding with IX and the
    // same 39 again with 0xFD and IY, each displaced form carrying
    // the signed displacement byte after the opcode.
    // ---------------------------------------------------------------
    // 0xDD 0x00
    i("add", "ix,bc", &[0xDD, 0x09], 15),
    // 0xDD 0x10
    i("add", "ix,de", &[0xDD, 0x19], 15),
    // 0xDD 0x20
    i("ld", "ix,nn", &[0xDD, 0x21], 14),
    i("ld", "(nn),ix", &[0xDD, 0x22], 20),
    i("inc", "ix", &[0xDD, 0x23], 10),
    i("add", "ix,ix", &[0xDD, 0x29], 15),
    i("ld", "ix,(nn)", &[0xDD, 0x2A], 20),
    i("dec", "ix", &[0xDD, 0x2B], 10),
    // 0xDD 0x30
    i("inc", "(ix+d)", &[0xDD, 0x34], 23),
    i("dec", "(ix+d)", &[0xDD, 0x35], 23),
    i("ld", "(ix+d),n", &[0xDD, 0x36], 19),
    i("add", "ix,sp", &[0xDD, 0x39], 15),
    // 0xDD 0x40
    i("ld", "b,(ix+d)", &[0xDD, 0x46], 19),
    i("ld", "c,(ix+d)", &[0xDD, 0x4E], 19),
    // 0xDD 0x50
    i("ld", "d,(ix+d)", &[0xDD, 0x56], 19),
    i("ld", "e,(ix+d)", &[0xDD, 0x5E], 19),
    // 0xDD 0x60
    i("ld", "h,(ix+d)", &[0xDD, 0x66], 19),
    i("ld", "l,(ix+d)", &[0xDD, 0x6E], 19),
    // 0xDD 0x70
    i("ld", "(ix+d),b", &[0xDD, 0x70], 19),
    i("ld", "(ix+d),c", &[0xDD, 0x71], 19),
    i("ld", "(ix+d),d", &[0xDD, 0x72], 19),
    i("ld", "(ix+d),e", &[0xDD, 0x73], 19),
    i("ld", "(ix+d),h", &[0xDD, 0x74], 19),
    i("ld", "(ix+d),l", &[0xDD, 0x75], 19),
    i("ld", "(ix+d),a", &[0xDD, 0x77], 19),
    i("ld", "a,(ix+d)", &[0xDD, 0x7E], 19),
    // 0xDD 0x80
    i("add", "a,(ix+d)", &[0xDD, 0x86], 19),
    i("adc", "a,(ix+d)", &[0xDD, 0x8E], 19),
    // 0xDD 0x90
    i("sub", "(ix+d)", &[0xDD, 0x96], 19),
    i("sbc", "a,(ix+d)", &[0xDD, 0x9E], 19),
    // 0xDD 0xA0
    i("and", "(ix+d)", &[0xDD, 0xA6], 19),
    i("xor", "(ix+d)", &[0xDD, 0xAE], 19),
    // 0xDD 0xB0
    i("or", "(ix+d)", &[0xDD, 0xB6], 19),
    i("cp", "(ix+d)", &[0xDD, 0xBE], 19),
    // 0xDD 0xE0
    i("pop", "ix", &[0xDD, 0xE1], 14),
    i("ex", "(sp),ix", &[0xDD, 0xE3], 23),
    i("push", "ix", &[0xDD, 0xE5], 15),
    i("jp", "(ix)", &[0xDD, 0xE9], 8),
    // 0xDD 0xF0
    i("ld", "sp,ix", &[0xDD, 0xF9], 10),
    // 0xFD 0x00
    i("add", "iy,bc", &[0xFD, 0x09], 15),
    // 0xFD 0x10
    i("add", "iy,de", &[0xFD, 0x19], 15),
    // 0xFD 0x20
    i("ld", "iy,nn", &[0xFD, 0x21], 14),
    i("ld", "(nn),iy", &[0xFD, 0x22], 20),
    i("inc", "iy", &[0xFD, 0x23], 10),
    i("add", "iy,iy", &[0xFD, 0x29], 15),
    i("ld", "iy,(nn)", &[0xFD, 0x2A], 20),
    i("dec", "iy", &[0xFD, 0x2B], 10),
    // 0xFD 0x30
    i("inc", "(iy+d)", &[0xFD, 0x34], 23),
    i("dec", "(iy+d)", &[0xFD, 0x35], 23),
    i("ld", "(iy+d),n", &[0xFD, 0x36], 19),
    i("add", "iy,sp", &[0xFD, 0x39], 15),
    // 0xFD 0x40
    i("ld", "b,(iy+d)", &[0xFD, 0x46], 19),
    i("ld", "c,(iy+d)", &[0xFD, 0x4E], 19),
    // 0xFD 0x50
    i("ld", "d,(iy+d)", &[0xFD, 0x56], 19),
    i("ld", "e,(iy+d)", &[0xFD, 0x5E], 19),
    // 0xFD 0x60
    i("ld", "h,(iy+d)", &[0xFD, 0x66], 19),
    i("ld", "l,(iy+d)", &[0xFD, 0x6E], 19),
    // 0xFD 0x70
    i("ld", "(iy+d),b", &[0xFD, 0x70], 19),
    i("ld", "(iy+d),c", &[0xFD, 0x71], 19),
    i("ld", "(iy+d),d", &[0xFD, 0x72], 19),
    i("ld", "(iy+d),e", &[0xFD, 0x73], 19),
    i("ld", "(iy+d),h", &[0xFD, 0x74], 19),
    i("ld", "(iy+d),l", &[0xFD, 0x75], 19),
    i("ld", "(iy+d),a", &[0xFD, 0x77], 19),
    i("ld", "a,(iy+d)", &[0xFD, 0x7E], 19),
    // 0xFD 0x80
    i("add", "a,(iy+d)", &[0xFD, 0x86], 19),
    i("adc", "a,(iy+d)", &[0xFD, 0x8E], 19),
    // 0xFD 0x90
    i("sub", "(iy+d)", &[0xFD, 0x96], 19),
    i("sbc", "a,(iy+d)", &[0xFD, 0x9E], 19),
    // 0xFD 0xA0
    i("and", "(iy+d)", &[0xFD, 0xA6], 19),
    i("xor", "(iy+d)", &[0xFD, 0xAE], 19),
    // 0xFD 0xB0
    i("or", "(iy+d)", &[0xFD, 0xB6], 19),
    i("cp", "(iy+d)", &[0xFD, 0xBE], 19),
    // 0xFD 0xE0
    i("pop", "iy", &[0xFD, 0xE1], 14),
    i("ex", "(sp),iy", &[0xFD, 0xE3], 23),
    i("push", "iy", &[0xFD, 0xE5], 15),
    i("jp", "(iy)", &[0xFD, 0xE9], 8),
    // 0xFD 0xF0
    i("ld", "sp,iy", &[0xFD, 0xF9], 10),
    // ---------------------------------------------------------------
    // The 0xDD 0xCB and 0xFD 0xCB spaces. The displacement comes
    // between the 0xCB and the opcode byte, which is what the
    // encoding tables print and what `encode` writes.
    // ---------------------------------------------------------------
    // 0xDD 0xCB 0x06
    i("rlc", "(ix+d)", &[0xDD, 0xCB, 0x06], 23),
    // 0xDD 0xCB 0x0E
    i("rrc", "(ix+d)", &[0xDD, 0xCB, 0x0E], 23),
    // 0xDD 0xCB 0x16
    i("rl", "(ix+d)", &[0xDD, 0xCB, 0x16], 23),
    // 0xDD 0xCB 0x1E
    i("rr", "(ix+d)", &[0xDD, 0xCB, 0x1E], 23),
    // 0xDD 0xCB 0x26
    i("sla", "(ix+d)", &[0xDD, 0xCB, 0x26], 23),
    // 0xDD 0xCB 0x2E
    i("sra", "(ix+d)", &[0xDD, 0xCB, 0x2E], 23),
    // 0xDD 0xCB 0x3E
    i("srl", "(ix+d)", &[0xDD, 0xCB, 0x3E], 23),
    // 0xDD 0xCB 0x46
    i("bit", "0,(ix+d)", &[0xDD, 0xCB, 0x46], 20),
    i("bit", "1,(ix+d)", &[0xDD, 0xCB, 0x4E], 20),
    i("bit", "2,(ix+d)", &[0xDD, 0xCB, 0x56], 20),
    i("bit", "3,(ix+d)", &[0xDD, 0xCB, 0x5E], 20),
    i("bit", "4,(ix+d)", &[0xDD, 0xCB, 0x66], 20),
    i("bit", "5,(ix+d)", &[0xDD, 0xCB, 0x6E], 20),
    i("bit", "6,(ix+d)", &[0xDD, 0xCB, 0x76], 20),
    i("bit", "7,(ix+d)", &[0xDD, 0xCB, 0x7E], 20),
    // 0xDD 0xCB 0x86
    i("res", "0,(ix+d)", &[0xDD, 0xCB, 0x86], 23),
    i("res", "1,(ix+d)", &[0xDD, 0xCB, 0x8E], 23),
    i("res", "2,(ix+d)", &[0xDD, 0xCB, 0x96], 23),
    i("res", "3,(ix+d)", &[0xDD, 0xCB, 0x9E], 23),
    i("res", "4,(ix+d)", &[0xDD, 0xCB, 0xA6], 23),
    i("res", "5,(ix+d)", &[0xDD, 0xCB, 0xAE], 23),
    i("res", "6,(ix+d)", &[0xDD, 0xCB, 0xB6], 23),
    i("res", "7,(ix+d)", &[0xDD, 0xCB, 0xBE], 23),
    // 0xDD 0xCB 0xC6
    i("set", "0,(ix+d)", &[0xDD, 0xCB, 0xC6], 23),
    i("set", "1,(ix+d)", &[0xDD, 0xCB, 0xCE], 23),
    i("set", "2,(ix+d)", &[0xDD, 0xCB, 0xD6], 23),
    i("set", "3,(ix+d)", &[0xDD, 0xCB, 0xDE], 23),
    i("set", "4,(ix+d)", &[0xDD, 0xCB, 0xE6], 23),
    i("set", "5,(ix+d)", &[0xDD, 0xCB, 0xEE], 23),
    i("set", "6,(ix+d)", &[0xDD, 0xCB, 0xF6], 23),
    i("set", "7,(ix+d)", &[0xDD, 0xCB, 0xFE], 23),
    // 0xFD 0xCB 0x06
    i("rlc", "(iy+d)", &[0xFD, 0xCB, 0x06], 23),
    // 0xFD 0xCB 0x0E
    i("rrc", "(iy+d)", &[0xFD, 0xCB, 0x0E], 23),
    // 0xFD 0xCB 0x16
    i("rl", "(iy+d)", &[0xFD, 0xCB, 0x16], 23),
    // 0xFD 0xCB 0x1E
    i("rr", "(iy+d)", &[0xFD, 0xCB, 0x1E], 23),
    // 0xFD 0xCB 0x26
    i("sla", "(iy+d)", &[0xFD, 0xCB, 0x26], 23),
    // 0xFD 0xCB 0x2E
    i("sra", "(iy+d)", &[0xFD, 0xCB, 0x2E], 23),
    // 0xFD 0xCB 0x3E
    i("srl", "(iy+d)", &[0xFD, 0xCB, 0x3E], 23),
    // 0xFD 0xCB 0x46
    i("bit", "0,(iy+d)", &[0xFD, 0xCB, 0x46], 20),
    i("bit", "1,(iy+d)", &[0xFD, 0xCB, 0x4E], 20),
    i("bit", "2,(iy+d)", &[0xFD, 0xCB, 0x56], 20),
    i("bit", "3,(iy+d)", &[0xFD, 0xCB, 0x5E], 20),
    i("bit", "4,(iy+d)", &[0xFD, 0xCB, 0x66], 20),
    i("bit", "5,(iy+d)", &[0xFD, 0xCB, 0x6E], 20),
    i("bit", "6,(iy+d)", &[0xFD, 0xCB, 0x76], 20),
    i("bit", "7,(iy+d)", &[0xFD, 0xCB, 0x7E], 20),
    // 0xFD 0xCB 0x86
    i("res", "0,(iy+d)", &[0xFD, 0xCB, 0x86], 23),
    i("res", "1,(iy+d)", &[0xFD, 0xCB, 0x8E], 23),
    i("res", "2,(iy+d)", &[0xFD, 0xCB, 0x96], 23),
    i("res", "3,(iy+d)", &[0xFD, 0xCB, 0x9E], 23),
    i("res", "4,(iy+d)", &[0xFD, 0xCB, 0xA6], 23),
    i("res", "5,(iy+d)", &[0xFD, 0xCB, 0xAE], 23),
    i("res", "6,(iy+d)", &[0xFD, 0xCB, 0xB6], 23),
    i("res", "7,(iy+d)", &[0xFD, 0xCB, 0xBE], 23),
    // 0xFD 0xCB 0xC6
    i("set", "0,(iy+d)", &[0xFD, 0xCB, 0xC6], 23),
    i("set", "1,(iy+d)", &[0xFD, 0xCB, 0xCE], 23),
    i("set", "2,(iy+d)", &[0xFD, 0xCB, 0xD6], 23),
    i("set", "3,(iy+d)", &[0xFD, 0xCB, 0xDE], 23),
    i("set", "4,(iy+d)", &[0xFD, 0xCB, 0xE6], 23),
    i("set", "5,(iy+d)", &[0xFD, 0xCB, 0xEE], 23),
    i("set", "6,(iy+d)", &[0xFD, 0xCB, 0xF6], 23),
    i("set", "7,(iy+d)", &[0xFD, 0xCB, 0xFE], 23),
];

/// The encoding of one mnemonic in one operand shape, or `None` if it has
/// none.
///
/// The shape is matched literally, so it has to be spelled the way
/// [`TABLE`] spells it: `find("ld", "a,(hl)")`, not `find("ld", "a, (hl)")`.
pub(crate) fn find(name: &str, shape: &str) -> Option<&'static Insn> {
    TABLE
        .iter()
        .find(|entry| entry.name == name && entry.shape == shape)
}

/// Every shape a mnemonic has, which is what an error message offers when
/// the one that was written is not among them.
pub(crate) fn shapes(name: &str) -> Vec<&'static str> {
    TABLE
        .iter()
        .filter(|entry| entry.name == name)
        .map(|entry| entry.shape)
        .collect()
}

/// The bytes of one instruction, with its literal field filled in.
///
/// `disp` is the displacement of an `(ix+d)` or `(iy+d)` operand and is
/// ignored by a shape that has none; `operand` is the immediate, the
/// address or the port number, and for a `label` shape it is the signed
/// displacement the encoding holds rather than the target, because
/// nothing here knows where the instruction is.
fn encode(insn: &Insn, disp: i32, operand: u16) -> Vec<u8> {
    let low = u8::try_from(operand & 0xFF).expect("a byte");
    let high = u8::try_from(operand >> 8).expect("a byte");
    let displacement = u8::try_from(disp & 0xFF).expect("a byte");
    let mut out = insn.code.to_vec();
    match insn.field() {
        Field::None => {}
        Field::Byte | Field::Port | Field::Rel => out.push(low),
        Field::Word => {
            out.push(low);
            out.push(high);
        }
        Field::Disp => place_displacement(&mut out, displacement),
        Field::DispByte => {
            place_displacement(&mut out, displacement);
            out.push(low);
        }
    }
    out
}

/// Puts the displacement byte where the published encoding puts it: after
/// the opcode for a plain `(ix+d)` form, but between the `cb` and the
/// opcode byte for a `dd cb` bit operation, which is rule 7 of the module
/// documentation. The three-byte fixed encodings are exactly the `dd cb`
/// and `fd cb` ones.
fn place_displacement(out: &mut Vec<u8>, displacement: u8) {
    if out.len() == 3 && out[1] == 0xCB {
        out.insert(2, displacement);
    } else {
        out.push(displacement);
    }
}

/// The bytes of one instruction, for a test that builds a program by hand
/// rather than from source.
///
/// `operand` means what it means to [`encode`]. A shape that carries a
/// displacement is a panic here, naming [`bytes_indexed`], because the
/// displacement would otherwise go in silently as a zero.
pub(crate) fn bytes(name: &str, shape: &str, operand: u16) -> Vec<u8> {
    let insn = find(name, shape)
        .unwrap_or_else(|| panic!("`{name} {shape}` is not a documented encoding"));
    assert!(
        !matches!(insn.field(), Field::Disp | Field::DispByte),
        "`{name} {shape}` carries a displacement; use `bytes_indexed`"
    );
    encode(insn, 0, operand)
}

/// The bytes of one instruction whose operand is `(ix+d)` or `(iy+d)`.
pub(crate) fn bytes_indexed(name: &str, shape: &str, disp: i8, operand: u16) -> Vec<u8> {
    let insn = find(name, shape)
        .unwrap_or_else(|| panic!("`{name} {shape}` is not a documented encoding"));
    assert!(
        matches!(insn.field(), Field::Disp | Field::DispByte),
        "`{name} {shape}` carries no displacement; use `bytes`"
    );
    encode(insn, i32::from(disp), operand)
}

// ---------------------------------------------------------------------------
// Source text
// ---------------------------------------------------------------------------

/// An assembled program: the bytes it places, and what its labels mean.
pub(crate) struct Image {
    /// Byte by byte, so a program with several `.org`s stays sparse.
    pub(crate) bytes: BTreeMap<u16, u8>,
    /// Every label and `.equ` name.
    pub(crate) labels: BTreeMap<String, u16>,
}

impl Image {
    /// The address a label names, or a panic saying which one is missing.
    pub(crate) fn label(&self, name: &str) -> u16 {
        *self
            .labels
            .get(name)
            .unwrap_or_else(|| panic!("the program has no label `{name}`"))
    }

    /// Whether a label exists.
    pub(crate) fn has(&self, name: &str) -> bool {
        self.labels.contains_key(name)
    }

    /// The bytes from `addr` on, for comparing against what a test
    /// expects. A hole left by an `.org` or a `.ds` stops it.
    pub(crate) fn run(&self, addr: u16) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = addr;
        while let Some(byte) = self.bytes.get(&at) {
            out.push(*byte);
            at = match at.checked_add(1) {
                Some(next) => next,
                None => break,
            };
        }
        out
    }
}

/// Every operand token that is a name rather than a literal: the
/// registers, the register pairs, the condition codes and flags, the
/// register indirections, and `af'`. A label may not be one of these,
/// which is rule 3 of the module documentation.
pub(crate) const NAMES: &[&str] = &[
    "a", "af", "af'", "b", "bc", "c", "d", "de", "e", "h", "hl", "i", "ix", "iy", "l", "m", "nc",
    "nz", "p", "pe", "po", "r", "sp", "z", "(bc)", "(c)", "(de)", "(hl)", "(ix)", "(iy)", "(sp)",
];

/// A number: `$` or `0x` hexadecimal, `%` binary, or decimal, optionally
/// negative, with `_` allowed between digits.
fn number(text: &str) -> Option<i64> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let digits = digits.replace('_', "");
    let value = if let Some(hex) = digits.strip_prefix('$') {
        i64::from_str_radix(hex, 16).ok()?
    } else if let Some(hex) = digits.strip_prefix("0x") {
        i64::from_str_radix(hex, 16).ok()?
    } else if let Some(bin) = digits.strip_prefix('%') {
        i64::from_str_radix(bin, 2).ok()?
    } else if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        digits.parse().ok()?
    } else {
        return None;
    };
    Some(if negative { -value } else { value })
}

/// How one operand was written, before an encoding is chosen for it.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Arg {
    /// One of [`NAMES`], folded to lower case and with its spaces taken
    /// out, so `ld A, (HL)` is `ld a,(hl)`.
    Name(String),
    /// `(ix+d)` or `(iy-d)`: which index register, and the displacement
    /// as it was written, with its sign.
    Indexed(&'static str, String),
    /// A parenthesised expression: `($1234)`, `($fe)`, `(vector)`.
    Mem(String),
    /// A bare expression: `$12`, `3`, `loop`.
    Expr(String),
}

/// Splits one operand into its shape and the expression inside it.
fn argument(text: &str) -> Result<Arg, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("an operand is missing".to_owned());
    }
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if NAMES.contains(&compact.as_str()) {
        return Ok(Arg::Name(compact));
    }
    let Some(inner) = text.strip_prefix('(') else {
        return Ok(Arg::Expr(text.to_owned()));
    };
    let Some(inner) = inner.strip_suffix(')') else {
        return Err(format!("`{text}` has no closing parenthesis"));
    };
    let inner = inner.trim();
    // `(ix+5)` and `(iy-2)`, but not a symbol that merely starts with
    // those two letters: `(ixtab)` is an address.
    for register in ["ix", "iy"] {
        if !inner
            .get(..2)
            .is_some_and(|head| head.eq_ignore_ascii_case(register))
        {
            continue;
        }
        let rest = inner[2..].trim();
        if let Some(plus) = rest.strip_prefix('+') {
            return Ok(Arg::Indexed(register, plus.trim().to_owned()));
        }
        if let Some(minus) = rest.strip_prefix('-') {
            return Ok(Arg::Indexed(register, format!("-{}", minus.trim())));
        }
    }
    Ok(Arg::Mem(inner.to_owned()))
}

/// The operands of one statement, in the order they were written.
fn arguments(text: &str) -> Result<Vec<Arg>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    items(text).into_iter().map(argument).collect()
}

/// What every name means once the first pass has placed everything.
///
/// Unlike the 6502 assembler's, this needs nothing but the value: the
/// Z80 has no zero page, so how a literal was written never decides
/// which encoding it selects.
struct Symbols {
    names: BTreeMap<String, u16>,
}

impl Symbols {
    /// A literal or a defined name.
    ///
    /// `<` and `>` in front take the low and the high byte of what
    /// follows, which is how a program puts half of a label's address
    /// into an eight-bit register: `ld a,<message` then `ld b,>message`.
    fn value(&self, text: &str) -> Result<u16, String> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix('<') {
            return self.value(rest).map(|value| value & 0xFF);
        }
        if let Some(rest) = text.strip_prefix('>') {
            return self.value(rest).map(|value| value >> 8);
        }
        if let Some(value) = number(text) {
            return Ok(u16::try_from(value.rem_euclid(0x1_0000)).expect("sixteen bits"));
        }
        self.names
            .get(text)
            .copied()
            .ok_or_else(|| format!("`{text}` is neither a number nor a defined name"))
    }

    /// The same, as a signed number, which is what a displacement is.
    fn signed(&self, text: &str) -> Result<i32, String> {
        let text = text.trim();
        match number(text) {
            Some(value) => i32::try_from(value).map_err(|_| format!("`{text}` is far too large")),
            None => self.value(text).map(i32::from),
        }
    }

    fn define(&mut self, name: &str, value: u16) -> Result<(), String> {
        if NAMES.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(format!("`{name}` is a register name and cannot be a label"));
        }
        if TABLE.iter().any(|entry| entry.name == name) {
            return Err(format!("`{name}` is a mnemonic and cannot be a label"));
        }
        match self.names.insert(name.to_owned(), value) {
            None => Ok(()),
            Some(_) => Err(format!("`{name}` is defined twice")),
        }
    }
}

/// What matching an instruction against a shape filled in, as text: the
/// values wait until the second pass, because a branch to a label that
/// has not been placed yet still has to have a length in the first.
#[derive(Clone, Default)]
struct Fill {
    /// The displacement of an `(ix+d)` operand, as it was written.
    disp: Option<String>,
    /// The immediate, address, port number or branch target, as it was
    /// written.
    value: Option<String>,
}

/// Whether the operands were written the way `shape` says, and if so what
/// they fill in.
///
/// `symbols` is here only for rule 5: a shape token that is a number is
/// matched against the operand's *value*, so `rst 56` and `rst $38` are
/// one instruction, and an operand that cannot be evaluated yet cannot
/// match such a token.
fn match_shape(shape: &str, args: &[Arg], symbols: &Symbols) -> Option<Fill> {
    let mut fill = Fill::default();
    let mut wanted = tokens(shape);
    for arg in args {
        let token = wanted.next()?;
        match (token, arg) {
            ("n" | "nn" | "label", Arg::Expr(text)) | ("(n)" | "(nn)", Arg::Mem(text)) => {
                fill.value = Some(text.clone());
            }
            ("(ix+d)", Arg::Indexed("ix", text)) | ("(iy+d)", Arg::Indexed("iy", text)) => {
                fill.disp = Some(text.clone());
            }
            (_, Arg::Expr(text)) if number(token).is_some() => {
                let constant = number(token).expect("just checked");
                let written = symbols.value(text).ok()?;
                if i64::from(written) != constant {
                    return None;
                }
            }
            (_, Arg::Name(name)) if token == name => {}
            _ => return None,
        }
    }
    wanted.next().is_none().then_some(fill)
}

/// The encoding one statement selects, and what its operands fill in.
fn select(name: &str, args: &[Arg], symbols: &Symbols) -> Result<(&'static Insn, Fill), String> {
    for entry in TABLE.iter().filter(|entry| entry.name == name) {
        if let Some(fill) = match_shape(entry.shape, args, symbols) {
            return Ok((entry, fill));
        }
    }
    let shapes = shapes(name);
    if shapes.is_empty() {
        return Err(format!(
            "`{name}` is not an instruction this assembler knows"
        ));
    }
    Err(format!(
        "`{name}` has no form taking {} operand{}; it has {}: {}",
        args.len(),
        if args.len() == 1 { "" } else { "s" },
        shapes.len(),
        shapes.join(" | ")
    ))
}

/// One statement, once its labels are off.
struct Statement<'s> {
    line: usize,
    addr: u16,
    /// The length the first pass gave it, which the second pass checks
    /// it still has.
    size: u16,
    op: String,
    rest: &'s str,
}

/// A line without its comment: everything before the first `;` that is
/// not inside a string.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (at, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ';' if !quoted => return &line[..at],
            _ => {}
        }
    }
    line
}

/// A comma-separated list, with commas inside a quoted string left
/// alone, so `.db "a,b", 0` is two items.
fn items(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (at, c) in text.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.push(&text[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

/// The bytes of a string literal, with the escapes `\n`, `\r`, `\t`,
/// `\0`, `\\` and `\"`.
fn string_literal(text: &str) -> Result<Vec<u8>, String> {
    let inner = text
        .trim()
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .ok_or_else(|| format!("`{text}` is not a quoted string"))?;
    let mut out = Vec::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        let c = if c == '\\' {
            match chars.next() {
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                Some('0') => '\0',
                Some('\\') => '\\',
                Some('"') => '"',
                other => return Err(format!("unknown escape `\\{}`", other.unwrap_or(' '))),
            }
        } else {
            c
        };
        let byte = u8::try_from(u32::from(c))
            .ok()
            .filter(u8::is_ascii)
            .ok_or_else(|| format!("`{c}` is not ASCII"))?;
        out.push(byte);
    }
    Ok(out)
}

/// Whether a `.db` item is a string rather than an expression.
fn is_string(item: &str) -> bool {
    item.trim().starts_with('"')
}

/// How many bytes a `.db` list places.
fn db_size(rest: &str) -> Result<u16, String> {
    let mut size = 0usize;
    for item in items(rest) {
        size += if is_string(item) {
            string_literal(item)?.len()
        } else {
            1
        };
    }
    u16::try_from(size).map_err(|_| "the list is too long".to_owned())
}

/// Whether `name` can be a label.
fn is_label(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Assembles Z80 source text.
///
/// One statement per line, `;` comments, any number of `label:`
/// prefixes, and the directives
///
/// - `.org $nnnn` places what follows at an address, starting from
///   `$0000`, which is where a Z80 fetches its first instruction;
/// - `.equ name, value` defines a constant;
/// - `.db a, "text", …` places bytes, a quoted string placing its own
///   without a terminator;
/// - `.dw a, b, …` places little-endian words;
/// - `.ds n` skips `n` bytes, leaving a hole.
///
/// An error names the line it is on. Nothing in here panics on bad
/// input: a malformed program is an `Err` with a reason.
pub(crate) fn assemble(source: &str) -> Result<Image, String> {
    // Pass 1: place every statement and define every name.
    let mut symbols = Symbols {
        names: BTreeMap::new(),
    };
    let mut statements = Vec::new();
    let mut addr: u16 = 0x0000;
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let at = |message: String| format!("line {line}: {message}");
        let mut text = strip_comment(raw).trim();
        while let Some((name, rest)) = text.split_once(':') {
            if !is_label(name.trim()) {
                break;
            }
            symbols.define(name.trim(), addr).map_err(at)?;
            text = rest.trim();
        }
        if text.is_empty() {
            continue;
        }
        let (op, rest) = match text.split_once(char::is_whitespace) {
            Some((op, rest)) => (op, rest.trim()),
            None => (text, ""),
        };
        let op = op.to_ascii_lowercase();
        let size = match op.as_str() {
            ".org" => {
                addr = symbols.value(rest).map_err(at)?;
                0
            }
            ".equ" | ".set" => {
                let (name, value) = rest
                    .split_once(',')
                    .ok_or_else(|| at(format!("`{op}` takes a name and a value")))?;
                let value = symbols.value(value).map_err(at)?;
                symbols.define(name.trim(), value).map_err(at)?;
                0
            }
            ".db" => db_size(rest).map_err(at)?,
            ".dw" => 2 * u16::try_from(items(rest).len()).map_err(|_| at("too many".to_owned()))?,
            ".ds" => symbols.value(rest).map_err(at)?,
            _ if op.starts_with('.') => return Err(at(format!("unknown directive `{op}`"))),
            _ => {
                let args = arguments(rest).map_err(at)?;
                select(&op, &args, &symbols).map_err(at)?.0.len()
            }
        };
        statements.push(Statement {
            line,
            addr,
            size,
            op,
            rest,
        });
        addr = addr.wrapping_add(size);
    }

    // Pass 2: every name is known, so every statement can be encoded.
    let mut bytes: BTreeMap<u16, u8> = BTreeMap::new();
    let mut place = |addr: u16, value: u8, line: usize| -> Result<(), String> {
        match bytes.insert(addr, value) {
            None => Ok(()),
            Some(_) => Err(format!("line {line}: {addr:#06x} is written twice")),
        }
    };
    for statement in &statements {
        let line = statement.line;
        let at = |message: String| format!("line {line}: {message}");
        let mut addr = statement.addr;
        match statement.op.as_str() {
            ".org" | ".equ" | ".set" | ".ds" => {}
            ".db" => {
                for item in items(statement.rest) {
                    if is_string(item) {
                        for byte in string_literal(item).map_err(at)? {
                            place(addr, byte, line)?;
                            addr = addr.wrapping_add(1);
                        }
                        continue;
                    }
                    let value = symbols.value(item).map_err(at)?;
                    if value > 0xFF {
                        return Err(at(format!("{value} does not fit a byte")));
                    }
                    place(addr, u8::try_from(value).expect("a byte"), line)?;
                    addr = addr.wrapping_add(1);
                }
            }
            ".dw" => {
                for item in items(statement.rest) {
                    let value = symbols.value(item).map_err(at)?;
                    place(addr, u8::try_from(value & 0xFF).expect("a byte"), line)?;
                    place(
                        addr.wrapping_add(1),
                        u8::try_from(value >> 8).expect("a byte"),
                        line,
                    )?;
                    addr = addr.wrapping_add(2);
                }
            }
            name => {
                let args = arguments(statement.rest).map_err(at)?;
                let (insn, fill) = select(name, &args, &symbols).map_err(at)?;
                // The two passes have to agree about how long this is,
                // or everything after it is at the wrong address. They
                // can only differ if a name that selects an encoding by
                // its value was defined between them, which rule 5
                // forbids; if that ever happens, say so rather than
                // emit a program that is quietly wrong.
                if insn.len() != statement.size {
                    return Err(at(format!(
                        "`{}` was {} bytes long in the first pass and {} in the second",
                        insn.syntax(),
                        statement.size,
                        insn.len()
                    )));
                }
                let disp = match &fill.disp {
                    Some(text) => {
                        let disp = symbols.signed(text).map_err(at)?;
                        if !(-128..=127).contains(&disp) {
                            return Err(at(format!(
                                "the displacement {disp} does not fit a signed byte"
                            )));
                        }
                        disp
                    }
                    None => 0,
                };
                let value = match &fill.value {
                    Some(text) => symbols.value(text).map_err(at)?,
                    None => 0,
                };
                let value = match insn.field() {
                    Field::Byte | Field::DispByte | Field::Port => {
                        if value > 0xFF {
                            return Err(at(format!("{value} does not fit a byte")));
                        }
                        value
                    }
                    Field::Rel => {
                        // A branch names where it goes; the encoding
                        // holds the distance from the instruction after
                        // it.
                        let next = i32::from(statement.addr.wrapping_add(insn.len()));
                        let distance = i32::from(value) - next;
                        if !(-128..=127).contains(&distance) {
                            return Err(at(format!(
                                "the branch to {value:#06x} is {distance} away, which does not fit"
                            )));
                        }
                        u16::from(u8::try_from(distance & 0xFF).expect("a byte"))
                    }
                    Field::None | Field::Word | Field::Disp => value,
                };
                for (offset, byte) in encode(insn, disp, value).into_iter().enumerate() {
                    let offset = u16::try_from(offset).expect("four bytes at most");
                    place(addr.wrapping_add(offset), byte, line)?;
                }
            }
        }
    }

    Ok(Image {
        bytes,
        labels: symbols.names,
    })
}
