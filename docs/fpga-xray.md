# A real Xilinx 7-series fabric, and a real `.bit`

## A status word whose fields came back moved, and what it is not

**MEASURED on a Basys 3, 8 October 2026, and NOT YET EXPLAINED.** This
section is here because the path to the answer is the useful part and
because four plausible causes have been ruled out with a measurement each.

`examples/basys3/iso7816_terminal.v` builds a 128-bit status word as a
concatenation of registers and constants, loads it into a 128-bit shift
register and emits it as 32 hex nibbles. Three readings, taken with the
design loaded and answering:

| field | holds | read |
|---|---|---|
| `CARD_DIV16`, a `localparam [15:0]` | 14 | `0x00A4` |
| `etu_div`, a `reg [15:0]` | 5208 | `0x14D0` |
| `etu_div` after a PPS | 56 | `0x0290` |

and the **pure constant** `128'h0123456789ABCDEFFEDCBA9876543210`, pushed
through the same shift register and the same emitter, came back byte for
byte exact.

### The arithmetic the reading fits

One rule reproduces all three fields exactly — 48 measured bits, with no
free parameter beyond the rule itself:

```text
R[i] = T[i]      for even i
R[i] = T[i - 4]  for odd  i
```

`T` is the word the source says, `R` the word that came back. It is
`R = (T & 0x5555…) | ((T << 4) & 0xAAAA…)`, and it is **forced** to be
something of this shape: no single offset fits, because bit 34 must have
offset 0 and bit 57 must have offset 4. Said physically, the odd bits are
**one shift step ahead** of the even ones. Applied to the known constant
the same rule gives `0321476D8BA9CFEF…`, which is not what came back — so
whatever it is, it acts on the status leg and not on the constant leg.

### What it is not

Everything here was measured in-tree, on the mapped netlist of the design
that was on the board (`examples/basys3/iso7816_terminal_pad.v` plus the
core and `ip/bus/{uart,iso7816_uart}`), not on a reduction of it.

- **Not the front end.** `reticle sim` on the construct emits the right
  32 nibbles.
- **Not generic synthesis or LUT covering.** Each of the 128 `LUT6`s that
  drive the shift register's `D` was evaluated through its own `INIT`: the
  status load, the known-constant load and the four-bit shift are all
  exactly right, for every bit. `reticle fpga --verify` agrees on the
  mapping, and `tests/fpga_xray_wide.rs` now pins the whole construct.
- **Not placement legality.** 351 slices hold flip-flops in that design
  and **not one** of them mixes clock, clock enable, set/reset or
  synchronous-versus-asynchronous. The placer's own
  `SiteRules` enforces the first three because they are *wires* a slice
  shares; the fourth comes along for the ride.
- **Not the routing.** The 7-series bitstream path never checked this —
  see the next section — and with the check in place the design routes
  2867 of 2867 signals and every sink is reached **from its own signal's
  driver**, with no node carrying two signals.

### The measurement that no static defect can produce

Bit 37 of the word is `CARD_DIV16`'s bit 5. Both the status word and the
known constant have **zero** there, so the mapper gave that bit a
two-input lookup table, `f(host_valid, status_sh[33])`, with no
`want_known` input at all: **the same logic, the same truth table, the
same inputs under both commands.** Yet bit 37 read one under `s` and zero
under `k`.

No mis-wiring, no permuted truth table and no wrong tap can do that: all
three are the same for both commands. So either the defect is **dynamic**
— timing, or a slice field whose value depends on what the design is doing
— or **the two readings were not taken from one bitstream**, which would
make a placement-dependent static defect fit again and is the cheapest
thing to check next. That the Basys 3's other open symptom changes which
LED is stuck between builds is a reason to suspect the second.

### What to measure next

- Take the `s` and the `k` readings **from one load of one bitstream**,
  in one session, and say so. If they disagree across builds the search
  narrows to placement.
- Emit the word twice in a row without reloading, and emit 64 nibbles
  instead of 32. A register one step ahead shows up as a repeat.
- Read `status_left` and `pos` out alongside the word: the model says the
  odd bits took one extra step, and a counter that disagrees with the
  nibble count would say where.

## The 7-series bitstream path never checked its own routing

**CHECKED and FIXED, 8 October 2026.** `reticle fpga --bitstream` went
place → route → generate, and the only thing it said about the result was
a count: `N of N signal(s) with a reader routed`. The ECP5 path has walked
every sink backwards through the pips it was given since it was written —
`routing.verify`, a dozen lines above where it writes the file — and the
7-series path did not. A route that occupies the right wires without
joining them is exactly the defect the section above this one is about, and
on this family it was the one that could not be caught.

It is called now. Both designs it was tried on pass it, so **it did not
find anything**: `examples/basys3/iso7816_terminal_pad.v` (2867 signals)
and the 128-bit shift register of `tests/fpga_xray_wide.rs`. What it costs
is a walk per sink and it runs only when the router succeeded.

What `verify` cannot see is a pin the right signal reaches **and another
signal also drives**, because it only ever looks at one signal's pips.
`tests/fpga_xray_wide.rs` builds the reverse map over every route at once
and asks the question the other way round; that is also green on both
designs, and the router's own capacity model is why.

## A register's declared initial value never reaches `INIT`

**CHECKED, 8 October 2026. Real, reproducible without a board, and NOT
FIXED.** `tests/fpga_xray_wide.rs`'s
`a_registers_declared_initial_value_reaches_the_flip_flops_init` is the
reproducer; it is `#[ignore]`d so the gate stays green, and
`cargo test --all-features --test fpga_xray_wide -- --ignored` shows it.

`reg [15:0] r = 16'h1234;` maps to sixteen `FDRE #(INIT=1'd0)`. The cause
is one level below the FPGA flow: [`ir::CellKind::Dff`] has **no field for
a power-up value**, so `techcells::set_params` copies the device file's
declared default for whichever variant the bit chose and nothing else —
`FDRE` is `INIT=1'b0` and `FDSE` is `INIT=1'b1` in `src/fpga/devices/xc7.dev`,
whose `ff` line does not carry the `init` flag that `ice40.dev`'s does.

On this fabric that is not cosmetic. `<letter>FF.ZINI` is an **inverted**
field — the bit has to be *set* for a flip-flop to come out of
configuration holding zero — so `INIT` decides the state the part starts
in. Two consequences, both read off the netlist and neither on a part:

- a register with a declared value starts at **zero**, except in the bits
  its set/reset drives to one, which start at **one**: `reg [15:0] etu_div
  = 16'd5208` with `if (fast) etu_div <= 16'd56` maps to flip-flops whose
  `INIT`s spell **56**, the reset value;
- a register declared zero that is *set* somewhere starts holding **one**,
  because that bit took `FDSE`'s default.

**It does not explain either Basys 3 symptom, and this is worth saying.**
`iso7816_terminal.v` resets `etu_div` to `SLOW_DIV16` as well as declaring
it, and `ip/bus/iso7816_uart` declares no initial value at all and resets
everything through `rst_n` — so both designs are indifferent to it. A
design that relies on a power-up value and has no reset is the one this
bites, and `examples/basys3/` has several registers declared `= 1'b1`.

Fixing it is a change to the IR: an initial value on `CellKind::Dff`,
carried through elaboration, generic synthesis, the equivalence checker and
every backend. Until then the habit is the workaround: **reset what you
rely on.**

[`ir::CellKind::Dff`]: ../src/ir/cell.rs

## A join on the wrong tile, which welded two rows of interconnect together

**CHECKED and FIXED, 8 October 2026.** This is the defect behind the
designs on this board that place, route, report every signal routed and
every bit decoded — and compute nothing. It is not about pads, and it is
not about how many of them a design has.

A 7-series node is **wires joined across tile boundaries**, and
`tileconn.json` is the only statement of which wire of which tile is the
same metal as which wire of its neighbour. Each of its entries names
**two tile types** and a grid delta, and lists the wire pairs that are
one node at that delta. The loader turned each pair into two zero-bit
pips, and a pip belongs to a tile *type*, so all it could say was *"the
wire called N, `(dx, dy)` tiles from here"*. **The partner type was
dropped.**

That is fine while two tile types that both have a wire called N can
never sit at the same delta. On this fabric they can, and the clearest
case is the clock-management column. A tall `CMT_TOP` is cut into
`_UPPER_B`, `_UPPER_T`, `_LOWER_B` and `_LOWER_T`; all four name their
wires `CMT_TOP_*`, and the file pairs, **both at delta `(1, 2)`**:

| | |
|---|---|
| `CMT_FIFO_EE4A0_0` | with `CMT_TOP_R_UPPER_B`'s `CMT_TOP_EE4A0_0` |
| `CMT_FIFO_EE4A0_3` | with `CMT_TOP_R_LOWER_T`'s `CMT_TOP_EE4A0_0` |

`CMT_FIFO_R_X7Y20` is at grid `(7, 136)` and the tile at `(8, 138)` is
`_LOWER_T`. Both pips were declared on `CMT_FIFO_R`, both resolved, and
lanes 0 and 3 of that column became **one node** — two different pieces
of metal, three interconnect rows apart.

What that does to a route is visible in one line of `io_exercise`'s:

```text
X4Y142/EE4BEG0 -> X5Y142/EE4A0 -> X6Y142/INT_INTERFACE_EE4A0
   -> X7Y136/CMT_FIFO_EE4A0_0 -> X8Y138/CMT_TOP_EE4A0_0
   -> X9Y139/VBRK_EE4A0 -> X10Y139/CLBLL_EE4A0 -> X11Y139/EE4B0
```

An east-going wire cannot change row, and that one enters the column on
interconnect row 13 and leaves it on row 16. The signal — `btn_c`, a
pushbutton — was routed, its arcs were in the bitstream, every bit of it
decoded, and on silicon it arrived nowhere. **68 of `io_exercise`'s hops
were of this kind.**

Counted over the whole die from `tilegrid.json` and `tileconn.json`
alone, the old rule made **21 956 joins the file does not state**,
against 4 717 252 it does:

| | |
|---|---|
| `CMT_FIFO_R` → `CMT_TOP_R_LOWER_T` / `_UPPER_B` | 5874 each |
| `CMT_FIFO_L` → `CMT_TOP_L_LOWER_T` / `_UPPER_B` | 3916 each |
| `PCIE_INT_INTERFACE_L` / `_R` → `PCIE_BOT` | 1084 each |
| `BRKH_CLB` → `CLBLM_R`, `CLBLM_L`, `CLBLL_L`, `CLBLL_R` | 72, 48, 44, 24 |
| `LIOI3` → `LIOI3_TBYTESRC`, `RIOI3` → `RIOI3_TBYTESRC` | 12, 8 |

Two of those matter beyond the count. The **clock-management column** is
what a signal must cross to get from one side of the die to the other,
and almost every bogus join is in it. `BRKH_CLB` is the tile that
**breaks a CLB column at a clock-region boundary**, so a route leaving
its clock region could be welded to the wrong row on the way. `sw_led`
and `blink` — the two designs that were ever watched working — take no
bogus hop at all, measured on the code before the fix; `carry_probe`
takes six and `io_exercise` sixty-eight. The table below has all four.

The fix is in the model, not in the loader: [`arch::WireRef`] gained a
`tile_type`, and a reference that names one resolves **only** against a
tile of that type. `xray` uses it for every join. Nothing else does, and
the text format grew `name@dx,dy:TYPE` so that `parse(to_text(a)) == a`
still holds.

### What was measured, and what is still unknown

- `tests/fpga_xray_joins.rs` reads `tileconn.json` a second time,
  independently of the loader, and checks that **every cross-tile pip of
  a loaded region is a pair that file states, for the tile types at both
  ends**. 421 680 cross-tile pips of a 25-by-41 corner of the grid: the
  left-hand IO column, the clock-management column beside it and the
  first logic columns. It fails before the fix and passes after, and a
  second test names the `CMT_FIFO` / `CMT_TOP` pair directly.
- Four designs, counted the same way before the fix and after. The two
  that a person ever watched working had **no** bogus hop; the two that
  did not work had all of them:

  | design | bogus hops before | after | watched |
  |---|---|---|---|
  | `sw_led` | 0 | 0 | works |
  | `blink` | 0 | 0 | works |
  | `carry_probe` (`carry_probe_trusted.rcf`) | **6** | 0 | wrong |
  | `io_exercise` | **68** | 0 | wrong |

- And in `io_exercise` the hops land on the nets that read wrong, with
  no exception either way. All sixteen switch nets carry two to four of
  them (each switch feeds its LED *and* the display multiplexer, and a
  hop on one branch leaves the others alone). Of the fifteen LED nets,
  exactly four carry one — `led[1]`, `led[11]`, `led[12]`, `led[14]`,
  which are LD1, LD12, LD13 and LD15 — and **all four are in the set of
  eight that read stuck**, while none of the seven that read correctly
  carries any. `dp` carries three, which is why the decimal point never
  blinked; `an[0]` carries one, which is why three digits could be lit
  at once; `seg[0]`, `seg[4]`, `seg[5]` and `seg[6]` carry one or two,
  so the glass could not be read at all. `btn_c`, `btn_u` and `btn_l`
  carry one each. `btn_r` carries none and still read wrong — but it was
  read *through* the display, which `dp`, `an[0]` and four segments had
  already corrupted.
- It does **not** follow that anything now works on the part. Nothing
  in this section has been on silicon; it says the routing graph no
  longer contains connections the vendor's own database denies. The
  board is still the only oracle, and the readings taken on 8 October
  were taken through this defect and cannot be trusted — see the
  retractions below.
- The sweep would not catch a join `tileconn.json` **states** and the
  loader omits: nothing counts the file's pairs back. A missing join
  makes a design fail to route, which is loud; a false one was silent,
  which is why this is the shape of defect worth a test.

### It accounts for every LED of the `carry_probe_trusted` reading

Those six hops of `carry_probe` are not spread at random. Five of the
seven LEDs read wrong, and the nets carrying the bogus hops are:

| net | bogus hops |
|---|---|
| `plain[2]` | **2** |
| `led_alive` (`plain[22]`) | 1 |
| `led_plain` (`plain[25]`) | 1 |
| `led_carry` (`chain[25]`) | 1 |
| `led_moved` (`moved`) | 1 |

Downstream of a bogus hop a route is connected, by muxes the bitstream
really does program, to metal that **nothing in the design drives**.
What such a wire reads is not something the database states — the only
`default` lines `ppips_int_l.db` has are on the slice's own input muxes
(`IMUX_L*`, `BYP_ALT*`, `FAN_ALT*`), all `VCC_WIRE` — but this
repository's experience on both families is that an input nothing drives
reads as a one, and that is what the readings look like.

Taking that as the level, the whole glance follows:

| LED | net | read | why |
|---|---|---|---|
| LD10 `led_alive` | `plain[22]` | solid on | `plain[2]`'s two bogus hops land on the lookup tables of the toggle tree, where `toggle[i] = &plain[i-1:0]`. A constant one there makes every toggle above lane 2 fire on every cycle, so the counter's upper bits run at the oscillator rather than at 12 Hz and 1.5 Hz — which is a solid LED at half brightness, not a blink |
| LD8 `led_plain` | `plain[25]` | solid on | the same, and a bogus hop of its own |
| LD9 `led_carry` | `chain[25]` | solid on | a bogus hop of its own |
| LD7 `led_moved` | `moved` | solid on | a bogus hop of its own; and the latch would close honestly anyway |
| LD5 `led_differed` | `differed` | solid on | **honestly**: with `plain` corrupted the two counters really do differ, so the latch really does close. This is the LED the retracted carry finding rested on |
| LD11 `led_zero` | `1'b0` | dark | correct |
| LD14 `led_one` | `1'b1` | dark | the separate `drive_constant_data` gap, below. Still unfixed |

Nothing is left over, and **none of it needs the clock to be at fault.**
That matters because it is the opposite of the reading the section below
drew. It also means the carry chain is **not** implicated: `led_differed`
closing is fully explained by `plain` being wrong, and says nothing
about `chain`.

### What this does to "It is the clock"

**The clock-distribution diagnosis below is withdrawn as a conclusion,
and kept as a reading.** It was inferred from "a 4-bit counter ticks and
a 26-bit one on the same net does not" and from the five solid LEDs; the
five solid LEDs are now explained without it, and the `diag` observation
is explained the same way if the 26-bit counter's path to its pad went
through a welded join — which is what `diag`'s own placement would have
to be re-measured to say. The rebuffer-enable counts in the table below
(4, 4, 6, 8) are real numbers from `--report` and are **not** evidence of
a fault: nothing has shown that the columns a design needs are not among
the ones it enables.

What is true of the clock either way: `carry_probe`'s clock net, with the
fix in, is a single clock region — along `CLK_HROW_R` at grid row 130
and down one leaf network — and all 54 of its sinks are reached, with
every cross-tile hop one `tileconn.json` states. A design whose
flip-flops span *several* clock regions still has nothing measured about
it, and the oracle for that is still unread: no Vivado bitstream in
`artix7/harness/` has registers in more than one region.


## Two designs produced by this have run on a real part

On 2026-09-24 the milestone design — two slide switches through one
lookup table to one LED — was built by this flow from Verilog, loaded
onto a Digilent Basys 3 (XC7A35T-1CPG236C) by `reticle program`, and
**confirmed working by a person flipping the switches**: LED 0 followed
the exclusive-or of SW0 and SW1 through all four combinations. No vendor
tool took part at any step.

That is one design, of one lookup table and three pins, on one board. It
establishes that the chain from Verilog to configured silicon closes.
**It does not establish that anything larger works**, and the list at the
end of this document of what is untried on a part — a memory, a carry
chain, or an IO standard other than LVCMOS33 — is unchanged by it.

Before the cable, the strongest statement available was a comparison, and
it is still worth having because it is what predicted the result:
decoded back into the database's own feature names, the configuration
this flow puts on the three IO blocks and the two IO-logic tiles is
**identical, feature for feature, to what Vivado put there** in the
working bitstream `prjxray-db` ships for this same board. The routing
between them differs, because two routers chose two different legal
paths; the ends of those paths are the same wires. That comparison is
`tests/fpga_xray.rs::the_io_path_is_the_one_vivado_built`.

One thing the LED settled that no test could: Vivado's bitstream sets
1315 bits that nothing in the database names, and it was an open question
whether some of them were configuration a part needs. For a design of
this shape, they are not.

What is established structurally, checked against a bitstream Vivado
made for the very part in question:

- the container is the one Xilinx UG470 describes: the sync word
  `0xAA995566`, the type 1 and type 2 packets, the register and command
  numbers, the frame address register's fields, 101 words per frame;
- the IDCODE it writes is `0x0362D093`, which `part.json` and the
  reference bitstream agree is the XC7A35T's, and the flow refuses to
  emit if the database's IDCODE and the device file's disagree;
- every frame address it writes is one `part.json` describes, and the
  frame count is 5420 — the 5408 `part.json` states plus two zero pad
  frames after each of the six (bus, half, row) groups, which is what
  Vivado's bitstream carries;
- both CRC check words are right, computed bit by bit in the library and
  again by an independent table-driven implementation in the test;
- Reticle's own reader parses Vivado's file, and parses Reticle's own
  output back to identical frames;
- the packet sequence matches Vivado's register for register, and the
  48 bytes before the sync word are byte-identical.

What is **not** established is that any of it configures anything *on
silicon*. The gaps are named below under *What remains before an LED
could light*.

## And a second, clocked one, which blinks

