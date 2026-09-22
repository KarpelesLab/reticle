//! A minimal RV32I assembler, shared by the tests that run machine code.
//!
//! Every encoder returns the thirty-two bits of one instruction, so a test
//! program is a `Vec<u32>` that reads like the assembly it is. The
//! encodings are written out from the base ISA's field layout rather than
//! taken from a table, which is the point: a test that assembled its
//! programs with the same decoder the core uses would prove nothing.
//!
//! [`assemble`] is a front end over the same encoders for a program kept
//! as source text, such as `examples/soc/sw/hello.s`: labels, the base
//! instructions, three pseudo-instructions and four directives, in the
//! syntax the GNU assembler accepts for them. It is deliberately small —
//! no relocations, no `%hi` / `%lo`, no expression beyond a number or a
//! name — because what it has to be is obviously right, and every word
//! it emits comes from an encoder above that the core's own tests in
//! `tests/ip_library.rs` already exercise.
//!
//! `tests/ip_library.rs` mounts this file as its `mod asm` and
//! `tests/soc.rs` as its own; each uses a different part of it, so
//! `dead_code` is allowed here rather than at every use.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// The two's complement bits of a signed immediate, which is what an
/// instruction encoding holds rather than the number itself.
fn twos(value: i32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

/// A CSR number in the twelve-bit immediate field an I-type
/// instruction carries it in.
fn csr_number(csr: u32) -> i32 {
    i32::try_from(csr).expect("a twelve-bit CSR number")
}

fn r(funct7: u32, rs2: u32, rs1: u32, funct3: u32, rd: u32, op: u32) -> u32 {
    (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | op
}

fn i(imm: i32, rs1: u32, funct3: u32, rd: u32, op: u32) -> u32 {
    ((twos(imm) & 0xFFF) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | op
}

fn s(imm: i32, rs2: u32, rs1: u32, funct3: u32, op: u32) -> u32 {
    let v = twos(imm);
    (((v >> 5) & 0x7F) << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | ((v & 0x1F) << 7) | op
}

fn b(imm: i32, rs2: u32, rs1: u32, funct3: u32, op: u32) -> u32 {
    let v = twos(imm);
    (((v >> 12) & 1) << 31)
        | (((v >> 5) & 0x3F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | (((v >> 1) & 0xF) << 8)
        | (((v >> 11) & 1) << 7)
        | op
}

fn u(imm: u32, rd: u32, op: u32) -> u32 {
    (imm & 0xFFFF_F000) | (rd << 7) | op
}

fn j(imm: i32, rd: u32, op: u32) -> u32 {
    let v = twos(imm);
    (((v >> 20) & 1) << 31)
        | (((v >> 1) & 0x3FF) << 21)
        | (((v >> 11) & 1) << 20)
        | (((v >> 12) & 0xFF) << 12)
        | (rd << 7)
        | op
}

pub(crate) fn lui(rd: u32, imm: u32) -> u32 {
    u(imm, rd, 0x37)
}
pub(crate) fn auipc(rd: u32, imm: u32) -> u32 {
    u(imm, rd, 0x17)
}
pub(crate) fn jal(rd: u32, off: i32) -> u32 {
    j(off, rd, 0x6F)
}
pub(crate) fn jalr(rd: u32, rs1: u32, off: i32) -> u32 {
    i(off, rs1, 0, rd, 0x67)
}

pub(crate) fn beq(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(off, rs2, rs1, 0b000, 0x63)
}
pub(crate) fn bne(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(off, rs2, rs1, 0b001, 0x63)
}
pub(crate) fn blt(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(off, rs2, rs1, 0b100, 0x63)
}
pub(crate) fn bge(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(off, rs2, rs1, 0b101, 0x63)
}
pub(crate) fn bltu(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(off, rs2, rs1, 0b110, 0x63)
}
pub(crate) fn bgeu(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(off, rs2, rs1, 0b111, 0x63)
}

pub(crate) fn lb(rd: u32, rs1: u32, off: i32) -> u32 {
    i(off, rs1, 0b000, rd, 0x03)
}
pub(crate) fn lh(rd: u32, rs1: u32, off: i32) -> u32 {
    i(off, rs1, 0b001, rd, 0x03)
}
pub(crate) fn lw(rd: u32, rs1: u32, off: i32) -> u32 {
    i(off, rs1, 0b010, rd, 0x03)
}
pub(crate) fn lbu(rd: u32, rs1: u32, off: i32) -> u32 {
    i(off, rs1, 0b100, rd, 0x03)
}
pub(crate) fn lhu(rd: u32, rs1: u32, off: i32) -> u32 {
    i(off, rs1, 0b101, rd, 0x03)
}

pub(crate) fn sb(rs2: u32, rs1: u32, off: i32) -> u32 {
    s(off, rs2, rs1, 0b000, 0x23)
}
pub(crate) fn sh(rs2: u32, rs1: u32, off: i32) -> u32 {
    s(off, rs2, rs1, 0b001, 0x23)
}
pub(crate) fn sw(rs2: u32, rs1: u32, off: i32) -> u32 {
    s(off, rs2, rs1, 0b010, 0x23)
}

pub(crate) fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(imm, rs1, 0b000, rd, 0x13)
}
pub(crate) fn slti(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(imm, rs1, 0b010, rd, 0x13)
}
pub(crate) fn sltiu(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(imm, rs1, 0b011, rd, 0x13)
}
pub(crate) fn xori(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(imm, rs1, 0b100, rd, 0x13)
}
pub(crate) fn ori(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(imm, rs1, 0b110, rd, 0x13)
}
pub(crate) fn andi(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(imm, rs1, 0b111, rd, 0x13)
}
pub(crate) fn slli(rd: u32, rs1: u32, sh: u32) -> u32 {
    r(0b0000000, sh, rs1, 0b001, rd, 0x13)
}
pub(crate) fn srli(rd: u32, rs1: u32, sh: u32) -> u32 {
    r(0b0000000, sh, rs1, 0b101, rd, 0x13)
}
pub(crate) fn srai(rd: u32, rs1: u32, sh: u32) -> u32 {
    r(0b0100000, sh, rs1, 0b101, rd, 0x13)
}

pub(crate) fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0000000, rs2, rs1, 0b000, rd, 0x33)
}
pub(crate) fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0100000, rs2, rs1, 0b000, rd, 0x33)
}
pub(crate) fn sll(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0000000, rs2, rs1, 0b001, rd, 0x33)
}
pub(crate) fn slt(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0000000, rs2, rs1, 0b010, rd, 0x33)
}
pub(crate) fn sltu(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0000000, rs2, rs1, 0b011, rd, 0x33)
}
pub(crate) fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0000000, rs2, rs1, 0b100, rd, 0x33)
}
pub(crate) fn srl(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0000000, rs2, rs1, 0b101, rd, 0x33)
}
pub(crate) fn sra(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0100000, rs2, rs1, 0b101, rd, 0x33)
}
pub(crate) fn or(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0000000, rs2, rs1, 0b110, rd, 0x33)
}
pub(crate) fn and(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(0b0000000, rs2, rs1, 0b111, rd, 0x33)
}

