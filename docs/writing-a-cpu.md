# Packaging a CPU as Reticle IP

A processor is the hardest thing to put in an IP library, and not
because it is big. A FIFO either holds the bytes or it does not; a
processor has an *architecture* behind it — a document that says what
every instruction does, what the flags mean afterwards, where it starts
after reset and, on some parts, how many cycles each of those takes —
and the package has to be judged against that document rather than
against itself. A UART you can test by reading its own output back. A
CPU you cannot, and most of this page is about why and what to do
instead.

Reticle's library has two of them, chosen to be as unlike each other as
possible:

| | [`ip/rv32i`](../ip/rv32i) | [`ip/mos6502`](../ip/mos6502) |
|---|---|---|
| machine | 32-bit load/store, 32 registers | 8-bit accumulator, three registers |
| instructions | fixed 32 bits, one addressing mode | one to three bytes, thirteen addressing modes |
| ports | separate instruction and data | one bus, one access per cycle |
| timing claim | 2 or 3 clocks per instruction | the documented cycle count, exactly |
| tested against | the base ISA's field layout | the documented opcode matrix |
| dropped into | [`examples/soc`](../examples/soc) | [`examples/mos6502_computer`](../examples/mos6502_computer) |

Everything below is drawn from those two packages. Where something is
general it says so; where it is one core's problem it says whose. Where
it is not implemented, it says that too.

Read [`docs/ip.md`](ip.md) first for the manifest machinery and
[`docs/ip-library.md`](ip-library.md) for the library as a whole; this
page is only about the processor-shaped parts.

## 1. The manifest

A core's `reticle.ip` is the same manifest every other package has.
`ip/mos6502/reticle.ip`, entire:

```text
# mos6502: the documented MOS 6502 instruction set in a cycle-counting
# core — thirteen addressing modes, variable-length instructions and one
# bus access per clock, so a program's cycle count is the one the tables
# print. DECIMAL_MODE builds the packed binary-coded decimal arithmetic
# ADC and SBC use when the D flag is set; turn it off and the flag stays
# but the two adders it needs are not synthesised.
name mos6502
version 1.0.0
license MIT
description "MOS 6502 core: all 56 documented opcodes, 13 addressing modes, cycle-accurate counts, decimal mode, RES / NMI / IRQ / BRK"

top mos6502
target ice40
target ecp5

source rtl/mos6502.v

param DECIMAL_MODE int 1 0..1

port clk in
port rst_n in
port addr out 16
port dout out 8
port din in 8
port we out
port ready in
port sync out
port irq in
port nmi in
port dbg_pc out 16
port dbg_retire out
port dbg_trap out
```

### Naming

The package is named after the architecture, not after the
implementation: `rv32i` and `mos6502`, not `tinyriscv` or `fast6502`.
That is the useful convention for a core, because the name is the thing
a user searches for and the thing that says what it is compatible with.
The `description` line then carries the *scope*: "all 56 documented
opcodes, 13 addressing modes, cycle-accurate counts, decimal mode, RES /
NMI / IRQ / BRK". A reader who needs the 105 undocumented opcodes learns
from that line that this is not the core for them, before resolving
anything.

`top` is the module a project instantiates. Both cores are one module in
one file, so `top` is the package name; `uart` is the counter-example in
this library, where the package ships three modules and `top` names the
pair.

### Versioning

`version 1.0.0`, and the meaning of a major bump on a *core* is worth
pinning down, because it is not the same as on a FIFO. The ports and the
parameters are the API in the ordinary sense, but a program is an API
too: a core that stops reproducing a documented quirk, or changes the
number of cycles an instruction takes, has broken software that a port
list change would not have. Treat the architectural contract — the
instruction set, the flags, the vectors, and for `mos6502` the cycle
counts — as part of the major version.

### Parameters

```text
param DECIMAL_MODE int 1 0..1                 # mos6502
param RESET_VECTOR bits 32'h00000000          # rv32i
param REGFILE_BRAM int 0 0..1                 # rv32i
```

The form is `param <name> <type> [default] [lo..hi]`. The range is
checked against the manifest's own default — a default outside it is
`P0008` — and beyond that it is what a consumer reads to know what the
parameter will accept. Be honest about how far that goes: a project
manifest has no way to override a parameter today, so nothing in the
resolver is validating a user's value against the range. Write it
anyway, because it is the only statement of the legal values there is.

Both cores have exactly one kind of parameter worth having, and it is
not "make it configurable in case someone wants it". It is **compiling
out cost**:

- **`DECIMAL_MODE`** builds or does not build the packed binary-coded
  decimal arithmetic `ADC` and `SBC` use when the D flag is set. With
  `0` the flag still exists — `SED`, `CLD`, `PHP` and `PLP` all see it —
  and the arithmetic is always binary. The saving is two adders and a
  pair of comparators: 64 LUT4s, about four per cent (§7).
- **`REGFILE_BRAM`** chooses how `x1`..`x31` are stored: asynchronously
  read (LUT RAM or flip-flops) or clocked, with the addresses presented
  during `FETCH` and the data there in `EXEC`. It costs no cycle,
  because `FETCH` was going to happen anyway, and on an iCE40 HX1K it is
  the difference between 5047 and 2355 `SB_LUT4`.
- **`RESET_VECTOR`** is the third kind: not a cost, a *fact about the
  system* the core cannot know. Which brings us to the interesting
  asymmetry — `mos6502` has no such parameter and cannot have one,
  because a 6502 is not told where to start. It reads `$FFFC`. See §5.

What neither core has is a parameter for anything architectural. There
is no `SUPPORT_MUL`, no `TRAP_ON_UNALIGNED`, no `PIPELINE_DEPTH`. A
parameter that changes what programs run is a second core wearing the
first one's name, and it doubles the test matrix for every test in the
file. `DECIMAL_MODE` is on the edge of this and pays for itself by being
run **both ways through almost every test** — `const DECIMALS: [&str; 2]
= ["1", "0"];` in `tests/ip_library.rs`, with the same programs and the
same expectations, because a program that does not use decimal mode has
to behave identically on each.

