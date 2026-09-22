//! A minimal MOS 6502 assembler, shared by the tests that run machine
//! code on the `mos6502` core.
//!
//! The whole of it hangs off [`TABLE`], which is the documented opcode
//! matrix typed out in hexadecimal order: for each of the 151 official
//! encodings, the mnemonic, the addressing mode, the opcode byte and the
//! cycle count the 6502's own documentation prints for it. Nothing here
//! is taken from the core's decoder — a testbench that assembled its
//! programs with the implementation it is testing would agree with it
//! however wrong both were — and the cycle counts are the reference's,
//! which is what `mos6502_counts_the_cycles_of_every_instruction`
//! compares the hardware against.
//!
//! [`assemble`] is a two-pass assembler over that table, in the syntax
//! every 6502 assembler accepts: `#$nn` for an immediate, `$nn,x` for
//! zero page indexed, `($nn,x)` and `($nn),y` for the two indirections,
//! `($nnnn)` for the indirect jump, `a` or nothing for the accumulator,
//! and a label for a branch or a jump. It is deliberately small: no
//! expressions beyond a number or a name, no macros, no segments.
//!
//! One rule needs stating because the assembly is otherwise ambiguous:
//! **a hexadecimal operand written with three or more digits is an
//! absolute address even when it fits in a byte**. `lda $10` is the
//! zero-page form and `lda $0010` the absolute one, which is how a test
//! reaches both. Anything else picks zero page when the value fits in a
//! byte and the mnemonic has a zero-page form.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// The thirteen addressing modes of the 6502.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Mode {
    /// No operand: `clc`, `nop`, `rts`.
    Imp,
    /// The accumulator is the operand: `asl a`.
    Acc,
    /// `#$nn`.
    Imm,
    /// `$nn`.
    Zp,
    /// `$nn,x`.
    ZpX,
    /// `$nn,y`.
    ZpY,
    /// `$nnnn`.
    Abs,
    /// `$nnnn,x`.
    AbsX,
    /// `$nnnn,y`.
    AbsY,
    /// `($nnnn)`, which only `jmp` has.
    Ind,
    /// `($nn,x)`: the pointer is at `$nn + X`, wrapped in page zero.
    IzX,
    /// `($nn),y`: the pointer is at `$nn` and Y is added to what it says.
    IzY,
    /// A branch's signed displacement from the next instruction.
    Rel,
}

impl Mode {
    /// How many bytes follow the opcode.
    pub(crate) fn operand_bytes(self) -> u16 {
        match self {
            Mode::Imp | Mode::Acc => 0,
            Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind => 2,
            _ => 1,
        }
    }

    /// The whole instruction's length in bytes.
    pub(crate) fn len(self) -> u16 {
        1 + self.operand_bytes()
    }

    /// How the operand is written, for a message.
    pub(crate) fn syntax(self) -> &'static str {
        match self {
            Mode::Imp => "",
            Mode::Acc => "a",
            Mode::Imm => "#$nn",
            Mode::Zp => "$nn",
            Mode::ZpX => "$nn,x",
            Mode::ZpY => "$nn,y",
            Mode::Abs => "$nnnn",
            Mode::AbsX => "$nnnn,x",
            Mode::AbsY => "$nnnn,y",
            Mode::Ind => "($nnnn)",
            Mode::IzX => "($nn,x)",
            Mode::IzY => "($nn),y",
            Mode::Rel => "label",
        }
    }
}

/// One documented encoding.
#[derive(Clone, Copy)]
pub(crate) struct Insn {
    /// The mnemonic, lower case.
    pub(crate) name: &'static str,
    pub(crate) mode: Mode,
    /// The opcode byte.
    pub(crate) code: u8,
    /// The cycle count the 6502's documentation prints for it, before
    /// any page-crossing or branch penalty.
    pub(crate) cycles: u8,
    /// Whether the documentation marks it `+1 if a page is crossed`,
    /// which is the indexed *reads* and none of the writes.
    pub(crate) page_penalty: bool,
}

const fn i(name: &'static str, mode: Mode, code: u8, cycles: u8) -> Insn {
    Insn {
        name,
        mode,
        code,
        cycles,
        page_penalty: false,
    }
}

const fn p(name: &'static str, mode: Mode, code: u8, cycles: u8) -> Insn {
    Insn {
        name,
        mode,
        code,
        cycles,
        page_penalty: true,
    }
}