The same day, `examples/basys3/blink.v` — the board's 100 MHz oscillator
through a `BUFG`, down the global clock column, out along a leaf network
into twenty-six flip-flops, and one LED off the top bit — was built by
this flow, **routed completely** (92 of 92 signals), loaded onto the same
board, and the part answered `DONE` high with no CRC error.

**A person watched that LED blink**, the same day, loaded again from a
fresh build. The design should blink LED 0 at 1.49 Hz, a period of
0.671 s; one period timed by hand with a stopwatch, from one turn-on to
the next, came to about 0.60 s. That is a single press of a stopwatch
button, so it establishes that the clock arrives and the counter counts,
and that the rate is the right one to within what a hand can time: a
wrong counter bit would be off by a factor of two, and nothing else
available here could make a 100 MHz oscillator run 11 % fast. It is not
a measurement of the clock's frequency.

So a clock does travel on silicon the way this flow routes it: from the
pad, through the `BUFG`, down the global clock column and along a leaf
network into flip-flops that toggle.

What *is* checked, and is the reason to expect it to work, is the same
comparison that predicted the first milestone, now over the clock:
decoded back into feature names, the configuration this flow puts on the
clock pin, on its hop into the clock backbone, and on the `BUFGCTRL` is
**identical, feature for feature, to what Vivado put there** for the very
same pin of the very same board. The one clock row and the rebuffers
differ because the two designs drive different halves of the die, and the
differences are enumerated below. That comparison is
`tests/fpga_xray.rs::the_clock_path_is_the_one_vivado_built`.

## A `*_SING` IO tile drives a pin on silicon

**CHECKED, 8 October 2026.** `examples/basys3/io_exercise.v`, extended to all
sixteen LEDs, was loaded and a person held BTND: **all sixteen lit, with no
gap where LD6 used to be.** Ball U14 sits in `LIOB33_SING_X0Y0`, which this
flow could not drive until today, and `*_SING` tiles are the single-IOB tiles
at both ends of every IO bank — so this is a class of pins on every 7-series
part, not one LED.

The structural work behind it is in the commit; what the board adds is the
one thing simulation cannot give, since a bitstream whose bits are placed
from a misread offset decodes perfectly against the same misreading.

### The oracle was in the repository the whole time

Three of the four Vivado reference designs under `artix7/harness/` drive a
ball in a `*_SING` tile, and **`basys3/swbut` drives U14 itself** — Vivado's
own bitstream for this very board has been lighting LD6 all along. Between
the three they cover both die edges, both halves and both directions. Nothing
here had read them for this, and two documents had instead recorded LD6 as
"skipped", which read as if the ball were dead. The board's owner corrected
that; the oracle confirmed them.

### What the database says, and what it does not

The tempting hypothesis — that a `_SING` tile is its non-SING neighbour with
one IOB unpopulated at the same offsets — is **wrong**, and `tilegrid.json`
kills it: `LIOB33` has `words: 4` and `LIOB33_SING` has `words: 2`. Coded
that way, every feature lands two words out.

What the database does supply is an **`alias`** on those windows, naming the
type whose `segbits` describe them, with a `start_offset`. A feature at bit
`c` of the aliased type is at `c − 32·start_offset` here, and one whose bits
then fall outside this tile's own extent is simply not a feature of this
tile — **and that filter picks which half the tile holds, with nothing else
needed.** CHECKED across every half-named feature of
`segbits_{l,r}iob33.db` and `segbits_{l,r}ioi3.db`: of 382, every `_Y0`
feature's bits lie in words 2–3 and every `_Y1` feature's in words 0–1,
without exception.

A correction this forced: an earlier section here said `_Y0` is "the only
half a `_SING` tile has". `arty-a7/pmod` uses `_Y1`, as an **input**.

### It also closed the blind spot

Decoding the Basys 3 harness went from **11 tiles with no segbits file to
zero**. That count is the one this document flagged as a blind spot, because
bits in such tiles are excluded from `unexplained` rather than counted — so a
clean "0 bits unexplained" could coexist with eleven tiles nobody had
checked. Most of that gap was this.

## An ISO 7816 card, brought up and answering

**CHECKED, 8 October 2026.** `examples/basys3/iso7816_probe.v` activated the
board owner's own device and read its answer-to-reset:

```
+VCC
+CLK
+RST
3B 1B 87 05 32 2E 35 2E 31 04 33 00 00 04
```

Fourteen characters, **zero parity errors and zero framing errors**, with
the status line confirming `ETU_DIV = 5208`, `CARD_DIV = 14` and
`RST_HOLD = 400`, and `-OFF` confirming the supply switched back off. A
well-formed T=0 ATR: `TS = 3B` for the direct convention, `T0 = 1B` so TA1
follows with eleven historical bytes, `TA1 = 87`, and "2.5.1" in ASCII among
the historical bytes.

One run established all of this at once, which is worth listing because
each item had been a separate open question an hour earlier:

| | |
|---|---|
| an AQV210 PhotoMOS relay switching the device's 3.3 V supply from a pad | works |
| a `PLLE2_BASE` output at 112 MHz, divided by 14 to **8.000 MHz** | works |
| the reset hold counted in **card clock cycles**, 400 of them | works |
| an etu of 372 card cycles = 5208 system cycles = 21505 baud | works |
| the direct convention, 8E2, over fourteen consecutive characters | works |
| an open-drain line read against an external 1 k pull-up | works |
| the activation and deactivation orders, power/clock/reset and reverse | works |

### The negative run, and why it was worth recording

The attempt before it — with the supply not yet wired — read **zero bytes,
zero parity errors, zero framing errors**. That combination is diagnostic
rather than merely disappointing. A wrong etu, a wrong convention or a wrong
clock frequency would each have produced characters *with* errors, because
the receiver would have found start bits and decoded nonsense. No characters
at all means the line never went low, which puts the fault upstream of
framing entirely. The probe prints raw hex and flags parity instead of
hiding errors precisely so that the two cases cannot be confused.

### One thing the card taught us about the probe

At a 24-etu idle timeout the ATR arrived as two lines: `3B`, then the other
thirteen bytes. The device pauses between TS and T0, as ISO 7816-3 permits —
up to 9600 etu between characters of an answer-to-reset. So the timeout was
too tight and is now 256 etu, which is 12 ms at the default rate: long
enough to hold an ATR together, short enough to end a finished exchange
promptly. The timeout is in etu and not in clocks, so it tracks the card
frequency; a timeout in clocks would behave at 8 MHz and split every answer
at 1 MHz.

## The serial port works, and with it four subsystems at once

**CHECKED, 8 October 2026.** The board's USB-UART bridge carries traffic
both ways: `uart_loop.v` — receive on B18, retransmit on A18 — echoed
`RETICLE-7SERIES` back to the host **byte-identical**, 15 bytes for 15.

Every earlier serial attempt failed, including a bare counter bit driven
onto A18 with no UART in it, and the pin assignment was suspected for it.
**The pins were right all along**; the welded `tileconn` joins were
corrupting the path. The fix in `ad7897e` is what made the port work.

This matters out of proportion to itself: it is the first channel on this
board that a *machine* can read. Every reading before it cost a person's
attention and several were ambiguous because of it.

### What the part then reported about itself

`examples/basys3/selftest.v` exercises four subsystems and prints 32 hex
characters about three times a second. Decoded:

```
EADBEEF0 0003FFC2 0000 0000 A5C3 17 02
   carry      PLL  BRAM  LUT magic  st seq
```

| field | read | means |
|---|---|---|
| carry accumulator | `EADBEEF0` | exactly `0xDEADBEEF * 16` truncated — **CHECKED** |
| block RAM mismatches | `0000` | all 256 words of a `RAMB18E1`: contents, address order, data order — **CHECKED** |
| distributed RAM mismatches | `0000` | all 16 words of four `RAM64X1D` on a `SLICEM` — **CHECKED**, and this is the address permutation `src/fpga/xray/lutram.rs` says it took from nextpnr-xilinx and X-Ray's fuzzer rather than measuring |
| `A5C3` | correct | baud rate, bit order and framing |
| status `17` | bits 0,1,2,4 | carry, block RAM, distributed RAM and **PLL locked** all pass |
| PLL cycles | `0003FFC2` | 262 082 against 262 144 expected: **99.976 %** |
| sequence | increments | the part is running, not repeating one line |

The PLL deficit is 0.024 %, and a PLL built from integer dividers cannot
be off by that — a divisor of four either holds or is out by 25 %. It is a
few cycles lost where the window's level crosses into the PLL's domain at
each end. **The test was wrong, not the PLL**: it demanded exact equality
across a clock-domain crossing. It now allows a tenth of a per cent, which
still refuses every wrong answer it exists to catch, and the reported
field stays the raw count so the margin is always visible.

### A third variant of the constant-driver defect, found by accident

Giving that tolerance its own expression shifted the placement, and the
same design that had just built then refused:

```
error: block RAM `rom$ram_w0_d0`: tying `p0_addr2` to Zero: no free
GND_WIDE path reaches X19Y119/BRAM_FIFO18_ADDRARDADDR2 within two pips
```

So a block RAM address input tied to a constant is reached by a search of
**two pips** for a ground path, and whether one exists depends on where
the block RAM landed. That is placement roulette in a flow that is
supposed to be deterministic, and it is the third variant of one defect:

| a constant reaching | gets |
|---|---|
| a flip-flop's `D` | a real driver (`drive_constant_data`) |
| a distributed RAM's inputs | a real driver |
| a carry cell's operands | a real driver, since the ECP5 round |
| **an output buffer's input** | **nothing**, when this was written — `data_ports` was built from the `Ff`, `LutRam` and `Carry` roles only. A real driver now |
| **a block RAM's address** | **a two-pip search that can fail**, when this was written. A real driver now |

The first three were each added after something broke. The remedy for the
other two is the same one: build the constant rather than hunt for it.

### Both of them are closed now, and they needed different remedies

**CHECKED, 8 October 2026.** The table above predicts one remedy for both.
It is wrong about that, and finding out why took reading two databases
rather than arguing about cost.

#### The address: the search was never too narrow, and the bits are not read

`GND_WIRE` reaches exactly two wires in an `INT_L` tile.
`segbits_int_l.db` contains two lines with `GND_WIRE` in them —
`INT_L.GFAN0.GND_WIRE` and `INT_L.GFAN1.GND_WIRE` — and every one of the
48 `IMUX_L<n>` has a pip from one of those two fans and from exactly one
of them (`GFAN0` serves `IMUX_L0..3`, `8..11`, `16..19` and so on,
`GFAN1` the rest). So the two-pip path exists for **every** interconnect
input, and a tile offers **two** ground sources for 48 of them while
`GFAN0` and `GFAN1` are ordinary routing wires the router may spend on a
signal. Widening the search would have been fixing the wrong thing: what
fails is contention, not reach.

And the bits in question are bits the block **does not read**. A
`RAMB18E1` addresses its words through the top of `ADDRARDADDR` and
ignores the rest, by the width mode: 18 bits uses `[13:4]`, 9 bits
`[13:3]`, 4 bits `[13:2]`, 2 bits `[13:1]`, 1 bit all fourteen. The
mapper pads the bits below with zeros because
`src/fpga/devices/xc7.dev`'s `addr N` clause says where the word address
starts, and those zeros then arrived at the tie pass as netlist constants
with `TiePolicy::Zero` — which hunts a ground path and **fails a build**
when it cannot find one.

A bit nothing reads has no business failing anything. `unused_address_bit`
in `src/fpga/xray/bram.rs` now recognises them from the cell's own
`READ_WIDTH_*`/`WRITE_WIDTH_*` parameters and gives them
`TiePolicy::Idle`: tied to zero when a fan is free, left at the
interconnect's default when none is. No extra lookup table, no extra
route, and the lottery is gone.

`examples/basys3/ssd1306_console.v` builds. It is a 128x64 SSD1306
emulator with a UART, an SPI receiver and a block RAM — 1775 placed
instances, 1990 routed signals, 61950 configuration bits, 0 bits
unexplained — and it builds at **nine different placement settings**
rather than at one. On `8970c98` the same nine all fail, every one of them
on `p0_addr2`.

#### The pad: nothing absorbs it here, so the constant is built

The other half of the table needs the opposite answer, and an ECP5
settled it by having a mechanism this family has not got.

A `TRELLIS_IO`'s data wire can be tied in the `CIB` tile beside the pad
(`CIB.J<x>MUX = 0` or `= 1`), `TrellisFabric::configure_io` writes it, and
that tie is in every bitstream this flow has put in a Cynthion — six LEDs
lit through it, and the USB device's `R4` held low through it. A 7-series
`OBUF` has no such field: the pin is an ordinary `IMUX` and an unrouted
`IMUX` is whatever the fabric gives it. So here the constant has to be
built, and `techcells::drive_constant_data` now builds it, which is also
what nextpnr's `pack_constants` does and what `ecppack`'s own output for a
Cynthion contains in all 318 of its output pads (see
`docs/fpga-trellis.md`).

That the two families need different answers is now written in the device
files rather than in code: a `bel` line may carry an `absorbs` clause
naming the pin roles whose constant that family's backend applies itself,
and `ecp5.dev`'s `TRELLIS_IO` is the one line in the tree that has one.

#### And a fourth variant, in the ECP5 block RAM, that nobody had looked for

Asking the general question found one more, and it is the worst of the
four because it was wrong on silicon rather than merely fragile:
`src/fpga/devices/ecp5.dev` says `addr 4 pad 4'b0011`, so a `DP16KD` in
18-bit mode **does** read the four bits below its word address — and two
of them were a constant zero that nothing routed, which on that family
reads as a one. `docs/fpga-trellis.md` has it. The same change fixes it,
and `tests/fpga_constants.rs` is what would have caught it.

## A carry chain on the part, and it is right

**CHECKED, 8 October 2026, and this is the first measurement from this
board that rests on an instrument checked before the subject.**
`examples/basys3/carry_where.v` runs a 26-bit `chain + 1` counter against
a 26-bit carry-free counter, holds both at zero until one shared edge
releases them, and latches — in four groups covering every bit — whether
they ever differ. A person at the board read:

| LED | | read |
|---|---|---|
| LD10 | carry-free counter, bit 25 | **blinking ~1.5 Hz** |
| LD14 | carry-chain counter, bit 25 | **blinking, in step with LD10** |
| LD5 LD7 LD8 LD9 | ever differed in bits 0–5, 6–12, 13–18, 19–25 | **all dark** |
| LD11 | a register only ever assigned zero | **dark** — the control |

So the `CARRY4` chain computes plus one across all 26 bits. Bit 25 cannot
move unless every carry below it propagates, and the four latches say the
two counters were never once unequal over the whole 2^26 count. **This is
also the first design containing a carry chain seen working on a part
here**, and so the first silicon confirmation of the `tileconn` join fix.

### Why this design and not `carry_probe.v`

`carry_probe.v` asks "do they differ" with one latch, and over one
afternoon that latch closed for **three** different reasons, each
producing an identical lit LED: a pad that did not follow its net, a
welded routing join corrupting `plain[2]`, and finally the two counters
being released from configuration one clock edge apart. A single bit
cannot distinguish a fault from a bad instrument from a startup artefact.

Three rules came out of that, and they are why this reading is worth
something:

- **Nothing an eye must judge.** An earlier design put 12 Hz on an LED and
  asked whether it was blinking; the honest answer was "lit", and that
  answer was then used as data. The only moving lights here are 1.5 Hz,
  countable by eye; every other claim is a latch that is on or off.
- **A control that fails loudly.** LD11 is a *register* holding zero, not
  a constant — because a pad tied to a constant is itself a defect on
  this family, so a constant control could not tell the instrument apart
  from the fault it exists to exclude.
- **An arm that cannot disarm.** Both counters are released on one edge
  so a ragged release cannot leave them permanently one apart. The first
  attempt walked a single one up a shift register under an enable; it
  walked off the end, `armed` went low for good, both counters froze and
  the latches stayed lit from the brief armed window. On the board that
  read as "nothing blinking, two LEDs on". `arm` now shifts **ones** in
  and saturates.

### What this does and does not establish

It establishes the carry chain, the join fix, flip-flops, the clock and
seven output pads, in one clock region. It does **not** establish
multi-region clocking: all of this design's clock sinks are in one
region, nothing in `artix7/harness/` has registers in more than one, and
so there is no oracle for that case yet. It says nothing about the block
RAM, the distributed RAM or the PLL.

## A carry chain on the part: RETRACTED, and what the retraction found

> **This section's conclusion was wrong, and the way it was wrong is the
> useful part.** It rested on LD4 being lit. LD4 is ball W18, and W18 was
> later found to be one of several LED balls that do not follow their
> net in a larger design — so "the latch closed" and "the pad is stuck
> high" predict the same lit LED, and the reading cannot distinguish
> them. The carry chain may well be fine. See "It is the clock" below,
> which is where the evidence actually leads.
>
> The original text is kept because the mistake is instructive: the
> instrument was never checked. `CLAUDE.md` already records one round of
> this exact error, on a report whose label table was indexed backwards,
> and the lesson did not transfer.

### What was measured, and why it does not support the conclusion

**CHECKED, 8 October 2026.** `examples/basys3/carry_probe.v` runs two
26-bit counters off the board's oscillator. One is written `chain + 1`
and maps onto seven `CARRY4`; the other is written as
`plain <= plain ^ toggle` with `toggle[i] = &plain[i-1:0]` and maps onto
no carry cell at all — `blink.v`'s idiom, kept for exactly this purpose.
Both start at zero and both add one per cycle, so they are equal on every
cycle, and LD4 latches the first cycle they are not.

A person at the board read **LD4 lit**. LD5, which latches "the carry
counter left zero", was **also lit**, so the chain is not frozen: it
moves and it computes something other than plus one.

What the build said about the same bitstream, which is the part worth
sitting with:

- `7 CARRY4`, `200 of 200 signal(s) with a reader routed`
- `5969 configuration bit(s) set`, and on decode **`0 bit(s)
  unexplained, 0 tile(s) with no segbits file`**
- `carry_probe_tb.v` passes: 5000 cycles with the two counters agreeing
  exactly, so the RTL is not at fault

So this is the ECP5 lesson again, on a different family and found the
same way. A design can place, route, produce a bitstream in which every
set bit decodes back through the vendor's own database into the arcs the
router chose, pass its own simulation — and compute the wrong number.
Only the part says so.

**What is not yet established is which carry is wrong.** A chain whose
`COUT` never reaches the next cell's `CIN`, and a chain whose first
`CYINIT` is not the one that `+ 1` needs, both show as one lit LED. The
two are distinguishable: the first makes every fifth bit wrong and the
second makes the whole count wrong by one, and a design that reports the
counter's low byte rather than one bit of it would separate them. That is
the next measurement, not a conclusion to draw from this one.

Note that `97fd6bc` is called *"give a slice's `CARRY4` its pins, its
muxes and its constants"* and `68bc282` adds a config entry for a pin
tied to a constant, including `PRECYINIT.C1` for a carry-in of one. Those
are where to look first, and the fact that they exist is why this needs
measuring rather than assuming.

### And a constant on a pad: the code gap is real, the measurement is not

