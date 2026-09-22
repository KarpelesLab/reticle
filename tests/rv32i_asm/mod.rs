//! A minimal RV32I assembler, shared by the tests that run machine code.
//!
//! Every encoder returns the thirty-two bits of one instruction, so a test
//! program is a `Vec<u32>` that reads like the assembly it is. The
//! encodings are written out from the base ISA's field layout rather than
//! taken from a table, which is the point: a test that assembled its
//! programs with the same decoder the core uses would prove nothing.
//!
//! `tests/ip_library.rs` mounts this file as its `mod asm`. A test binary
//! that mounts it uses the encoders its programs need, not all of them,
//! so `dead_code` is allowed here rather than at every use.

#![allow(dead_code)]

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