use Mode::{Abs, AbsX, AbsY, Acc, Imm, Imp, Ind, IzX, IzY, Rel, Zp, ZpX, ZpY};

/// The documented opcode matrix, row by row of sixteen. The 105 opcodes
/// not here are the undocumented ones, which are out of scope for the
/// core and behave as NOP there.
pub(crate) const TABLE: &[Insn] = &[
    // 0x00
    i("brk", Imp, 0x00, 7),
    i("ora", IzX, 0x01, 6),
    i("ora", Zp, 0x05, 3),
    i("asl", Zp, 0x06, 5),
    i("php", Imp, 0x08, 3),
    i("ora", Imm, 0x09, 2),
    i("asl", Acc, 0x0A, 2),
    i("ora", Abs, 0x0D, 4),
    i("asl", Abs, 0x0E, 6),
    // 0x10
    i("bpl", Rel, 0x10, 2),
    p("ora", IzY, 0x11, 5),
    i("ora", ZpX, 0x15, 4),
    i("asl", ZpX, 0x16, 6),
    i("clc", Imp, 0x18, 2),
    p("ora", AbsY, 0x19, 4),
    p("ora", AbsX, 0x1D, 4),
    i("asl", AbsX, 0x1E, 7),
    // 0x20
    i("jsr", Abs, 0x20, 6),
    i("and", IzX, 0x21, 6),
    i("bit", Zp, 0x24, 3),
    i("and", Zp, 0x25, 3),
    i("rol", Zp, 0x26, 5),
    i("plp", Imp, 0x28, 4),
    i("and", Imm, 0x29, 2),
    i("rol", Acc, 0x2A, 2),
    i("bit", Abs, 0x2C, 4),
    i("and", Abs, 0x2D, 4),
    i("rol", Abs, 0x2E, 6),
    // 0x30
    i("bmi", Rel, 0x30, 2),
    p("and", IzY, 0x31, 5),
    i("and", ZpX, 0x35, 4),
    i("rol", ZpX, 0x36, 6),
    i("sec", Imp, 0x38, 2),
    p("and", AbsY, 0x39, 4),
    p("and", AbsX, 0x3D, 4),
    i("rol", AbsX, 0x3E, 7),
    // 0x40
    i("rti", Imp, 0x40, 6),
    i("eor", IzX, 0x41, 6),
    i("eor", Zp, 0x45, 3),
    i("lsr", Zp, 0x46, 5),
    i("pha", Imp, 0x48, 3),
    i("eor", Imm, 0x49, 2),
    i("lsr", Acc, 0x4A, 2),
    i("jmp", Abs, 0x4C, 3),
    i("eor", Abs, 0x4D, 4),
    i("lsr", Abs, 0x4E, 6),
    // 0x50
    i("bvc", Rel, 0x50, 2),
    p("eor", IzY, 0x51, 5),
    i("eor", ZpX, 0x55, 4),
    i("lsr", ZpX, 0x56, 6),
    i("cli", Imp, 0x58, 2),
    p("eor", AbsY, 0x59, 4),
    p("eor", AbsX, 0x5D, 4),
    i("lsr", AbsX, 0x5E, 7),
    // 0x60
    i("rts", Imp, 0x60, 6),
    i("adc", IzX, 0x61, 6),
    i("adc", Zp, 0x65, 3),
    i("ror", Zp, 0x66, 5),
    i("pla", Imp, 0x68, 4),
    i("adc", Imm, 0x69, 2),
    i("ror", Acc, 0x6A, 2),
    i("jmp", Ind, 0x6C, 5),
    i("adc", Abs, 0x6D, 4),
    i("ror", Abs, 0x6E, 6),
    // 0x70
    i("bvs", Rel, 0x70, 2),
    p("adc", IzY, 0x71, 5),
    i("adc", ZpX, 0x75, 4),
    i("ror", ZpX, 0x76, 6),
    i("sei", Imp, 0x78, 2),
    p("adc", AbsY, 0x79, 4),
    p("adc", AbsX, 0x7D, 4),
    i("ror", AbsX, 0x7E, 7),
    // 0x80
    i("sta", IzX, 0x81, 6),
    i("sty", Zp, 0x84, 3),
    i("sta", Zp, 0x85, 3),
    i("stx", Zp, 0x86, 3),
    i("dey", Imp, 0x88, 2),
    i("txa", Imp, 0x8A, 2),
    i("sty", Abs, 0x8C, 4),
    i("sta", Abs, 0x8D, 4),
    i("stx", Abs, 0x8E, 4),
    // 0x90
    i("bcc", Rel, 0x90, 2),
    i("sta", IzY, 0x91, 6),
    i("sty", ZpX, 0x94, 4),
    i("sta", ZpX, 0x95, 4),
    i("stx", ZpY, 0x96, 4),
    i("tya", Imp, 0x98, 2),
    i("sta", AbsY, 0x99, 5),
    i("txs", Imp, 0x9A, 2),
    i("sta", AbsX, 0x9D, 5),
    // 0xA0
    i("ldy", Imm, 0xA0, 2),
    i("lda", IzX, 0xA1, 6),
    i("ldx", Imm, 0xA2, 2),
    i("ldy", Zp, 0xA4, 3),
    i("lda", Zp, 0xA5, 3),
    i("ldx", Zp, 0xA6, 3),
    i("tay", Imp, 0xA8, 2),
    i("lda", Imm, 0xA9, 2),
    i("tax", Imp, 0xAA, 2),
    i("ldy", Abs, 0xAC, 4),
    i("lda", Abs, 0xAD, 4),
    i("ldx", Abs, 0xAE, 4),
    // 0xB0
    i("bcs", Rel, 0xB0, 2),
    p("lda", IzY, 0xB1, 5),
    i("ldy", ZpX, 0xB4, 4),
    i("lda", ZpX, 0xB5, 4),
    i("ldx", ZpY, 0xB6, 4),
    i("clv", Imp, 0xB8, 2),
    p("lda", AbsY, 0xB9, 4),
    i("tsx", Imp, 0xBA, 2),
    p("ldy", AbsX, 0xBC, 4),
    p("lda", AbsX, 0xBD, 4),
    p("ldx", AbsY, 0xBE, 4),
    // 0xC0
    i("cpy", Imm, 0xC0, 2),
    i("cmp", IzX, 0xC1, 6),
    i("cpy", Zp, 0xC4, 3),
    i("cmp", Zp, 0xC5, 3),
    i("dec", Zp, 0xC6, 5),
    i("iny", Imp, 0xC8, 2),
    i("cmp", Imm, 0xC9, 2),
    i("dex", Imp, 0xCA, 2),
    i("cpy", Abs, 0xCC, 4),
    i("cmp", Abs, 0xCD, 4),
    i("dec", Abs, 0xCE, 6),
    // 0xD0
    i("bne", Rel, 0xD0, 2),
    p("cmp", IzY, 0xD1, 5),
    i("cmp", ZpX, 0xD5, 4),
    i("dec", ZpX, 0xD6, 6),
    i("cld", Imp, 0xD8, 2),
    p("cmp", AbsY, 0xD9, 4),
    p("cmp", AbsX, 0xDD, 4),
    i("dec", AbsX, 0xDE, 7),
    // 0xE0
    i("cpx", Imm, 0xE0, 2),
    i("sbc", IzX, 0xE1, 6),
    i("cpx", Zp, 0xE4, 3),
    i("sbc", Zp, 0xE5, 3),
    i("inc", Zp, 0xE6, 5),
    i("inx", Imp, 0xE8, 2),
    i("sbc", Imm, 0xE9, 2),
    i("nop", Imp, 0xEA, 2),
    i("cpx", Abs, 0xEC, 4),
    i("sbc", Abs, 0xED, 4),
    i("inc", Abs, 0xEE, 6),
    // 0xF0
    i("beq", Rel, 0xF0, 2),
    p("sbc", IzY, 0xF1, 5),
    i("sbc", ZpX, 0xF5, 4),
    i("inc", ZpX, 0xF6, 6),
    i("sed", Imp, 0xF8, 2),
    p("sbc", AbsY, 0xF9, 4),
    p("sbc", AbsX, 0xFD, 4),
    i("inc", AbsX, 0xFE, 7),
];