pub(crate) fn fence() -> u32 {
    i(0x0FF, 0, 0b000, 0, 0x0F)
}
pub(crate) fn ecall() -> u32 {
    i(0x000, 0, 0b000, 0, 0x73)
}
pub(crate) fn ebreak() -> u32 {
    i(0x001, 0, 0b000, 0, 0x73)
}
pub(crate) fn mret() -> u32 {
    i(0x302, 0, 0b000, 0, 0x73)
}
pub(crate) fn wfi() -> u32 {
    i(0x105, 0, 0b000, 0, 0x73)
}

pub(crate) fn csrrw(rd: u32, csr: u32, rs1: u32) -> u32 {
    i(csr_number(csr), rs1, 0b001, rd, 0x73)
}
pub(crate) fn csrrs(rd: u32, csr: u32, rs1: u32) -> u32 {
    i(csr_number(csr), rs1, 0b010, rd, 0x73)
}
pub(crate) fn csrrc(rd: u32, csr: u32, rs1: u32) -> u32 {
    i(csr_number(csr), rs1, 0b011, rd, 0x73)
}
pub(crate) fn csrrwi(rd: u32, csr: u32, imm: u32) -> u32 {
    i(csr_number(csr), imm, 0b101, rd, 0x73)
}
pub(crate) fn csrrsi(rd: u32, csr: u32, imm: u32) -> u32 {
    i(csr_number(csr), imm, 0b110, rd, 0x73)
}

/// A word that decodes to nothing: opcode 0x0B is one of the four
/// slots the base ISA leaves for a custom extension, so no future
/// version of this core can accidentally make it legal.
pub(crate) fn illegal() -> u32 {
    0x0000_000B
}

// ---------------------------------------------------------------------------
// Source text
// ---------------------------------------------------------------------------