### Ports

`port <name> <in|out|inout> [width]`, one line each. These are the
package's declared interface for a reader and for a registry index;
nothing cross-checks them against the RTL today, which an `interface`
line does get (below). Keeping them right is therefore a discipline, and
one worth keeping: a manifest that disagrees with its own Verilog is
worse than one that says nothing. The `dbg_*` ports are worth a note:

```text
port dbg_pc out 16
port dbg_retire out
port dbg_trap out
```

Both cores expose a **retirement trace** — the address of the
instruction that just finished, a pulse when one finishes, a pulse when
a trap or interrupt sequence is entered — and both headers say the same
thing about it. `rv32i`: the `dbg_*` signals "exist so a testbench can
follow retirement and are free to be left unconnected". `mos6502`: "The
three `dbg_*` outputs exist for a testbench and can be left
unconnected." `examples/soc` and `examples/mos6502_computer` both leave
them unconnected; once the core is flattened into the top level the
logic feeding a dangling output is dead, and synthesis removes it.

They are not a debug module. There is no halt, no single step, no
register read-back over a port. They are three signals that let a test
say "run until the instruction at `0x1234` retires" instead of "run for
about forty cycles and hope", and that is most of the value of a debug
port at none of the cost.

### Declaring a bus interface

A package whose ports form a *standard* bus declares it in one line
instead of nineteen. `ip/axil_gpio/reticle.ip`:

```text
port s_axi_aclk in
port s_axi_aresetn in
port gpio_i in WIDTH
port gpio_o out WIDTH
port gpio_oe out WIDTH
interface s_axi axi4lite subordinate prefix s_axi_
```

`bus::match_ports` then finds the bus on the module's ports by the
prefix plus each standard signal name and reports a missing signal
(`P0201`), a direction the wrong way round (`P0202`) or a width that
contradicts the parameters (`P0203`); `bus::connect` wires two
interfaces together without nineteen port lines per side. Clocks and
resets stay `port` lines, because one clock usually serves several
interfaces.

**Neither processor declares one**, and that is the honest answer rather
than an oversight: their memory ports are not AXI, Wishbone, APB or
Avalon-MM, they are the two-wire handshake of §2, so there is no bus
definition for `bus::match_ports` to check them against. A core that
*did* present AXI4-Lite on its data port would declare it and get that
checking for free. Adding a bus is adding a `.bus` file under
`src/ip/buses/` and touching no Rust, so a core with a house bus worth
standardising has a path; a core with a two-wire handshake does not need
one.

## 2. The bus contract

Both cores use the same handshake, and they arrived at it from opposite
directions.

`rv32i` has two ports and states it once for both:

```
//   Both memory ports use the same handshake: the core holds the request
//   and its address stable until it sees `*_ready` high at a rising clock
//   edge, and that edge is the transfer. A memory that answers in the
//   cycle it is asked ties `*_ready` high and needs nothing else.
```

`mos6502` has one and states it the same way:

```
//   The bus. One access per cycle: `addr` and `we` are valid for the
//   whole cycle, a write presents `dout`, a read takes `din` at the
//   rising edge. `ready` low stalls the core with the access held, so
//   slow memory costs whole cycles and nothing else
```

One asymmetry is worth naming, because it is the clearest case of a
port existing only when it has something to say. `rv32i` has `imem_req`
and `dmem_req`, because a multi-cycle core has cycles in which it wants
neither memory — `EXEC` is one. `mos6502` has no `req` at all: it makes
**exactly one access per bus cycle**, so a request line would be
constant. What is left on both is the same two-wire agreement, and a
memory written for one is a memory written for the other with `req`
tied high.

### Why a request/ready handshake

Three properties, and a core that wants to be dropped into a system it
has never seen needs all three.

**A memory that is always ready costs nothing.** Tie `*_ready` high and
the handshake disappears: no extra state, no extra cycle, no wrapper.
This is the case that has to be free, because it is the common one —
a register file, a small ROM read combinationally, a peripheral that
answers immediately.

**A memory that is sometimes slow needs no negotiation.** There is no
burst length, no transaction id, no response channel, no outstanding
count. The slave's entire obligation is: while `req` is high, either
present the data and raise `ready`, or do not raise it. That is a
contract a beginner can implement correctly on the first try, which
matters more for a core in a teaching-sized library than throughput
does.

**The stall is invisible to the architecture.** This is the one that is
easy to get wrong. Holding the core still is not the same as inserting a
cycle into it: `mos6502`'s claim is that its programs spend the cycle
counts the 6502's tables print, and that claim survives a slow memory
only because `ready` low *freezes* a bus cycle instead of adding one.
The test that holds it says so in one line:

```rust
assert_eq!(
    counts[0], counts[1],
    "a wait state costs clocks, not bus cycles"
);
```

`mos6502_survives_a_memory_that_makes_it_wait` runs the same
sum-an-array loop with zero, one and three wait states and insists all
three take the same number of **bus cycles** and reach the same answer.
`rv32i_survives_memories_that_make_it_wait` does the same on both ports.

### How a core should stall

The rule for the core side: **hold everything.** The address, the write
data, the byte enables, the request itself, and every piece of internal
state that the transfer will update. `mos6502` does it with one guard
around the whole sequencer:

```verilog
if (ready) begin
    st <= nxt;
    ...
end
```

and `rv32i` does it by making `*_ready` a condition on leaving a state
rather than a signal that is sampled and remembered. Neither latches
`ready` — the edge where it is high *is* the transfer, and there is
nothing to remember afterwards.

The rule for the memory side, from `examples/mos6502_computer`, is that
a synchronous read is one wait state and nothing more:

```verilog
always @(posedge clk or negedge rst_n) begin
    if (!rst_n) bus_ready <= 1'b0;
    else        bus_ready <= ~bus_ready;
end
```

The block RAM is read on every edge; the core holds its address while
`bus_ready` is low, so the data is there on the edge where it goes high.
Two clocks per bus cycle, the same two for every access, the cycle
counts unchanged.