/// The encoding of one mnemonic in one mode, or `None` if it has none.
pub(crate) fn find(name: &str, mode: Mode) -> Option<&'static Insn> {
    TABLE.iter().find(|e| e.name == name && e.mode == mode)
}

/// Whether a mnemonic has a mode at all.
fn has_mode(name: &str, mode: Mode) -> bool {
    find(name, mode).is_some()
}

/// The bytes of one instruction, for a test that builds a program by
/// hand rather than from source.
pub(crate) fn bytes(name: &str, mode: Mode, operand: u16) -> Vec<u8> {
    let insn = find(name, mode)
        .unwrap_or_else(|| panic!("`{name} {}` is not a documented encoding", mode.syntax()));
    let mut out = vec![insn.code];
    match mode.operand_bytes() {
        0 => {}
        1 => out.push(u8::try_from(operand & 0xFF).expect("a byte")),
        _ => {
            out.push(u8::try_from(operand & 0xFF).expect("a byte"));
            out.push(u8::try_from(operand >> 8).expect("a byte"));
        }
    }
    out
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
}

/// A number: `$` or `0x` hexadecimal, `%` binary, or decimal, optionally
/// negative, with `_` allowed between digits. The second value is how
/// many hexadecimal digits were written, which is what decides zero page
/// against absolute.
fn number(text: &str) -> Option<(i64, usize)> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let digits = digits.replace('_', "");
    let (value, hex_digits) = if let Some(hex) = digits.strip_prefix('$') {
        (i64::from_str_radix(hex, 16).ok()?, hex.len())
    } else if let Some(hex) = digits.strip_prefix("0x") {
        (i64::from_str_radix(hex, 16).ok()?, hex.len())
    } else if let Some(bin) = digits.strip_prefix('%') {
        (i64::from_str_radix(bin, 2).ok()?, 0)
    } else if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        (digits.parse().ok()?, 0)
    } else {
        return None;
    };
    Some((if negative { -value } else { value }, hex_digits))
}

