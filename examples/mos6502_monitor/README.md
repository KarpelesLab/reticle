# A 6502 you can type at, over USB

A machine-language monitor on a 6502, with its console on a USB serial
port. Plug a Great Scott Gadgets Cynthion into a host, open
`/dev/ttyACM1`, and a backslash appears; type an address and it tells you
what is there, type `XXXX: dd dd` and it puts bytes there, type `XXXXR`
and it runs them. The processor is [`mos6502`](../../ip/mos6502), the
serial port is [`usb_cdc_acm`](../../ip/usb_cdc_acm) on the board's ULPI
transceiver, and the baud divider is [`uart`](../../ip/uart)'s. Five
files in `rtl/` and one 6502 program in `sw/` are the only design here,
and one of the five is the program assembled.

This is [`examples/mos6502_computer`](../mos6502_computer) grown a
keyboard. That machine prints one line down a wire and stops; this one
has a memory map, an ACIA, and 272 bytes of software that answers.

**The monitor reproduces a well-known interface and is not a copy of
anybody's implementation of it.** Apple's 1976 Woz Monitor is Apple's and
Ben Eater's port of it is his; neither was read to write `sw/monitor.s`,
no disassembly or hex dump of either was consulted, and not one byte of
either is in this directory. What is here was written from a
*description* — the prompt, the items, the output format — and then
**checked against a running original**, which is a different thing from
copying one. [Below](#the-oracle-what-was-checked-against-a-running-original)
is exactly what was run, what matched, and what could not be checked that
way. That is the same position [`examples/apple2`](../apple2) takes about
the Apple II's memory map and soft switches, for the same reason: an
interface is a published fact about a machine rather than anybody's
creative expression of it, and that example's own monitor ROM is where
this one's hexadecimal printing and line editor come from.

**What this proves and what it does not is at the end of this page**,
including the terminal session it was used from and the one measurement
that could not be taken.

## What is here

```text
examples/mos6502_monitor/
  rtl/monitor_cynthion.v  the board: the three-state buffers, the reset, six LEDs, ball C11
  rtl/monitor_ulpi.v      the machine behind the USB serial port, with the ULPI bus outside
  rtl/monitor_machine.v   the machine: the 6502, the RAM, the ROM, the decoder
  rtl/monitor_acia.v      a 65C51-style ACIA whose wire is a USB pipe
  rtl/monitor_rom.v       the monitor, assembled, as 505 lookup tables' worth of constant
  sw/monitor.s            the monitor, in 6502 assembly
  tb/monitor_bench.v      the machine with its byte interface bare, for tests/mos6502_monitor.rs
  tb/monitor_tb.v         a session written in Verilog, for `reticle sim`
  board/cynthion.rcf      thirteen ULPI balls, an oscillator and six LEDs
```

## The memory map

It is a published 6502 breadboard computer's, decoded by one quad NAND
gate. Written out as the chip-select equations that gate actually
computes:

```text
    ROM  /CE = A15                      -> $8000-$FFFF
    RAM  /CE = A15, /OE = A14           -> $0000-$3FFF
    VIA  CS1 = A13, /CS2 = !A15 & A14   -> $6000-$7FFF
    ACIA CS0 = !A13, /CS1 = !A15 & A14  -> $4000-$5FFF
```

| Address | What |
|---------|------|
| `$0000`–`$3FFF` | RAM. Page zero is the monitor's variables, page one the stack, `$0200` the line being typed |
| `$4000`–`$5FFF` | the ACIA, four registers repeating. `$5000` is the spelling everything uses |
| `$6000`–`$7FFF` | where the board's timer would be. Nothing here; it reads zero |
| `$8000`–`$FFFF` | ROM. `$FE00`–`$FFFF` answers and the rest reads zero |

Two rows of that are not choices, they are the part. Zero page and the
stack have to be RAM, because `$0000`–`$00FF` is the 6502's fastest
addressing mode and `$0100`–`$01FF` is the stack with eight bits of
pointer and a hard-wired `$01` in front. And `$FFFA`–`$FFFF` has to be
ROM, because the core reads `$FFFC` when reset is released and jumps
there, before anything could have written anywhere.

Three of the four windows differ from the board, and each difference is
something this part forced rather than something that was simplified:

* **The RAM window is 16 KiB and the array is 4 KiB**, so it repeats four
  times over — `$0300` and `$1300` are the same byte. That is not a hole
  and not a simplification: it is what a smaller part in that socket
  does, because the decoder does not look at the address bits the part
  has not got. Why 4 KiB is [measured below](#resource-use).
* **The ROM window is 32 KiB and two pages of it are built.** A ROM on
  this part is lookup tables, so 32 KiB of one is not affordable; see
  the same section.
* **There is no timer at `$6000`.** The board has a 65C22 there and
  nothing in this machine touches it.

## The monitor

`sw/monitor.s` is 266 bytes of code and six of vectors. A line is parsed
left to right as a series of **items**; an item is a run of hexadecimal
digits, or one of `.`, `:` and `R`. The parser has a mode, reset to
EXAMINE at the start of every line:

| Mode | Set by | A number means |
|---|---|---|
| EXAMINE | the start of a line | an address: print it and the byte there, and remember it |
| BLOCK | `.` | the end of a range: print from one past the last byte printed up to it, then go back to EXAMINE |
| STORE | `:` | a byte: write it where the last store left off, and stay in STORE until the line ends |

and `R` runs from the address last examined. So:

```text
FE00                  one byte
FFFA.FFFF             a range
0300: A9 2A 60        a deposit
.0307                 more of the last range
: 60                  more of the last deposit
0300R                 run it
```

Anything below `.` in the character set — a space, a comma, an asterisk —
separates items and is otherwise ignored. Anything else ends the line
with a fresh `\`. A new address label starts a line whenever the address
has its low three bits clear, so a range comes out eight bytes to a row.

**Two behaviours are quirks rather than decisions**, and both were
confirmed against a running original rather than invented here:

* `XXXX: dd` prints the byte that **was** at `XXXX`, before the deposit.
* `XXXXR` prints the byte at `XXXX` before jumping there.

Both fall out of the same structure: the address item ends while the mode
is still EXAMINE, so it is examined, and the `:` or the `R` only then
does its own work. Reproducing them was a choice; discovering that they
happen was a measurement.

### Where each behaviour came from

| Behaviour | Where it came from |
|---|---|
| `\` as the prompt, with a carriage return after it | described, and **confirmed** |
| a carriage return printed *before* every line rather than after | **confirmed** — the byte order was read off a recorded transcript |
| `XXXX` examines; `XXXX.YYYY` a range; `XXXX: dd dd` deposits; `XXXXR` runs | described, and **confirmed** |
| a bare `.YYYY` and a bare `: dd` continue | described, and **confirmed** |
| rows of eight, broken where the low three address bits are clear | **confirmed**; the description says "rows with an address label" and not where they break |
| a range that runs backwards prints one byte and stops | **confirmed** |
| an unparseable line is refused with a fresh `\` and nothing else | described, and **confirmed** |
| a good address followed by a bad character is examined *first*, then refused | **confirmed** — inferred from the parser's shape, then measured |
| lower case is refused rather than folded up | **confirmed** |
| more than four hexadecimal digits keeps the last four | **confirmed** |
| more than two digits in a deposit keeps the last two | **confirmed** |
| **backspace is `$08`** and not `_` | **chosen, after measuring both.** See below |
| escape (`$1B`) cancels the line, and is echoed before it is acted on | described, and **confirmed** |
| a line stops growing at 128 characters, the 129th replacing the 128th | **chosen here.** Not measured, and not what the original does |
| all three vectors point at the reset entry | **chosen here**, because there is no room for a handler |

**On backspace.** The task description said `_`, and that is right for the
1976 machine: an Apple I keyboard had no backspace key at all, so `_` was
the character available. The published 65C02 port this monitor was
checked against uses **`$08`**, the ASCII BS a terminal sends — and both
were measured: typing `0399_8.039A` at it examines `$0399` and then
refuses the `_`, while `0399<BS>8.039A` examines `$0398` through `$039A`.
This machine's console is a terminal emulator and not a 1976 keyboard, so
`$08` is the one that works when a person presses the key marked
backspace. `_` is not accepted at all, and that is a deliberate
difference from the older of the two originals.

### How much of `examples/apple2/sw/monitor.s` was reused

That example's monitor is this project's own 6502 monitor and it was read
first, as the task asked. What came across, re-spelled for a quarter of
the space:

* **`prbyte` and `prnib`** — the two-nibble hexadecimal printer, including
  the `adc #$06` that turns 10 into `A` by exploiting the carry the
  comparison left. Shortened by making `prnib` fall into `echo` instead of
  jumping to `cout`.
* **The shape of `gethex`** — accumulate digits by shifting the value left
  four and OR-ing the digit in. The four shifts are a loop here rather
  than four unrolled pairs, and the digit test is a different routine (see
  below).
* **The shape of `getln`** — read into a buffer, echo as you go, handle
  backspace by stepping the index back, end on a carriage return.
* **The `(ptr),y` idiom** for reaching a computed address through page
  zero, which becomes `(ptr,x)` here because X is pinned at zero.

What did **not** come across: the screen driver, the scroller, the cursor,
the line table, the attribute handling and the test card, all of which are
about a video display this machine has not got; and `hexval`, which is
four range comparisons there and the ordinary `eor #$30` idiom here
because 22 bytes would not fit.

## The ACIA, and which bits are real

### Why an ACIA and not a PIA

The 1976 machine this interface comes from has no serial port at all: its
console is a **6821 PIA** at `$D010`–`$D013`, one port for a keyboard and
one for a character display, with a key-ready bit and a display-busy bit
and no notion of speed. That is what a monitor of this shape was written
against, and it is the obvious thing to build.

It is not what was built, for two reasons and one of them is measured.

* **The machine this memory map comes from uses a 65C51 ACIA**, at
  `$5000`, and matching the map exactly costs nothing. A 65C02 core here
  later could then run somebody's own copy of a published ROM without any
  of it ever entering this repository.
* **A PIA has nowhere for a baud rate to land.** The other half of this
  work is making a host's `SET_LINE_CODING` rate real, and a PIA's four
  registers have no field that could hold one. An ACIA's CONTROL register
  does, so the rate the host sets is a number the 6502 can read — which
  is a more honest machine than one where the rate exists only in a wire
  nothing inside can see.

The cost is stated rather than hidden: **the 65C51's baud field cannot
express 115200**, and what this design does about that is the next
section. A PIA would not have had the problem, because it would not have
had the register.

`rtl/monitor_acia.v` is the W65C51N's four registers from its data sheet,
with a byte stream on the other side instead of a serial line.

| RS | Name | Read | Write |
|---|---|---|---|
| `00` | DATA | the byte received, and the read takes it | a byte to send |
| `01` | STATUS | below | a programmed reset |
| `10` | COMMAND | the last byte written | DTR, the interrupt enables, parity |
| `11` | CONTROL | the last byte written | stop bits, word length, receiver clock, baud |

STATUS, bit by bit:

| Bit | Name | Here |
|---|---|---|
| 7 | IRQ | **read-as-zero.** Nothing in this machine raises an interrupt |
| 6 | DSR (active low) | **read-as-zero**, meaning ready. There is no modem |
| 5 | DCD (active low) | **read-as-zero**, meaning carrier present, for the same reason |
| 4 | TDRE | **real.** High when the transmit holding register is free |
| 3 | RDRF | **real.** High when a byte has arrived and has not been read |
| 2 | Overrun | **read-as-zero**, and it cannot happen: `rx_ready` is low while RDRF is high, so the block upstream holds the byte |
| 1 | Framing error | **read-as-zero.** There are no frames; a bulk endpoint delivers bytes or does not |
| 0 | Parity error | **read-as-zero** |

Every bit of COMMAND and every bit of CONTROL except the four baud bits
is **stored and not acted on**: a program can write one and read it back,
and nothing downstream looks. That is the honest shape for a register
whose effect has nowhere to land — there is no parity to generate, no
second stop bit to hold and no RTS pin to drive.

**TDRE is the correct behaviour and not the erratum.** Real W65C51N
silicon leaves TDRE permanently set, so a polling loop never waits and
characters are lost. That is a defect of a part rather than a feature of
the interface, and modelling it here would only teach software to spin on
a timer instead — which is what the published port of this monitor does,
and which this one deliberately does not.

### The baud rate, which is where the two halves of this work meet

CONTROL's low four bits select a bit rate from a fixed table: sixteen
codes from 50 baud to 19200, with code 0 meaning *"the receiver and
transmitter are clocked from outside this part"* rather than naming a
rate.

**That table cannot express 115200.** It is four bits wide and it stops at
19200. So the block does two separate things with a host's `dwDTERate`:

1. **It writes a code into CONTROL when the host changes rate** — the
   matching one if the table names the rate, and **code 0 if it does
   not**. Code 0 is not a fudge: the data sheet's own meaning for it is
   "clocked externally", and a USB serial bridge is exactly a part whose
   rate arrives from outside. A 6502 reading CONTROL learns something
   true either way.
2. **It reports the rate the part is programmed to** to whatever wants to
   put a real waveform on a pin, with code 0 resolving to the host's own
   `dwDTERate`.

A write by the processor wins until the host moves again, which is the
only arbitration rule that needs no arbiter, and the load is an edge on
the *rate* rather than a level on the code — so two rates the table
flattens to the same code still count as two changes.

`monitor_cynthion` then puts that rate on **ball C11** as real 8N1,
through [`uart_baud_div`](../../ip/uart). Everything the processor prints
goes there as well as to the host, so a bit period on C11 is a number an
instrument reads off a pin — and it is the only thing in this design that
a wrong divisor cannot hide from, because a USB pipe has no bit rate and
would not notice.

The mirror is a **diagnostic and not a channel**: one byte of holding
register, overwritten if the line is still busy. At 300 baud a character
takes 33 ms and the monitor prints faster than that, so most of what it
prints is dropped. That is the right trade — the alternative is a console
whose speed is set by an LED.

### Where the ACIA's facts came from

Every register layout above is **quoted** from a W65C51N data sheet and
not measured: there is no 65C51 here to compare against, and there could
not be, because this one's other side is a USB pipe. What *is* measured is
that the layout is self-consistent and that software written for it works
— the monitor polls bit 4 before every character and bit 3 before every
key, and it runs.

Three things in this block are **inferred rather than read**, and are
marked so that a reader with the data sheet open knows which sentences to
check:

* **That code 0 is the right answer for a rate the table cannot name.**
  The data sheet says code 0 selects an external receiver clock at
  sixteen times the bit rate. Calling a USB host "external" is this
  design's reading of that, not the data sheet's sentence.
* **That codes 3 and 4 answer to 110 and 134.** The data sheet's numbers
  are 109.92 and 134.58 — the teleprinter rates a 1.8432 MHz crystal
  divides to — and hosts ask for the rounded ones. No host was observed
  asking for either.
* **That a programmed reset leaves CONTROL alone.** The data sheet lists
  what a write to the status register clears; whether the control register
  is among them is read here as "no", because a rate a host set surviving
  a program resetting the part is the behaviour that matters on this
  board. If that is wrong about the part, it is deliberately wrong.

### The rate a rate cannot be

`uart_baud_div` divides 60 000 000 by the rate and rounds to nearest, so
the error is at most half a clock in a bit — `0.5 / div`. It refuses any
divisor under **30**, because half a clock in 30 is 1.67% and 8N1's
practical budget is about 2%; a divisor of 4 would be 12.5%, which is a
port that reports success and drops every character. A rate of zero, a
rate whose divisor will not fit sixteen bits, and a rate fast enough that
the rounding would break framing all get the same answer: the divisor
falls back to the parameter and `ok` goes low.

| Rate | Divisor | Actual | Error |
|---|---|---|---|
| 9600 | 6250 | 9600.00 | 0.000% |
| 19200 | 3125 | 19200.00 | 0.000% |
| 115200 | 521 | 115163.15 | −0.032% |
| 230400 | 260 | 230769.23 | +0.160% |
| 921600 | 65 | 923076.92 | +0.160% |
| 2000000 | 30 | 2000000.00 | 0.000% |
| 3000000 | — | refused | a divisor of 20 is 2.5% |

The **worst case it accepts is 1.67%**, at any rate whose divisor lands on
30 exactly.

## When the machine is switched on

The 6502 is held in reset until the host has **configured** the port, and
put back into it by a bus reset, because `configured` goes low on one.

That is not a nicety, and it was measured on the part before it was
fixed. The monitor prints its prompt about four milliseconds after it
starts — 240 000 clocks, which
`the_processor_runs_at_one_cycle_in_fifty_nine` prints — and enumeration
takes longer than that. So a machine released at power-on printed `\`
into an endpoint that a bus reset then cleared, and **the first thing a
person saw when they opened the port was nothing at all**: no prompt, no
sign of life, until they typed something that happened to produce output.
Holding it until the port exists means the prompt is produced into a live
endpoint, buffered there, and delivered the moment a terminal reads.

It also matches what the machine is: a computer whose power comes from
the port. Unplugging it and plugging it in again is a cold start, which is
what it looks like from the other end too.

## The processor's speed, and a multi-cycle path

One bus cycle every 59 clocks: `ready` is high for one clock in 59 and
the core holds the access through the rest, which is what `ready` is for.
At 60 MHz that is **1.0169 MHz**.

The RAM and the ROM are read **asynchronously** — `ram[addr]` in a
continuous assignment, not `q <= ram[addr]` on an edge — for two reasons,
one about this flow and one about the machine:

1. A clocked read is what makes `fpga::primitives` choose a block RAM,
   and **there is no block RAM site on this fabric**: the design would
   map onto `DP16KD` and then fail to place with *"the design needs N
   `bram` site(s) and the part has 0"*. An asynchronous read is declined
   by the block-RAM step and lowered to `TRELLIS_DPR16X4` for the RAM and
   to lookup tables for the ROM, which are the only two things this part
   has.
2. It is also what the machine being modelled does. A 6502 with static
   RAM and an EPROM presents an address and reads what comes back.

That makes the path from `addr` through the read multiplexer to the
core's capture flip-flop a **multi-cycle path**: the address is stable for
all 59 clocks of the bus cycle and only the last one matters. Static
timing analysis does not know that, and **the design works on the part**
— which is the measurement that settles it, and is [below](#on-the-part).

## Building

From the repository root, because there is no library search path and a
design instantiating library IP must name its sources:

```sh
reticle fetch prjtrellis-db
reticle fpga examples/mos6502_monitor/rtl/monitor_rom.v \
    examples/mos6502_monitor/rtl/monitor_acia.v \
    examples/mos6502_monitor/rtl/monitor_machine.v \
    examples/mos6502_monitor/rtl/monitor_cynthion.v \
    ip/mos6502/rtl/mos6502.v \
    ip/uart/rtl/uart_tx.v ip/uart/rtl/uart_baud_div.v \
    ip/usb_cdc_acm/rtl/usb_cdc_acm.v \
    ip/usb_cdc_acm/rtl/usb_cdc_acm_ulpi.v \
    ip/usb_cdc_acm/rtl/usb_cdc_req.v \
    ip/usb_device_ulpi/rtl/usb_ulpi_link.v \
    ip/usb_device_fs/rtl/usb_ctrl_ep.v \
    --device ecp5-12f-CABGA256 \
    --constraints examples/mos6502_monitor/board/cynthion.rcf \
    --bitstream /tmp/monitor_cynthion.bit
reticle program --list
reticle program --device <serial> /tmp/monitor_cynthion.bit
```

From this directory, `reticle build --synth reticle.proj` resolves the
manifest and synthesises it without going as far as a bitstream.

To change the monitor, edit `sw/monitor.s` and regenerate the ROM:

```sh
UPDATE_EXPECT=1 cargo test --all-features --test mos6502_monitor monitor_rom
```

The assembler is [`tests/mos6502_asm/mod.rs`](../../tests/mos6502_asm/mod.rs),
the same opcode matrix that runs `mos6502`'s own tests, built from the
**documented encoding table** and never from the core's decoder — which is
what stops a wrong core and a wrong test agreeing with each other.

## Testing

From the repository root:

```sh
cargo test --all-features --test mos6502_monitor
```

| Test | What it shows |
|------|---------------|
| `monitor_rom_is_the_assembled_source` | `rtl/monitor_rom.v` is `sw/monitor.s` assembled, inside the ROM, with the three vectors pointing at the label the source names |
| `the_monitor_uses_no_instruction_the_core_has_not_got` | every opcode in the ROM is one of the 151 documented NMOS encodings |
| `the_monitor_prompts_when_it_is_reset` | a backslash and a carriage return, from a cold start |
| `the_monitor_examines_one_location` | `FE00` answers with the byte the ROM holds there |
| `the_monitor_examines_a_range` | `FFFA.FFFF` answers with the six vector bytes, on one line, and they are the vectors the ROM was assembled with |
| `the_monitor_breaks_a_range_every_eight_bytes` | a range across `$0300` starts a new labelled line there and nowhere else |
| `the_monitor_deposits_bytes_and_reads_them_back` | `0300: ...` writes, and a range reads back what was written |
| `a_bare_colon_and_a_bare_dot_carry_on_from_the_last_one` | the two continuations |
| `the_monitor_runs_what_was_deposited` | `0300R` transfers control, and a deposited program prints through the ACIA |
| `the_monitor_rejects_a_line_it_cannot_parse` | four bad lines, each answered with a fresh `\` and nothing else |
| `the_monitor_edits_a_line_with_backspace_and_cancels_it_with_escape` | the two editing keys, including a backspace at the left margin |
| **`the_whole_session_matches_the_one_a_real_monitor_gives`** | **the oracle** — see below |
| `the_acia_reports_the_rate_the_host_set` | six rate changes reach CONTROL's baud bits, the processor reads them back over its own bus, and a rate of zero is code 0 |
| `the_processor_runs_at_one_cycle_in_fifty_nine` | the prompt's cost in clocks at both divisors |
| `a_partly_filled_packet_goes_when_the_machine_falls_silent` | `in_commit` after a silence, so one keystroke does not wait for sixty-three more |
| `the_project_resolves_and_elaborates` | the manifest builds from five library packages and five sources, with no black box |
| `the_machine_synthesises_without_errors_or_latches` | generic synthesis: no error, no warning, no latch |
| `the_machine_maps_onto_the_ecp5_and_fits_the_part` | the ECP5 flow fits it on an LFE5U-12F, every memory lowered, no `DP16KD` anywhere |
| `the_rom_is_lookup_tables_and_this_is_what_they_cost` | `monitor_rom` on its own is 505 `LUT4` and no storage at all |
| `the_testbench_session_comes_out_of_the_simulator` | the same machine driven by `tb/monitor_tb.v`, which is what `reticle sim` runs |

And two in [`tests/ip_library.rs`](../../tests/ip_library.rs), because
what they need lives there:

| Test | What it shows |
|------|---------------|
| **`a_6502_monitor_answers_through_the_transceiver_that_is_on_the_board`** | the whole machine — 6502, ACIA, class layer, link layer — enumerating and answering through a model of the part on this board, **including its late LineState**; then a live `SET_LINE_CODING` at 115200 and at 1200, read back by the 6502 out of `$5003` |
| `a_hosts_rate_becomes_a_bit_period` | `uart_baud_div` and `uart` joined: seven rates, each becoming a bit period measured in clocks off a transmit line |

The tests drive `tb/monitor_bench.v` — the machine with its byte
interface bare — a character at a time from Rust, and compare what comes
back against a string the test computed. That is how one tests a parser,
because that is what a monitor is.

Two things make those runs cheap and one makes them honest:

* **`CPU_DIV` is 1 in the bench and 59 in the design.** The bus is the
  same bus at one clock a cycle as at fifty-nine, so a simulation that
  spent fifty-nine clocks on each would be fifty-nine times longer and
  prove the same thing. What it would *additionally* prove is that the
  divider counts, and that is a separate, cheap test.
* **Each command's answer ends at a silence, not at a byte count.** The
  monitor prints a carriage return at the *start* of the next line rather
  than at the end of the last one, so waiting for a fixed number of bytes
  would race with that return. Waiting for four thousand clocks with
  nothing printed does not.
* **The test zeroes the RAM before reset, and says so.** A distributed
  RAM cannot be given initial contents, so in simulation every byte of it
  is `x`; a real part comes up with whatever its cells held, and the
  emulator the transcripts came from starts at zero. Asserting against
  `x` would have been asserting against nothing.

### The oracle: what was checked against a running original

`the_whole_session_matches_the_one_a_real_monitor_gives` asserts eight
command-and-answer pairs. Each right-hand side was **printed by a
published 65C02 build of the same interface, running in a third-party
emulator** on this machine, driven with the same input; the left-hand
sides are what `monitor_bench` prints. Nothing of that build is in this
repository, none of it was read, and what was compared is what came out
of a terminal.

The addresses in the session are all RAM, so the bytes are the session's
own rather than the other machine's ROM — which is the trick that lets
two machines with different memory maps be compared at all.

```text
0300: AA BB     0300: AA BB||0300: 00|
: CC DD         : CC DD||
0300.0303       0300.0303||0300: AA BB CC DD|
.0307           .0307| 00 00 00 00|
0305.0300       0305.0300||0305: 00|
02FE.0310       02FE.0310||02FE: 00 00|0300: AA BB CC DD 00 00 00 00|0308: 00 00 00 00 00 00 00 00|0310: 00|
0300 0400       0300 0400||0300: AA|0400: 00|
12345           12345||2345: 00|
```

(`|` is a carriage return. A test whose expected value contains returns
is a test nobody can read the failure of.)

**What the oracle would not catch:** anything the original also gets
wrong, and anything neither was asked. It is a check against a second
implementation, not against a specification. In particular the original
runs on a 65C02 and this core is NMOS-only, so the two ROMs cannot be
compared instruction for instruction and were not; only terminal
behaviour was.

**What could not be checked against it at all**, and is therefore
description or judgement:

* the `R` command's effect, because the two machines have different
  memory maps and the program being run would have to be different;
* the 128-character line limit, which the original handles differently;
* the vectors, which are this machine's;
* anything about the ACIA's registers, because the original's initialisation
  writes them and never reads them back.

## Resource use

The ECP5 flow in `tests/mos6502_monitor.rs`, for `ecp5-12f-CABGA256`:

| Resource | Used | The LFE5U-12F has |
|----------|------|-------------------|
| `LUT4` | 6183 | 12 144 |
| `TRELLIS_FF` | 988 | 12 144 |
| `TRELLIS_DPR16X4` | 530 | 3036 |
| `TRELLIS_IO` | 20 | 120 |
| `DCCA` | 1 | 16 |

LUT depth 28. The distributed RAMs are 512 for the 4 KiB of main memory
— sixteen words of four bits each, so 4096 bytes is 256 rows of two — and
18 for `usb_cdc_acm`'s three endpoint buffers, which is the number
`docs/ip-library.md` already records for that block.

Against the budget the task set: `mos6502` is about 1320 `LUT4`,
`usb_cdc_acm_ulpi` about 1209, the ROM is 505, and the rest is the RAM's
address decoding and read multiplexer, the ACIA, and the glue. **It fits,
with half the part left.**

The baud machinery is part of that and is not broken out here;
`docs/ip-library.md`'s footprint table has both blocks on their own, and
the two numbers worth knowing from it are that `uart_baud_div` is 282
`LUT4` and 184 flip-flops, and that giving `uart` a run-time divisor took
it from 120 `LUT4` at logic depth 6 to 215 at depth 8. The depth is the
interesting half: reading the divisor every bit instead of once per
character would have made it 229 at depth **19**, and that measurement is
why it is latched.

### The ROM is two pages and not one, and that is measured

The monitor is **266 bytes of code and six of vectors**. The target was
256, which is what the interface it reproduces manages, and it did not
fit.

What the second page costs was measured rather than estimated.
`the_rom_is_lookup_tables_and_this_is_what_they_cost` maps `monitor_rom`
on its own:

```text
monitor_rom: 505 LUT4, depth 6, for 512 bytes
  which is 1.0 lookup tables a byte
```

So the page that was not needed is about **250 `LUT4` of the part's
12 144 — two per cent**, and that is what overrunning by sixteen bytes
cost. It is also eight times cheaper than the task's arithmetic
budgeted: eight lookup tables a byte would have made 512 bytes four
thousand, and the reason it does not is that a ROM is a mux tree over
*constants*, and the mapper folds sixteen of them into one `LUT4` before
it starts multiplexing.

The same measurement says what a bigger ROM would cost: 2 KiB would be
about two thousand `LUT4`, which this design could afford and the task's
budget said it could not.

Where the extra bytes went, honestly:

* **`echo` polls the transmitter.** The published port stores the byte
  and then spins a delay loop long enough for it to have gone; that is
  eleven bytes here against its nine, and it is not a saving worth having
  when the rate changes whenever a host says so.
* **The line editor keeps a length guard**, so a 129th character cannot
  walk out of the buffer.
* **The parser is written to be read.** The mode is a variable and the
  actions are separate routines rather than one interleaved block.

Six things in `sw/monitor.s` *are* written for size, and each is marked
where it happens: X is pinned at zero for a whole line so that `(zp,x)`
reaches a pointer in two bytes and nothing clobbers Y; the four shifts
that move a digit in are a loop; `show`, `prbyte`, `prnib` and `echo` are
one chain of fall-throughs; MODE holds the character that set it;
`setmode` falls into `item` instead of branching back to it; and the digit
test is the ordinary `eor #$30` idiom.

**A 2 KiB ROM was never considered**, and the task's arithmetic says why:
about eight lookup tables a byte puts 2 KiB at roughly sixteen thousand
`LUT4` on a part with 12 144.

### Why the RAM is 4 KiB

A `TRELLIS_DPR16X4` is sixteen words of four bits, so a byte-wide memory
costs two per sixteen bytes: 4 KiB is 512 of the part's 3036, and 16 KiB
would be 2048. The storage would fit. What would not is the read
multiplexer: a 4 KiB array is a 256-way byte multiplexer and a 16 KiB one
is a 1024-way, four times the lookup tables for memory this machine's
monitor cannot fill. 4 KiB leaves page zero, the stack, the input buffer
and fourteen pages to put programs in.

## On the part

Nothing about this design is proved by simulation alone. Simulation has
no unrouted wires, no ULPI transceiver and no host; `CLAUDE.md` records a
defect on this very board that cost eight rounds of investigation and
that simulation could not have seen at all.

So this was built, loaded and used.

```text
$ reticle fpga examples/mos6502_monitor/rtl/monitor_rom.v ... --bitstream monitor_cynthion.bit
note: wrote monitor_cynthion.bit, 253335 byte(s) compressed, 317539 configuration bit(s) set,
      20 pad(s), 6183 lookup table(s), 988 flip-flop(s) and 530 distributed RAM(s) configured,
      988/24288 ff, 1/56 gb, 20/120 io, 6183/24288 lut, 530/3036 lutram
note: routed 9294 of 9303 signal(s) with 147584 pip(s) over 156878 wire(s), and every sink
      was walked back to its driver
note: 988 flip-flop(s), every clock on a global network: G_HPBX0000 to 1518 of them
note: all 317539 set bit(s) decode back through the database into 96110 arc(s), 8504 field(s)
      and 9362 word(s), with 0 unexplained, and the arcs they select are exactly the 96110 the
      router chose
note: for IDCODE 0x21111043 (LFE5U-12F-8CABGA256)

$ reticle program --device 35L6H2CMGJJVCIBAEA3GCLAN74 monitor_cynthion.bit
253335 bytes of bitstream shifted in 1.2 s
status after ISC_DISABLE: 0x00200100 (DONE)
DONE is high: the part accepted the bitstream and is running it.

$ for d in /sys/class/tty/ttyACM*; do ...; done
ttyACM0 1d50:615c cdc_acm
ttyACM1 1209:0001 cdc_acm

$ stty -F /dev/ttyACM1 115200 raw -echo clocal min 0 time 20
```

**Every one of the 317,539 set bits decodes back through the fabric
database, and the arcs they select are exactly the 96,110 the router
chose, with none unexplained.** The nine of 9,303 signals that were not
routed are the ones with nothing to route — every sink in the design was
walked back to its driver, which is the sentence above that matters.

### The session

Typed at `/dev/ttyACM1` with a script that writes a line and reads what
comes back; `^M` is a carriage return, printed by `cat -v` so that the
byte order is visible rather than overwritten.

```text
FE00^M^MFE00: A2^M
FFFA.FFFF^M^MFFFA: 00 FE 00 FE 00 FE^M
0300: AD 01 50 29 10 F0 F9 A9 2A 8D 00 50 4C 00 FE^M^M0300: 00^M
0300.030E^M^M0300: AD 01 50 29 10 F0 F9 A9^M0308: 2A 8D 00 50 4C 00 FE^M
5003^M^M5003: 10^M
0300R^M^M0300: AD*\^M
```

Line by line, because every one of them is a claim:

* **`FE00`** answers `FE00: A2`. `$A2` is `LDX #`, the first byte of the
  reset entry — the ROM reading itself out of the lookup tables it is
  made of.
* **`FFFA.FFFF`** answers six bytes on one row: `00 FE` three times, the
  three vectors all pointing at `$FE00`.
* **`0300: AD 01 50 ...`** deposits fifteen bytes, and prints `0300: 00`
  first — the byte that *was* there, which is the quirk this interface
  has, on the part.
* **`0300.030E`** reads them back, broken into rows of eight at `$0308`
  and nowhere else.
* **`5003`** answers `10`. That is the ACIA's CONTROL register: the
  monitor wrote `$1F` on its way up, and the host's 115200 — a rate the
  65C51's four baud bits cannot name — replaced them with code 0,
  "clocked from outside this part". **The rate a host set is a number the
  6502 read.**
* **`0300R`** prints `0300: AD`, the byte at the run address, then jumps
  there. The fifteen bytes are `LDA $5001 / AND #$10 / BEQ / LDA #$2A /
  STA $5000 / JMP $FE00` — poll the transmitter, print `*`, restart the
  monitor. The answer is `*` and then a fresh `\` prompt, which is all
  three of those things happening.

### And the baud rate, rate by rate

`stty` sets a rate, then the monitor is asked what `$5003` says. The
right-hand column is the W65C51N data sheet's baud table, which nothing
in this repository could have got right by accident.

| `stty -F /dev/ttyACM1` | `5003` answers | code | the data sheet's rate |
|---|---|---|---|
| 9600 | `1E` | 14 | 9600 |
| 19200 | `1F` | 15 | 19200 |
| 1200 | `18` | 8 | 1200 |
| 115200 | `10` | 0 | *clocked externally* |
| 4800 | `1C` | 12 | 4800 |
| 230400 | `10` | 0 | *clocked externally* |
| 115200 | `10` | 0 | *clocked externally* |

The top nibble stays `1` throughout: eight data bits, one stop bit, the
receiver clocked from the baud generator — what the monitor programmed,
which the class layer does not touch.

### The one measurement that was not taken

**The bit period on ball C11 was not measured.** There is no oscilloscope
and no logic analyser here, the board's only free user IO are two PMOD
headers, and `testdata/fpga/cynthion/bidir_loopback.v` already argues that
nothing in the fabric can tell whether anything is plugged into one. So
the waveform is asserted by a chain of measurements rather than by an
instrument, and here is the whole chain:

| Link | How it was measured |
|---|---|
| a host's `SET_LINE_CODING` reaches `usb_cdc_acm`'s `baud` | `usb_cdc_acm_ulpi_answers_the_line_coding_and_control_line_requests`, and GET_LINE_CODING reads it back on the part |
| `baud` reaches the ACIA's CONTROL register | **on the part**, in the table above |
| CONTROL's code becomes a rate in bits per second | `the_acia_reports_the_rate_the_host_set`, and `a_6502_monitor_answers_through_the_transceiver_that_is_on_the_board` through the whole USB stack |
| a rate becomes a divisor | `uart_baud_div_computes_the_divisor_for_the_rates_a_host_asks_for`, against `round(60e6/rate)` computed in the test |
| a divisor becomes a bit period on a transmit line | `a_hosts_rate_becomes_a_bit_period` — the two blocks joined, seven rates, the gaps between the edges of a frame measured in clocks |
| that transmit line is ball C11 | **not measured.** It is one line of `rtl/monitor_cynthion.v` and one line of `board/cynthion.rcf` |

The last row is the gap, and it is one `assign` wide. Everything above it
is a number somebody can check.

## What this proves, and what it does not

**Proved on the part:** the session above, the ACIA's rate register
following a host through seven changes of `stty`, and a bitstream every
bit of which decodes back to the arcs the router chose.

**Proved by `cargo test`:**

* the ROM is `sw/monitor.s` assembled, and every byte of it decodes as a
  documented NMOS 6502 instruction from every entry point;
* the monitor examines, ranges, deposits, continues, runs and refuses,
  against a transcript recorded from a second implementation;
* the ACIA's rate register follows a host and a processor, in that order
  of priority, and the processor can read it back over its own bus;
* the design maps onto the part with every memory lowered, no block RAM
  anywhere, and half the lookup tables spare.

**Not proved, and stated rather than left out:**

* **The bit period on ball C11 was not measured with an instrument.**
  There is no oscilloscope or logic analyser here. What was done instead
  is [above](#the-one-measurement-that-was-not-taken).
* **The USB stack is not in the `tests/mos6502_monitor.rs` runs.**
  `monitor_bench` is the machine with its byte interface bare, on purpose:
  a parser is tested as a parser. The stack *is* in
  `a_6502_monitor_answers_through_the_transceiver_that_is_on_the_board`,
  which is the same machine through `usb_device_ulpi` and through a model
  of the transceiver that reports LineState late — but that run is one
  session and not the thirteen the bench tests are.
* **Timing is not closed.** The read path is a deliberate multi-cycle
  path and static analysis reports it against one clock period. What
  settles it is the part.