/// How an instruction's operands are written and which encoder takes
/// them.
#[derive(Clone, Copy)]
enum Shape {
    /// `op rd, rs1, rs2`.
    Reg(fn(u32, u32, u32) -> u32),
    /// `op rd, rs1, imm`, a twelve-bit signed immediate.
    Imm(fn(u32, u32, i32) -> u32),
    /// `op rd, rs1, shamt`, a five-bit shift amount.
    Shift(fn(u32, u32, u32) -> u32),
    /// `op rd, off(rs1)`.
    Load(fn(u32, u32, i32) -> u32),
    /// `op rs2, off(rs1)`.
    Store(fn(u32, u32, i32) -> u32),
    /// `op rs1, rs2, target`.
    Branch(fn(u32, u32, i32) -> u32),
    /// `op rd, imm20`: the upper twenty bits, as GNU `as` writes them.
    Upper(fn(u32, u32) -> u32),
    /// `jal rd, target`.
    Jal,
    /// `jalr rd, off(rs1)`.
    Jalr,
    /// No operands.
    Bare(fn() -> u32),
    /// `j target`, which is `jal zero, target`.
    J,
    /// `ret`, which is `jalr zero, 0(ra)`.
    Ret,
}

/// `nop`, which is `addi zero, zero, 0`.
fn nop() -> u32 {
    addi(0, 0, 0)
}

/// The instruction a mnemonic names.
fn shape(mnemonic: &str) -> Option<Shape> {
    Some(match mnemonic {
        "add" => Shape::Reg(add),
        "sub" => Shape::Reg(sub),
        "sll" => Shape::Reg(sll),
        "slt" => Shape::Reg(slt),
        "sltu" => Shape::Reg(sltu),
        "xor" => Shape::Reg(xor),
        "srl" => Shape::Reg(srl),
        "sra" => Shape::Reg(sra),
        "or" => Shape::Reg(or),
        "and" => Shape::Reg(and),
        "addi" => Shape::Imm(addi),
        "slti" => Shape::Imm(slti),
        "sltiu" => Shape::Imm(sltiu),
        "xori" => Shape::Imm(xori),
        "ori" => Shape::Imm(ori),
        "andi" => Shape::Imm(andi),
        "slli" => Shape::Shift(slli),
        "srli" => Shape::Shift(srli),
        "srai" => Shape::Shift(srai),
        "lb" => Shape::Load(lb),
        "lh" => Shape::Load(lh),
        "lw" => Shape::Load(lw),
        "lbu" => Shape::Load(lbu),
        "lhu" => Shape::Load(lhu),
        "sb" => Shape::Store(sb),
        "sh" => Shape::Store(sh),
        "sw" => Shape::Store(sw),
        "beq" => Shape::Branch(beq),
        "bne" => Shape::Branch(bne),
        "blt" => Shape::Branch(blt),
        "bge" => Shape::Branch(bge),
        "bltu" => Shape::Branch(bltu),
        "bgeu" => Shape::Branch(bgeu),
        "lui" => Shape::Upper(lui),
        "auipc" => Shape::Upper(auipc),
        "jal" => Shape::Jal,
        "jalr" => Shape::Jalr,
        "fence" => Shape::Bare(fence),
        "ecall" => Shape::Bare(ecall),
        "ebreak" => Shape::Bare(ebreak),
        "mret" => Shape::Bare(mret),
        "wfi" => Shape::Bare(wfi),
        "nop" => Shape::Bare(nop),
        "j" => Shape::J,
        "ret" => Shape::Ret,
        _ => return None,
    })
}

/// A register by its architectural (`x5`) or ABI (`t0`) name.
fn register(name: &str) -> Option<u32> {
    const ABI: [&str; 32] = [
        "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
        "t5", "t6",
    ];
    if name == "fp" {
        return Some(8);
    }
    if let Some(index) = ABI.iter().position(|abi| *abi == name) {
        return u32::try_from(index).ok();
    }
    let number: u32 = name.strip_prefix('x')?.parse().ok()?;
    (number < 32).then_some(number)
}

/// One statement of the source, once its labels are taken off.
struct Statement<'s> {
    /// The line it is on, for messages.
    line: usize,
    /// The byte address it is placed at.
    addr: u32,
    /// The mnemonic or directive, lower-cased.
    op: String,
    /// Everything after it.
    rest: &'s str,
}

/// A line without its comment: everything before the first `#` that is
/// not inside a string.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (at, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..at],
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