/// How an operand was written, before the addressing mode is chosen.
enum Operand {
    /// No operand, or the word `a`.
    None,
    /// `#value`.
    Immediate(String),
    /// `value`, `value,x` or `value,y`, with the digit count of a
    /// hexadecimal literal.
    Direct(String, Option<char>),
    /// `(value)`, `(value,x)` or `(value),y`.
    Indirect(String, Option<char>),
}

/// Splits an operand into its shape and the expression inside it.
fn operand(text: &str) -> Result<Operand, String> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("a") {
        return Ok(Operand::None);
    }
    if let Some(rest) = text.strip_prefix('#') {
        return Ok(Operand::Immediate(rest.trim().to_owned()));
    }
    if let Some(rest) = text.strip_prefix('(') {
        // `($nn,x)` or `($nnnn)` or `($nn),y`.
        if let Some(inner) = rest.strip_suffix(')') {
            return match inner.rsplit_once(',') {
                Some((value, index)) if index.trim().eq_ignore_ascii_case("x") => {
                    Ok(Operand::Indirect(value.trim().to_owned(), Some('x')))
                }
                Some(_) => Err(format!("`({inner})` indexes by something that is not X")),
                None => Ok(Operand::Indirect(inner.trim().to_owned(), None)),
            };
        }
        let (inner, tail) = rest
            .rsplit_once(')')
            .ok_or_else(|| format!("`{text}` has no closing parenthesis"))?;
        let index = tail.trim().strip_prefix(',').map(str::trim);
        return match index {
            Some(y) if y.eq_ignore_ascii_case("y") => {
                Ok(Operand::Indirect(inner.trim().to_owned(), Some('y')))
            }
            _ => Err(format!("`{text}` is not `($nn),y`")),
        };
    }
    match text.rsplit_once(',') {
        Some((value, index)) => {
            let index = index.trim();
            let letter = if index.eq_ignore_ascii_case("x") {
                'x'
            } else if index.eq_ignore_ascii_case("y") {
                'y'
            } else {
                return Err(format!("`{index}` is neither X nor Y"));
            };
            Ok(Operand::Direct(value.trim().to_owned(), Some(letter)))
        }
        None => Ok(Operand::Direct(text.to_owned(), None)),
    }
}

/// What every name means once the first pass has placed everything: its
/// value, and the hexadecimal digit count that decides zero page against
/// absolute where it is used. A `label:` always counts as four digits,
/// so a forward reference is absolute and the two passes agree on how
/// long the instruction is; an `.equ` carries the digits its literal was
/// written with, so `.equ ptr, $10` is a zero-page name.
struct Symbols {
    names: BTreeMap<String, (u16, usize)>,
}