### The trade-off: a simple interface against cycle fidelity

This is where a core has to be honest, and `mos6502` is the interesting
case because it makes a timing claim at all. Its header:

```
//   **The cycle *count* is the original's; the cycle *shape* is not.**
//   This is a synchronous core on one clock edge, not a two-phase part:
//   there is no φ1 / φ2, no address setup before the clock, and no
//   half-cycle data window. A cycle here is one clock, an access is
//   valid for all of it, and `ready` stalls reads and writes alike —
//   where the original's RDY only stops it on a read and lets a write
//   through. So a program's cycle count matches the tables exactly, and
//   an oscilloscope would not.
```

Everything in that paragraph is a consequence of choosing a plain
synchronous request/ready bus over modelling the real pins. What is
bought: the core drops into any synchronous system, maps onto block RAM
without a wrapper, and simulates at one event per clock. What is sold:
you cannot put this core on a board next to a real 6502's peripherals
and expect the analogue timing to match, and `RDY` does not have its
documented asymmetry.

The general shape of the decision:

| You need | Interface |
|---|---|
| the core to drop into an FPGA system | request/ready, one clock edge |
| software cycle counts to be right | request/ready, and *stall whole cycles* |
| a logic analyser trace to match | model the real pins, and accept that nothing else will fit |
| bus-level compatibility with existing peripherals | the real bus, as a wrapper, not as the core |

Pick the simplest row that covers the use, say in the header which row
you picked, and name what the rows below it would have bought.

## 3. Testing a core so the test cannot agree with a wrong core

This is the section that matters. Everything else on this page is
engineering taste; this is the part where a library either has a
processor in it or has something that looks like one.

### The rule

> **The assembler is built from the instruction set's own encoding
> table, and never from the core's decoder.**

Both cores follow it, and both test files say so where a reader will
trip over it. `tests/mos6502_asm/mod.rs`:

```
//! The whole of it hangs off [`TABLE`], which is the documented opcode
//! matrix typed out in hexadecimal order: for each of the 151 official
//! encodings, the mnemonic, the addressing mode, the opcode byte and the
//! cycle count the 6502's own documentation prints for it. Nothing here
//! is taken from the core's decoder — a testbench that assembled its
//! programs with the implementation it is testing would agree with it
//! however wrong both were
```

### Why it matters

Suppose you generate the test programs from the core's decoder, or —
more insidiously — write the assembler by reading the decoder's `case`
statement while you write it. Now suppose the core has the wrong opcode
byte for `LDA $nnnn,X`. The assembler emits that wrong byte; the core
decodes that wrong byte as `LDA $nnnn,X`; the test passes. The core is broken for
every program that was not assembled by this assembler, which is every
program that exists. Nothing in the test suite can see it, because the
suite contains exactly one statement of the encoding and the core agrees
with it by construction.

The fix is to have **two independent statements of the same table** and
compare them. `TABLE` in `tests/mos6502_asm/mod.rs` is the reference's,
typed out row by row of sixteen:

```rust
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
```

The `case` statement in `ip/mos6502/rtl/mos6502.v` is the core's:

```verilog
8'h69: begin op = OP_ADC; am = AM_IMM; end
8'h65: begin op = OP_ADC; am = AM_ZP;  end
8'h75: begin op = OP_ADC; am = AM_ZPX; end
```

They were written from the same document and from nothing else, and
`mos6502_counts_the_cycles_of_every_instruction` walks all 143
non-branch encodings comparing them — the opcode byte first, then what
the hardware does with it:

```rust
let mut cpu = Mos::boot(&design, &source, 0);
// The assembler and the table agree on the encoding, which is
// also a check that the source above says what it means.
assert_eq!(cpu.byte_at(M6502_ORIGIN), insn.code, ...);
let took = cpu.next();
assert_eq!(took, u32::from(insn.cycles), ...);
```

`rv32i` is the same rule with a different shape, because RV32I's
encoding is fields rather than a matrix. `tests/rv32i_asm/mod.rs` says
it in its own words:

```
//! The encodings are written out from the base ISA's field layout rather
//! than taken from a table, which is the point: a test that assembled its
//! programs with the same decoder the core uses would prove nothing.
```

so each instruction is built from the fields the specification names,

```rust
fn r(funct7: u32, rs2: u32, rs1: u32, funct3: u32, rd: u32, op: u32) -> u32 {
    (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | op
}
```

and `the_assembler_matches_hand_encoded_instructions` in `tests/soc.rs`
checks the text front end over those encoders against a handful of
encodings worked out by hand:

```rust
assert_eq!(words[0], 0xfff0_0093);  // addi x1, x0, -1
assert_eq!(words[1], 0x1234_5537);  // lui  a0, 0x12345
assert_eq!(words[2], 0x0011_2423);  // sw   ra, 8(sp)
```

Three independent statements, then: the specification, the encoder, and
a person with the field layout and a pencil.

The same rule reaches past processors. `tests/ip_library.rs` computes the
Ethernet frame check sequence in Rust rather than borrowing the MAC's
arithmetic, for exactly the reason the file gives: "a testbench that
borrowed the implementation's own arithmetic would agree with it however
wrong both were". `examples/soc` and `examples/mos6502_computer` both
decode the serial line with a receiver written from the 8N1 frame
(`tests/serial/mod.rs`) rather than instantiating `ip/uart`'s receiver.
It is the same idea every time: **the checker and the thing checked must
not share a parent.**

### Reading architectural state out after each step

A program that prints the right answer at the end proves less than it
looks like it does, because a wrong core can reach a right answer. Both
test harnesses therefore read the *architectural state* — the thing the
specification talks about — after every step.

For `rv32i` that is the register file, which is a memory in the IR, so
the simulator hands it over by name:

```rust
/// The architectural value of `x{index}`.
fn reg(&self, index: u64) -> u32 {
    if index == 0 {
        return 0;
    }
    self.sim
        .get_mem(self.regs, index)
        .and_then(|v| v.to_u64())
        .map_or_else(|| panic!("x{index} holds x"), narrow)
}
```