> **Also retracted as a measurement.** LD0 is ball U16, which read dark
> with a constant here, worked correctly in `sw_led`, and read stuck
> *lit* in `io_exercise` — three behaviours on three designs, so it
> cannot carry a claim. A later re-run put the constant on ball P1 and
> read it dark, which is suggestive and still not proof, because in that
> same run a counter bit that should have flickered was solid.
>
> What survives is the **source** observation, which needs no board:
> `techcells::drive_constant_data` genuinely does not cover an output
> buffer's input, so a pad driven by a constant has its input left
> unrouted. That is a real gap in the code and worth fixing. Whether it
> is what the board showed is unproven.
>
> **Closed on 8 October 2026**: the pass covers an `io` primitive's data
> pin now, and `assign led = 1'b1` reaches the pad from a lookup table.
> The gap was real; what the board showed about it is still unproven, and
> nothing here has been back on a part since.

On the same glance, LD0 — driven by the constant `1'b1` — read **dark**,
beside LD1 driven by `1'b0`, also dark. Both constant-driven pads are
dark whatever the constant is, while every pad driven by a register or a
lookup table behaves: `sw_led`'s exclusive-or followed the switches
through all four combinations on the same afternoon, and
`(por == 4'hF)` lit after fifteen clock edges.

The cause is in the flow and not in the fabric.
`techcells::drive_constant_data` gives a real driver to a constant that
reaches a flip-flop's data pin, a distributed RAM's inputs, or — since
the ECP5 carry round — a carry cell's operands. It does **not** cover an
output buffer's input, so `assign led = 1'b1` leaves the pad's input
unrouted, and an unrouted input reads low on this family and **high** on
an ECP5. The same source defect therefore has opposite symptoms on the
two families, and neither produces a diagnostic.

Rebuilding the identical design on `1f99eb9`, which is where the ECP5
round's carry-operand fix landed, produced a **byte-identical**
bitstream — the pass reports `0 constant lookup table(s) added` — so
neither fault is addressed by it and both stand on current `master`.

### Most IO pads do not follow their logic in a design with many of them

> **RETRACTED as a diagnosis, and the title is wrong.** It is not about
> pads and it is not about how many. All 49 of `io_exercise`'s pads are
> configured correctly, which was measured on 8 October 2026 and is
> below; the fault was 68 welded joins in the routing, which the section
> at the top of this document records and fixes. The readings are kept
> because they are real readings of a real part; the heading and the
> conclusion are not.
>
> **The measurement that killed this theory.** `io_exercise`'s bitstream
> was decoded and every one of the 49 pads was resolved through
> `package_pins.csv` to its `IOB` site, its `LIOB33`/`RIOB33` tile and
> its `_Y0`/`_Y1` half, and the features compared. Every input pad —
> **including every stuck one** — carries exactly
> `IN_ONLY`, `LVCMOS25_LVCMOS33_LVTTL.IN`, `PULLTYPE.NONE` with
> `ILOGIC_Y<n>.ZINV_D` behind it, and every output pad exactly
> `SLEW.SLOW`, `DRIVE.I12_I16`, `PULLTYPE.NONE` with
> `OLOGIC_Y<n>.OMUX.D1`, `OQUSED`, `OSERDES.DATA_RATE_TQ.BUF`. For the
> 32 balls `artix7/harness/basys3/swbut` also drives, that is **feature
> for feature what Vivado put there** — the same comparison
> `the_io_path_is_the_one_vivado_built` makes for three pins, now made
> for thirty-two. Each buffer is also bound to the site its `set_io`
> names, checked instance by instance against `package_pins.csv`.
>
> So the pads were never the problem, and the stuck/working split
> separates on nothing about the IO: it separates on whether that
> signal's route took a welded join.

**CHECKED, 8 October 2026**, with `examples/basys3/io_exercise.v`: fifteen
LEDs each mirroring one switch, five buttons, and the four digits. The
design was simulated first and passes
(`io_exercise_tb.v`: every switch to its own LED with the LD6 gap, all
five buttons, all four digit enables), so what follows is the part's
answer and not the RTL's.

> **The design has since grown a sixteenth LED**, which is why the
> readings below mention fifteen and `sw[6]` with no LED of its own. LD6
> (ball U14) was unreachable then and is not now — see "LD6 was a
> missing feature, not a dead ball" — and `io_exercise.v` now drives
> `led[n]` from `sw[n]` for all sixteen. The readings are left as they
> were taken.

Two readings, all switches down and then all switches up:

| | reads | balls |
|---|---|---|
| `sw[5] sw[7] sw[8] sw[9] sw[10] sw[11] sw[14]` | **correctly** | V15 W13 V2 T3 T2 R3 T1 |
| `sw[0] sw[1] sw[2] sw[3] sw[4] sw[12] sw[13] sw[15]` | **stuck high** | V17 V16 W16 W17 W15 W2 U1 R2 |
| `sw[6]` | not observable — no LED | W14 |
| `btn_d` | **correctly** | U17 |
| `btn_c btn_u btn_l btn_r` | **stuck low** | U18 T18 W19 T17 |
| all fifteen LEDs | **correctly** (all lit under BTND) | — |

The outputs are affected too, and `an` proves it arithmetically rather
than by eye: `an = ~(4'd1 << digit)` can only ever have **one** bit low,
so at most one digit can be enabled at a time — and three digits were lit
with the fourth dark. The decimal point, which is `~count[25]`, did not
blink at all, while BTND worked, so the clock and the flip-flops are
fine and it is that pad that is not following its net.

**The part that makes this a flow defect and not a board fact: V17 and
V16 are in the stuck set, and those same two balls worked correctly in
`sw_led`** — a person flipped them and watched the exclusive-or follow
through all four combinations on the same afternoon. Same balls, same
board, two designs, opposite results. So the pads are reachable and
something about configuring many of them at once is wrong.

What the build claims about the same bitstream, which is again the
uncomfortable part: `22 x IBUF`, `27 x OBUF`, `179 of 179 signal(s) with
a reader routed`, `0 feature(s) missing`, and on decode `0 bit(s)
unexplained, 0 tile(s) with no segbits file`.

One figure that looks like evidence and is not: the report's
`16 io buffer(s)` is the same in a three-pad design as in this
forty-nine-pad one, so it counts something in the database rather than
what was configured. Noted because it cost a wrong conclusion.

Not yet established: whether the split is by IO tile type, by bank, by
which pads share a tile, or by placement order. The cheap next experiment
is the same design cut down to only the eight stuck switches — if they
work in a small design, the fault depends on how many pads are in play.

**Answered, and by none of those.** The split is by whether the net took
a welded join — all four LED nets with a bogus hop are in the stuck set
of eight and none of the seven that worked has one; the top section has
the breakdown. A later board reading also
withdrew the "stuck high inputs" reading itself: the eight LEDs in
question were already lit, so flipping their switches proved nothing
about the *inputs*, and a switch-at-a-time reading found no permutation
— every responding switch lit the LED directly above it.

### LD6 was a missing feature, not a dead ball — and it is done

**DONE, 8 October 2026.** `examples/basys3/io_exercise.v` has sixteen
LEDs and no gap, `led[6]` is ball U14, and what this flow writes in that
ball's tile is feature for feature what Vivado writes there. The rest of
this section is how, and what it cost; the full account of the mechanism
is in `fpga::xray::TileAlias`.

**The wrong reading it replaces.** `blink_carry.rcf`, `blink_carry.v` and
`bram_rom.v` all described LD6 (ball U14) as skipped because it sits in a
`LIOB33_SING` tile this flow's IO tables did not describe. That was
accurate about the flow and read as if the ball were unusable, which it
is not: the board's owner confirms LD6 works and that this was a
programming gap. Those comments now say which it was.

**It was never one LED.** `*_SING` tiles are the single-IOB tiles at
*both* ends of *every* IO bank — twenty of them on an `xc7a50t` die,
four tile types — so this was the top and bottom ball of every bank on
every 7-series part this backend reaches, and U14 was just the one a
person could see.

#### The oracle, read in full

`artix7/harness/basys3/swbut/design.txt` gives U14 as `dout[6]`,
`package_pins.csv` gives it as `IOB_X0Y0` of `LIOB33_SING_X0Y0`, and
`design.json`'s `required_features` show Vivado configuring it with
**exactly the ordinary output recipe** on the `_Y0` half. Two more of
the harness designs drive such a ball, which between them cover the
other edge of the die, the other half and the other direction:

| design | tile | what Vivado sets |
|---|---|---|
| `basys3/swbut` | `LIOB33_SING_X0Y0` | `IOB_Y0.IN_TERM.NONE`, `IOB_Y0.…SLEW.SLOW`, `IOB_Y0.LVCMOS33_LVTTL.DRIVE.I12_I16`, `IOB_Y0.PULLTYPE.NONE` |
| `basys3/swbut` | `LIOI3_SING_X0Y0` | `IDELAY_Y0.IDELAY_TYPE_FIXED`, `ILOGIC_Y0.IDELMUXE3.P1`, `ILOGIC_Y0.IFF.SRTYPE.ASYNC`, `ILOGIC_Y0.ISERDES.MODE.MASTER`, `ILOGIC_Y0.ISERDES.NUM_CE.N1`, `OLOGIC_Y0.OMUX.D1`, `OLOGIC_Y0.OQUSED`, `OLOGIC_Y0.OSERDES.DATA_RATE_TQ.BUF` |
| `arty-a7/swbut` | `RIOB33_SING_X43Y50` | the same four, on `IOB_Y0` |
| `arty-a7/swbut` | `RIOI3_SING_X43Y50` | the same eight, on `_Y0` |
| `arty-a7/pmod` | `LIOB33_SING_X0Y99` | `IOB_Y1.IN_TERM.NONE`, `IOB_Y1.…IN_ONLY`, `IOB_Y1.…SLEW.FAST`, `IOB_Y1.LVCMOS25_LVCMOS33_LVTTL.IN`, `IOB_Y1.PULLTYPE.NONE` — an **input**, on the `_Y1` half |
| `arty-a7/pmod` | `LIOI3_SING_X0Y99` | `IDELAY_Y1.IDELAY_TYPE_FIXED`, `ILOGIC_Y1.IDELMUXE3.P1`, `ILOGIC_Y1.IFF.SRTYPE.ASYNC`, `ILOGIC_Y1.ISERDES.MODE.MASTER`, `ILOGIC_Y1.ISERDES.NUM_CE.N1`, `ILOGIC_Y1.ZINV_D` |

So the feature *names* are the ordinary ones and the half is `_Y0` at the
bottom of a bank and `_Y1` at the top. That much was already quoted here
before any of it was built; the first version of this section said
`_Y0` was "the only half a `_SING` tile has", which the `arty-a7/pmod`
row above disproves.

#### The hypothesis that was half wrong, and what replaced it

The obvious guess was that a `_SING` tile is its non-`SING` neighbour
with one IOB unpopulated, so the neighbour's `segbits` would apply at
the `_SING` tile's own frame address. **Wrong about the shape**, and
`tilegrid.json` says so: a `LIOB33` window is `words: 4` and a
`LIOB33_SING` window is `words: 2`. Had that guess been coded, every
feature would have landed two words out.

What `tilegrid.json` has instead is an `alias` member on those tiles'
bit windows, which nothing here read:

```json
"LIOB33_SING_X0Y0": { "bits": { "CLB_IO_CLK": {
  "baseaddr": "0x00400000", "frames": 42, "offset": 0, "words": 2,
  "alias": { "type": "LIOB33", "start_offset": 2,
             "sites": { "IOB33_Y0": "IOB33_Y0" } } } } }
```

`type` names the tile type whose `segbits` describe these bits and
`start_offset` the word of *that* type's bitmap where this tile's own
words begin. So a feature of the aliased type at bit offset `c` is at
`c - 32 * start_offset` here, and a feature whose bits then fall outside
this tile's own `32 * words` is not a feature of this tile at all.

**That filter is what picks the half, and it needs nothing else.
CHECKED** over the whole of `segbits_liob33.db`, `segbits_riob33.db`,
`segbits_lioi3.db` and `segbits_rioi3.db`: of 382 half-named features,
every `_Y0` one's bits lie in words 2..3 and every `_Y1` one's in words
0..1, with no exceptions. A tile with `start_offset` 2 therefore keeps
exactly the `_Y0` half and one with `start_offset` 0 exactly the `_Y1`
half — which is the table above, derived rather than transcribed.

The `alias`'s `sites` member says the same thing a second way for the
`IOB` tiles (the top tile's lone site is aliased `IOB33_Y0` →
`IOB33_Y1`) and is **empty for `LIOI3_SING` and `RIOI3_SING`**, although
Vivado still names the top tile's features `ILOGIC_Y1`. So it is not
read: the bit window is the rule, and `sites` agrees with it wherever it
says anything.

#### Two things that had to come apart

- **Two `_SING` tiles of one type are two tile types here.** The two
  halves' bits are not at the same offsets within the tile — after the
  shift `IOB_Y0.…SLEW.SLOW` lands at bits 41..47 and `IOB_Y1`'s at
  16..22 — so one `arch::TileType` cannot describe both. Each alias
  offset gets its own (`LIOB33_SING_W2`, `LIOB33_SING_W0`), and a
  `tileconn.json` join that names the far end's *database* type is
  declared once per variant, of which at most one resolves because a
  tile is of exactly one type.
- **The wires are numbered 0 in both.** `ppips_lioi3_sing.db` and
  `tileconn.json`'s `LIOB33_SING`/`LIOI3_SING` pairs name `IOB_IBUF0`,
  `IOB_O0`, `IOI_OLOGIC0_D1` and nothing with a 1 in it. So a tile whose
  features are `IOB_Y1.…` still has its buffer on `IOB_IBUF0`, and
  `sites::bel_pins` must not take the wire index from the feature
  prefix the way the two-IOB tiles do. `sites::pass_throughs` offers
  *both* halves' features on the one wire pair and lets the tile's own
  (already filtered) feature set keep the one it has.

#### What was measured, and what was not

- **CHECKED.** `every_single_io_tile_vivado_drove_decodes_to_exactly_what_it_needed`
  decodes all three harness bitstreams and asserts the features in every
  `_SING` tile are exactly the ones that design's `required_features`
  names — six in `basys3/swbut`, six in `arty-a7/swbut`, four in
  `arty-a7/pmod`, with the rest dropped because their `segbits` line has
  no bit that must be one and so can never be decoded.
- **CHECKED.** `the_single_io_tile_this_flow_now_drives_is_configured_as_vivado_configures_it`
  builds a one-pin design driving U14 and compares it with the Basys 3
  harness, feature for feature, on both `LIOB33_SING_X0Y0` and
  `LIOI3_SING_X0Y0` — and does it **three times**, with two different
  companion switches and two region sizes, because a result from one
  build is a property of that build (see "retracted" above). The three
  routes differ (32, 34 and 32 bits over 15, 13 and 15 tiles) and the
  tile's features do not.
- **CHECKED.** Decoding the Basys 3 harness used to report `11 tile(s)
  with no segbits file`; it now reports **0**. Those bits were excluded
  from the `unexplained` count rather than added to it, which this
  document already called a blind spot in the strongest check here. It
  is one tile type's worth smaller.
- **NOT established.** A `_SING` tile used as a bottom-half **input** or
  a top-half **output**. No harness drives either, so both rest on the
  shift alone — which is measured four ways and is the same arithmetic —
  and neither has been seen.
- **NOT established.** Anything on silicon. The 20 `_SING` tiles of an
  `xc7a50t` include 14 whose ball this package does not bond at all; of
  the two `cpg236` bonds, only U14 has been built for.
- **Nothing here is a transceiver or an `XADC` ball.** `GTP_COMMON`,
  `GTP_CHANNEL_*` and `MONITOR_BOT` have no `segbits` and no alias, and
  a design constraining a port to one of their balls now gets a
  diagnostic naming the ball, the site, the tile and the tile type
  instead of a bare "maps to no usable site".

### It is the clock: flip-flops outside some set of columns never tick

> **WITHDRAWN as a conclusion, 8 October 2026, by the section at the top
> of this document.** Every reading quoted below is explained by a
> routing-graph defect that welded two rows of interconnect together, and
> that defect has been measured and fixed. The text is kept because the
> reasoning is sound given what was known: it is what you conclude when
> five LEDs on five different nets all read one, and it was wrong only
> because an undriven wire reads one too.

**This is where the evidence points, and it explains every reading above
including the two retracted ones.**

The decisive observation was already in `diag` and was misread as a
success: its **4-bit** `por` counter reached 15 and held, while its
**26-bit** counter sat frozen. Both are clocked by the same net off the
same `BUFG`. So the clock reaches some flip-flops and not others, and
whether a given register ticks depends on where it was placed.

Then `carry_probe.v` was re-run with every output moved onto balls that
had been *watched changing state*. Expected: LD5 dark, LD7 lit, LD8 and
LD9 blinking, LD10 flickering, LD11 dark, LD14 lit. Read: **LD5, LD7,
LD8, LD9, LD10 all solid on; LD11 and LD14 dark.** That is not a pattern
of random pads — it is exactly a frozen counter, with `plain[22]`,
`plain[25]`, `chain[25]`, `differed` and `moved` all stuck at one and
`1'b0` correctly dark. The same signal `plain[22]` flickered correctly
when an earlier constraint file put it on ball V14, and is solid on W3
here: identical RTL, different placement.

What `--report` says about the same designs, on the same part:

| design | clock pins | `global clock rebuffer enable bit(s) over the column` | behaves? |
|---|---|---|---|
| `blink` | 26 | 4 | **yes**, watched 2026-09-24 |
| `carry_probe` with `carry_probe.rcf` | 54 | 4 | no |
| `carry_probe` with `carry_probe_trusted.rcf` | 54 | 6 | no |
| `io_exercise` | 36 | 8 | partly |

Two constraint files over one unchanged design give **4 enable bits and
6**, so the count tracks placement; and `carry_probe` asks 54 flip-flops
to run off the same number of enabled columns that sufficed for `blink`'s
26. A design that happens to fit inside the enabled columns works, which
is why the two designs that were ever watched working are the two
smallest, and why every larger one has looked like scattered dead pads.

The question to answer is whether this flow enables the global clock in
every clock column and clock region holding a clocked flip-flop, and the
method is the one that has found everything else here: read what Vivado
writes, in full, for a design whose registers span several clock regions,
and check ours feature by feature.

**Everything on this board is unmeasurable until that is settled.** A
counter that cannot be trusted to count cannot report on a carry chain, a
block RAM, a PLL or a pad.

### A PLL works on the part, and its own two tiles do not decode

**CHECKED, 8 October 2026, both halves.**

The working half: `examples/basys3/selftest.v` asks for 25 MHz by frequency
and reports, from the part, `locked` high and **262 081 PLL cycles against
262 144 expected** over a window of 2^20 oscillator cycles — 99.976 %, a
deficit of a few cycles lost where the window's level crosses into the
PLL's domain, not a frequency error. A `PLLE2_BASE` from this flow locks
and runs at the ratio it was asked for. The flow's own note said *"no PLL
from this flow has run on a part"*; that is no longer true and the note
now says what was measured instead.

The exactness is better than it needs to be. Asked for 8 MHz from the
board's 100 MHz, the solver answers **+0.0 ppm** — vco 800 MHz,
`DIVCLK_DIVIDE=1 CLKFBOUT_MULT=8 CLKOUT0_DIVIDE=100` — and a PLL output
routes straight to a pad (`OBUF`), so a generated clock can leave the part.

The other half: **a design with a PLL has two tiles with no segbits file**,
and the same design with the PLL removed has none. So the PLL's own CMT
tiles cannot be decoded back through the database, and "every bit decodes"
goes quiet exactly there — 166 PLL register bits are written and not
checkable. They are evidently *right*, since the part locks and counts
correctly, but they are not *verified*, and on this project that is a
different claim.

So a design needing an exact frequency pays two undecodable tiles for it.
Where a ratio will do, a fabric divider costs nothing and stays fully
decodable: 100/12 is 8.333 MHz, and anything deriving its own timing by
counting the clock — a smart card's etu is `F/D` cycles of the terminal's
clock — tracks it with no error at all.

### RETRACTED: there are no "bad balls" — and what the count really means

> **This section claimed six Pmod balls could not be used bidirectionally,
> and it was wrong.** The board's owner pushed back, correctly: a
> factory-tested Basys 3 has no bad balls, and Digilent's documentation says
> all eight JA signals are bidirectional by design. The error was mine, the
> phrase "bad balls" was indefensible, and worse, I used the table to tell
> the owner which pins to avoid. That advice had no basis.

**What the measurement actually was.** Building `pmod_bidir.v` once per
ball and reading the decode line, six balls reported `1 tile with no
segbits file`. I recorded that as a property of the ball. It is not:

| ball | LEDs U16/E19 | LEDs V19/W18 | LEDs U15/V14 |
|---|---|---|---|
| J1 | 0 | 0 | 0 |
| **L2** | **1** | **0** | **1** |
| **K2** | **1** | **0** | **1** |
| H2 | 0 | 0 | 0 |

**Same ball, same design, different LED pins, different answer.** Moving
two unrelated pins moves the result, so what varies is the *route*, not the
ball. One build each was never enough to claim a property, and the tile
types say the same: all eight JA balls are `RIOB33`, and `RIOI3_X43Y89`
(reported) and `RIOI3_X43Y95` (clean) are the same type.

**What the count does mean**, which is worth knowing and is not nothing.
`Decoded::tiles_without_a_segbits_file` is, in its own words, "of those
[tiles holding at least one set bit], how many have no `segbits` file at
all, so nothing their bits say could have been named". So the flow sets
bits in a tile whose contents cannot be checked — **and `unexplained` reads
zero alongside it**, so those bits are excluded from the unexplained count
rather than counted in it. That is a blind spot in this project's strongest
check, and it is not reported as one. Making those bits count as
unexplained, or refusing them, is the fix; the number is a symptom.

**Why most types have no segbits, and why that is normal.** Of 112 tile
types on this die only **56** have a segbits file. The 70 without are
mostly tiles with nothing to configure — `NULL` (2491), `VBRK` (1400),
`INT_FEEDTHRU_1/2` (1125), `INT_INTERFACE_L/R` (850), the `TERM` tiles,
`VFRAME`. `IO_INT_INTERFACE_L/R` (250 tiles) is exactly what an IO route
crosses. So "no segbits file" is the ordinary state for an interface tile
and says nothing by itself; it only matters when bits are *set* there.

### LD6 was a flow gap too, and it is closed

**The reading this replaces, kept because the path to the answer is the
useful part.** Ball `U14` sits in a `LIOB33_SING` (this once said
`RIOB33_SING`, which was wrong about the edge), and the database ships
segbits for `liob33`, `riob33`, `lioi3`, `rioi3` and the `tbyte`
variants but for **no `_SING` variant at all**. The conclusion drawn
from that was that the single-IOB tiles at the ends of each bank are
simply unsupported, and the to-do was to reverse-engineer them.

**That was the wrong conclusion from a true observation.** There is no
`segbits_liob33_sing.db`, but `tilegrid.json` carries an `alias` on
those tiles' bit windows that says which type's `segbits` do describe
them and at what word offset — so nothing needed reverse-engineering,
only reading. See "LD6 was a missing feature, not a dead ball" above for
the mechanism and for what Vivado's own bitstreams say about it.

### A PLL works on the part, and its own two tiles do not decode

**CHECKED, 8 October 2026, both halves.**

The working half: `examples/basys3/selftest.v` asks for 25 MHz by frequency
and reports, from the part, `locked` high and **262 081 PLL cycles against
262 144 expected** over a window of 2^20 oscillator cycles — 99.976 %, a
deficit of a few cycles lost where the window's level crosses into the
PLL's domain, not a frequency error. A `PLLE2_BASE` from this flow locks
and runs at the ratio it was asked for. The flow's own note said *"no PLL
from this flow has run on a part"*; that is no longer true and the note
now says what was measured instead.

The exactness is better than it needs to be. Asked for 8 MHz from the
board's 100 MHz, the solver answers **+0.0 ppm** — vco 800 MHz,
`DIVCLK_DIVIDE=1 CLKFBOUT_MULT=8 CLKOUT0_DIVIDE=100` — and a PLL output
routes straight to a pad (`OBUF`), so a generated clock can leave the part.

The other half: **a design with a PLL has two tiles with no segbits file**,
and the same design with the PLL removed has none. So the PLL's own CMT
tiles cannot be decoded back through the database, and "every bit decodes"
goes quiet exactly there — 166 PLL register bits are written and not
checkable. They are evidently *right*, since the part locks and counts
correctly, but they are not *verified*, and on this project that is a
different claim.

So a design needing an exact frequency pays two undecodable tiles for it.
Where a ratio will do, a fabric divider costs nothing and stays fully
decodable: 100/12 is 8.333 MHz, and anything deriving its own timing by
counting the clock — a smart card's etu is `F/D` cycles of the terminal's
clock — tracks it with no error at all.

### What is now known about this part, in order of confidence

| | |
|---|---|
| lookup tables and LVCMOS33 pads | CHECKED twice, by two designs and two people's glances |
| the 100 MHz oscillator, `BUFG` and the clock tree | CHECKED: a 4-bit counter reaches 15 and stays there, and a 26-bit carry-free counter blinks |
| flip-flops | CHECKED, by the same two counters |
| a `CARRY4` chain | **CHECKED CORRECT** — `carry_where.v`, all 26 bits, with a control |
| a pad tied to a constant | **UNKNOWN** on the board. The gap in `drive_constant_data` is real in the source and needs no board to see |
| the global clock reaching every flip-flop | **UNKNOWN.** Called "CHECKED WRONG" on the strength of five solid LEDs and one frozen counter; the five LEDs turned out to be welded joins (top section) and the frozen counter is explained the same way. Nothing has been shown wrong about the clock itself |
| `RAMB18E1`, `RAM64X1D`, `PLLE2_BASE` | the part accepts the bitstreams with `DONE` high, which means CRC passed and nothing more. Not run. |
| the board's serial port | UNRESOLVED. Nothing this flow drives onto A18 has reached the host, including a bare counter bit with no UART in it, so the pin assignment in `examples/soc/board/basys3.rcf` is in doubt and `examples/basys3/selftest.v` cannot yet report. |

All seven designs in `examples/basys3/` were loaded on 8 October 2026 and
every one brought `DONE` high with no CRC error. That is worth exactly
what it says and no more: the part accepted the image. Two of them have
since been shown to compute the wrong thing.

## Getting the database

It is [`f4pga/prjxray-db`](https://github.com/f4pga/prjxray-db), Project
X-Ray's chip database, **CC0-1.0 — public domain**. It is not in this
repository and must not be. The whole repository is large; 275 files and
45 MB of it are needed for the Artix-7.

**The command line fetches it by itself.** The first `reticle fpga
--bitstream` that needs it downloads that part into the per-user cache,
`$XDG_CACHE_HOME/reticle/prjxray-db/<commit>` (`~/.cache/reticle/...`
without `XDG_CACHE_HOME`), and every later run finds it there. To fetch
ahead of time, before going offline, or for the tests:

```sh
reticle fetch prjxray-db
```

The copy is pinned to one commit, `0a0adde`, and each file is checked
against a SHA-256 digest recorded from a checkout of that commit and
built into the binary (`src/bin/reticle/prjxray-db.manifest`). Upstream
moving on changes nothing here, and a file that does not match is refused
with nothing installed. The download is done by the system's `curl`,
over HTTPS only.

**The library still never fetches anything.** It is sans-I/O: the
database reaches it through a `FileProvider` the caller supplies, rooted
wherever the caller says. Fetching is the command line's job, in
`src/bin/reticle/datadir.rs`.

A copy made by hand works just as well, and is what `RETICLE_CHIPDB` or
`--chipdb <dir>` is for:

```sh
git clone --filter=blob:none --no-checkout --depth 1 \
    https://github.com/f4pga/prjxray-db.git
cd prjxray-db
git sparse-checkout set --no-cone \
    '/artix7/*.db' '/artix7/*.csv' '/artix7/xc7a35tcpg236-1/' \
    '/artix7/xc7a50t/' '/artix7/mapping/' '/artix7/harness/'
export RETICLE_CHIPDB=$PWD
```

Then:

```sh
reticle fpga --device xc7a35t-cpg236 \
    --constraints examples/basys3/sw_led.rcf \
    --bitstream sw_led.bit \
    examples/basys3/sw_led.v

# and the clocked one, which loads the clock column as well and takes a
# few seconds longer:
reticle fpga --device xc7a35t-cpg236 \
    --constraints examples/basys3/blink.rcf \
    --bitstream blink.bit --report \
    examples/basys3/blink.v

# and the PLL, which nothing has run on a part yet (see "The PLL"):
reticle fpga --device xc7a35t-cpg236 \
    --constraints examples/basys3/pll_blink.rcf \
    --bitstream pll_blink.bit --report \
    examples/basys3/pll_blink.v
```

`--chipdb <dir>` names the database explicitly; `RETICLE_CHIPDB` is the
fallback; the cache comes after both, and a download after that unless
`--offline` or `RETICLE_OFFLINE=1` forbids it. A path that is named and
does not hold the database is an error, never a reason to download.

The tests find the database the same way, minus the download:
`RETICLE_CHIPDB`, then the cache. **CI has neither**, and every test
that needs the database skips with a line saying what is missing. A
missing database never fails the build, and a test never fetches one.

## What each file gives

| File | What is taken from it |
|---|---|
| `artix7/mapping/devices.yaml` | which fabric a die uses. This is the one fact that makes a Basys 3 possible at all: `xc7a35t` **is** an `xc7a50t` die, so the fabric lives in `artix7/xc7a50t/` |
| `artix7/xc7a35tcpg236-1/part.json` | the IDCODE and the frame layout, column by column. `part.yaml` says the same and is ignored, so that no YAML parser is needed |
| `artix7/xc7a35tcpg236-1/package_pins.csv` | package pin to site |
| `artix7/xc7a50t/tilegrid.json` | every tile: name, type, grid position, its sites, and where its bits live in the frames |
| `artix7/xc7a50t/tileconn.json` | which wire of a tile is the same metal as which wire of its neighbour |
| `artix7/segbits_<type>.db` | which bits switch on which pip and which bel feature |
| `artix7/ppips_<type>.db` | the fixed, unprogrammable wiring *inside* a tile — which is how a site pin reaches the interconnect, and the file phase one did not know it needed |

The JSON is read by `crate::json`, the hand-written parser the language
server already had; it moved to the crate root rather than being written
a second time. No dependency was added, here or anywhere.

`devices.yaml` is read by a deliberately narrow reader that accepts the
two-line shape that file has and refuses anything else. Pretending a
loose reader is a YAML parser would be worse than admitting it is not
one.

### What `artix7/harness/` contained

It is the best thing in the repository for this purpose, and it was not
what its README suggests. The README describes harness bitstreams for
placing designs in a region of interest with open tools. What is
actually there is four complete, Vivado-produced `.bit` files with their
checkpoints and a port map — and one of them,
`artix7/harness/basys3/swbut/design.bit`, is **for this exact part**:
2 192 111 bytes, `7a35tcpg236`, Vivado 2017.2, 11 September 2019.

That file is the oracle this whole module was built against. Everything
in the list at the top of this document is a comparison with it. Without
it, the pad-frame rule and the command sequence would have been guesses;
with it they are measurements.

Beside it is `design.json`, whose `required_features` array names all 863
features Vivado's placement and routing occupy. That is what the IO
buffer configuration was read off: filtered to the two `LIOB33` tiles
holding a switch and an LED it gives four feature names for an input and
four for an output, out of the eighty-three that tile type has. The
`LIOI3` tiles beside them give one feature for the input path and three
for the output path. Nothing else in those tiles is set, which is how
the tables in `src/fpga/xray/sites.rs` know when to stop.

`design.txt` names each port's package pin and the interconnect node it
reaches, which is what fixed the two orientations nothing else in the
database states — see *Which half is `_Y0`* below.

## The measured size of the fabric

The numbers the loader reports for `xc7a50t`, produced by the loader
itself rather than written down here by hand:

| | |
|---|---|
| tiles | 18 055 |
| tile types | 112 |
| nodes (the database's own `element_counts.csv`) | 7 857 396 |
| segbit features over the die | 23 536 635 |
| of those, pips | 20 648 208 |
| frames | 5420 (5408 data + 12 pad) |
| tiles with a frame window | 10 373 |

20.6 million pips was the scale problem, and it was real. Most of it is
interconnect: an `INT_L` or `INT_R` tile has 3636 features, all of them
pips, and the die has 5650 of those tiles. Phase one's `RoutingGraph`
gave every pip a `Vec<ConfigBit>` of its own, which meant 5650 identical
copies of the same 3636 patterns and several gigabytes before anything
was routed.

**The patterns are interned now.** Every distinct list of bits is stored
once in the graph and a pip keeps a four-byte index into it, so a pip is
twenty bytes with no allocation of its own. That is a change to how the
bits are stored and nothing else: the router's algorithm is untouched.

The whole die now fits, and these are measurements, not estimates —
`RoutingGraph::heap_bytes` counts the vectors themselves and
`tests/fpga_xray.rs::the_whole_die_is_a_graph_this_crate_can_hold`
prints them:

| | |
|---|---|
| graph edges declared | 44 304 488 |
| of those, kept (the rest reference a tile the grid has not got there) | 30 918 986 |
| nodes | 6 081 818 |
| distinct bit patterns, shared by all of them | 9 774 |
| `RoutingGraph::heap_bytes` | 1386 MiB |
| peak resident while building | 2.0 GiB |
| time to build | about 8 s in a release build |

Two gigabytes is still more than a three-cell design should pay, so
`XrayOptions::region` still says which rectangle of tiles gets wires,
pips and bels, and `XrayOptions::max_pips` still makes the loader *count
first and allocate after*: a region that will not fit is refused with the
numbers rather than with a dead machine. The default limit is now set
from the measurement above instead of from a guess. `reticle fpga` picks
the region from the pins the constraints name, expanded by twelve tiles,
because that is where the design has to be.

A 325-tile region around the Basys 3's switch and LED pins is 101 153
wires, 527 822 pips and 793 bels, and the whole load — including parsing
the 6.6 MB `tilegrid.json` and the 11.9 MB `tileconn.json` — takes well
under a second in a release build. A 4462-tile region covering a quarter
of the die is 7.2 million pips and still loads in a couple of seconds.

The frame map always covers the whole part, so the bitstream is a
whole-part bitstream however small the region.

**What is left is the nodes.** A `Wire` owns its name as a `String`, and
six million of those are most of the 1386 MiB that remains. Interning
those the same way is the obvious next step and was not needed to route
the milestone.

## Where Reticle's model fitted, and where it did not

It fitted better than expected. The tile-bitmap model that
`arch::synthetic` invented for an iCE40 turned out to be almost exactly
7-series frame addressing: a tile's `ConfigBit { row, col }` *is* frame
`baseaddr + row`, word `offset + col / 32`, bit `col % 32`. The
`ConfigEntry::Param` mechanism that carries `LUT_INIT` into an iCE40
bitstream carries a 7-series `INIT[63:0]` unchanged, and
`src/fpga/bitstream.rs` needed no edit at all — which is what its header
has been claiming since it was written.

Three mismatches are real, and none of them is papered over.

### 1. A 7-series wire is tile-local; a node is wires joined

`Arch` describes a wire by a *span*: `sp4e` starts here and reaches four
tiles east, and every tile it crosses sees the same node. A 7-series long
line has a **different name in each tile** it passes through, and
`tileconn.json` says which name equals which. That cannot be written as
a span.

The loader gives every wire span `0 0` and emits each `tileconn` pair as
two pips with no configuration bits — which the model already means as "a
connection that is always there". It is faithful, and it costs one graph
edge per join per direction: 176 477 of the 527 822 pips in the 325-tile
region the milestone loads are joins, and over the whole die a third of
the 30.9 million edges are.

### 2. A tile's bits live at a frame address, not in a per-tile bitmap

`Arch` has nowhere to put a frame address. The mapping comes out beside
the architecture as an `xc7::FrameMap`, and `XrayFabric` carries the two
together. Nothing is lost, but an `.arch` file written with
`Arch::to_text` cannot carry it, so **a real 7-series architecture does
not round-trip through the text format** the way the synthetic one does.
Adding a `tilebits` directive would fix that; it was not done, because
18 055 such lines is not a file anyone would read.

### 3. The database ships bits, not a wire list

`prjxray-db` ships `segbits_<type>.db`, whose lines are
`<TILE_TYPE>.<A>.<B> <bits>`. It does **not** ship the per-tile-type wire
and pip lists; prjxray generates those from Vivado into
`tile_type_*.json`, which is not in the repository.

Phase one read that as "which wire a bel pin reaches is not in the
database", and declared bels with no pins. That was too pessimistic. The
wiring *is* there, in a file phase one did not read: `ppips_<type>.db`.
Its lines are `<TILE_TYPE>.<dest>.<source> <kind>`, and the kind is one
of three words that mean three quite different things.

| Word | What it is | What the loader does |
|---|---|---|
| `always` | the connection is simply there. `CLBLL_L.CLBLL_L_A1.CLBLL_IMUX6 always` is a slice's `A1` pin reaching interconnect index 6 | a pip with no bits — **this is the site-pin wiring that was missing** |
| `default` | on unless something else drives the same wire. `INT_L.BYP_ALT0.VCC_WIRE default` is a tie-off | ignored; taking it would offer the router a constant one everywhere |
| `hint` | Vivado reports it, but it goes *through* a site. `CLBLL_L.CLBLL_L_A.CLBLL_L_A1 hint` is a lookup table used as a wire | ignored; taking it would route through logic a cell is sitting on |

So what the database really does not name is only the *pin*: the strings
`A1`, `O6`, `I`, `O` and which of those wires carries each. That is what
`src/fpga/xray/sites.rs` supplies, and every line of it is labelled with
where it came from — UG474 *7 Series FPGAs Configurable Logic Block* for
the CLB, UG471 *7 Series FPGAs SelectIO Resources* for the IO, and the
three derivations in the next section.

**Whether `TYPE.A.B` is a pip or a bel feature has to be inferred.** The
rule: it is a pip unless `A` also heads a *longer* feature of the same
tile type, because that is what a site prefix does.
`CLBLL_L.SLICEL_X0.AFF.ZINI` makes `SLICEL_X0` a site, so
`CLBLL_L.SLICEL_X0.CLKINV` is a bel feature and not a pip; `INT_L` has no
feature longer than two components at all, so all 3636 of its features
are pips, which is right. The known place it is wrong is
`LIOB33.DIFF.*`, three features of eighty-three, which describe the
differential pair; the bogus pips they produce name wires that exist
nowhere else and the graph drops them as dangling. `DIFF` carries no
`_X<n>` or `_Y<n>` suffix, so at least it never claims to be a site —
which it did in phase one, where it displaced every `LIOB33` pin of the
package map by one.

## Three things the database implies but does not state

Each of these had to be worked out, each is now a test that re-derives it
from the database so it cannot drift, and each is a place this could
still be wrong.

### Which half is `_Y0`

`segbits_liob33.db` names the two halves of an IO tile `IOB_Y0` and
`IOB_Y1`, and `tilegrid.json` names the tile's two sites `IOB_X0Y11` and
`IOB_X0Y12`. Nothing says which is which, and the obvious guess —
ascending, `_Y0` is the lower — is **wrong**.

`LIOB33_X0Y111` settles it. `design.txt` says A18 is `IOB_X0Y111` and an
output, and B18 is `IOB_X0Y112` and an input; the bitstream sets
`IOB_Y0.…IN_ONLY` and `IOB_Y1.…DRIVE.I12_I16`. So `_Y0` is
`IOB_X0Y112`, the **higher** site `Y`. Nine more witnesses across the
four harness designs agree and none disagrees: prjxray numbers a bel by
its position down the tile as the grid draws it, and the grid's rows run
opposite to a site's `Y`. The same holds for `ILOGIC_Y*` and `OLOGIC_Y*`.

Getting this backwards would put the LED's configuration on a switch's
pad, which is exactly the sort of error that survives every structural
check and lights nothing.

### Which slice owns which wires

`ppips_clbll_l.db` names its two slices' wires `CLBLL_L_*` and
`CLBLL_LL_*`, and `segbits_clbll_l.db` names its two slices `SLICEL_X0`
and `SLICEL_X1`. Two `SLICEL`s, so the site type cannot tell them apart.

A `CLBLM` can: it holds one `SLICEM` and one `SLICEL`, its prefixes are
`SLICEM_X0` and `SLICEL_X1`, and its wires are `CLBLM_M_*` and
`CLBLM_L_*` — the letters match the types, and the `SLICEM` is the
lower-`X` site. The bridge is the interconnect, which is the same silicon
in both tile types: `CLBLM_M_A1` and `CLBLL_LL_A1` are both fed from
index 7 and both drive `LOGIC_OUTS12`, while `CLBLM_L_A1` and
`CLBLL_L_A1` are both index 6 and `LOGIC_OUTS8`. So `CLBLL_LL` is the
`X`-index-0 slice, which is not what the names suggest.

### What a path through the IO logic costs

A pad does not reach the interconnect directly. It goes through the
`ILOGICE3` or `OLOGICE3` beside it, and those are sites, not wires.
`ppips_lioi3.db` records the output hop (`LIOI_OLOGIC0_OQ` from
`IOI_OLOGIC0_D1`) as unconditional and does not record the input hop at
all — but neither is free: using either turns bits on inside the site.

So both hops are declared in `sites.rs` as pips that carry those bits,
and the `ppips` copy of the output hop is suppressed, or the router would
find a free version of the same hop and turn nothing on. The bits are
the ones Vivado's own bitstream sets and no others:
`ILOGIC_Y<n>.ZINV_D` for the input, and `OLOGIC_Y<n>.OMUX.D1`,
`OLOGIC_Y<n>.OQUSED`, `OLOGIC_Y<n>.OSERDES.DATA_RATE_TQ.BUF` for the
output.

### And one thing that is a policy, not a derivation

The IO standard is a **whole-load setting**, not a per-pin one, and only
`LVCMOS33` is transcribed. A design whose constraints ask for two
different standards is refused; one that asks for a standard this flow
has no bits for is refused by name. An IO standard is a voltage, and
quietly substituting a different one is how a board gets damaged.

## The milestone design

`examples/basys3/sw_led.v`: two slide switches through one LUT to LED 0.
Combinational, three IO buffers, one LUT, no clock, no global buffer.
Verified — when it can be verified — by a human flipping a switch.
`examples/basys3/blink.v` is the clocked one; it has its own section
below.

A plain `assign led = sw` would not serve. Synthesis turns that into a
wire, no LUT survives, and nothing carries a truth table into the
bitstream; the exclusive-or needs a lookup table and is still a
one-glance test.

What the flow does with it:

```
loaded: 325 tile(s) of 20 type(s), 101153 wire(s), 527822 pip(s), 793 bel(s)
site pins: 60 resolved, 0 not; 12 fixed path(s) through a site, 0 not; 4 io buffer(s)
wrote sw_led.bit, 2192115 byte(s), 5420 frame(s), 85 configuration bit(s) set
4 instance(s) placed on real sites, 3 held at a package pin, 3 pin(s) the fabric gives no wire
3 of 3 signal(s) routed
```

The three pins the fabric gives no wire are the three *pads*. That is
deliberate and it is not a gap: an IO buffer's package side is a ball,
not a wire a router can reach, and a design's port net ends there. The
`io` bel therefore declares `din` and `dout` and no `pad`.

Decoded back into the database's own vocabulary, the 85 bits are:

```
CLBLL_L_X2Y11.SLICEL_X1.BLUT.INIT[..]            x32   the truth table
INT_L_X0Y11.NL1BEG_N3.LOGIC_OUTS_L18                   sw0 into the fabric
INT_L_X0Y11.EE2BEG3.NL1BEG_N3
INT_L_X0Y12.SE2BEG0.LOGIC_OUTS_L18                     sw1 into the fabric
INT_R_X1Y11.ER1BEG1.SE2END0
INT_L_X2Y11.IMUX_L14.EE2END3                           sw0 to the LUT's B1
INT_L_X2Y11.IMUX_L19.ER1END1                           sw1 to the LUT's B2
INT_L_X2Y11.SW6BEG1.LOGIC_OUTS_L9                      the LUT's B out
INT_L_X0Y7.SS2BEG1.SW6END1
INT_L_X0Y5.SS2BEG1.SS2END1
INT_L_X0Y3.IMUX_L34.SS2END1                            into the output logic
LIOB33_X0Y11.IOB_Y1.{IN_ONLY, IN, PULLTYPE.NONE}       sw0's pad  (V17)
LIOB33_X0Y11.IOB_Y0.{IN_ONLY, IN, PULLTYPE.NONE}       sw1's pad  (V16)
LIOI3_X0Y11.ILOGIC_Y1.ZINV_D                           sw0's input logic
LIOI3_X0Y11.ILOGIC_Y0.ZINV_D                           sw1's input logic
LIOI3_X0Y3.OLOGIC_Y1.{OMUX.D1, OQUSED, DATA_RATE_TQ.BUF}   the LED's output logic
LIOB33_X0Y3.IOB_Y1.{SLEW.SLOW, DRIVE.I12_I16, PULLTYPE.NONE}  the LED's pad (U16)
```

The `INIT` bits are the ones where the two lowest inputs differ, which is
`a ^ b` — and the router independently chose `IMUX_L14` and `IMUX_L19`,
which `ppips_clbll_l.db` says are the `B1` and `B2` pins of the very
slice the placer put the LUT on. Nothing coordinates those two facts;
they agree because both came from the database.

## The oracle: what Vivado put in the same tiles

The harness bitstream drives the Basys 3's sixteen switches to its
sixteen LEDs, and three of those pins are the milestone's three pins.
Decoding it with the same `XrayDatabase::decode` and looking at the same
tiles:

| Tile | Vivado | Reticle |
|---|---|---|
| `LIOB33_X0Y11` (V17, V16) | `IOB_Y0` and `IOB_Y1`: `IN_ONLY`, `IN`, `PULLTYPE.NONE` | **identical** |
| `LIOI3_X0Y11` | `ILOGIC_Y0.ZINV_D`, `ILOGIC_Y1.ZINV_D` | **identical** |
| `LIOB33_X0Y3` (U16, U15) | `IOB_Y0` *and* `IOB_Y1`: `SLEW.SLOW`, `DRIVE.I12_I16`, `PULLTYPE.NONE` | **identical on `IOB_Y1`**; Reticle does not drive U15, so it leaves `IOB_Y0` alone |
| `LIOI3_X0Y3` | `OLOGIC_Y0` *and* `OLOGIC_Y1`: `OMUX.D1`, `OQUSED`, `DATA_RATE_TQ.BUF` | **identical on `OLOGIC_Y1`**, same reason |
| `INT_L_X0Y11` | `NR1BEG0.LOGIC_OUTS_L18`, `LV_L0.NR1END0` | `NL1BEG_N3.LOGIC_OUTS_L18`, `EE2BEG3.NL1BEG_N3` — *same source wire*, different line out of it |
| `INT_L_X0Y3` | `IMUX_L34.WW2END0`, `SR1BEG1.SS6END0` | `IMUX_L34.SS2END1` — *same destination wire*, different line into it |

The two differences are the router's, not an error: a pad leaves the
input logic on `LOGIC_OUTS_L18` and enters the output logic on
`IMUX_L34` in both, and which of the interconnect's long lines carries
the signal between them is a free choice. That is asserted rather than
described: `the_io_path_is_the_one_vivado_built` checks the site
configuration for equality and the interconnect for *the ends* being
equal, and prints both routes so a reader can see the difference.

This is the best evidence available on a machine with no cable. It says
the site configuration is, feature for feature, what a working bitstream
has. It does **not** say the routing is electrically sound, that the
frames are written in an order the configuration engine accepts for a
full reconfiguration, or that anything else in the 2 192 115 bytes is
right.

## The clocked design, and how its clock crosses the die

`examples/basys3/blink.v`: the board's 100 MHz oscillator on pin W5,
through a `BUFG`, into a 26-bit counter, with bit 25 on LED 0 — 1.49 Hz,
evenly on and off. It maps to one `BUFG`, twenty-six `FDRE` and
sixty-four `LUT6` three deep, places on real sites, and **routes
completely**: 92 of 92 signals, 2731 pips, congestion resolved in five
iterations. The only pins the fabric gives no wire are the two package
pads, which are balls rather than wires.

The clock's path, and the resources it needs, each of which had to be
described before any of it moved:

| Stage | What carries it |
|---|---|
| the pad | `RIOB33_X43Y25`, `IOB_Y0`: the ordinary LVCMOS33 input recipe |
| the input logic | `RIOI3_X43Y25`, `ILOGIC_Y0.ZINV_D`, then the `I2GCLK` hop `ppips_rioi3.db` records as free |
| into the backbone | across the `HCLK` row on `CCIO0` to `HCLK_CMT_L_X106Y26`, where `HCLK_CMT_CK_IN0` costs `HCLK_CMT_CCIO0_ACTIVE` and `_USED` |
| to the buffer | westwards to `CLK_HROW_BOT_R_X60Y26`, out on `CK_BUFG_CASCO0` (which costs `CLK_HROW_CK_IN_R0_ACTIVE`), up the column to `CLK_BUFG_BOT_R_X60Y48` |
| the buffer | `BUFGCTRL_X0Y0`: `IN_USE`, `IS_IGNORE1_INVERTED`, `ZINV_CE0`, `ZINV_S0` |
| down the column | `GCLK0` through `CLK_BUFG_REBUF_X60Y38`, which cuts the track: the `BOT` ← `TOP` pip plus both `GCLK0_ENABLE_*` bits |
| the clock row | back at `CLK_HROW_BOT_R_X60Y26`: `CK_MUX_OUT_L0` from `R_CK_GCLK0`, `CLK_HROW_R_CK_GCLK0_ACTIVE`, and `BUFHCE_X0Y0`'s `IN_USE` and `ZINV_CE`, out on `CK_BUFHCLK_L0` |
| the leaves | westwards along the row's `L` half on `HCLK_CK_BUFHCLK0`, tapped in four `HCLK_R` tiles (`ENABLE_BUFFER.HCLK_CK_BUFHCLK0` each) and down `GCLK_B0` into the interconnect |
| the slices | `CLK_L0` / `CLK_L1` / `CLK0` from `GCLK_*_B0`, then `CLBLM_CLK0` to the slice's `CLK` pin, which `ppips` gives free |

Two of those stages are not pips and are what a clock needs beyond
routing. The first is the **wire that costs bits to be touched** —
`_ACTIVE`, `_USED`, `ENABLE_BUFFER.*` — which `xray::sites`'s
`wire_enable_features` finds by rule rather than by table. The second is
the **rebuffer enable**, which is the one bit in this whole module that
belongs to no pip at all: a global clock track is cut at every
`CLK_BUFG_REBUF` of its column, a route crosses exactly one of them, and
the others are tiles the route never enters. `XrayFabric::enable_global_clocks`
switches those on over the whole column once the routing is known, which
is nextpnr-xilinx's rule; which of the two enables belongs to which side
of a cut is measured from all four Vivado harness designs and asserted by
`the_rebuffer_enables_pair_with_the_ends_vivado_marks`.

### The oracle again, on the clock

The Basys 3 harness drives *its* clock from the same pin, W5, so the
comparison is exact where the two designs want the same thing.

| Tile | Vivado | Reticle |
|---|---|---|
| `RIOB33_X43Y25` (W5) | `IOB_Y0`: `IN_ONLY`, `IN`, `PULLTYPE.NONE` | **identical** |
| `RIOI3_X43Y25` | `ILOGIC_Y0.ZINV_D` | **identical** |
| `HCLK_CMT_L_X106Y26` | `HCLK_CMT_CCIO0_ACTIVE`, `_USED`, `CK_IN0.CCIO0` | **identical** |
| `CLK_BUFG_BOT_R_X60Y48` | four `BUFGCTRL_X0Y0` bits, `BUFGCTRL0_I0.CK_MUXED0`, `CK_GCLK0.BUFGCTRL0_O` | **identical, all six** |
| `CLK_HROW_BOT_R_X60Y26` | the two features of the row the pad arrives at | those two **and** the four Vivado puts in `CLK_HROW_TOP_R_X60Y130` |
| `CLK_HROW_TOP_R_X60Y130` | `BUFHCE_X0Y0.IN_USE`, `ZINV_CE`, `CK_MUX_OUT_L0.R_CK_GCLK0`, `R_CK_GCLK0_ACTIVE` | nothing |
| `CLK_BUFG_REBUF_X60Y38` | `GCLK0_ENABLE_BELOW` | that, plus `GCLK0_ENABLE_ABOVE` and the `BOT` ← `TOP` pip |
| `CLK_BUFG_REBUF_X60Y13` | nothing | both enables |
| `CLK_BUFG_REBUF_X60Y65` | the `TOP` ← `BOT` pip and both enables | nothing |
| `HCLK_R_*`, `INT_*` | `HCLK_LEAF_CLK_B_BOT5`, `CLK_L1.GCLK_L_B5` | the same shapes on other tiles and `GCLK0` |

Six rows of that table differ, and they differ for four reasons. None of
the four is an error:

- **The clock region.** The harness's loads are in the die's top clock
  region and blink's are in the bottom one. Each design drives the row
  its flip-flops hang off, so the same four features land in
  `CLK_HROW_BOT_R_X60Y26` here and in `CLK_HROW_TOP_R_X60Y130` there.
  Both use `BUFHCE_X0Y0` and `CK_MUX_OUT_L0`, which is the one place the
  buffer-index mapping in `sites.rs` is corroborated at all.
- **The direction down the column.** For the same reason the clock leaves
  the buffer downwards here and upwards there, so blink takes the `BOT` ←
  `TOP` rebuffer pip that no harness design takes, and the enable pattern
  is the mirror image of theirs.
- **`CLK_BUFG_REBUF_X60Y65`, which Reticle leaves alone.** Vivado marks
  *both* ends of every live segment of the track, and the segment holding
  the buffer runs from `Y38` up to `Y65`. `Y65` is outside the rectangle
  of tiles this load covers — the region is the pins grown to reach the
  nearest `BUFGCTRL`, not the whole column — so there is no tile there to
  set a bit in. **This is the one difference on the clock path that is
  not clearly a free choice.** The segment is driven from the buffer in
  the middle and consumed at `Y38`, whose own enable is set, so nothing
  in the model says the far end matters; that it does not is an
  assumption, and the way to remove it is to make the loaded region
  include the whole rebuffer column rather than a rectangle.
- **The leaf tiles.** A leaf network taps the horizontal clock beside the
  interconnect column it feeds, so which `HCLK_R` and which `GCLK_*_B<n>`
  differ by construction. The test asserts the shape — every `HCLK_*`
  feature is either the buffer enable or a `HCLK_LEAF_CLK_B_*` pip — and
  not the tile.

## A tristate, and a pad that reads itself

**Nothing in this section has been on a part.** It builds, routes, and
decodes; whether the pad lets go when it is told to is what
`examples/basys3/pmod_bidir.v` exists to find out.

### First, the oracle that is not there

The question this project asks first — what does Vivado write, in full,
for this cell? — has no answer here. All four designs in
`artix7/harness/` were searched, every `IOB`, `ILOGIC` and `OLOGIC`
feature of their `design.json` tallied: there is **no** `ZINV_T1`, no
`TQ` path other than the plain-output `DATA_RATE_TQ.BUF`, and no pad
with both a drive strength and an input receiver. Every pad Vivado built
for those boards is an input or an output. So everything below about the
tristate is a reading of the database and of two other programs, and it
says which.

What the harnesses *do* have is `PULLTYPE.PULLUP` and
`PULLTYPE.PULLDOWN` on pads — the names, though not the effect.

### Where the tristate goes

| Stage | Wire or feature | Source |
|---|---|---|
| fabric to the IO logic | `INT_L`'s `IMUX_L15` → `IOI_IMUX15_<1-n>` → `IOI_OLOGIC<n>_T1` | `ppips_lioi3.db`, `always`; the router found it unaided |
| through the `OLOGIC` | `LIOI_OLOGIC<n>_TQ` ← `IOI_OLOGIC<n>_T1` | `ppips_lioi3.db` calls it `always`; it is not — see below |
| to the pad | `LIOI_T<n>` ← `LIOI_OLOGIC<n>_TQ`, joined to `IOB_T<n>` | `ppips_lioi3.db` and `tileconn.json` |
| the pad's pin | `IOB_T<n>`, the IO bel's `oe` | the bel's third pin, beside `din` and `dout` |

The hop through the `OLOGIC` costs two features, declared in
`sites.rs`'s `tristate_through` the way the data hop beside it is:

- **`OLOGIC_Y<n>.OSERDES.DATA_RATE_TQ.BUF`** — what `TQ` is: `T1` passed
  straight through, not the `SDR` or `DDR` register. **Checked**, in the
  sense that matters here: the field is one-hot in `segbits_lioi3.db`, so
  a blank tile is none of the three, and every plain output Vivado built
  sets `BUF`. The data hop already set it; the tristate hop sets it too,
  so a tristate whose data is not routed through the same site still has
  it.
- **`OLOGIC_Y<n>.ZINV_T1`** — the polarity. **Quoted.** prjxray's
  `fuzzers/036-iob-ologic/generate.py` tags it as
  `ZINV_T1 = 1 ^ IS_T1_INVERTED`, so set means `T1` is not inverted and a
  blank tile inverts it; nextpnr-xilinx's `xilinx/fasm.cc` puts this one
  feature, and only it, on this very pseudo-pip. Both were read on
  2026-10-08. With it set, a one from the fabric is a one at the pad's
  `T`, and UG471's `OBUFT` releases the pad on a one.

**The one board result that touches this cannot tell the readings
apart.** `sw_led`'s LED lit with `ZINV_T1` clear and `T1` unrouted. Under
prjxray's reading that is "inverted, and an unrouted `T1` reads one";
under the opposite reading it is "not inverted, and it reads zero". Both
drive the pad. It would be easy to cite that LED as evidence for the
polarity, and it is not.

### What the pad costs

`segbits_liob33.db` has eighty-three features and **none mentions a
tristate**. So an `OBUFT` costs at the pad exactly what an `OBUF` does,
and the tristate is entirely the `OLOGIC`'s business — the same finding
the ECP5 made in its own vocabulary ("the tristate is a routed wire
rather than a setting", `docs/fpga-trellis.md`).

An `IOBUF` is the output recipe plus `LVCMOS25_LVCMOS33_LVTTL.IN`, the
input receiver — and **not** `IN_ONLY`, which an `IBUF` also takes. The
bits say why the two differ: `.IN` is `38_86 39_85 39_87` and touches no
drive bit, while `IN_ONLY` is a value of the eighteen-bit drive field
every `DRIVE.*` is written over, so an `IOBUF` given both would be
configured as two drive strengths at once. That split is **quoted** from
nextpnr-xilinx's `write_io_config` (`.IN` for a pad with an input buffer,
`IN_ONLY` only `if (!is_output)`) and consistent with the bit patterns;
no Vivado design here has a bidirectional pad to measure it.

### Pull-ups

`PULLTYPE` is a three-bit field. For `IOB_Y0`: `NONE` is
`!38_92 38_94 !39_93`, `PULLUP` is `!38_92 38_94 39_93`, `KEEPER` is
`38_92 38_94 !39_93`, and **`PULLDOWN` is all three clear** — which is
also what a pad nothing configures gets. Every recipe here sets `NONE`;
`set_io -pullup yes` adds `PULLUP`'s ones on top, which turns the field
into `PULLUP` and makes `NONE` stop decoding.

A pull-up is a constraint on this family, not an `IBUF` parameter (Vivado
takes it from an XDC), so it cannot ride a bel's configuration entry
without handing Vivado an instance parameter its library lacks.
`XrayFabric::apply_pullups` reads the cell's `pullup` attribute after
placement instead, and `reticle fpga --bitstream` runs it. `-pullup no`
leaves `NONE`, as before; there is no way to ask for a pull-down or a
keeper.

### What was checked instead

`tests/fpga_xray_tristate.rs`, against the real database, skipping
without it:

| Design | Set bits | Unexplained | Arcs decoded = arcs routed | Hops through a site | Signals |
|---|---|---|---|---|---|
| `pmod_bidir` (`IOBUF`, pull-up) | 155 | 0 | 26 = 26 | 7 | 4 of 4 |
| a tri-stated `output` (`OBUFT`) | 96 | 0 | 14 = 14 | 4 | 3 of 3 |

"Arcs decoded = arcs routed" is a set equality: every interconnect pip
the frames switch on is one the router took, and every pip it took with
bits is in the frames. On top of that the test asserts, half by half,
K17's pad (`.IN`, `DRIVE.I12_I16`, `SLEW.SLOW`, `PULLTYPE.PULLUP`, no
`IN_ONLY`, no `NONE`, the other half untouched) and its IO logic
(`ZINV_D`, `OMUX.D1`, `OQUSED`, `DATA_RATE_TQ.BUF`, `ZINV_T1`, nothing on
`Y0`), that the two LEDs' plain outputs got **no** `ZINV_T1`, and that
the `OBUFT`'s pad has no input setting at all.

`the_enable_reaches_the_pad_the_right_way_round` checks the whole
polarity chain *given* prjxray's reading: it walks the synthesised netlist
from the `IOBUF`'s `T` back to the lookup table that drives it and on to
SW0's buffer, evaluates the table, applies `ZINV_T1`, and asserts that
SW0 up gives the pad `T = 0` (drive) and SW0 down `T = 1` (release). It
would catch the mapper's inversion lost or doubled, the enable wired to
the wrong switch, or the bit missing. If prjxray's reading were the wrong
way round, it would pass anyway — which is the honest limit of a desk.

### The demo, and what it should do

`examples/basys3/pmod_bidir.v` with `pmod_bidir.rcf`: SW0 (V17) enables
the driver, SW1 (V16) is the value, Pmod JC pin 1 (K17) is the
bidirectional pad with its pull-up, LD0 (U16) shows what K17 **reads**
through its own input buffer, LD1 (E19) shows SW0.

```sh
reticle fpga --device xc7a35t-cpg236 \
    --constraints examples/basys3/pmod_bidir.rcf \
    --bitstream pmod_bidir.bit \
    examples/basys3/pmod_bidir.v
```

| SW0 | SW1 | K17 | LD0 | LD1 |
|---|---|---|---|---|
| down | down | released, pulled up | on | off |
| down | up | released, pulled up | on | off |
| up | down | driven low | **off** | on |
| up | up | driven high | on | on |

LD0 is dark in exactly one position, and from there flipping SW0 down
must relight it at once: that is the pad letting go and the pull-up
taking it. With the enable inverted the dark position moves to SW0 down,
SW1 down, and LD0 follows SW1 only while SW0 is down. Nothing should be
plugged into JC: K17 goes to that header and nowhere else Digilent's
`Basys-3-Master.xdc` names, so the FPGA is the only driver the net has
whichever way the enable turns out to be.

### What is not verified

- That `ZINV_T1` set means "not inverted" on silicon. Quoted twice,
  measured never.
- That `LVCMOS25_LVCMOS33_LVTTL.IN` without `IN_ONLY` gives a working
  receiver beside a working driver. Quoted.
- That the pull-up pulls. Its name and bits are the database's; its
  effect has not been seen.
- `OBUFT` instantiated by name in Verilog. Reticle has no library of
  vendor primitives to elaborate it against, so only the mapper's own
  `OBUFT` — from a tri-stated `output` port — is built.
- A tristate whose data is a constant (`assign pin = en ? 1'b0 : 1'bz`,
  an open drain). The tristate hop sets `DATA_RATE_TQ.BUF` for itself for
  that case, but nothing has built one.
- A `_SING` IO tile at the **bottom** end of a bank used as an *input*,
  or one at the **top** end used as an *output*. The other two
  combinations are measured against Vivado's own bitstreams and all four
  rest on the same shift, but no harness design drives either of these
  and nothing has.

## A distributed RAM on a `SLICEM`

`examples/basys3/lutram.v` — sixteen words of four bits, written from
the switches on a button and read at another address onto four LEDs —
maps to four `RAM64X1D`, places, **routes completely** (112 of 112
signals) and comes out as a `.bit` for the XC7A35T-CPG236. Decoded back
through the database:

```
decoded: 4033 bit(s) over 178 tile(s) into 2185 feature(s); 0 bit(s) unexplained, 0 tile(s) with no segbits file
112 of 112 signal(s) routed; 1008 interconnect pip(s) routed, 1008 decoded; 53 fixed path(s) through a site
```

and the two sets of 1008 are the same set.

(4033 is with an `INIT` of three set bits given to one RAM by the test, in
both of its copies; the design as `reticle fpga` builds it sets 4027.)
Every bit is named, and the interconnect the decoding finds is exactly the
interconnect the router chose. That is
`tests/fpga_xray_lutram.rs::a_distributed_ram_reaches_a_bit_file_and_every_bit_decodes`.

**Nobody has loaded it into a part.** Everything below is either read
from the database or quoted, and each line says which.

### What a `RAM64X1D` is on this fabric

One `lutram` bel per `CLBLM`, `SLICEM_X0_RAM64X1D`, declared by
`src/fpga/xray/lutram.rs`:

| | | Source |
|---|---|---|
| write port, and `SPO` | the `D` lookup table: `A0..A5` on `CLBLM_M_D1..D6` | quoted |
| read port | the `C` lookup table: `DPRA0..5` on `CLBLM_M_C1..C6`, `DPO` on `CLBLM_M_C` | quoted |
| contents | the cell's `INIT`, unpermuted, into **both** `DLUT.INIT` and `CLUT.INIT` | quoted |
| mode | `SLICEM_X0.DLUT.RAM` and `SLICEM_X0.CLUT.RAM`, one bit each | read |
| left clear | `SMALL` (32 words), `SRL`, `WA7USED`, `WA8USED`, `WEMUX.CE` (enable from `CE` rather than `WE`), `CLKINV` | read |
| the read port's data | `CLUT.DI1MUX`: its no-bit value is `DI_DMC31`, the `D` table's `DI` | read |
| `din`, `we`, `wclk` | `CLBLM_M_DI` off `FAN3`, `CLBLM_M_WE` off `FAN4`, `CLBLM_M_CLK` off `CLK1`, all `always` in `ppips` | read |
| blocks | `SLICEM_X0_DLUT` and `SLICEM_X0_CLUT`, nothing else | follows from the two above |

So a RAM costs two lookup tables. The slice's `A` and `B` lookup tables,
its four flip-flops and the whole `SLICEL` of the same tile stay usable,
and in the demo eleven lookup tables and flip-flops share a tile with a
RAM in the build the test makes. `BelDecl::blocks`, which the ECP5's distributed RAM introduced, is the
mechanism; nothing in the placer was changed. Only a `CLBLM` offers the
bel: a `CLBLL` has no `RAM` feature and no `WE` wire.

Empty contents cost nothing: these `INIT` bits are not `!`-marked, so an
empty RAM is the two mode bits. The mapper still declines a memory with
initial contents, so the `INIT` path is exercised only by the test, which
sets one by hand on the netlist.

### The address permutation, and why it is quoted

Which lookup table is which port, and in what order the address bits
reach its inputs, is not in `prjxray-db`, and the one Vivado bitstream it
ships for this board has no RAM in it. Getting it wrong is silent: the
design places, routes, decodes bit for bit and reads every word from the
wrong address. The reading taken here comes from two sources that agree:

- **nextpnr-xilinx** (`gatecat/nextpnr-xilinx`, branch `xilinx-upstream`),
  `xilinx/pack_dram.cc`: for a `RAM64X1D` the "write address input" cell
  goes at the top `z` of the slice, `SPO` is folded into it, and `DPO`
  goes at the next `z` down. `xilinx/fasm.cc`'s `write_luts_config`
  names those `"ABCD"[3]` and `[2]`, the `D` and the `C`. Its `dram_rules`
  map `RADR<i>` to `A<i+1>` and `WADR<i>` to `WA<i+1>`, copy the cell's
  `INIT` to every lookup table of the RAM, and `get_lut_init` writes it
  with the identity physical-to-logical map.
- **Project X-Ray**, `fuzzers/018-clb-ram/generate.py`: a single
  `RAM64X1D` is tagged as occupying `(a, b, c, d) = (0, 0, 1, 1)`, "D is
  always occupied first (due to WA/A sharing on D)", and a sample is only
  kept when Vivado's own placement matched that tuple. That is the
  nearest thing to a Vivado observation available, and it covers the
  lookup tables, not the order of the inputs.

That the write address `WA1..6` *is* the `D` lookup table's `D1..6` is
also how Xilinx UG474 describes a `SLICEM`. **No measurement backs any of
it.** `lutram.rs`'s own test pins the table so it cannot drift, and it
would pass with the table wrong.

### What a person should do with the demo

```sh
reticle fpga --device xc7a35t-cpg236 \
    --constraints examples/basys3/lutram.rcf \
    --bitstream lutram.bit \
    examples/basys3/lutram.v
```

`SW0..SW3` is the write address, `SW4..SW7` the word, `SW8..SW11` the read
address; `BTNC` writes while held; `LD0..LD3` show the word at the read
address, `LD5` lights while writing, `LD7` blinks to say the design is
alive. With every switch down the four LEDs are dark. Then, for each `k`
of 0 to 3, write the word `1<<k` at the address `1<<k` (`SW<k>` and
`SW<4+k>` up, press `BTNC`, switches down). Reading `SW8` alone should
light `LD0` alone, `SW9` alone `LD1` alone, and so on; every other read
address should stay dark. A disagreement between the two ports about
which input is which address bit puts some other word, or none, on one of
those four. The header of `lutram.v` has the full procedure.

`LD6` is not used because its pin, U14, is the only buffer of a
`LIOB33_SING` tile, which this flow had no IO table for when this demo
was written; the write strobe went on LD5 instead and has stayed there.
The tile type is reachable now — see "LD6 was a missing feature, not a
dead ball" — and a ball that still cannot be driven is refused with a
diagnostic naming the ball, its site, its tile and its tile type rather
than a bare "maps to no usable site".

### What is not established

- Anything on silicon: the permutation, `DI_DMC31` meaning `DI` for a RAM,
  and that `WEMUX` clear selects the `WE` pin.
- A slice's clock is one for all its storage elements, and its `CLKINV` is
  one bit. A RAM and a flip-flop on different clocks in one `SLICEM` fail
  to route rather than mis-configure, because both pins are the one wire
  `CLBLM_M_CLK`; but a falling-edge flip-flop beside a RAM sets `CLKINV`
  and would invert the RAM's write clock too, and nothing refuses that —
  the caveat `sites.rs`'s `slice_pass_throughs` already states for two
  kinds of flip-flop in one slice.
- One RAM per `SLICEM`. Two `RAM64X1D` that share a write address could
  share one slice (the `B`/`A` pair, written through the `D` inputs), and
  `RAM32X1D`, `RAM128X1D` and the `RAM64M` family are not offered.

## The PLL

`examples/basys3/pll_blink.v` asks for a 25 MHz clock by leaving a net
undriven with `(* clock_mhz = 25 *)` on it, and for that clock's lock
indicator with `(* clock_locked = "clk_pll" *)` on another (`docs/fpga.md`,
"Generated clocks"). Mapping makes one `PLLE2_BASE` with `DIVCLK_DIVIDE =
1`, `CLKFBOUT_MULT = 8`, `CLKOUT0_DIVIDE = 32`. It builds to a `.bit` for
the XC7A35T, **routed completely** (177 of 177 signals), and decoded back
through the database:

```
decoded: 6297 bit(s) over 151 tile(s) into 3563 feature(s); 0 bit(s) unexplained
1426 arc(s) decoded, the same 1426 the router chose
```

**Nothing of it has run on a part.** What the user should see when it
does, and why that test is sharp, is in the file's header: two LEDs
blinking in step, one off the raw oscillator and one off the PLL, and
`LOCKED` lit on a third. `tests/fpga_xray_pll.rs` holds both checks below.

### Where it sits, and how the clock gets in and out

A clock management tile is four tiles of the grid. Only its `UPPER_T`
quarter has a site of interest, `PLLE2_ADV`; `LOWER_B` holds the
`MMCME2_ADV`, and the two in between have no bits at all, only wires
(`ppips` and `tileconn`). In both orientations prjxray calls the PLL's own
wires `CMT_TOP_R_UPPER_T_PLLE2_*`, even in an `L` tile. The PLL's feature
prefix, `PLLE2_ADV`, carries no `_X`/`_Y` suffix, so the prefix machinery
made it a pinless bel of kind `other`; `src/fpga/xray/cmt.rs` gives it the
kind `pll` and seven pins and keeps every feature the reader had filed
under it.

| Pin | Wire, and what reaches it |
|---|---|
| `CLKIN1` | the `PLLE2_CLKIN1` mux: from the tile's `CLKIN1` (the clock row's `HCLK_CMT_MUX_PLLE2_CLKIN1`, which takes a clock-capable pad's `CCIO` or a global clock's `BUFHCLK`), from a frequency backbone, or from general routing |
| `CLKFBIN` | the `PLLE2_CLKFBIN` mux, which can take `CLKFBOUT2IN` |
| `CLKOUT0`, `CLKFBOUT` | `CLKPLL0`, `CLKPLL6` and on into the clock row's `HCLK_CMT_MUX_CLK_PLL*`, the way to a `BUFG` |
| `LOCKED` | `CMT_TOP_LOGIC_OUTS_L_B21_11`, through the `CMT_FIFO` tile into an interconnect tile: an ordinary fabric signal |
| `RST`, `PWRDWN` | an `IMUX` of that interconnect tile |

In the demo the placer chose the left-hand PLL, `PLLE2_ADV_X0Y0`, because
the LEDs are on the left. So the reference arrives from the raw clock's
`BUFG`, along the bottom clock row into `HCLK_CMT_X8Y26` and its
`CK_BUFHCLK0`, and not straight from the pad's `CCIO`. Both are legal, and
both end on the PLL's dedicated `CLKIN1` input rather than general routing.

**The feedback closes inside the tile.** `ppips` records `CLKFBOUT` reaching
`CMT_TOP_{L,R}_CLKFBOUT2IN` unconditionally, and the `CLKFBIN` mux selects
it with a pip of its own, so the router closes the `CLKFBOUT` to `CLKFBIN`
net that mapping makes in one arc and the test asserts exactly that. That
is `COMPENSATION = INTERNAL`, whose only other bit is
`COMPENSATION.Z_ZHOLD_OR_CLKIN_BUF`, the one f4pga and nextpnr-xilinx both
write. A loop through a `BUFG` would be `ZHOLD`, which needs bits this flow
does not set.

### The registers: computed, quoted, and how each was checked

A PLL's parameters are not stored as parameters. The frames hold its DRP
register file, the words a design could rewrite at run time, and
prjxray-db names every field after XAPP888's register map (its fuzzer's
`write_pll_reg.py` lays the register space out bit by bit). The generic
feature reader already turns `PLLE2_ADV.CLKOUT0_CLKOUT1_HIGH_TIME[4]` into a
parameter bit, so what was missing was only the values, which
`cmt::pll_registers` computes:

| Field | Source |
|---|---|
| high time, low time, `EDGE`, `NO_COUNT` of `DIVCLK`, `CLKFBOUT` and `CLKOUT0`..`5` | **computed**, from XAPP888's divider arithmetic for a 50 % duty cycle, no phase shift. An unused output keeps the library's division by one, with its output enable off |
| `LKTABLE[39:0]`, the lock detector | **quoted**: XAPP888's table as f4pga-arch-defs transcribes it (`pll_lktable_lookup`), extracted by script and not retyped |
| `TABLE[9:0]`, the loop filter | **quoted** the same way (`pll_table_lookup`). f4pga's `OPTIMIZED` table equals its `HIGH` one |
| `FILTREG1_RESERVED = 0x008`, `LOCKREG3_RESERVED = 1`, the rest and `POWER_REG` zero | **quoted**: f4pga and nextpnr-xilinx both write these, and neither says why |
| `IN_USE`, `Z_ZHOLD_OR_CLKIN_BUF` | **quoted**, from the same two |
| `ZINV_RST`, `ZINV_PWRDWN` | **derived**; see the next section |

An earlier draft of the lock table in this module was typed from memory,
and a script comparing it with f4pga's file found it wrong from the
eleventh entry on. The table is now the script's output, and the
comparison is the reason to trust it.