impl Symbols {
    /// A literal or a defined name, with its digit count.
    fn value(&self, text: &str) -> Result<(u16, usize), String> {
        let text = text.trim();
        if let Some((value, digits)) = number(text) {
            let wrapped = u16::try_from(value.rem_euclid(0x1_0000)).expect("sixteen bits");
            return Ok((wrapped, digits));
        }
        self.names
            .get(text)
            .copied()
            .ok_or_else(|| format!("`{text}` is neither a number nor a defined name"))
    }

    fn define(&mut self, name: &str, value: u16, digits: usize) -> Result<(), String> {
        match self.names.insert(name.to_owned(), (value, digits)) {
            None => Ok(()),
            Some(_) => Err(format!("`{name}` is defined twice")),
        }
    }
}

/// Whether a value written that way is a zero-page address: it has to
/// fit in a byte, and it must not have been written with three or more
/// hexadecimal digits, which is how a test asks for absolute.
fn is_zero_page(value: u16, hex_digits: usize) -> bool {
    value < 0x100 && hex_digits < 3
}

/// The mode an instruction is written in, given its mnemonic and how the
/// operand was spelled.
fn mode_of(name: &str, operand: &Operand, symbols: &Symbols) -> Result<(Mode, u16), String> {
    Ok(match operand {
        Operand::None => {
            if has_mode(name, Mode::Acc) && !has_mode(name, Mode::Imp) {
                (Mode::Acc, 0)
            } else {
                (Mode::Imp, 0)
            }
        }
        Operand::Immediate(text) => {
            let (value, _) = symbols.value(text)?;
            if value > 0xFF {
                return Err(format!("the immediate {value} does not fit a byte"));
            }
            (Mode::Imm, value)
        }
        Operand::Indirect(text, index) => {
            let (value, _) = symbols.value(text)?;
            match index {
                Some('x') => (Mode::IzX, value),
                Some(_) => (Mode::IzY, value),
                None => (Mode::Ind, value),
            }
        }
        Operand::Direct(text, index) => {
            let (value, digits) = symbols.value(text)?;
            let zp = is_zero_page(value, digits);
            match index {
                Some('x') if zp && has_mode(name, Mode::ZpX) => (Mode::ZpX, value),
                Some('x') => (Mode::AbsX, value),
                Some('y') if zp && has_mode(name, Mode::ZpY) => (Mode::ZpY, value),
                Some(_) => (Mode::AbsY, value),
                None if has_mode(name, Mode::Rel) => (Mode::Rel, value),
                None if zp && has_mode(name, Mode::Zp) => (Mode::Zp, value),
                None => (Mode::Abs, value),
            }
        }
    })
}

/// One statement, once its labels are off.
struct Statement<'s> {
    line: usize,
    addr: u16,
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

/// The bytes of a `.string` operand, without the terminating NUL.
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