Note the panic: a register that holds `x` is a failure, not a zero. A
test that quietly read `x` as `0` would pass on a core that never wrote
the register at all.

For `mos6502` there is no register file — three registers, a stack
pointer, a program counter and six flags, each its own net — so the
harness names each one:

```rust
a_n: name("a_r"),
x_n: name("x_r"),
y_n: name("y_r"),
s_n: name("s_r"),
pc_n: name("pc"),
flag_n: [
    name("p_c"),
    name("p_z"),
    name("p_i"),
    name("p_d"),
    name("p_v"),
    name("p_n"),
],
```

and reassembles the status byte the way `PHP` would push it, which is
its own small piece of architecture:

```rust
/// The status byte as PHP would push it, without bit 4: bit 5 reads
/// as one and B is not a flag.
fn p(&self) -> u8 { ... }
```

Both harnesses also step by *instruction* rather than by clock, using
the retirement trace, so a test says what it means:

```rust
/// Runs one whole instruction, from this opcode fetch to the next,
/// and answers how many cycles it took.
fn next(&mut self) -> u32 {
    assert!(self.at_fetch(), "`next` starts at an opcode fetch");
    ...
}
```

This is where the `dbg_*` ports and `sync` earn their place. Without
them every test is written in clocks and breaks when the core's internal
timing changes for a good reason.

A practical note on reaching this state at all: both harnesses look
inside the core, at nets like `a_r` and `regs`, which is a coupling to
the implementation's *names*. That is a deliberate trade — the
alternative is a read-out port that exists only for tests — and it is
safe here because the names are checked by the tests themselves: rename
`a_r` and every 6502 test fails loudly at `net()` rather than silently
passing.

### Why an end-to-end program proves what per-instruction tests do not

Per-instruction tests prove that each instruction, run from a known
state into a known state, does what the book says. They do not prove
that the instructions compose, and composition is where a
multi-cycle core actually goes wrong: a flag left over from the previous
instruction, a stack pointer off by one that only shows after a hundred
pushes, a register the sequencer forgot to hold during a wait state, an
interrupt decision made with flags from the wrong instruction.

So both cores finish with programs. `mos6502`'s hardest is Fibonacci
computed recursively:

```asm
fib:    cmp #$02
        bcc base        ; fib(0) = 0 and fib(1) = 1 are the argument
        pha             ; [n]
        sec
        sbc #$01
        jsr fib         ; A = fib(n-1)
        pha             ; [n][fib(n-1)]
        tsx
        lda $0102,x     ; n again, out of this frame
        sec
        sbc #$02
        jsr fib         ; A = fib(n-2)
        tsx
        clc
        adc $0101,x     ; + fib(n-1)
```

and the assertions are the ones that make it worth running:

```rust
cpu.run_to_done(120_000);
assert_eq!(u32::from(cpu.byte_at(0x0020)), fib(10), "fib(10) computed recursively");
assert_eq!(cpu.s(), 0xFF, "every frame was popped again");
assert_eq!(cpu.traps, 0, "and nothing faulted on the way");
assert!(cpu.retired > 1000, "the whole recursion ran: {} instructions", cpu.retired);
```

Four separate claims. The answer is right; **the stack pointer came back
to where it started**, which catches a push without its pull anywhere in
177 calls; nothing trapped on the way, which catches an instruction
decoded as something else; and more than a thousand instructions
actually ran, which catches the case where the program fell into its own
`done` loop early and the answer was a coincidence.

The 6502 version also exercises something no per-instruction test
reaches: a 6502 cannot address its own stack frame with a register, so
the program does it with `TSX` and absolute,X — `lda $0102,x` — which
means the subtraction, the index register, the stack pointer and an
addressing mode are all proved *together* and in the combination real
code uses. `rv32i`'s equivalent runs the same recursion through
`sw ra, 0(sp)` and a conventional frame, and `examples/soc`'s
`sw/hello.s` and `examples/mos6502_computer`'s `sw/hello.s` are a third
kind of end-to-end test: a program that talks to a peripheral through a
memory map, and whose output is checked off a wire.

### A checklist

If you are packaging a core, these are the tests that have to exist:

1. an assembler built from the instruction set's own encoding, not the
   core's;
2. every instruction, in every addressing mode it has, with the
   architectural state read out after each;
3. the flags after every operation that touches them, including the ones
   that must *not* move;
4. every branch given a state that takes it and a state that does not;
5. loads and stores at every width and every offset within the word;
6. the same programs with wait states, proving they cost clocks and not
   architecture;
7. traps and interrupts (§5);
8. the documented quirks, each named (§6);
9. at least one program with a stack and a subroutine, long enough that
   a leak shows;
10. the whole thing at every value of every parameter that could change
    behaviour.

## 4. Cycle accuracy

Only one of the two cores makes a cycle claim, which is itself the first
thing to say: **cycle accuracy is a choice, and most cores should not
make the claim.**

`rv32i` does not. Its header says what it costs and leaves it there:

```
//   Against memories that answer in the cycle they are asked, one
//   instruction takes two clocks and one that touches data memory takes
//   three, plus a clock per wait state either memory adds.
```

That is a performance statement, not a compatibility statement. No
RISC-V program depends on `ADD` taking two clocks; the ISA has no
architectural timing and software that needs to know reads `mcycle`.

`mos6502` does make the claim, because 6502 software counts cycles —
video timing, serial bit-banging, and every loop that was tuned by
someone with the table in front of them. So the core states the count
for every addressing mode in its header:

```
//     implied / accumulator            2
//     immediate                        2
//     zero page                read 3, read-modify-write 5, write 3
//     zero page,X / zero page,Y  read 4, read-modify-write 6, write 4
//     absolute                 read 4, read-modify-write 6, write 4
//     absolute,X / absolute,Y  read 4 (+1 across a page), RMW 7, write 5
//     indexed indirect (zp,X)  read 6,                          write 6
//     indirect indexed (zp),Y  read 5 (+1 across a page),       write 6
//     indirect (JMP only)              5
//     relative (branches)      2, +1 if taken, +1 more across a page
```