What checks these values, and how far:

- `the_pll_registers_land_where_the_database_says` writes two settings
  into the same tile, (1, 8, 32, `OPTIMIZED`) and (2, 20, 7, `LOW`,
  `STARTUP_WAIT`, routed reset), decodes the frames, and compares the
  whole tile with a feature list worked out by hand: 56 and 60 features,
  162 and 166 bits, nothing else set. It catches a field at the wrong
  bit, high and low time swapped or `EDGE` lost on an odd division, a
  table indexed off by one, and the wrong bandwidth's table.
- At a multiplier of 8 the two quoted tables give `0xB5BE8FA401` and
  `0x3B4`, **the constants nextpnr-xilinx writes for every PLL**. That
  is a second, independent source for one entry of each table, and no
  more than that.
- **Nothing here has been compared with a Vivado bitstream holding a
  PLL.** The four harness designs in `artix7/harness/` contain none, and
  Vivado was not available. A reserved field Vivado sets that both open
  tools leave clear would be invisible to every check above.

### The reset that reads as one

An unrouted interconnect input on this fabric is not left floating.
Every `IMUX` of an `INT_L` or `INT_R` has a `default` pseudo-pip from
`VCC_WIRE`, so a site pin nothing drives **reads one**. The ECP5 has the
same trap, and it cost a USB device eight rounds there. A PLL whose
`RST` and `PWRDWN` are "tied low" by being left unrouted is held in reset
and powered down, and never locks.