/// Whether `name` can be a label.
fn is_label(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// How long a statement is, for the placement pass.
fn size_of(op: &str, rest: &str, symbols: &Symbols) -> Result<u16, String> {
    if let Some(insn) = shape_size(op, rest, symbols)? {
        return Ok(insn);
    }
    Err(format!("`{op}` is not an instruction this assembler knows"))
}

/// The size of an instruction statement, or `None` if the mnemonic is
/// unknown.
fn shape_size(op: &str, rest: &str, symbols: &Symbols) -> Result<Option<u16>, String> {
    if !TABLE.iter().any(|e| e.name == op) {
        return Ok(None);
    }
    let operand = operand(rest)?;
    // A forward reference is a sixteen-bit name, which is what
    // `Symbols::value` reports for an unknown one too, so the size is
    // the same either way for every mode but zero page — and a program
    // that branches or jumps forward never writes a zero-page label.
    let mode = match mode_of(op, &operand, symbols) {
        Ok((mode, _)) => mode,
        Err(_) => match &operand {
            Operand::None => Mode::Imp,
            Operand::Immediate(_) => Mode::Imm,
            Operand::Indirect(_, Some('x')) => Mode::IzX,
            Operand::Indirect(_, Some(_)) => Mode::IzY,
            Operand::Indirect(_, None) => Mode::Ind,
            Operand::Direct(_, index) => {
                if has_mode(op, Mode::Rel) {
                    Mode::Rel
                } else {
                    match index {
                        Some('x') => Mode::AbsX,
                        Some(_) => Mode::AbsY,
                        None => Mode::Abs,
                    }
                }
            }
        },
    };
    Ok(Some(mode.len()))
}

/// Assembles 6502 source text.
///
/// One statement per line, `;` comments, any number of `label:`
/// prefixes, and the directives
///
/// - `.org $nnnn` places what follows at an address;
/// - `.equ name, value` defines a constant;
/// - `.byte a, b, …` and `.word a, b, …` place bytes and little-endian
///   words;
/// - `.string "text"` (or `.asciz`) places the bytes and a terminating
///   NUL, with the escapes `\n`, `\r`, `\t`, `\0`, `\\` and `\"`;
/// - `.res n` skips `n` bytes.
///
/// An error names the line it is on.
pub(crate) fn assemble(source: &str) -> Result<Image, String> {
    // Pass 1: place every statement and define every name.
    let mut symbols = Symbols {
        names: BTreeMap::new(),
    };
    let mut statements = Vec::new();
    let mut addr: u16 = 0x0200;
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let at = |message: String| format!("line {line}: {message}");
        let mut text = strip_comment(raw).trim();
        while let Some((name, rest)) = text.split_once(':') {
            if !is_label(name.trim()) {
                break;
            }
            symbols.define(name.trim(), addr, 4).map_err(at)?;
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
                let (value, _) = symbols.value(rest).map_err(at)?;
                addr = value;
                0
            }
            ".equ" | ".set" => {
                let (name, value) = rest
                    .split_once(',')
                    .ok_or_else(|| at(format!("`{op}` takes a name and a value")))?;
                let (value, digits) = symbols.value(value).map_err(at)?;
                symbols.define(name.trim(), value, digits).map_err(at)?;
                0
            }
            ".byte" => u16::try_from(rest.split(',').count()).map_err(|_| at("too many".into()))?,
            ".string" | ".asciz" => {
                let bytes = string_literal(rest).map_err(at)?;
                u16::try_from(bytes.len() + 1)
                    .map_err(|_| at("the string is too long".to_owned()))?
            }
            ".word" => {
                2 * u16::try_from(rest.split(',').count()).map_err(|_| at("too many".into()))?
            }
            ".res" => symbols.value(rest).map_err(at)?.0,
            _ if op.starts_with('.') => return Err(at(format!("unknown directive `{op}`"))),
            _ => size_of(&op, rest, &symbols).map_err(at)?,
        };
        statements.push(Statement {
            line,
            addr,
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
            ".org" | ".equ" | ".set" | ".res" => {}
            ".string" | ".asciz" => {
                // The bytes, then the NUL every 6502 string routine stops on.
                let mut bytes = string_literal(statement.rest).map_err(at)?;
                bytes.push(0);
                for byte in bytes {
                    place(addr, byte, line)?;
                    addr = addr.wrapping_add(1);
                }
            }
            ".byte" => {
                for item in statement.rest.split(',') {
                    let (value, _) = symbols.value(item).map_err(at)?;
                    if value > 0xFF {
                        return Err(at(format!("{value} does not fit a byte")));
                    }
                    place(addr, u8::try_from(value).expect("a byte"), line)?;
                    addr = addr.wrapping_add(1);
                }
            }
            ".word" => {
                for item in statement.rest.split(',') {
                    let (value, _) = symbols.value(item).map_err(at)?;
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
                let operand = operand(statement.rest).map_err(at)?;
                let (mode, mut value) = mode_of(name, &operand, &symbols).map_err(at)?;
                let insn = find(name, mode)
                    .ok_or_else(|| at(format!("`{name}` has no `{}` form", mode.syntax())))?;
                if mode == Mode::Rel {
                    // A branch names where it goes; the encoding holds
                    // the distance from the instruction after it.
                    let next = i32::from(statement.addr.wrapping_add(2));
                    let distance = i32::from(value) - next;
                    if !(-128..=127).contains(&distance) {
                        return Err(at(format!(
                            "the branch to {value:#06x} is {distance} away, which does not fit"
                        )));
                    }
                    value = u16::from(u8::try_from(distance & 0xFF).expect("a byte"));
                }
                place(addr, insn.code, line)?;
                for (offset, byte) in [
                    (1u16, u8::try_from(value & 0xFF).expect("a byte")),
                    (2, u8::try_from(value >> 8).expect("a byte")),
                ]
                .into_iter()
                .take(usize::from(mode.operand_bytes()))
                {
                    place(addr.wrapping_add(offset), byte, line)?;
                }
            }
        }
    }

    Ok(Image {
        bytes,
        labels: symbols
            .names
            .into_iter()
            .map(|(name, (value, _))| (name, value))
            .collect(),
    })
}