### How it is checked

By the reference table, not by the core. The fourth argument of every
row of `TABLE` in `tests/mos6502_asm/mod.rs` is the documented count,
and `p(...)` rather than `i(...)` marks the rows the reference flags
`+1 if a page is crossed`:

```rust
/// The cycle count the 6502's documentation prints for it, before
/// any page-crossing or branch penalty.
pub(crate) cycles: u8,
/// Whether the documentation marks it `+1 if a page is crossed`,
/// which is the indexed *reads* and none of the writes.
pub(crate) page_penalty: bool,
```

and three tests consume it:

- `mos6502_counts_the_cycles_of_every_instruction` — all 143 non-branch
  encodings, each assembled, run, and its cycle count compared with the
  table's. The branches are excluded because their count depends on the
  flags, and they have their own test.
- `mos6502_spends_an_extra_cycle_when_an_indexed_read_crosses_a_page` —
  every encoding with `page_penalty`, at a base that crosses and one
  that does not; and the indexed writes and read-modify-writes proved to
  spend that cycle *either way*, which is why `STA $1234,X` is five
  cycles and `LDA $1234,X` is four.
- `mos6502_times_branches_by_whether_they_are_taken_and_cross` — two
  cycles not taken, three taken, four taken onto another page, forwards
  and backwards.

The measurement itself is `Mos::next`, which counts bus cycles from one
opcode fetch to the next using `sync`. That is the definition the tables
use, and it is the reason `sync` is a port.

### What is claimed and what is not

The core claims **the cycle count, not the cycle shape**, and the
distinction is sharp:

| Claimed | Not claimed |
|---|---|
| an instruction takes the documented number of bus cycles | a bus cycle looks like the original's |
| the addresses are the real ones, discarded reads included | there is a φ1 / φ2 or an address setup window |
| a wait state costs clocks, not cycles | `RDY` lets a write through, as the real part's does |

The "addresses are the real ones" half is not free and is worth calling
out, because it is what makes the cycle count reproducible rather than
merely right in total. `mos6502`'s forty states are each exactly one bus
access, including the ones that read a byte and throw it away:

```verilog
localparam [5:0] S_IMPL   = 6'd1;   // discarded read at PC; implied execute
localparam [5:0] S_ZPIDX  = 6'd5;   // discarded read at $00nn; add the index
localparam [5:0] S_WRFIX  = 6'd13;  // the always-spent cycle of an indexed write
localparam [5:0] S_RMWW1  = 6'd16;  // read-modify-write: write the byte back
```

### When it matters

It matters when software can see it: a 6502, a Z80, anything driving a
display or a serial line by counting instructions, and any core whose
users will run existing binaries. It does not matter for a
general-purpose core in a new system, and claiming it there costs real
design freedom — you can never add a pipeline stage, never cache, never
widen a bus.

If you do claim it, claim the *count* and say so, because claiming the
shape means modelling the real pins and giving up the simple synchronous
bus of §2.

## 5. Interrupts and reset

The two cores split cleanly here, and the split is instructive: on a
RISC-V core reset is a parameter and interrupts are a CSR protocol; on a
6502 reset is an address in ROM and interrupts are a priority ladder in
hardware.

### Vectors

`rv32i` is told where to start:

```text
param RESET_VECTOR bits 32'h00000000
```

and traps go to `mtvec`, which software writes. Both are under the
system's control.

`mos6502` is told nothing. It reads `$FFFC` when reset is released and
jumps there. So the *memory map is part of the core's contract*:

| Address | Vector |
|---|---|
| `$FFFA`–`$FFFB` | NMI |
| `$FFFC`–`$FFFD` | RES |
| `$FFFE`–`$FFFF` | IRQ and BRK |

Those six bytes have to be ROM, because they are read before anything
could have written anywhere. That is why `examples/mos6502_computer`
puts its ROM at the top of the address space and its RAM at the bottom,
and why `tests/mos6502_computer.rs` checks the vectors against the
labels the assembly source gave them:

```rust
assert_eq!(word_at(&image, VEC_RES), image.label("reset"));
assert_eq!(word_at(&image, VEC_NMI), image.label("nmi"));
assert_eq!(word_at(&image, VEC_IRQ), image.label("irq"));
```

A core with fixed vectors should say so in its header, in its manifest
description, and in the example that uses it. A user who discovers it
from a simulation that fetches from `$0000` has been failed by the
package.

### Reset as a sequence

Reset on a real 6502 is not "registers go to a known value"; it is a
seven-cycle sequence that reads three stack slots (rather than writing
them), leaves `S` three lower, sets `I` and jumps through the vector.
`mos6502` reproduces it, and the reproduction is what the test harness
boots through:

```rust
/// A core at its first opcode fetch, the reset sequence behind it.
fn boot(design: &'d Design, source: &str, stalls: u32) -> Mos<'d> {
    let mut cpu = Mos::new(design, source, stalls);
    // RES is seven cycles, like every other interrupt sequence.
    for _ in 0..7 {
        cpu.cycle();
    }
    ...
    assert!(cpu.at_fetch(), "the reset sequence ends in an opcode fetch");
    assert_eq!(cpu.bus_addr(), vector, "and it fetches from the RES vector");
    assert_eq!(cpu.s(), 0xFD, "with S three below where it started");
    assert!(cpu.flag(2), "and I set");
```

Every 6502 test that boots the core asserts those four things before it
runs a single instruction, so a reset regression fails the whole file at
once with a message that names what moved.

Where the real part leaves something undefined, a core has to pick, and
must say which:

```
//   RES leaves N, V, Z, C and D undefined on the real part; here they
//   are cleared, because a hardware register has to start somewhere and
//   "undefined" is not a thing this core is allowed to be.
```