The bit that rescues it is prjxray's `ZINV_RST`, and the name points the
wrong way. prjxray's `032-cmt-pll` fuzzer tagged it `1 ^ IS_RST_INVERTED`
on PLLs whose `RST` was **unconnected**, so what it found is the bit
Vivado sets when it ties a reset low by inverting the default one.
nextpnr-xilinx writes `ZINV_RST = IS_RST_INVERTED` for a reset it routes,
with a comment that the name looks wrong. f4pga writes `ZINV_RST = 1` with
the pin tied to `VCC` for a constant zero, and 0 for a routed reset. All
three agree once read that way: **the bit set means the inverter is on**.
So a reset tied low stays unrouted with the bit set, and a routed reset
gets the bit clear. `CLKINSEL` needs nothing: `PLLE2_BASE` holds it high,
which selects `CLKIN1`, and the default one already reads high.

That reading is consistent across three sources and has not been
measured. If it is wrong, the PLL never locks: `LOCKED` stays dark, and
since the PLL then outputs nothing, the LED counting its output stays
dark too.

The DRP's own inputs (`DEN`, `DWE`, `DADDR`, `DI`) are left unrouted as
well, and so read one. `DCLK` is left unrouted too, so it never changes,
and a DRP access needs a `DCLK` edge. That is the reasoning, and it has
not been tried.