/// A number: decimal, `0x` hexadecimal or `0b` binary, optionally
/// negative, with `_` allowed between digits.
fn number(text: &str) -> Option<i64> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let digits = digits.replace('_', "");
    let value = if let Some(hex) = digits.strip_prefix("0x") {
        i64::from_str_radix(hex, 16).ok()?
    } else if let Some(bin) = digits.strip_prefix("0b") {
        i64::from_str_radix(bin, 2).ok()?
    } else if digits.chars().all(|c| c.is_ascii_digit()) && !digits.is_empty() {
        digits.parse().ok()?
    } else {
        return None;
    };
    Some(if negative { -value } else { value })
}

/// What every name means, once the first pass has placed everything.
struct Symbols {
    names: BTreeMap<String, i64>,
}

impl Symbols {
    /// A number, or a name defined by a label or `.equ`.
    fn value(&self, text: &str) -> Result<i64, String> {
        let text = text.trim();
        if let Some(n) = number(text) {
            return Ok(n);
        }
        self.names
            .get(text)
            .copied()
            .ok_or_else(|| format!("`{text}` is neither a number nor a defined name"))
    }

    /// Defines `name`, refusing a second definition.
    fn define(&mut self, name: &str, value: i64) -> Result<(), String> {
        match self.names.insert(name.to_owned(), value) {
            None => Ok(()),
            Some(_) => Err(format!("`{name}` is defined twice")),
        }
    }
}

/// A value that must fit a signed field of `bits` bits.
fn signed(value: i64, bits: u32, what: &str) -> Result<i32, String> {
    let limit = 1i64 << (bits - 1);
    if value < -limit || value >= limit {
        return Err(format!("{what} {value} does not fit {bits} signed bits"));
    }
    i32::try_from(value).map_err(|_| format!("{what} {value} is out of range"))
}

/// A value that must fit an unsigned field of `bits` bits.
fn unsigned(value: i64, bits: u32, what: &str) -> Result<u32, String> {
    if value < 0 || value >= 1i64 << bits {
        return Err(format!("{what} {value} does not fit {bits} unsigned bits"));
    }
    u32::try_from(value).map_err(|_| format!("{what} {value} is out of range"))
}

/// Splits `offset(register)` into its two halves; a missing offset is 0.
fn memory_operand(text: &str) -> Result<(&str, &str), String> {
    let text = text.trim();
    let open = text
        .find('(')
        .filter(|_| text.ends_with(')'))
        .ok_or_else(|| format!("`{text}` is not of the form `offset(register)`"))?;
    let offset = text[..open].trim();
    let reg = text[open + 1..text.len() - 1].trim();
    Ok((if offset.is_empty() { "0" } else { offset }, reg))
}

/// Encodes one instruction.
fn encode(statement: &Statement<'_>, symbols: &Symbols) -> Result<u32, String> {
    let shape = shape(&statement.op).ok_or_else(|| {
        format!(
            "`{}` is not an instruction this assembler knows",
            statement.op
        )
    })?;
    let operands: Vec<&str> = if statement.rest.is_empty() {
        Vec::new()
    } else {
        statement.rest.split(',').map(str::trim).collect()
    };
    let want = match shape {
        Shape::Reg(_) | Shape::Imm(_) | Shape::Shift(_) | Shape::Branch(_) => 3,
        Shape::Load(_) | Shape::Store(_) | Shape::Upper(_) | Shape::Jal | Shape::Jalr => 2,
        Shape::J => 1,
        Shape::Bare(_) | Shape::Ret => 0,
    };
    if operands.len() != want {
        return Err(format!(
            "`{}` takes {want} operand(s), found {}",
            statement.op,
            operands.len()
        ));
    }
    let reg = |text: &str| register(text).ok_or_else(|| format!("`{text}` is not a register"));
    // A branch or jump names where it goes; the encoding holds the
    // distance from this instruction.
    let target = |text: &str, bits: u32| -> Result<i32, String> {
        let distance = symbols.value(text)? - i64::from(statement.addr);
        if distance % 2 != 0 {
            return Err(format!("the target `{text}` is not half-word aligned"));
        }
        signed(distance, bits, "the offset")
    };
    Ok(match shape {
        Shape::Reg(f) => f(reg(operands[0])?, reg(operands[1])?, reg(operands[2])?),
        Shape::Imm(f) => {
            let imm = signed(symbols.value(operands[2])?, 12, "the immediate")?;
            f(reg(operands[0])?, reg(operands[1])?, imm)
        }
        Shape::Shift(f) => {
            let amount = unsigned(symbols.value(operands[2])?, 5, "the shift amount")?;
            f(reg(operands[0])?, reg(operands[1])?, amount)
        }
        Shape::Load(f) | Shape::Store(f) => {
            let (offset, base) = memory_operand(operands[1])?;
            let offset = signed(symbols.value(offset)?, 12, "the offset")?;
            f(reg(operands[0])?, reg(base)?, offset)
        }
        Shape::Branch(f) => f(
            reg(operands[0])?,
            reg(operands[1])?,
            target(operands[2], 13)?,
        ),
        Shape::Upper(f) => {
            let imm = unsigned(symbols.value(operands[1])?, 20, "the upper immediate")?;
            f(reg(operands[0])?, imm << 12)
        }
        Shape::Jal => jal(reg(operands[0])?, target(operands[1], 21)?),
        Shape::Jalr => {
            let (offset, base) = memory_operand(operands[1])?;
            let offset = signed(symbols.value(offset)?, 12, "the offset")?;
            jalr(reg(operands[0])?, reg(base)?, offset)
        }
        Shape::Bare(f) => f(),
        Shape::J => jal(0, target(operands[0], 21)?),
        Shape::Ret => jalr(0, 1, 0),
    })
}