`rv32i` makes the opposite choice in the one place the ISA is explicit
about it — `x1`..`x31` are left undefined after reset, "because clearing
it would cost a reset sequence and a mux, and correct code writes a
register before reading it" — and the test harness panics on reading an
`x`, so the choice is enforced rather than merely documented.

### Priority and masking

`mos6502`'s ladder is `RES > NMI > IRQ > BRK`, and the whole decision is
two lines:

```verilog
wire       take_int  = nmi_pend | (irq & ~p_i);
wire [5:0] nxt_fetch = take_int ? S_INTD : S_FETCH;
```

with the choice of vector made when the sequence starts:

```verilog
// An interrupt is decided at the end of an instruction.
if (last_cycle && take_int) begin
    if (nmi_pend) begin
        int_kind <= K_NMI;
        nmi_pend <= 1'b0;
    end else begin
        int_kind <= K_IRQ;
    end
end
```

Read off that: NMI is not masked by `I` and IRQ is; NMI wins when both
are up; and the decision is made at `last_cycle`, the end of an
instruction, never in the middle of one.

`rv32i`'s is the RISC-V rule, which is a mask register rather than a
ladder:

```verilog
wire irq_now = mstatus_mie & (irq_active != 32'd0);
```

with `mip` not a register at all — bits 11, 7 and 3 are the three input
pins — and external, timer and software taken in that order.

### Edge against level

`nmi` is edge triggered and `irq` is level triggered, which is not a
detail: it decides whether a pulse can be lost.

```verilog
// The NMI edge is watched whatever the bus is doing, and a
// fresh edge in the cycle one is serviced is kept.
nmi_q <= nmi;
if (nmi & ~nmi_q) nmi_pend <= 1'b1;
```

The latch is outside the `if (ready)` guard — the edge is caught even
while the bus is stalled — and it is set again if a new edge arrives in
the cycle the old one is serviced.

### The shapes worth testing

Every one of these is a separate test in `tests/ip_library.rs`, and
every one of them is a bug a core can have while passing all the
per-instruction tests:

| Shape | Test |
|---|---|
| taken between instructions, with the sequence's own cycle count | `mos6502_takes_an_irq_between_instructions` |
| the frame on the stack: return address, status byte, and which bit 4 | same, and `mos6502_breaks_and_returns_from_the_interrupt` |
| masked and *stays* masked | `mos6502_masks_an_irq_with_the_interrupt_flag` (300 cycles with `I` set, nothing taken) |
| a level that stays high fires again after the return | `mos6502_takes_an_irq_between_instructions` |
| an edge held high counts once, not many | `mos6502_takes_an_nmi_on_its_edge_and_through_the_mask` |
| a pulse narrower than an instruction is latched, not lost | same |
| both raised at once | `mos6502_prefers_an_nmi_to_an_irq` |
| the mask's one-instruction delay | `mos6502_delays_the_effect_of_cli_and_sei_by_one_instruction` |
| a software trap using the same vector, told apart by the frame | `mos6502_breaks_and_returns_from_the_interrupt` |
| return restores the flags it should and no others | same |
| each interrupt source has its own cause | `rv32i_takes_a_timer_interrupt_and_returns_from_it` |
| a synchronous exception blames the right instruction | `rv32i_traps_on_a_misaligned_access_and_on_nonsense` |

Three of those rows are where most cores go wrong. The edge against the
level, because a core that treats NMI as a level takes it twice and one
that forgets to latch it loses a short pulse. Bit 4 of the pushed status
byte, because a handler shared between `BRK` and `IRQ` reads it and
nothing else in the machine does. And the mask's one-instruction delay:
an IRQ pending when `SEI` runs is *still taken* after it, because the
decision for an instruction is made with the flags as they were before
it, and that is invisible to every test that does not raise the line at
an exact instruction boundary.

### What neither core does

Say this too. `mos6502`:

```
//   No interrupt hijacking: an NMI arriving during a BRK does not
//   redirect it to the NMI vector, as it does on the real part. No
//   interrupt polled inside a taken branch, so the "branch delays an
//   IRQ" corner is not reproduced. Interrupts are decided once, at the
//   end of each instruction; nothing is sampled mid-instruction.
```

`rv32i` has no `mideleg`, no `medeleg`, no `mtval`, and `mtvec` is
direct mode only. Both cores sample their interrupt inputs on `clk` and
require them to be synchronous already — "put `cdc_sync` in front of one
that is not" — which is the right division of labour: a core is not the
place for a synchroniser, and a library that has one should point at it.

## 6. Quirks and compatibility

A compatible core reproduces the documented bugs. This is not
sentimentality; it is the definition of compatible. Software worked
around the bugs, and software that worked around a bug breaks on a core
that fixed it — usually far from where the fix is, and usually silently.

### The worked example

`JMP ($10FF)` on an NMOS 6502 reads the low byte of its target from
`$10FF` and the high byte from `$1000`, not `$1100`: the pointer's low
byte is incremented without a carry into the high one. The core's state
names say what it does:

```verilog
localparam [5:0] S_JMPL   = 6'd10;  // JMP (): target low
localparam [5:0] S_JMPH   = 6'd11;  // JMP (): target high, low byte wrapped
```

and the test proves it from the **bus trace**, which is the only place
the difference is visible before the jump lands:

```rust
cpu.set_byte(0x10FF, 0x00);
cpu.set_byte(0x1000, 0x03); // what the part actually reads
cpu.set_byte(0x1100, 0x04); // what a core without the bug would read
cpu.record();
let took = cpu.next();
assert_eq!(took, 5, "an indirect JMP is five cycles");
assert_eq!(
    cpu.addresses(),
    vec![0x0200, 0x0201, 0x0202, 0x10FF, 0x1000],
    "the second pointer byte comes from the start of the same page"
);
assert_eq!(cpu.bus_addr(), 0x0300, "so the jump lands at $0300");
cpu.run_to_done(40);
assert_eq!(cpu.a(), 0x01, "which is the branch a compatible core takes");
```

Three independent witnesses in one test: the cycle count, the exact
sequence of addresses on the bus, and the value the program computes
afterwards. A core that fixed the bug fails all three, and the failure
message names the quirk.