### What is not done

- **`MMCME2_BASE`.** Its register map differs from the PLL's: fractional
  dividers, a `POWER_REG` f4pga computes, a filter table of its own. The
  mapper's solver can choose an MMCM for a frequency the PLL cannot hit
  exactly. Such a cell would land on a PLL site, which is the only kind
  `pll` here, and `configure_clock_managers` refuses it by name rather
  than configure an MMCM with a PLL's registers.
- **Phase shift, duty cycle other than one half, `CLKOUT1`..`5`.**
  `PLLE2_BASE` as `xc7.dev` maps it uses only `CLKOUT0`, and the arithmetic
  above is for 50 % and zero phase.
- **`COMPENSATION = ZHOLD`, or feedback through a `BUFG`.** Only the
  internal loop is written.

## Block RAM

**Status, 2026-10-08:** a `RAMB18E1` — inferred from a Verilog memory
with an `initial` block, or instantiated — builds through
`reticle fpga --bitstream` for the XC7A35T to a `.bit` in which every set
bit, contents included, decodes back through the database and the
decoded arcs are exactly the router's plus the ties'. **Nothing with a
block RAM has been loaded into a part.** `RAMB36E1` is not done: nothing
places one (the device file declares only the 18 kbit block, for the
reason it gives), and the seven `RAMB36.*` features are left alone.

The demo is `examples/basys3/bram_rom.v`: a 256-word ROM addressed by
SW0..SW7, each word its own address plus one, shown on LD7..LD14. Its
header lists what each kind of failure would look like on the LEDs.

```sh
reticle fpga examples/basys3/bram_rom.v --device xc7a35t-cpg236 \
    --constraints examples/basys3/bram_rom.rcf \
    --bitstream /tmp/bram_rom.bit --output-dir /tmp
```

```
note: 17 of 17 signal(s) with a reader routed
note: 1 block RAM(s) configured with 87 mode feature(s); 73 input(s) tied to zero
      over 82 pip(s), 5 left at the interconnect's VCC default, 0 idle input(s) left untied
note: decoded: 1735 bit(s) over 154 tile(s) into 1400 feature(s); 0 bit(s) unexplained,
      0 tile(s) with no segbits file
```

`tests/fpga_xray_bram.rs::the_rom_demo_decodes_bit_for_bit` adds that the
205 decoded arcs are exactly the 205 the router and the ties switched on
(17 more hops go through IO sites and cost site features, as they always
have), and that the 1024 decoded contents bits are exactly the set bits
of the cell's `INIT_00..INIT_3F`, which in turn are exactly the ROM in
the 16-bit layout.

### Why it did not route, which was not the reason item 4 gave

The loader decides a feature prefix is a *site* when it heads a feature
of three or more components (`SLICEL_X0.AFF.ZINI`). Every block RAM
feature has two: `RAMB18_Y0.IN_USE`, `RAMB18_Y0.INIT_00[000]`. So both
halves were read as wires, their 37 000 features as pips into nothing,
and a `BRAM_L` tile got **no bel**. The placer then said "the design needs
1 `bram` site(s) and the part has 0". `RAMB18_Y0` and `RAMB18_Y1` are now
named as sites (`bram::is_two_part_site`); naming rather than loosening
the rule, because a looser rule would misread interconnect wires.

### What else had to change, each found by building the design

| Problem | Found by | Fix |
|---|---|---|
| A block RAM tile has **two frame windows** — 28 configuration frames on `CLB_IO_CLK`, 128 contents frames on `BLOCK_RAM` — and both start at tile frame 0. The frame map put a bit in "whichever window claims it", so the first claimed every bit. | reading `tilegrid.json` for `BRAM_L` | the windows **stack**, `CLB_IO_CLK` first (sorted by the frame address block type, not by name, which put `BLOCK_RAM` first); contents rows are 28 and up. `xc7::FrameMap::locate` |
| `segbits_bram_l.block_ram.db` numbers its frames from the `BLOCK_RAM` window | the same | read with its rows moved by 28 and merged into the tile type's features |
| which half `RAMB18_Y0` is | the `IOB` rule (descending rank) would have named the *upper* half | parity, below |
| an undriven interconnect input reads **one** | `ppips_int_l.db`: every `IMUX_L*`, `BYP_ALT*` and `FAN_ALT*` is `default` from `VCC_WIRE` | every input the design leaves alone is tied after routing, below |
| the router carried four address bits up the **address cascade** through four unused block RAMs below the placed one (`ADDRARDADDRU<n> <- CASCINBOT`), which decoded and matched and is a path nobody takes for a lone block | the first decode of the demo | cascade inputs are left out of the graph; nothing here cascades |
| a data input of the idle port sat behind `BYP_ALT6`, which the router had used as a hop for another signal, so the pin *carries* that signal | the four-block test | an idle input (data, or anything on a port whose enable is tied low) is tied when it can be and otherwise left; an input that matters and is behind a routed wire is **refused** |

### Checked, and quoted

| Claim | Status | Source |
|---|---|---|
| which frame bit is `INIT_xx[k]` of each half | **quoted** — the database's statement, read and applied with no transformation | `segbits_bram_l.block_ram.db`, produced by prjxray fuzzer `026-bram-data`, whose generator gives Vivado random 256-bit `INIT_xx` values and tags bit `k` with `val & (1 << k)` (read in the fuzzer's source) |
| that `INIT_xx[k]` is bit `k` of the Verilog parameter `INIT_xx` | **quoted**, and the same reading as nextpnr-xilinx's `write_bram_init` (`init.str[k]` to `INIT_xx[k]`) | prjxray `026-bram-data/generate.py`; nextpnr-xilinx `fasm.cc` |
| which memory word lands in which `INIT_xx` bit | **quoted** from UG473 by `xc7.dev` (MEDIUM-HIGH there), and **checked** here only in the sense that the test recomputes the 16-bit layout independently and compares | `xc7.dev`, `init_params` |
| the decoded contents equal the cell's `INIT` | **checked**: 1024 of 1024 bits, both halves (`the_upper_half_holds_the_same_rom` moves the block to `RAMB18_Y1`), and four blocks of different contents in the 4-bit mode | `tests/fpga_xray_bram.rs` |
| `RAMB18_Y0` is the even-`Y` site, the lower, `FIFO18E1`-typed one | **quoted** from prjxray `segmaker.py` (`name_bram18`); **corroborated** by the database: all 75 block RAM tiles have the even site lower and typed `FIFO18E1`, and every `RAMB18_Y0` contents bit is in bits 0..175 of the window, `Y1`'s in 176..319 — and a higher word is a higher site everywhere else this was checked | `the_lower_half_is_ramb18_y0_by_both_readings` |
| a `BRAM_FIFO18_<PIN>` tile wire is the lower half's site pin `<PIN>` | **quoted**: the naming every site-pin wire of the database follows | `ppips_bram_l.db` |
| `ADDRARDADDR[i]` is site pin `ADDRARDADDR<i>`, `WEA[0]` is `WEA0` and `WEA1`, `WEA[1]` is `WEA2` and `WEA3`, `WEBWE[7:4]` tied low | **quoted** | nextpnr-xilinx `pack.cc`, `XC7Packer::pack_bram` |
| the mode features a 16-bit block sets: `IN_USE`, the four `*_WIDTH_*_18`, ten `ZINV_*`, `ZINIT_A/B` and `ZSRVAL_A/B` all eighteen bits (so `INIT_A = SRVAL_A = 0`); nothing for `WRITE_FIRST` or `DO*_REG = 0` | **quoted** | nextpnr-xilinx `fasm.cc`, `write_bram_half`; the feature names and bits are the database's |
| an undriven `IMUX`, `BYP_ALT`, `FAN_ALT` reads one | **the database's statement** (`default` from `VCC_WIRE`); not seen on silicon | `ppips_int_l.db` |
| a zero is `GND_WIRE -> GFAN0/1 -> IMUX/CTRL` | **the database's statement** (`INT_L.GFAN0.GND_WIRE` has bits); in `INT_L` every `IMUX_L`, `CTRL_L`, `BYP_ALT` and `FAN_ALT` is one hop from `GFAN0` or `GFAN1` (counted; `INT_R` not counted) | `segbits_int_l.db` |

### What a block RAM's inputs are tied to