/// Whether `name` can be a label.
fn is_label(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Assembles RV32I source text into the little-endian words of a memory
/// image whose first byte is at address 0.
///
/// The syntax is the GNU assembler's, cut down: one statement per line,
/// `#` comments, any number of `label:` prefixes, registers by `x` number
/// or ABI name, and a memory operand as `offset(register)`. Besides the
/// base instructions it takes `nop`, `j target` and `ret`, and the
/// directives
///
/// - `.equ name, value` defines a constant;
/// - `.word value` places four bytes;
/// - `.string "text"` places the bytes and a terminating NUL, with the
///   escapes `\n`, `\r`, `\t`, `\0`, `\\` and `\"`;
/// - `.align n` pads with zeros to a multiple of `2^n` bytes.
///
/// An error names the line it is on.
pub(crate) fn assemble(source: &str) -> Result<Vec<u32>, String> {
    // Pass 1: place every statement and define every name.
    let mut symbols = Symbols {
        names: BTreeMap::new(),
    };
    let mut statements = Vec::new();
    let mut addr: u32 = 0;
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let at = |message: String| format!("line {line}: {message}");
        let mut text = strip_comment(raw).trim();
        while let Some((name, rest)) = text.split_once(':') {
            if !is_label(name.trim()) {
                break;
            }
            symbols.define(name.trim(), i64::from(addr)).map_err(at)?;
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
            ".equ" | ".set" => {
                let (name, value) = rest
                    .split_once(',')
                    .ok_or_else(|| at(format!("`{op}` takes a name and a value")))?;
                let value = number(value.trim())
                    .ok_or_else(|| at(format!("`{}` is not a number", value.trim())))?;
                symbols.define(name.trim(), value).map_err(at)?;
                0
            }
            ".word" => 4,
            ".string" | ".asciz" => {
                let bytes = string_literal(rest).map_err(at)?;
                u32::try_from(bytes.len() + 1).map_err(|_| at("the string is too long".into()))?
            }
            ".align" => {
                let power = number(rest)
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n < 16)
                    .ok_or_else(|| at(format!("`.align {rest}` is not a small power of two")))?;
                addr.next_multiple_of(1 << power) - addr
            }
            _ if op.starts_with('.') => return Err(at(format!("unknown directive `{op}`"))),
            _ => {
                if !addr.is_multiple_of(4) {
                    return Err(at(format!(
                        "`{op}` would be at {addr:#x}, which is not word aligned; \
                         add `.align 2` before it"
                    )));
                }
                4
            }
        };
        statements.push(Statement {
            line,
            addr,
            op,
            rest,
        });
        addr += size;
    }

    // Pass 2: every name is known, so every statement can be encoded.
    let mut bytes: Vec<u8> = Vec::new();
    for statement in &statements {
        let at = |message: String| format!("line {}: {message}", statement.line);
        bytes.resize(statement.addr as usize, 0);
        match statement.op.as_str() {
            ".equ" | ".set" | ".align" => {}
            ".word" => {
                let value = symbols.value(statement.rest).map_err(at)?;
                let word = u32::try_from(value)
                    .or_else(|_| i32::try_from(value).map(twos))
                    .map_err(|_| at(format!("{value} does not fit a word")))?;
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            ".string" | ".asciz" => {
                bytes.extend(string_literal(statement.rest).map_err(at)?);
                bytes.push(0);
            }
            _ => {
                let word = encode(statement, &symbols).map_err(at)?;
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
    }
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    Ok(bytes
        .chunks(4)
        .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
        .collect())
}