### The rest of the list

`mos6502`'s header carries a section of them, each with the name of its
test beside it, so a reader who wonders "does this core do the thing
where…" finds the answer and the proof in the same place. Abridged to
the quirk and the test — the source spells each one out:

```
//     * the indirect JMP page-boundary bug.
//       (`mos6502_reproduces_the_indirect_jmp_page_bug`)
//     * the extra cycle when an indexed *read* crosses a page.
//       (`mos6502_spends_an_extra_cycle_when_an_indexed_read_crosses_a_page`)
//     * branch timing.
//       (`mos6502_times_branches_by_whether_they_are_taken_and_cross`)
//     * a read-modify-write writes twice.
//       (`mos6502_writes_a_read_modify_write_byte_back_before_the_result`)
//     * the stack wraps inside page one.
//       (`mos6502_wraps_the_stack_inside_page_one`)
//     * zero-page indexing wraps inside page zero.
//       (`mos6502_wraps_zero_page_indexing_and_its_pointers`)
//     * the one-instruction delay of CLI, SEI and PLP.
//       (`mos6502_delays_the_effect_of_cli_and_sei_by_one_instruction`)
```

One of those is not a bug at all but *observable behaviour* a
simplification would destroy: a read-modify-write writes its address
twice — `INC`, `DEC` and the four shifts read the byte, write back what
they read, then write the result. A memory-mapped register can see
that, and some hardware was built to exploit it. A core that wrote once
would be faster and wrong.

The same family holds one thing that is not in the list because it is
not a quirk but the definition of the register: the status byte's bit 4
is not a flag at all but a property of *how the byte reached the stack*.
`PHP` and `BRK` push it set, an IRQ, an NMI and the reset sequence push
it clear, and `PLP` and `RTI` ignore bits 4 and 5 of what they pull.
That is how a handler tells a `BRK` from an interrupt, since the two
share a vector, and `mos6502` models it by not having a B register at
all — `mos6502_pushes_and_pulls_the_status_byte` holds it.

### Decimal mode

`mos6502`'s decimal arithmetic is where "reproduce the documented
behaviour" gets uncomfortable, because the NMOS part's flags after a
decimal `ADC` look like a bug and are not:

```
//     ADC  the accumulator and C are the decimal result and its carry,
//          but Z is the *binary* sum's, and N and V come from the
//          intermediate after the low nibble is corrected and before
//          the high one is — so SED / SEC / LDA #$99 / ADC #$00 leaves
//          A = $00 with C = 1 and Z = 0.
//     SBC  the accumulator is the decimal difference, and N, V, Z and C
//          are all exactly the binary subtraction's.
```

`mos6502_adds_and_subtracts_in_decimal_mode` drives fourteen known `ADC`
results and nine known `SBC` ones against that, then runs all of them
again with `DECIMAL_MODE = 0` where the arithmetic is binary and `D` is
still a flag. This is the part of a 6502 that most implementations get
wrong, which is exactly why it has a table of known-good answers rather
than a property.

### Where to draw the line

Not every quirk is worth reproducing, and the core says which it
declined:

```
//   The 105 undocumented opcodes are **out of scope and defined
//   anyway**: every one of them behaves exactly like NOP — two cycles,
//   one discarded read of the byte after the opcode, no register and no
//   flag touched. That is deterministic and documented, but it is not
//   what the NMOS part does, so a program that uses LAX or SLO will run
//   off the rails here. It will do so the same way every time.
```

Three things to copy from that paragraph. It says what is not done; it
says what happens instead; and it makes the substitute *deterministic
and tested*
(`mos6502_treats_an_undocumented_opcode_as_a_nop`). "Undefined" would
have been easier and much worse, because a user's program would fail
differently on different days.

The same paragraph rules out the CMOS successor and says why the
differences are differences: "No 65C02 and no 65816: none of the added
opcodes, none of the added addressing modes, and none of the fixes — the
indirect JMP bug is *kept*, and decimal mode does not clear V or cost
the extra cycle the CMOS part spends on it."

## 7. Resource cost

A footprint in a README goes stale the first time someone optimises
something. The library's is generated from a test, so it cannot.

`footprints_match_the_documentation` in `tests/ip_library.rs` measures
every block at the parameters a table of variants names, renders the
whole table, and compares it with the text between two markers in
`docs/ip-library.md`:

```rust
const TABLE_BEGIN: &str = "<!-- footprints: generated by tests/ip_library.rs -->\n";
const TABLE_END: &str = "<!-- end footprints -->\n";
```

A mismatch fails the test with the first differing line, documented
against measured, and `UPDATE_EXPECT=1` rewrites the document. So the
numbers are always the ones this crate's own synthesis produces today,
and a change that doubles a block's size shows up as a diff in a pull
request rather than as a surprise on a board.

Each variant is four rows — `LUT4` and `LUT6` through `synth::run` plus
`techmap::map_module`, then the whole `fpga::synthesize_for` flow for an
iCE40 HX1K and an ECP5 45F — and each core is listed at both settings of
its cost parameter, so eight rows each. Every block is measured **as the
top of its own design**, so every port takes an IO buffer; dropped into
a system the buffers disappear and the logic does not.

### What the two cores cost

From `docs/ip-library.md`, trimmed to the LUT counts:

| Core | Parameters | LUT4 | LUT6 | iCE40 HX1K | ECP5 45F |
|---|---|---|---|---|---|
| `rv32i` | `REGFILE_BRAM=0` | 2436 (depth 34) | 2044 (28) | 5047 `SB_LUT4` (36) | 2486 `LUT4`, 32 `TRELLIS_DPR16X4` (34) |
| `rv32i` | `REGFILE_BRAM=1` | 2445 (depth 34) | 2075 (28) | 2355 `SB_LUT4`, 4 `SB_RAM40_4K` (34) | 2445 `LUT4`, 4 `DP16KD` (34) |
| `mos6502` | `DECIMAL_MODE=1` | 1710 (depth 18) | 1336 (18) | 1733 `SB_LUT4`, 179 `SB_CARRY` (16) | 1720 `LUT4` (16) |
| `mos6502` | `DECIMAL_MODE=0` | 1646 (depth 18) | 1302 (18) | 1641 `SB_LUT4`, 125 `SB_CARRY` (16) | 1661 `LUT4` (16) |