The mapper connects only the pins a memory uses. The ROM connects the
read port's clock, its enable (`1'b1`) and twelve address bits
(`{sw, 4'd0}`), and nothing on the other port. Left like that, `WEA`
would read one, `DIADI` all ones — so every read would also write
`0xFFFF` — and the idle port would be enabled and writing too.
`TiePolicy` says what each unconnected input becomes:

| Inputs | Value |
|---|---|
| write enables, `RSTRAM*`, `RSTREG*`, the enable of an unconnected port, address bits | zero, through `GND_WIRE`; refused if no free path |
| data and parity inputs, and everything but the enable of a port whose enable is tied low | zero when a path is free, otherwise left |
| `ADDRATIEHIGH0/1`, `ADDRBTIEHIGH0/1` | one: the default, accepted only once the walk back from the pin reaches a `VCC_WIRE`-default wire with nothing driving the way |
| the clocks of an unused port, `REGCE*` (nothing while `DO*_REG = 0`) | left |

The demo ties 73 inputs to zero over 82 pips and leaves 5 at one (the
four tie-highs and its own enable).

### Not verified, in order of how much rides on it

1. **Nothing has been on a board.** The demo exists so that one look
   settles the contents order, the address order and the data order at
   once.
2. The undriven-is-one reading and the ground tie are the database's
   word; if an undriven input reads zero instead, the ties are still
   right and only the four tie-highs and the demo's enable change.
3. The demo's clock reaches the block over general routing, not a
   `BUFG`: the mapper gives a clock with one load local routing (the
   threshold is eight), and the Verilog frontend has no `BUFG` to
   instantiate. With one clocked element nothing can skew against it.
4. Only the true-dual-port widths 1, 2, 4, 9 and 18 are described;
   `RAM_MODE = "SDP"` and the 36-bit width are refused by name.

## What remains before an LED could light

In rough order of how much stands behind each.

1. **Somebody looking at the board.** `reticle program` exists, the
   cable works, and `blink.bit` configures the part with `DONE` high.
   What is missing is a pair of eyes: LED 0 either blinks about three
   times every two seconds or it does not, and that single observation is
   worth more than anything else on this list.
2. **Everything outside the tiles that were compared.** The oracle
   covers the IO path and the clock path. It says nothing about the
   5420-frame image as a
   whole: whether the unconfigured tiles need bits Vivado sets and
   Reticle does not, whether a tile type has a "default" configuration
   that a zero-filled frame does not give it, or whether the
   configuration engine needs anything the container does not carry.
   `XrayDatabase::decode` counts this rather than leaving it to
   impression:

   | | Reticle's `sw_led` | Vivado's harness |
   |---|---|---|
   | set bits | 85 | 3146 |
   | tiles touched | 17 | 681 |
   | named features | 56 | 912 |
   | **bits no named feature explains** | **0** | **1315** |
   | tiles with no `segbits` file at all | 0 | 11 |

   Every bit Reticle sets, it can name. Vivado sets 1315 bits that
   nothing in `prjxray-db` names, over 681 tiles — a much bigger design,
   but also, very likely, configuration a real part needs and this one
   does not have. That is the largest known unknown, and no amount of
   structural checking will close it.
3. **A carry chain — placed, routed and decoded; not yet seen on a
   part.** `examples/basys3/blink_carry.v`, a 32-bit `count <= count + 1`,
   builds to a `.bit` whose eight `CARRY4` sit up one column of slices
   with their carry on the dedicated `COUT` → `CIN` path, and every one of
   its bits decodes. *Carry chains on this fabric* below has how, what is
   checked and what is only quoted. What is missing is the board: nobody
   has loaded it.

   What this item said before 2026-10-08 is kept, because each of its
   three obstacles is what the work below answers, and the first one was
   stated as a placer limitation when it is a fact about the metal:

   > **A carry chain.** This is now the biggest hole in what the loader can
   > route, and it is not a missing table — it is packing. Three things
   > stand in the way and each is a real piece of work:
   >
   > - **`S` has no tile wire.** A `CARRY4`'s four propagate inputs are
   >   wired inside the slice to the four lookup tables' `O6` outputs and
   >   reach no wire the interconnect can drive. `ppips_clbll_l.db` lists
   >   `CLBLL_LL_A.CLBLL_LL_A1 hint`, which is the lookup table used as a
   >   wire, and taking a `hint` means putting a cell on that lookup table.
   >   So every bit of the propagate needs a lookup table **in the same
   >   slice at the same position**, which the placer cannot express: it
   >   places bels independently and its only relative-placement mechanism,
   >   the `rloc` macro, works in whole tiles.
   > - **The chain must run up one column.** `CIN` comes only from the
   >   `COUT` of the slice below (`tileconn` joins `CLBLL_LL_CIN` to the
   >   next tile's `CLBLL_LL_COUT_N`), so a seven-element chain needs seven
   >   vertically adjacent slices of the same half. Nothing in the placer
   >   knows that, and annealing for wirelength does not produce it.
   > - **`CYINIT` is a constant.** An incrementer's carry in is a one, and
   >   the bit that says so is `PRECYINIT.C1` — a feature that depends on a
   >   *pin being tied to a constant*, which `ConfigEntry` has no variant
   >   for. `PRECYINIT.CIN` for the rest of the chain would fit the
   >   existing pass-through mechanism; `C1` would not.
   >
   > What does work is mapping: `count + 1` becomes seven `CARRY4` with the
   > right wiring, `tests/fpga_carry.rs` proves the chain against the
   > primitive's own model, and the Vivado export path uses it. It is only
   > this loader's placement and routing that cannot. `examples/basys3/blink.v`
   > therefore spells its increment out as a toggle chain, which
   > `the_blink_designs_toggle_chain_is_an_increment` proves equal to
   > `count + 1`, and says so in its header.

   How each was answered: (a) the propagate pin is now declared *on* the
   lookup table's output wire, so the requirement is a property of the
   routing graph, and the placer learned to read such properties off the
   graph in general rather than to know about slices; (b) the same reading
   gives the chain's vertical adjacency; (c) `ConfigEntry` gained the
   variant it had no room for, `Tied`. Mapping was never the gap, as the
   item said. One premise of (c) was wrong, though: an incrementer's
   carry in is **not** a one. The mapper adds the one through the
   propagate — `S[0]` is `~count[0]` — and ties the carry in to zero,
   which is `PRECYINIT.C0` and costs no bits at all, so `count + 1` never
   needed `C1`. It is supported anyway, for a design that asks for it.

4. **A memory, and a `SLICEM`.** ~~Nothing has looked at `RAMB18E1`, so a
   design with one will not route.~~ *Corrected 2026-10-08:* a `RAMB18E1`
   now places, routes and configures, contents included — see *Block RAM*
   above. What this sentence missed is *why* it did not route: not an
   absent table but the loader reading both block RAM halves as wires,
   so a `BRAM_L` tile had no bel at all. Block RAM is **not** tried on a
   part. A `SLICEM`'s **distributed RAM** has now
   been looked at, because the ECP5 gained one on 2026-09-28 and the
   question "does the 7 series need the same shape of work?" had to be
   answered rather than assumed. It does, and the answer is written out
   below because it is more specific than "not done".

   `src/fpga/devices/xc7.dev` declares
   `bel RAM64X1D lutram port wclk=WCLK we=WE waddr=A0..A5 din=D
   raddr=DPRA0..DPRA5 dout=DPO`, `fpga::primitives` maps a small memory onto
   it, and `tests/fpga_flow.rs`'s `logicram_xc7` case proves it fires. What
   is missing is the fabric half: `parse::site_kind` makes nine kinds of bel
   — `lut`, `ff`, `carry`, `site`, `io`, `gb`, `bram`, `dsp`, `other` — and
   no `lutram`, so `fpga::place` counts zero of them and refuses with
   `the design needs N lutram site(s) and the part has 0`. That is the same
   message the ECP5 gave, and the same cause: a `.dev` `count` is a budget
   and the placer counts `RoutingGraph::sites`.

   **It is not the same *relationship*, and that is the news.** The ECP5's
   defining oddity is that `SLICEA.MODE = DPRAM`, `SLICEB.MODE = DPRAM` and
   `SLICEC.MODE = RAMW` are **one bit**, so a distributed RAM is three
   slices and a placer that ignored it would write a lookup table's truth
   table into a RAM's contents. The 7 series has nothing like it.
   `prjxray-db`'s `artix7/segbits_clblm_l.db` gives four *independent* bits,
   one per lookup table of the one slice:

   ```
   CLBLM_L.SLICEM_X0.ALUT.RAM 31_16
   CLBLM_L.SLICEM_X0.BLUT.RAM 31_17
   CLBLM_L.SLICEM_X0.CLUT.RAM 31_46
   CLBLM_L.SLICEM_X0.DLUT.RAM 31_47
   ```

   plus `WEMUX.CE`, `WA7USED`, `WA8USED` and `CLKINV` **per slice**, and per
   lookup table a `SMALL` (32 words rather than 64), an `SRL` and a `DI1MUX`
   whose second arm is the cascade into the next lookup table. So a
   `RAM64X1D` is contained in **one `SLICEM`** and the `SLICEL` beside it in
   the same `CLBLM` stays fully usable. The exclusion a 7-series `lutram`
   bel would need is intra-slice and smaller than the ECP5's six-of-eight.

   **Only a `SLICEM` can be one**, and that is the database's own statement
   rather than an inference from the datasheet: `segbits_clbll_l.db`, whose
   two sites are both `SLICEL`, has **zero** `RAM`, `SRL`, `SMALL`, `WA*`,
   `WEMUX` or `DI1MUX` features, and `ppips_clbll_l.db` has no `WE` wire at
   all. The 23-line difference between the two files is exactly that set.
   `sites.rs` already knows a `CLBLM` holds `SLICEM_X0` and `SLICEL_X1`;
   what it does not do is *act* on it, because `site_kind` collapses
   `SLICEL | SLICEM` into one `"slice"` and nothing downstream can ask for
   the `M`.

   The write port's wires are already there, which is the part that was
   missing for the carry chain and is not missing here.
   `ppips_clblm_l.db` has `CLBLM_M_WE` and `CLBLM_M_{A,B,C,D}I` fed
   unconditionally off the shared `FAN` bus — the same `always` shape
   `sites.rs` already reads for a flip-flop's clock — and only on the `M`
   half. `sites.rs` knows none of the five.

   And the contents may need no new code at all: `ALUT.INIT[00..63]` are
   ordinary one-bit features and `parse::parameter_bit` already turns an
   `INIT[n]` into a `ConfigEntry::Param`, which is the generic path a lookup
   table's truth table takes. Unlike the ECP5's, these bits are **not**
   `!`-marked, so an empty 7-series distributed RAM is free where an empty
   ECP5 one costs 97 bits.

   **The one piece with no ground truth here is the address permutation** —
   which of the slice's lookup tables is the read port and which the write
   port, and in what order `A0..A5` reach their inputs. On the ECP5 that came
   from nextpnr (`RAD[0]` is the `D` input, and getting it wrong is silent:
   the design places, routes, decodes and reads every word from the wrong
   address). The Vivado harness bitstream this file is checked against is
   switches-to-LEDs and contains no RAM, so it cannot settle it. That needs
   either a Vivado design with one or a board.

   Nothing of this has been attempted. One family at a time.

   **Done on 2026-10-08, except on silicon** — see *A distributed RAM on a
   `SLICEM`* below. The paragraphs above are left as they were written
   because every structural claim in them held when the code was written
   against it: one bit per lookup table, one `SLICEM`, the `SLICEL` beside
   it untouched, the five write-port wires off the `FAN` bus, `INIT` through
   the ordinary parameter path. Two things were added that they did not
   say: `CLUT.DI1MUX` with **no** bit set is the one a `RAM64X1D` wants,
   and the open question about the permutation now has an answer — quoted
   from two sources, still not measured.
5. **Feature-name to primitive-name mapping.** The loader still emits
   `ConfigEntry::Cell { primitive: "ZINI", .. }` for a flip-flop feature
   because `ZINI` is what the database calls it; Reticle's primitive is
   `FDRE` with `INIT=1'b0`. Nothing connects the two. The IO buffers
   work because `sites.rs` gathers their features under the names `IBUF`
   and `OBUF` explicitly; nothing else does.
6. **IO standards other than LVCMOS33, and tristate.** *(Written before
   2026-10-08:)* `OBUFT` and `IOBUF` need `OLOGIC` `T` features that have
   not been measured, so a tristate design will not route here. Other
   standards are refused rather than approximated.

   **The tristate half of that is done as far as a desk can take it, and
   the "not measured" is still true.** Since 2026-10-08 an `inout` port
   builds an `IOBUF` and a tri-stated `output` an `OBUFT`; both route,
   every bit of both bitstreams decodes, and the arcs read back are
   exactly the arcs routed. The `T` features were identified from the
   database and from two named sources rather than from a Vivado
   bitstream, because no harness design has a tristate pin — so the
   polarity is quoted, not checked. *A tristate, and a pad that reads
   itself* below has all of it. Pull-ups (`set_io -pullup yes`) came with
   it. Other standards, `DRIVE` and `SLEW` other than the one recipe are
   still refused or ignored as before.

   Two things about that changed on 2026-09-27 and neither of them makes it
   work. `xc7.dev` used to declare `IOBUF`'s tristate as `oe=T`, an **output
   enable**, where a `T` is a tristate and a one *releases* the pad; it now
   says `oen=T`, which is the sense the silicon has. And the mapper now does
   build a bidirectional buffer for an `inout` port a tri-state driver
   drives, rather than an output with a warning. So a 7-series tristate gets
   further than it used to and then stops in the same place: at features
   `sites.rs` has never gathered. The family where this was carried through
   to a part is Lattice; `docs/fpga-trellis.md` has it, including the `ecppack`
   comparison that says a bidirectional pad on an ECP5 costs nothing beyond
   its base type.
   One more thing the paragraph above got wrong by omission: *where* the
   mapper stopped was not only at `sites.rs`. A tri-stated `output` port
   — as opposed to an `inout` one — never got as far as the backend; the
   mapper left the `$tristate` cell behind and the netlist check refused
   it as "a generic cell, not a primitive". `xc7.dev` had declared
   `OBUFT` as `other`, so nothing could have placed one on a pad anyway.
7. **A `DSP48E1`.** Mapped for Vivado since 2026-10-08, refused here by
   name; the section *A DSP48E1: mapped, not placed* below has exactly
   what is missing, measured. With `--bitstream` the command line keeps
   multiplies in lookup tables instead.
8. **Six million `String`s.** The routing graph holds the whole die in
   1386 MiB, most of it wire names. Interning those is what makes
   whole-die routing comfortable rather than merely possible.
9. **A PLL on a part.** This list did not mention the PLL at all until
   2026-10-08, which was itself a gap: `xc7.dev` declared `PLLE2_BASE` and
   mapping instantiated it, but no fabric bel existed, so a design with a
   generated clock could not be placed. One now can be (*The PLL* above).
   `examples/basys3/pll_blink.v` is the test that settles it on a board,
   and until someone runs it, every register value the PLL section
   calls quoted is a reading and not a measurement. The `MMCME2_BASE` is
   still not done.

Until 1 is done, what this flow writes for a clocked design is a
bitstream whose every feature on the clock path matches a working one and
whose effect on silicon nobody has seen.

## A DSP48E1: mapped, not placed

`xc7.dev` declares `DSP48E1` and `fpga::primitives` puts a multiply on
it (`docs/fpga.md`, *The DSP48E1*, and `tests/fpga_dsp.rs`, which proves
the mapping against the block's model). That is the Vivado route. This
backend does not place one, and **a netlist holding one is refused by
name** (`fpga::xray::dsp_refusal`) before the database is opened; with
`--bitstream` the command line turns DSP inference off, so a `*` stays in
lookup tables and the refusal only meets a `DSP48E1` instantiated by hand.
Before this, such a netlist met the placer's generic "the design needs 1
`dsp` site(s) and the part has 0", which is true of the loaded fabric and
false of the part.

What stands in the way, each item **checked** against the pinned database
unless it says otherwise:

1. **No site, no pins.** `parse::site_kind` maps the site type `DSP48E1`
   to `dsp`, but the loader names a tile's bels by the prefix of its
   features, and every `DSP_L` / `DSP_R` feature is `DSP48.DSP_0.*` or
   `DSP48.DSP_1.*`. Loaded around the `DSP_R` column at grid x 28, each
   DSP tile becomes **one** site called `DSP48`, of kind `other`, with
   **zero pins**, and the region has no `dsp` site at all.
   `the_fabric_has_no_dsp_site_and_the_database_no_use_mult` pins this.
   The pins are in the database: `ppips_dsp_l.db` is 560 `always` lines
   from site pin to tile wire (`DSP_L.DSP_0_A0.DSP_IMUX23_0 always`), the
   shape a slice's pins already have, so this part is ordinary work in
   `sites.rs` — two sites per tile, 560 pin lines between them.
2. **`USE_MULT` has no bits.** `segbits_dsp_{l,r}.db` hold 436 lines each
   from Project X-Ray's fuzzer `100-dsp-mskpat`: the registers (`AREG_0`,
   `BREG_0`, `ZMREG`, `ZPREG`, `ZCREG`, …), `A_INPUT`, `B_INPUT`,
   `USE_DPORT`, `USE_SIMD`, `MASK`, `PATTERN`, the `ZIS_*_INVERTED`
   inverters, and local `DSP_GND_L` / `DSP_VCC_L` ties for `D`, `INMODE`,
   `OPMODE6`, `ALUMODE2..3`, `CARRYINSEL2`, four clock enables and `RSTD`. The fuzzer's
   own `generate.py` (read upstream, **quoted**: f4pga/prjxray `master`)
   varies `USE_MULT` over `NONE` / `MULTIPLY` / `DYNAMIC` and tags it as
   `USE_MULT[0..1]`, and also tags `USE_PATTERN_DETECT` — and **neither
   survived into the database**. Either a multiply needs no bit, or the
   solver could not find it. Which one is the difference between a
   multiplier and a block that outputs its `C` input, and nothing here can
   tell them apart.
3. **The `Z` features are inverted.** `generate.py` writes a `Z` tag as
   the complement of the parameter, so a blank tile is a DSP with every
   register *on* and every input *inverted*. A configured block must set
   `ZMREG`, `ZPREG`, `ZCREG`, `ZOPMODEREG`, `ZALUMODEREG`, `ZINMODEREG`,
   `ZCARRYINREG` and `ZCARRYINSELREG` to turn the registers off, and
   `AREG_0` / `BREG_0`. `ConfigEntry::ParamZero` already expresses this;
   what is missing is the table from `DSP48E1` parameters to those names.
4. **Zeros need a constant route.** An unrouted 7-series input reads one:
   `ppips_int_l.db` has `INT_L.IMUX_L0.VCC_WIRE default` for all 48 `IMUX`es.
   `OPMODE`, `ALUMODE`, `INMODE` and `CARRYIN` can take their zeros from
   their own inverters, but `CARRYINSEL[1:0]` has neither an inverter nor
   a local tie, so `000` needs `GND_WIRE` routed through `GFAN0`/`GFAN1`
   into its `IMUX`, and `INT_L.GFAN0.GND_WIRE` is a five-bit pip with two
   bits that must be *clear*. The router has no constant source.
5. **No oracle.** Every tile this backend configures was compared with
   what Vivado wrote for the same cell; the Vivado bitstream in
   `artix7/harness/` is switches to LEDs and contains no DSP. Without a
   Vivado-built design holding a `DSP48E1`, items 2 and 3 would be
   guesses on silicon.

`segbits_dsp_*.db`, `ppips_dsp_*.db` and `mask_dsp_*.db` are already in
`src/bin/reticle/prjxray-db.manifest`; nothing needed fetching.

## Carry chains on this fabric

`examples/basys3/blink_carry.v` is `count <= count + 1` on 32 bits with
the top sixteen on the LEDs (fifteen of them; see the file for why LED 6
is left alone). Mapping makes it eight `CARRY4`. Built with

```sh
reticle fpga --device xc7a35t-cpg236 \
    --constraints examples/basys3/blink_carry.rcf \
    --bitstream blink_carry.bit \
    examples/basys3/blink_carry.v
```

it places, routes every signal that has a reader (105 of them; the other
25 are carry-out bits nothing reads), and **every one of the 2560 bits it
sets decodes back into a feature `prjxray-db` names, with none left over.
The 453 interconnect arcs those features name are exactly the 453
bit-costing pips the router took**, tile for tile and name for name.
`tests/fpga_xray_carry.rs` asserts both. The chain lands on
`SLICEL_X1` of `CLBLM_R_X11Y30` up to `CLBLM_R_X11Y37`, one tile per
link.

**Nothing of this has been on a board.** What a person should see is in
the design's header: a binary count, each visible LED blinking at half
the rate of the one to its right, LED 9 at the 1.49 Hz `blink.v` was
watched at.

### What the slice requires, and where each requirement now lives

Three facts about the silicon, each read off the database:

| Fact | Where it is in the database | How the flow obeys it |
|---|---|---|
| `S[n]` *is* the `O6` of the lookup table at position `n` | `ppips_clbll_l.db` gives `S` no wire; the only line near it is the `hint` that is the table used as a wire | the `CARRY4` bel's `p<n>` pin is declared **on the lookup table's output wire** (`CLBLL_LL_A`), so the router can reach it only from that table |
| `CIN` comes only from the `COUT` of the slice below, same half | `tileconn.json` joins `CLBLL_LL_CIN` to the next tile's `CLBLL_LL_COUT_N` (and through `HCLK_CLB` across a clock row) | `co3` is on `COUT`, `ci` on a wire fed from `CIN` by a hop costing `PRECYINIT.CIN` |
| a constant carry in is `PRECYINIT.C0` / `C1`; a constant generate is `O5` via `<L>CY0` | `segbits_clbll_l.db`; meaning from prjxray's fuzzers, see below | a new `ConfigEntry::Tied { pin, value, bits }`: bits set when the cell's pin is tied to that constant |

The first two are now properties of the **routing graph**, and the placer
reads them from there rather than knowing anything about slices:
`place::build_clusters` walks back from every sink pin a signal has; when
the walk closes within eight wires on every site of the sink's kind and
finds exactly one site of the driver's kind each time, the driver's site
is a function of the sink's, and the instances so related become one rigid
macro that legalisation and the annealer move as a whole. For the counter
that is one macro of 40 cells — eight `CARRY4` and 32 lookup tables. On a
family whose pins are all on the interconnect no walk closes and nothing
changes; the whole test suite, ECP5, Gowin and iCE40 goldens included,
is unchanged by it.

The third needs no placement: `PRECYINIT.C0` is every bit clear, so the
mapper's first `CARRY4` (carry in tied to zero) sets nothing, and
`PRECYINIT.C1` — which the mapper never asks for, because an `add` has no
carry-in port — is there for a design that ties a carry in to one.

What the mapped netlist lacked is supplied by `xray::legalise_carries`:
`count + 1` folds 31 of the 32 propagate bits into plain flip-flop
outputs (`count[i] ^ 0`), so each gets a **buffer lookup table**,
`INIT = 0xAAAAAAAAAAAAAAAA`, which is a route-through. A lane whose
generate is a constant gets a table of its own whose lower half is the
constant, for `O5`, with `I5` tied to one in the netlist so the cell's
model reads the upper half just as `O6` does with `A6` high. A lane whose
outputs nothing reads is left alone and its inputs become `x`.
`tests/fpga_carry.rs::legalising_a_chain_for_a_slice_changes_no_sum`
simulates the legalised netlist exhaustively against the arithmetic.

The sums leave the carry two ways and the router picks: into the
flip-flop beside them (`<L>FFMUX.XOR`) or out through the slice's output
mux (`<L>OUTMUX.XOR`) to a flip-flop elsewhere. The placer did not pack
the flip-flops: in the build above five sums take the first way and 27
the second. That is wire, not a fault.

### Checked, quoted, and not known

**Checked**, against the database or by a test:

- every bit set decodes, and the decoded arcs are the routed pips
  (`the_carry_counter_decodes_into_exactly_what_was_placed_and_routed`);
- the chain is one column, one slice half, each link one tile up or two
  across a clock row (`the_chain_runs_up_one_column_of_one_slice_half`);
- each `S[n]`'s driver is the lookup table at letter `n` of the same slice
  (`every_propagate_bit_comes_from_the_lookup_table_beside_it`);
- the first slice carries no `PRECYINIT` feature, each later one exactly
  `PRECYINIT.CIN`, and each carry is routed `COUT` → `COUT_N` → `CIN`
  without touching the interconnect
  (`the_chain_starts_from_a_constant_and_continues_on_cin`);
- the legalised netlist computes the same sums
  (`legalising_a_chain_for_a_slice_changes_no_sum`), and that test fails
  when the buffer table reads the wrong input — tried;
- no wire in `artix7/` contains `_CARRY_`, so the invented in-slice wires
  cannot collide (a `grep` over every `segbits`, `ppips` and `tileconn`
  file of the pinned commit).

**Quoted**, from documents and not measured here — there is no Vivado
bitstream with a carry chain in `artix7/harness/`, so none of this has an
oracle the way the IO and clock paths do:

- `PRECYINIT`'s four settings mean logic 0, logic 1, `AX` and `CIN`:
  prjxray's `fuzzers/017-clb-precyinit/README.md`;
- `<L>CY0` clear takes `DI` from the bypass input `<L>X`, set from `O5`:
  `fuzzers/013-clb-ncy0/README.md`. **This is the opposite of the reading
  first made while writing this**, which took the bit as "use `AX`"; the README's table and
  nextpnr-xilinx's `fasm.cc`, which writes `<L>CY0` only when the `CY0`
  mux's source pin is `O5`, both say otherwise. Had the guess stood, every
  generate would have come from an `O5` nobody configured;
- the output mux's `XOR` is the `CARRY4`'s `O[n]` and its `CY` is
  `CO[n]`: the variants of `fuzzers/016-clb-noutmux/top.py`;
- `S[n]` is position `n`'s `O6`: UG474's slice diagram;
- an unrouted interconnect input reads one: `ppips_int_l.db` records
  `VCC_WIRE` as every `IMUX`'s `default` source. The buffer tables do not
  depend on it — their halves are equal — but a lane with a constant
  generate does, through `A6`.

**Not known**: whether it counts. A carry chain is the first path in this
flow whose every feature is a reading of a fuzzer rather than a
transcription of a working bitstream, so the board is the first oracle it
will have.


## Where the code is

| | |
|---|---|
| `src/fpga/xc7.rs` | the UG470 container: frames, packets, the frame address register, the CRC, the `.bit` wrapper, a reader |
| `src/fpga/xray/mod.rs` | the loader: the database as an `Arch` plus a `FrameMap`, with the region and the measurement |
| `src/fpga/xray/parse.rs` | one reader per file of the database |
| `src/fpga/xray/lutram.rs` | a `SLICEM`'s distributed RAM: the `RAM64X1D` bel, its pins, its blocks and its bits, with the sources of the quoted part |
| `src/fpga/xray/carry.rs` | `legalise_carries`: a lookup table for every propagate bit of a `CARRY4` that has none, and the constant generates in `O5` |
| `src/fpga/xray/sites.rs` | the inside of a site: pin names from UG474 and UG471, the wire each sits on, the orientations the database only implies, the IO recipe read off Vivado's own bitstream, and the clock tables — the `BUFGCTRL`, the `BUFHCE` of a clock row, the wires that cost bits to touch and the rebuffer enables |
| `XrayFabric::enable_global_clocks` | the one bit that belongs to no pip: a global clock's rebuffer enables, over the whole column, once the routing is known |
| `src/fpga/xray/cmt.rs` | the clock management tile: the PLL as a bel, its DRP register values (computed and quoted, each said which), and `XrayFabric::configure_clock_managers` |
| `tests/fpga_xray_pll.rs` | the PLL design end to end, and two register settings decoded back |
| `src/fpga/devices/xc7.dev` | the device: primitives, pins, and now the IDCODE |
| `tests/fpga_xray.rs` | everything above, against the real database, skipping without it |
| `tests/fpga_xray_lutram.rs` | the distributed RAM, built from `examples/basys3/lutram.v` and decoded |
| `examples/basys3/` | the two designs that have reached a part, the tristate, distributed-RAM, PLL and carry-chain demos that have not yet, and their constraints |
| `src/fpga/xray/dsp.rs` | the refusal of a `DSP48E1`, and the measured reasons for it |
| `tests/fpga_xray_carry.rs` | the carry chain: placed up one column, propagate from the lookup table beside it, `CIN` and a constant carry in, every bit decoded and every arc the router's |
| `src/fpga/xray/bram.rs` | block RAM: the `RAMB18E1` bel's pins, its mode features, and `XrayFabric::configure_block_rams`, which also ties every input the design leaves alone |
| `tests/fpga_xray_bram.rs` | block RAM: both halves, the frame-window stacking, `examples/basys3/bram_rom.v` and a four-block design, every bit decoded, every arc the router's or a tie's, the contents bit for bit |
| `XrayDatabase::decode` | the other direction: a bitstream back into the database's feature names, with an accounting of every bit it could not name |

`src/fpga/arch/synthetic.rs` is untouched and still says what it always
said: that fabric is synthetic, it is not an iCE40, and it programs
nothing. Nothing here changes that.