Four things are worth reading out of that.

**An 8-bit core from 1975 is about seven tenths of a 32-bit one.** That
is not what most people guess. Thirteen addressing modes and a forty-state
sequencer cost real logic, and a 32-bit datapath is mostly wide but
shallow. The LUT *depth* is where the difference shows honestly: 34
against 18 in the LUT4 rows.

**A parameter that compiles out cost is measurable.** Decimal mode is
**64 more LUT4 and 34 more LUT6** — about four per cent either way — and
on the iCE40 92 more `SB_LUT4` and 54 more `SB_CARRY`. The depth does
not move, because the decimal correction is beside the binary adder
rather than after it. Four per cent is a small number, and the table is
how you find that out *before* deciding whether the parameter earns its
place.

**Where storage goes matters more than anything else.**
`REGFILE_BRAM=0` on an HX1K is 5047 `SB_LUT4`, because the iCE40 has no
distributed RAM primitive and a 32 x 32 register file with two
asynchronous read ports becomes flip-flops and multiplexers. Setting it
to `1` is 2355 `SB_LUT4` and four block RAMs — less than half the logic
— for no extra cycle. The ECP5 row shows the same core on a family that
*does* have distributed RAM (`TRELLIS_DPR16X4`) and so barely cares.
This is exactly the kind of thing a measured table tells you and a
guessed one does not.

**Neither core fits an HX1K in a system.** `rv32i` at 2355 `SB_LUT4` and
`mos6502` at 1733 both exceed the HX1K's 1280, which is why both worked
examples target the HX8K. The table is where that was found out, before
anyone wrote a top level.

In a real system the numbers come down, because the IO buffers go away
and the cores are flattened with everything else. On the HX8K:

| | `examples/soc` | `examples/mos6502_computer` |
|---|---|---|
| core | `rv32i`, `REGFILE_BRAM=1` | `mos6502`, `DECIMAL_MODE=0` |
| `SB_LUT4` | 2368 | 1714 |
| flip-flops | 372 | 151 |
| `SB_RAM40_4K` | 10 | 6 |
| LUT depth | 41 | 23 |

## 8. Dropping the core into a system

Both examples are the same shape: a project manifest naming the core and
a UART by path, and one file of user HDL.

```text
depends rv32i   ^1.0.0 path ../../ip/rv32i        # examples/soc
depends mos6502 ^1.0.0 path ../../ip/mos6502      # examples/mos6502_computer
depends uart    ^1.0.0 path ../../ip/uart         # both
```

- **[`examples/soc`](../examples/soc)** — `rv32i` and `uart`, a ROM and
  a byte-writable RAM behind two ports, a memory-mapped UART, and a
  RISC-V program that prints a line. Read this one for the two-port
  case: `soc_top` shares a single ROM port between the buses, which the
  multi-cycle core allows because it fetches or accesses data, never
  both in one cycle.
- **[`examples/mos6502_computer`](../examples/mos6502_computer)** —
  `mos6502` and `uart`, one bus, RAM at the bottom because zero page and
  the stack have to be there, ROM at the top because the vectors have to
  be there, and a 6502 program that prints the same line. Read this one
  for the one-bus case and for the vector story.

Both are driven end to end by a test — [`tests/soc.rs`](../tests/soc.rs)
and
[`tests/mos6502_computer.rs`](../tests/mos6502_computer.rs) — through
manifest resolution, elaboration, synthesis with no latch, a simulation
whose output is decoded off the serial pin, and the iCE40 flow that
exports what `nextpnr-ice40` reads with the program in the ROM's block
RAMs. Neither has been run on a board, and both say so.

The checklist for the integration side:

1. **one wait state, not one cycle.** A block RAM is read on a clock
   edge, so `*_ready` goes high in the cycle *after* the request. Both
   examples do this and both explain it in the top level.
2. **decode from the address the core is holding.** The core keeps the
   address stable until the transfer, so a combinational decode off it is
   valid at the transferring edge; there is nothing to latch.
3. **the program is data.** `$readmemh` from a `.hex` file, generated by
   a test from the assembly source, with the test failing if the
   checked-in hex has drifted. Synthesis loads it into the memory's
   initial contents through `SynthOptions::files`, and the iCE40 block
   RAM mapping carries it into `INIT_*` — so the exported netlist has
   the program in it.
4. **tie off what you do not use.** `irq`, `nmi` and the `dbg_*` ports
   cost nothing when nothing drives or reads them.
5. **check the string, not the register.** Both testbenches decode the
   serial line from the pin's waveform. A test that read `tx_byte`
   inside the design would pass on a system whose UART never transmits.

## 9. What is not here

Honest gaps, so nobody plans around them.

- **No core declares a bus interface** (§1). The two-wire handshake is
  not one of the built-in buses, so `bus::match_ports` has nothing to
  check. A core presenting AXI4-Lite would get that for free; one
  presenting a house handshake would need a `.bus` file first.
- **No formal verification of either core.** Reticle has a SAT solver,
  bounded model checking and k-induction
  ([`docs/equivalence.md`](equivalence.md)), and neither core has been
  put through them. The tests are simulation.
- **No cycle-shape model, on either core** (§4), and no two-phase 6502.
- **No undocumented 6502 opcodes, no 65C02, no 65816** (§6).
- **No M extension, no C extension, no supervisor mode, no PMP** on
  `rv32i`; machine mode only, `mtvec` direct mode only, misaligned
  accesses trap rather than being emulated.
- **No board run.** Both examples stop at the files `nextpnr-ice40`
  reads, for the reasons in [`docs/fpga.md`](fpga.md).
- **No timing closure claim.** Nothing checks that either system closes
  at 12 MHz on a real part.
