# A real Lattice ECP5, and a real `.bit`

## A bidirectional pad has been built, and loaded into a part

On 2026-09-27 `testdata/fpga/cynthion/bidir_loopback.v` — one pad that
**drives** while the USER button is held, is **released to high impedance**
when it is not, and is **read back through its own input buffer** onto other
LEDs — was compiled by this flow and loaded into the LFE5U-12F of the Great
Scott Gadgets Cynthion r1.4 attached to the machine this was written on.
**The part accepted it and asserted `DONE`** with no fault bit set: status
`0x00200100`, the same as every run before it, which is exactly why `DONE`
is not the claim being made. The board went on enumerating at the same USB
serial number afterwards, on the same bus and device number.

It displaced `clock_blink.bit`, which the part had been holding since the
previous milestone, and **only the volatile configuration SRAM was written** —
a power cycle reloads the analyzer gateware from the board's flash, which
nothing in this project touches. `docs/programming.md` and
`src/program/lattice.rs`'s `NOT_SHIFTED` are where that is made checkable
rather than promised.

What the flow reports for it:

```
2557 configuration bit(s) set, 8 pad(s), 72 lookup table(s) and 26
flip-flop(s) configured, 26/24288 ff, 1/56 gb, 8/120 io, 72/24288 lut
routed 102 of 102 signal(s) with 1126 pip(s) over 1228 wire(s), and every
sink was walked back to its driver
26 flip-flop(s), every clock on a global network: G_HPBX0000 to 26 of them
all 2557 set bit(s) decode back through the database into 722 arc(s), 190
field(s) and 72 word(s), with 0 unexplained, and the arcs they select are
exactly the 722 the router chose
```

So **100% of the bitstream decodes, with nothing left over**, which is the
same standard `button_led` (114 bits) and `clock_blink` (2491 bits) were held
to.

That is the first bidirectional pad this project has built on any family.
The 7-series backend declares `OBUFT` and `IOBUF` and has never routed a
tristate; until now `configure_io` read the direction off the netlist and
built an input or an output, and an `inout` port got an output with a warning.

**Nobody has looked at the board yet.** Everything below "What a person
should look for" is a prediction written down before the observation, which
is the order the previous three milestones were done in and the only order in
which the observation is worth anything. When somebody has pressed the
button, this section gets the paragraph the two below it have.

### Which pad, and why it is safe to drive

The bidirectional pad is ball **E13**, which is `led_n[0]` — the LED at the
end of the `FPGA LEDs` row farthest from the USER button, the diode `D7`.
Driving a ball that something else on the board also drives can damage
hardware, so the reason this one is free is written out:

1. **Great Scott Gadgets' own platform file mentions E13 exactly once**, in
   `cynthion/python/src/gateware/platform/cynthion_r1_4.py`, as `led[0]` of
   `*LEDResources(pins="E13 C13 B14 A15 D12 C11", …)`. Nothing else in that
   file claims it: not a ULPI bus, not the HyperRAM, not a Type-C
   controller, not a pseudo-supply pin, not a PMOD or mezzanine pin.
2. **The schematic says what is on the net and it is passive.** In
   `indicators_buttons.kicad_sch` all six LED anodes are on one wire up to
   the `+3V3` symbol and each cathode goes through a series resistor to the
   FPGA. So the only things on E13 are a resistor and a diode to a supply
   rail, and **the FPGA is the only driver**, which is the whole question.
3. **This project has already driven it low** — `leds.v`, `button_led.v` and
   `clock_blink.v`, each watched by a person on this board. Releasing a pin
   is strictly less demanding than driving it.
4. **A released E13 settles high on its own, and nothing is stressed.** The
   only current path is +3V3 through the resistor and the LED *into* the
   pad, so the board can only pull the pin up, never down; with the internal
   pull-up pulling the same way the pad sits at VCCIO, no current flows
   through the LED, and the readback is a firm one. That is also why the LED
   is dark when the pad is released rather than dimly lit.

The textbook answer would have been a **PMOD** pin — PMOD A is C9 B9 D11 C12
C8 D8 D9 C10 and PMOD B is B4 B5 B6 B7 C5 A5 A6 A7, all of them `dir="io"`
user IO on the top edge — and it was deliberately **not** taken: those go to
a 2×6 header and nothing this machine can read says whether anything is
plugged into it. A pin whose net is fully described by a schematic and has no
other driver is a better bet than a pin that is probably unconnected.

For the record, the only ball on the two edges this backend describes that
the platform file never mentions at all is **B3** (top edge, column 4, side
B, bank 0). It is a worse choice for the same reason turned around: the
file's silence is not a statement that the ball is free.

### What a person should look for

**Press and hold the button silkscreened `USER`.** The other two tactile
buttons end the experiment rather than perform it: `PROG` reloads the FPGA
from flash and `RESET` resets the debug microcontroller.

Watch **all six** FPGA LEDs, counted from the end of the row farthest from
that button (`led_n[0]` is that end, the diode `D7`; the row has one
silkscreen legend, `FPGA LEDs`, and no digits):

| | LED 0 (far end) | LED 1 | LED 2 | LED 3 | LED 4, 5 |
|---|---|---|---|---|---|
| nothing touched | dark | dark | **lit, steady** | blinking | dark |
| `USER` held | blinking | blinking, **with LED 0** | blinking, opposite | blinking, **with LED 0** | dark |

So: **one LED blinks alone until the button is held, and then four of the six
are moving.** LED 2 is the one to look at first, because it is lit and *still*
with nothing pressed and starts moving the moment a finger arrives. LED 1 and
LED 2 are complements, so exactly one of the two is lit at every instant,
whether the button is held or not — which is the steady state nothing else on
this board produces.

While the button is held, what a person sees at any instant is either **LEDs
0, 1 and 3 lit with LED 2 dark**, or **LED 2 lit with 0, 1 and 3 dark**,
alternating; with nothing pressed it is **LED 2 and LED 3** or **LED 2**
alone.

**At what rate.** 0.89 Hz — 0.56 s on, 0.56 s off, a little slower than one
blink a second; ten full blinks take 11.2 s. Bit 25 of a counter on the
board's 60.000 MHz oscillator, so 2²⁵/60e6 = 0.5592 s, and a counter bit out
by one would read 0.45 Hz or 1.8 Hz.

**What LED 0 is.** LED 0 *is* the bidirectional pad. It is lit exactly while
the pad is pulling its own pin low, which is while the button is held and the
counter bit is zero. LED 1 is lit exactly while the **input buffer of that
same pin** reads low. So LED 0 and LED 1 blinking in lockstep is the
turnaround seen rather than inferred: what goes out comes back in.

The observation is deliberately spelled with four LEDs rather than one,
because each way of getting this wrong produces a different picture:

| What would be wrong | What it would look like |
|---|---|
| the enable inverted | LED 0, 1, 2 move with nothing pressed and freeze when the button is held |
| the tristate tied instead of routed | LED 0 and LED 1 blink whether or not anybody presses anything |
| the pad never driven | LED 0 never lights and LED 2 never moves |
| the input path dead | LED 1 and LED 2 never move, whatever LED 0 does |
| `PULLMODE` left at the database's default | with nothing pressed LED 1 is lit and LED 2 dark — the other way round from the table — or the two flicker |
| the clock stopped | LED 3 frozen |

None of those is the table. And a design that merely *compiled* a
bidirectional pad could not produce it: a pad that always drives cannot make
LED 2 sit still, and a pad with no input path cannot make LED 1 move at all.

### What was checked instead, since `DONE` is not evidence

**1. Every bit of the bitstream was read back through Project Trellis' own
database, and it says what the router said.** All 2557 bits, with **nothing
left over**, and the set of connections the bits select — resolved back into
the same wires and positions the router works in — is **exactly** the 722
arcs the router chose. `reticle fpga --bitstream` runs that itself and
refuses to write a bitstream that fails it.

**2. The pad's own settings were read back out of the finished image** and
they are `PIOB.BASE_TYPE = BIDIR_LVCMOS33` and `PIOB.PULLMODE = UP` at
(col 63, row 0), which is where the top edge's rule puts side B of the ball
at column 62.

**3. The tristate is not tied, and that had to be asked of the pass rather
than of the image.** `configure_io` writes `CIB.JB0MUX = 0` for every
ordinary output, which holds the tristate wire low so the buffer always
drives. For this pad a *signal* drives that wire, and the tie and the route
are **one mux**, so the tie must not be written — writing it would be a
second driver on the wire the router already drove. A finished bitstream
cannot be asked whether a pad is tied, for exactly that reason, so
`tests/fpga_trellis.rs::the_bidirectional_design_routes_and_configures_what_its_header_promises`
runs `configure_io` into an empty bitmap of its own and asserts that E13's
tie is absent **and that C13's, an ordinary output next door, is present**.

**4. All three of the pad's wires carry a routed signal**, and the
tristate's driver is the USER button's own pad: M14's buffer is at
(col 72, row 32) on the right edge and E13's is at (col 62, row 0) on the
top, so the tristate crosses the die.

**5. The question was asked the other way round** — not "what differs" but
"what does `ecppack` write, in full, for a bidirectional pad" — against
`analyzer.bit`, which has **eight** of them. That is the section below, and
it is where the one surprise is.

### Everything Lattice's own packer writes for a bidirectional pad

This is the third time this question has been asked this way round and it is
the reason to keep asking. A diff against a reference only disagrees about
settings already emitted; it is silent about settings never emitted at all,
and `BANK.VCCIO` and an input's `PULLMODE` were both of the second kind with
`DONE` high either way.

There is a **direct oracle** for a bidirectional pad on this board. All three
of Great Scott Gadgets' USB ports go through ULPI transceivers whose
eight-bit data bus turns around, and their own `analyzer.bit` instantiates
the auxiliary one. Amaranth's `ULPIResource` makes `data` a
`Subsignal(dir="io")`, so `F16 G15 G16 H15 J15 J16 K15 K16` are eight
bidirectional pads built by `ecppack` for this very part. All eight are on
the **right** edge, which this backend describes.

Per ball, `analyzer.bit` has:

| Setting | Tile | Bits beyond the base type | Written here |
|---|---|---|---|
| `PIO<s>.BASE_TYPE = BIDIR_LVCMOS33` | the pad tile | — | yes |
| `PIO<s>.BASE_TYPE = BIDIR_LVCMOS33` again | the second-copy tile | — | yes |
| `PIO<s>.PULLMODE` | the pad tile | **one**, and it is the field's own | yes, always |
| `PIO<s>.HYSTERESIS = ON` | the pad tile | **none**: the base type's own bits contain it | yes |
| `PIO<s>.SLEWRATE = FAST` | the pad tile | one | **no**, and see "What remains" |
| `BANK.VCCIO = 3V3` | `BANKREF<n>` | — | yes |
| anything governing the **tristate** | — | **nothing at all** | nothing to write |

`tests/fpga_trellis.rs::what_lattices_own_packer_writes_for_a_bidirectional_pad`
asserts the first four rows bit by bit at absolute frame positions, for all
eight balls, which is the same standard the LEDs and the button were held to.

**The last row is the finding, and a diff could not have produced it.** The
field that governs where a pad's tristate comes from is
`PIO<s>.TRIMUX_TSREG`, in the second-copy tile, and its two values are
`PADDT` — the wire the fabric drives — and `IOLTO`, the `IOLOGIC` tristate
register. **`PADDT` is the default, so it costs no bits**, and it is exactly
what a fabric-driven tristate means. nextpnr writes the field only when its
packer moved a tristate flip-flop into `IOLOGIC`
(`pio->params[id_TRIMUX_TSREG] = "IOLTO"` in `ecp5/pack.cc`), and the test
asserts that **no `TRIMUX_TSREG` appears anywhere in the whole file**
although eight of its pads are bidirectional. The same is true of
`DATAMUX_ODDR`, `DATAMUX_OREG` and `DATAMUX_MDDR`, whose default `PADDO` is
likewise free — `analyzer.bit` does write `DATAMUX_ODDR = IOLDO` on its
HyperRAM pins, which are registered, and on no ULPI pin.

So: **a bidirectional pad differs from an output by its base type and
nothing else, and the tristate is a routed wire rather than a setting.** The
question that found two bugs found nothing this time, and that is worth as
much as the two finds: the next doubt about a bidirectional pad is not "a
missing bit".

The other half is what nextpnr does *not* write. `write_io` ties the tristate
in the `CIB` under

```cpp
if (dir != "INPUT" && (ports.find(id_T) == ports.end() || ports.at(id_T).net == nullptr) &&
    (ports.find(id_IOLTO) == ports.end() || ports.at(id_IOLTO).net == nullptr)) {
    …
    cc.tiles[cib_tile].add_enum("CIB." + cib_wirename + "MUX", "0");
}
```

which is *exactly the complement* of a real bidirectional pad: the tie
happens for an output and for a bidirectional pad whose `T` is unconnected,
and never for one a signal drives. `configure_io` follows the same rule, from
the pin rather than from the parameter. That half cannot be read off a
finished bitstream, which is why point 3 above asks the pass instead.

Two more rows of that table are worth their own sentence.

**`PULLMODE` is the third `BANK.VCCIO`, and on a bidirectional pad the
argument is stronger than on an input.** The field's default in `bits.db` is
`DOWN`. On a pad that spends half its time released, the pull is the **only**
thing deciding what it reads then — so a pad left at the default reads low
whatever is on the pin, which looks exactly like a dead input path. This
design asks for `UP` (`set_io -pullup yes`), which the `.dev` file turns into
`PULLMODE="UP"` on the cell and `configure_io` reads back off there. The
reference asks for `NONE` on its ULPI pins, which is right for a bus a
transceiver drives and wrong for a pin with nothing on it.

**`HYSTERESIS = ON` costs nothing here, and that is luck rather than
design.** `BIDIR_LVCMOS33`'s own pattern on the top edge contains `F14B0`,
which is the hysteresis bit, so writing the field adds no bits. It is written
anyway, because nextpnr writes it for `dir == "INPUT" || dir == "BIDIR"` and
because "we write what it writes" should stay literally true.

### A third place where two features share one bit

This file already had two: a `CIB`'s constant mux against the routing mux into
the same wire, and a centre mux's six-bit source code. Here is the third, and
it was found by trying to assert the obvious thing. ("What cannot be read
back", below, is the fourth, and the only one of the four that is not handled;
"The three things worth knowing if you change this" has all four in one
table.)

The one bit `OUTPUT_LVCMOS33` has that `BIDIR_LVCMOS33` does not is, on the
top edge, `F7B0` — **which is also `PULLMODE`'s low bit.** `DOWN` is
`!F7B0 !F8B0`, `NONE` is `F7B0`, `UP` is `F7B0 F8B0`; so on this family "an
output" and "a pull that is not a pull-down" are the same bit, and an output
pad gets `PULLMODE=NONE` for free whether it asked or not. The right edge
spells that bit differently for each of its four sides — `F2B0` for side A of
a `PICR1`, `F0B7` for side D — and the relation is the same on every one of
them.

Two consequences:

- **A finished image cannot be asked whether a pad is an output.** A
  bidirectional pad plus any pull is a strict superset of the output
  pattern, and `TrellisDatabase::decode` resolves it by the longer match,
  which is why it reads back `BIDIR_LVCMOS33` and not `OUTPUT_LVCMOS33`. The
  test asserts the bits `BIDIR_LVCMOS33` has and the other two patterns lack,
  which is the direction that *can* be asserted, and says so where it does it.
- **The two settings have to be written into one tile without one clobbering
  the other**, which is what they are: bits are OR-ed into a zeroed bitmap,
  and `dropped_clear_bits` is what notices a feature whose bits another
  feature wanted clear. It reports nothing for this design.

### What cannot be read back: two bidirectional pads on one right-edge tile

This was found by building the thing the milestone does not build — an
**eight-bit** bus, in the ULPI shape, on the auxiliary transceiver's own
balls — and it is the sharpest limitation this backend has.

An eight-bit bidirectional bus on the **top** edge builds and decodes
completely: 465 bits, 0 unexplained, every arc the router chose. On the
**right** edge the same design is **refused**, with eight bits belonging to
no feature — two per *pair* of bidirectional pads that share a pad tile:

```
error: 8 of this bitstream's 650 set bit(s) belong to no feature the
database names, so nothing was written
  F5B0 of PICR1 at (col 72, row 15)
  F6B0 of PICR1 at (col 72, row 15)
  …
```

The reason is the third shared-bit case again, one turn worse. On the right
edge four PIOs share one pad tile, and there a **pseudo-differential** value
of `PIO<s>.BASE_TYPE` reaches across the pair:

| Value of `PIOA.BASE_TYPE` in `PICR1` | Bits |
|---|---|
| `BIDIR_LVCMOS33` | `F0B0 F3B1 F4B1 F5B0 F5B1 F6B0 F6B1 F7B0` — eight, all PIOA's |
| `OUTPUT_LVCMOS33D` | `F0B0 F0B3 F1B3 F2B0 F3B1 F4B1 F5B1 F7B0 F8B3 F9B4` — **ten**, four of them **PIOB's** |

With side B also bidirectional, PIOB's own base type and pull mode set
`F0B3`, `F1B3`, `F8B3` and `F9B4`; PIOA's pull mode sets `F2B0`. So all ten
bits of `OUTPUT_LVCMOS33D` are set, and `decode` — which resolves a field by
the **longest** matching pattern, exactly as `libtrellis`' own
`Tile::get_config` does — reports side A as a differential output it is not,
and leaves `F5B0` and `F6B0`, the two bits only a bidirectional or an input
pad wants.

**Lattice's own packer produces a bitstream with the same property**, and
that is measured rather than argued. `analyzer.bit` has F16 and G15 — sides A
and B of (col 72, row 14) — as ULPI data pins, so both are bidirectional, and
decoding it reports

```
(72, 15): PIOA.BASE_TYPE=OUTPUT_LVCMOS33D … PIOB.BASE_TYPE=BIDIR_LVCMOS33
```

with the same bits left over.
`tests/fpga_trellis.rs::what_lattices_own_packer_writes_for_a_bidirectional_pad`
asserts both halves of that, so the day it stops being true the test says so.

So this is a limit of reading a `bits.db` back, not a wrong bitstream — the
silicon decodes bits and `F5B0` set is not a state `OUTPUT_LVCMOS33D`
produces; what `bits.db` cannot do is partition the tile's bits between two
PIOs of one pair. **The check was left in place anyway**, and the flow refuses
such a design, because "every bit decodes" is the strongest thing this backend
has and weakening it to admit a case would weaken it for every case. Three
things follow:

- a bidirectional **bus** works today on the **top** edge and is refused on
  the right one;
- so a ULPI data bus, whose eight balls are all on the right edge, is blocked
  on this and not on anything about the pad;
- and the fix is in `decode` rather than in `configure_io`: resolving a
  field by "the longest match" should become "the match that leaves fewest
  bits unexplained", which on this tile picks `BIDIR_LVCMOS33` for side A
  because `F0B3`, `F1B3`, `F8B3` and `F9B4` are covered by PIOB's own fields
  either way. That is a change to the most load-bearing check in this
  backend, so it wants its own milestone: the two reference bitstreams and
  every design in `testdata/fpga/cynthion/` must decode to the same thing
  afterwards, and `analyzer.bit`'s ULPI pads must read back as
  `BIDIR_LVCMOS33` on **both** halves of a pair, which is the check that
  would prove it right.

That is also why the milestone at the top of this file is **one** pad, and
why it is on the top edge.

### The enable had a side, and two files had it the wrong way round

A `.dev` file's `io` line names an IO buffer's pins by role, and the enable
had one role, `oe`, meaning "a one drives the pad". Two of the four families
wrote `oe=T`, and **a `T` is a tristate: a one *releases* the pad.** So
`src/fpga/devices/xc7.dev` and `src/fpga/devices/ecp5.dev` had the enable
inverted, and `src/fpga/devices/gowin.dev` said in prose that its own
`OEN` could not be expressed and declared its `IOBUF` unusable for that
reason.

Nothing had ever noticed, because nothing had ever driven the pin: the
mapper tied an `inout` port's enable to a constant **one** and printed
"the output enable is tied active", which on a `TRELLIS_IO` or an `IOBUF`
means permanently high impedance. On the ECP5 the two halves even disagreed
with each other — the netlist said "released" and `configure_io` tied the
hardware wire low, which is "driving" — and neither was checked against the
other.

There are now two role spellings and they are opposites:

| Role | Meaning | Who uses it |
|---|---|---|
| `oe=<port>` | an **output enable**: a one drives the pad | iCE40 `SB_IO.OUTPUT_ENABLE`, the generic device's `IOBUF.OE` |
| `oen=<port>` | a **tristate**: a one releases the pad | ECP5 `TRELLIS_IO.T`, Xilinx `IOBUF.T` and `OBUFT.T`, Gowin `IOBUF.OEN` |

`BelKind::enable_port` answers `(port, active_low)` and is where the reason a
marker would not have done is written down. `fpga::place`'s `describe`
renames `oen` to `oe` on the way into the netlist, because the two are one
*pin* and a routing graph knows nothing about logic; the inversion happens
once, where the buffer is built. `src/fpga/mod.rs`'s built-ins test now
asserts that every family's bidirectional buffer names exactly one of the
two, and `src/fpga/primitives.rs`'s
`an_inout_port_with_a_tristate_driver_becomes_a_bidirectional_buffer` asserts
that the same source produces opposite pin polarities on an ECP5 and an
iCE40.

Gowin's `IOBUF` is still declared `other` and still never instantiated, but
for a smaller reason than before: nothing has built a tristate on that family
or put one on a Gowin part, and the backend has no configuration bits for a
Gowin pad at all.

### What an `inout` port has to say to become one

Three spellings reach the IR's `tristate` cell, and the IO pass takes that
cell over wherever it drives the whole of an `inout` port's net:

```verilog
assign bus = oe  ? data : 8'bz;    // drive while `oe`
assign bus = dir ? 8'bz : data;    // release while `dir` — a ULPI bus
bufif1 t (bus, data, oe);
```

and in VHDL, `bus <= data when oe else 'Z';`, which already became that cell.
The conditional-assignment forms are new: Verilog's lowering used to leave
them as an assignment of an unknown value, so `assign bus = oe ? d : 8'bz`
produced a pad that drove at all times. `{8{1'bz}}` and `8'bz` are both taken,
because the operand is read after folding.

What the pass then does, per bit: the cell's data becomes what the buffer
drives *out* to the pad (`dout`), its enable becomes the buffer's enable pin
*in that pin's own sense*, the cell is dropped, and the port's own net is
re-driven from what the buffer reads *in* off the pad (`din`) — so everything
in the design that read the port now reads the **pin**. A net cannot have two
drivers, which is why the cell has to go rather than sit alongside.

Anything else is still buffered as an output, with the warning reworded to
say what to write instead: a net two tri-states share, a net a tri-state
drives only part of, a registered (DDR) port, or an ordinary driver. A
partial `z` (`{7'bz, 1'b0}`) is deliberately not this, because a partial
tri-state has no IR form and widening one would change what the design means.

### What this settles, and what it does not

It settles that the chain from Verilog to a configuration a real ECP5 accepts
closes for a pad the design **drives, releases and reads back through one
pin**, that the tristate is routed and not tied, that the bits which decide
the released level are written and read back as `PULLMODE = UP`, and that
every bit of the image is one the database explains and selects the
connection it was meant to.

And it settles all of that only as far as `DONE`, which the first milestone in
this file proved is worth nothing on its own: the LEDs were dark and `DONE`
was high. **Nobody has looked at this one yet.**

It settles nothing about a bidirectional **bus on a part**: one bit has been
loaded, not eight. An eight-bit bus *builds* on the top edge —
465 bits, 0 unexplained — and is refused on the right one, for a reason that
is about reading a bitstream back and not about writing one; "What cannot be
read back" above has it, and it is the thing standing between this and a ULPI
data bus. It settles nothing about `SLEWRATE`, which every ULPI pin of the
reference asks for and this writes for none. Both are in "What remains".

## A clocked design has been built, and loaded into a part

On 2026-09-26 `testdata/fpga/cynthion/clock_blink.v` — a 26-bit counter off
the board's own 60 MHz oscillator, blinking two of its LEDs in antiphase —
was compiled by this flow and loaded into the LFE5U-12F of the Great Scott
Gadgets Cynthion r1.4 attached to the machine this was written on. **The
part accepted it and asserted `DONE`** with no fault bit set: status
`0x00200100`, the same as every run before it, which is exactly why `DONE`
is not the claim being made. The board went on enumerating at the same USB
serial number afterwards.

What the flow reports for it:

```
2491 configuration bit(s) set, 7 pad(s), 72 lookup table(s) and 26
flip-flop(s) configured, 26/24288 ff, 1/56 gb, 7/120 io, 72/24288 lut
routed 100 of 100 signal(s) with 1089 pip(s) over 1189 wire(s), and every
sink was walked back to its driver
26 flip-flop(s), every clock on a global network: G_HPBX0000 to 26 of them
all 2491 set bit(s) decode back through the database into 693 arc(s), 189
field(s) and 72 word(s), with 0 unexplained, and the arcs they select are
exactly the 693 the router chose
```

**Somebody watched, and they blink.** On 2026-09-27 the board's owner
reported the two LEDs blinking and swapping between each other, at a rate
that looked right — which was the point of choosing 0.89 Hz, since a
counter bit out by one would read 0.45 or 1.8 Hz and be obvious by hand.

So a **clocked** design works on this part. That is the last of the four
things this backend needed: pads, interconnect, lookup tables and now
flip-flops driven from a global clock network, each confirmed by somebody
looking at the board rather than by anything this machine can check.

### What a person should look for

**Nothing has to be pressed.** No button, no switch, no host software: the
oscillator runs as soon as the board is powered, so this starts the moment
the bitstream is loaded and never stops.

Watch the two LEDs at the end of the `FPGA LEDs` row **farthest from the
`USER` button** — the same two `button_led.v` uses, `led_n[0]` at the very
end and `led_n[1]` next to it. They **trade places, over and over, and one
of them is lit at every instant**:

| | far end | next to it | the other four |
|---|---|---|---|
| half the time | **lit** | dark | dark |
| the other half | dark | **lit** | dark |

**At what rate.** Each LED is on for **0.56 s** and off for 0.56 s, so each
blinks at **0.89 Hz** — a little slower than one blink a second. Ten full
blinks take **11.2 seconds**, which is the easiest thing to time by hand.

The arithmetic, so that a wrong counter bit is a wrong *rate* rather than a
shrug: the oscillator is 60.000 MHz, bit 25 of the counter changes every
2^25 cycles, and 2^25 / 60e6 = 0.5592 s. A bit out by one would read
0.45 Hz or 1.8 Hz.

The observation is deliberately a strong one. Exactly one of the two lit,
always, is a steady state nothing else on this board produces: a dark
board, a board still running the analyzer gateware from flash, a bitstream
whose bank rail is missing and a configuration that does nothing all look
alike, and two LEDs trading places at about one hertz looks like none of
them. It also distinguishes a *running* clock from a present one — a stuck
clock freezes the pair in one of its two states, which is visibly different
from both being dark.

### What was checked instead, since `DONE` is not evidence

Four things, in order of how much they are worth.

**1. Every bit of the bitstream was read back through Project Trellis' own
database, and it says what the router said.** All 2491 bits, over 77
tiles, with **nothing left over**, and the set of connections the bits
select — resolved back into the same wires and positions the router works
in — is **exactly** the 693 arcs the router chose. That second half is the
check worth having, and it is now run by `reticle fpga --bitstream` itself
rather than only by a test: a bitstream whose bits select something else is
refused instead of written.

**2. The clock's path was read off the bits**, hop by hop, and it is the one
`ClockNetwork` describes at the positions `globals.json` gives:

| Hop | Arc the bits select |
|---|---|
| pad to the fabric | general interconnect from `JPADDIB_PIO` on ball A8 to a `JD7` |
| into a buffer | `G_TDCC0CLKI <- G_JTRQPCLKCIB1`, in `TMID_0` at (31, 0) |
| the buffer | **no bits at all**; see "what `ecppack` writes for a clock" |
| the centre mux | `G_URPCLK0 <- G_VPFS0000`, in `CMUX_UR_0` at (32, 13) |
| the spines | `G_VPTX0000 <- G_HPRX0000` at (41, 13) and (59, 13) |
| the taps | `R_HPBX0000 <- G_VPTX0000` at (42, 5), and `L_`/`R_` at column 60 for every row with a sink |
| into each logic tile | `CLK0 <- G_HPBX0000` at (50, 5), (51, 4), (51, 5), (52, 5) and (56, 3) |
| into each slice | `MUXCLK<c> <- CLK0` |

`tests/fpga_trellis.rs::the_clocked_design_routes_and_configures_what_its_header_promises`
asserts that walk from the bits, and that **every one of the 26 flip-flops'
clocks came off a branch wire** rather than off an ordinary interconnect
wire that happens to reach a clock mux. The flow refuses to write a
bitstream where one did not, which matters because such a clock routes,
verifies and configures — what it fails to do is control skew, and nothing
structural would notice.

**3. The question was asked the other way round for the clock as well** —
not "what differs" but "what does `ecppack` write, in full" — against
`analyzer.bit`, which is the one of Great Scott Gadgets' three bitstreams
that is clocked. See "Everything Lattice's own packer writes for a clock",
which is where the one surprise is.

**4. The one unsound simplification in this backend is now checked rather
than assumed**, for the arcs a route takes — which is where a clocked design
meets it, since a centre mux encodes its source as a six-bit code. See "A
bit a feature wants clear", which also says which half of the
simplification is still covered only by inspection.

### What this settles, and what it does not

It settles that the chain from Verilog to a *clocked* configuration a real
ECP5 accepts closes, that the clock goes on the global network rather than
through data wires, and that every bit of it is one the database explains
and selects the connection it was meant to. It settles nothing about
whether the LEDs blink.

## A design that routes has been built, and loaded into a part

On 2026-09-26 `testdata/fpga/cynthion/button_led.v` — the Cynthion's USER
button, through about thirty rows of the die and a lookup table, to two of
its LEDs — was compiled by this flow and loaded into the LFE5U-12F of the
Great Scott Gadgets Cynthion r1.4 attached to the machine this was written
on. **The part accepted it and asserted `DONE`** with no fault bit set:
status `0x00200100`, the same as every run before it, which is exactly why
`DONE` is not the claim being made. The board went on enumerating at the
same USB address afterwards.

What the flow reports for it:

```
114 configuration bit(s) set, 7 pad(s) and 1 lookup table(s) configured,
7/120 io, 1/24288 lut
routed 2 of 2 signal(s) with 23 pip(s) over 25 wire(s), and every sink was
walked back to its driver
```

**Whether the LEDs do what the design says was then watched**, and
"Somebody looked, and it does what it says" below is the answer. What they
were asked to press and to see is in `button_led.v`'s header and repeated
next, because the two halves are only worth anything together: the
prediction was written down before the board was asked.

### What a person should look for

Press and hold the button silkscreened **`USER`**. There are three
tactile buttons and the other two end the experiment instead of performing
it: `PROG` reloads the FPGA from flash and `RESET` resets the debug
microcontroller. On the r1.4.0 layout `USER` and `PROG` are on the same
edge of the board and `RESET` is on the opposite one.

Watch the two LEDs at the end of the `FPGA LEDs` row **farthest from that
button**:

| | far end | next to it | the other four |
|---|---|---|---|
| nothing touched | dark | **lit** | dark |
| button held | **lit** | dark | dark |

They swap back when it is let go, they are never both lit and never both
dark, and the other four never light.

**The board does not number its LEDs**, which is worth saying because two
designs in `testdata/fpga/cynthion/` used to claim it does. The r1.4.0
silkscreen has one legend, `FPGA LEDs`, over the row and no digits; the row
is `D2` to `D7` in `cynthion.kicad_pcb` and the schematic wires `led_n[0]`
to `D7`, which is the end away from the button. So "the far end" is
`led_n[0]`, derived from the layout rather than read off the board.

That also makes this design the first that can settle something the earlier
ones could not. `leds.v` lights all six and `leds_alternate.v` lights
alternate ones, and both look the same read from either end; here only two
LEDs move, so which two they are decides the order.

### Somebody looked, and it does what it says

On 2026-09-26 the board's owner reported: one LED lit with nothing
touched, and on holding `USER` **that one goes dark and its neighbour
lights**. They swap, and nothing else moves.

So **a routed design works on this part**. Two signals, twenty-three
programmable connections, a pad in through a lookup table to a pad out,
built by this flow from Verilog and configured over the board's own debug
microcontroller with no vendor tool at any step.

It also settles the numbering, in favour of the schematic: the LED that
lights when the button is held is the one at the end **away** from the
button, which the layout calls `led_n[0]`. Four agreeing files were right
and the reversed reading is ruled out.

Worth recording what this run did *not* need. The two bugs that made
earlier runs look fine and behave wrongly — a bank's `BANK.VCCIO`, and an
input pad's `PULLMODE`, whose database default fights this board's pull-up
and holds a released pin below `VIH` — were both found before the board
was asked, by reading what Lattice's own packer writes *in full* for a
cell rather than diffing against it. Neither would have been caught by any
check on this machine, and `DONE` was high either way.

The observation was deliberately a strong one. One LED lit rather than six
cannot be confused with the previous milestone; the lit one *moves* when a
finger moves, which no configuration that does nothing can do; and it moves
both ways, so a stuck input shows up as one of the two never changing rather
than as nothing happening.

### What was checked instead, since `DONE` is not evidence

Three things, in order of how much they are worth.

**1. The bitstream was read back through Project Trellis' own database,
and it says what the router said.** `TrellisDatabase::decode` matches every
`.mux`, `.config` and `.config_enum` record of every tile against the bits,
and
`tests/fpga_trellis.rs::the_bitstream_decodes_back_to_the_arcs_the_router_chose`
asserts two things about the result.

*Every set bit belongs to something.* All 114, over 31 tiles, with
**nothing left over**. A leftover bit is a bit this crate set for a reason
the database does not know, which is what a wrong tile rule looks like from
the inside.

*The arcs come back the same.* The set of connections the bits select,
resolved back into the same wires and positions the router works in, is
**exactly** the set the router chose. That is the check worth having,
because the bits of one tile are shared: `CIB.JA0MUX` and the `.mux JA0`
that routes into the same wire are literally one mux, and the bits a
feature wants *clear* are dropped rather than written, so a second feature
written into a tile can silently change what a first one selects. Nothing
structural would notice; this does.

What the decoding says, in full: the button as an `INPUT_LVCMOS33` with
`HYSTERESIS=ON` and `PULLMODE=NONE`; the six LED pads as
`OUTPUT_LVCMOS33` with their tristates tied low, four tied high (dark) and
two left for the router; `BANK.VCCIO = 3V3` on `BANKREF1` and `BANKREF3`;
one `SLICED.K0.INIT = 1010101010101010` — `~A`, read bit 0 first — with
`SLICED.B0MUX`, `C0MUX` and `D0MUX` tied to 1 and `A0MUX` left for the
route; and the arcs that make one path from the button to the lookup table
and one from the table to a LED.

A pip with **no** bits cannot be checked this way, because it leaves no
trace: the fixed connections and the bitless mux sources are eight of the
twenty-three pips the design takes. `Routing::verify` is what covers those,
by walking each sink backwards through the pips it was given until it
reaches the driver.

**2. The pads were compared against bitstreams Lattice's own packer wrote
for this very board**, the same way the LEDs were for the previous
milestone, and the comparison is what made the right edge describable at
all. See "Where a pad's bits are".

**3. The question was asked the other way round as well** — not "what
differs" but "what does `ecppack` write, in full, for this kind of thing"
— for the pad, for the lookup table and for an arc. See "Everything
Lattice's own packer writes".

### What this settles, and what it does not

It settles that the chain from Verilog to a *routed* configuration a real
ECP5 accepts closes, that every bit of it is one the database explains, and
— once somebody looked — that the design does what it says. The interesting
part is that the first two of those were true of a bitstream whose LEDs were
dark, which is the whole lesson of the section below.

It settled nothing about **anything clocked** either, and at the time that
was a gap in the backend rather than in the measurement: `globals.json` was
not read, so there was no clock network, no buffer and no path from a pad
to a clock spine. That is what the section at the top of this file changes,
and the network is described under "What the clock network is".

## The first design, and the lesson it cost

On 2026-09-25 a design built by this flow from Verilog was loaded into the
LFE5U-12F of a Great Scott Gadgets Cynthion r1.4 attached to the machine
this was written on, over the board's own Apollo debug microcontroller,
and **the part accepted it and asserted `DONE`** with no fault bit set:
status `0x00200100`, read back from the configuration status register
afterwards. Nothing Lattice had ever been configured by this project
before that.

The design is `testdata/fpga/cynthion/leds.v`: six output pads, each
driven by a constant zero, constrained to the board's six FPGA LEDs. The
LEDs are active low, so what it should produce is **all six lit**.

A second design was loaded the same way, ten minutes earlier:
`testdata/fpga/cynthion/leds_alternate.v`, which drives `6'b101010` and
should light LEDs 0, 2 and 4 and leave 1, 3 and 5 dark. It exists because
six lit LEDs are a weaker observation than three lit and three dark — the
second is not a state anything else on the board produces. It was also
accepted, with `DONE` high and the same status.

### Then somebody looked, and the LEDs were dark

On 2026-09-26 the board's owner reported that LEDs 0 to 5 were **dark**,
with that bitstream in the part and `DONE` high. So the first run above is
exactly the thing this file said it was: a part that accepted a
configuration and said it was running it, which is not the same claim as a
lit LED, and the difference turned out to be real.

**What was missing was the bank's rail.** An IO bank of an ECP5 has one
setting of its own — `BANK.VCCIO`, in a `BANKREF<n>` tile that is nowhere
near the pads it serves — and `configure_io` set every bit of a pad's own
three tiles and not that one. It is not a subtle omission once looked for:
all three of Great Scott Gadgets' own bitstreams for this board set
`BANK.VCCIO` to `3V3` on every bank of the part, and nextpnr writes it for
every bank a design puts an IO in (`init_io_banks` in its `bitstream.cc`).
A bitstream without it says nothing about what voltage the bank runs at.

That bit is now written, for the banks a design uses and no others, and
`tests/fpga_trellis.rs::the_bank_rail_is_the_bit_lattices_own_packer_sets_for_this_board`
checks it against all three reference bitstreams at absolute frame
positions rather than counting it. `leds.bit` went from 60 configuration
bits to 61.

The corrected bitstream was loaded into the same board on 2026-09-26 and
the part accepted it, status `0x00200100` — `DONE`, no fault — the same as
before, because `DONE` was never the thing in question.

### And then they were lit

On 2026-09-26, with the corrected bitstream in the part, the board's owner
reported **all six LEDs on**. So `BANK.VCCIO` was the whole of what was
missing, and the chain from Verilog to a lit LED on a Lattice part closes.

That is the third vendor this project has configured and had watched:
a Xilinx XC7A35T on a Digilent Basys 3, twice, and now an LFE5U-12F on a
Cynthion, in each case with no vendor tool at any step.

It is worth keeping what the sequence cost, because the lesson is cheap to
read and was expensive to learn. `DONE` went high on the *first* run, with
dark LEDs, and it went high on the corrected run too. `DONE` says the
configuration engine accepted a bitstream; it says nothing whatever about
whether the bitstream configures what the design asked for. Every
structural check this file describes passed on the broken bitstream. The
only thing that told the difference was a person looking at the board.

One difference from Lattice's own header was **checked and ruled out**:
`ecppack` wrote control register 0 as `0x40000038` for these three files
and Reticle writes `0x40000000`. The `0x38` is `ecppack --freq 38.8`, the
master SPI clock the part uses when it loads *itself* from flash
(`frequencies` in prjtrellis' `Bitstream.cpp`), and it reaches no IO
buffer. `CTRL0_DEFAULT` is what `ecppack` writes with no options.

Every run in this file is reproducible, and the current milestone is the
bidirectional pad at the top of this file:

```sh
reticle fetch prjtrellis-db
for design in leds leds_alternate button_led clock_blink bidir_loopback; do
    reticle fpga testdata/fpga/cynthion/$design.v \
        --device ecp5-12f-CABGA256 \
        --constraints testdata/fpga/cynthion/$design.rcf \
        --bitstream /tmp/$design.bit
done
reticle program --list                         # find the board's serial
reticle program --device <serial> /tmp/bidir_loopback.bit
```

(`leds_alternate.v` shares `leds.rcf`; the loop is shorthand.)

What that writes is **only the volatile configuration SRAM**. A power
cycle reloads the part from the board's flash, which nothing in this
project touches; `docs/programming.md` and `src/program/lattice.rs`'s
`NOT_SHIFTED` are where that is made checkable rather than promised.

### What that first run settled

That the chain from Verilog to a configuration a real ECP5 accepts closes:
the frontend, synthesis, IO primitive mapping, the placer's pin
constraints, the frame map, the `.bit` container, its 7562 check words, and
the JTAG configuration sequence.

It settled **nothing about routing or about logic**, and at the time that
was a gap in the backend and not in the measurement: `src/fpga/trellis`
declared no interconnect at all, and a design with anything to route was
refused by name before a file was written. That is what the section at the
top of this file changes, and the interconnect is described under "What the
routing graph is".

## What Project Trellis ships, and in what form

This decides everything after it, so it is written down in full.

### It is a separate repository, wired in as a submodule

`YosysHQ/prjtrellis` is the tooling — `ecppack`, `libtrellis`, the
fuzzers and the documentation. The **data** is `YosysHQ/prjtrellis-db`, a
separate repository that `prjtrellis` carries as a git submodule; its
`.gitmodules` is exactly:

```
[submodule "database"]
	path = database
	url = https://github.com/YosysHQ/prjtrellis-db
```

Reticle pins the commit that submodule currently points at,
`015e0330630d7c238c0e4f2cdd9c8157eb78c54a` (2025-09-15), so what is read
is what `ecppack` and nextpnr would read. `src/bin/reticle/datadir.rs`
holds the pin and `src/bin/reticle/prjtrellis-db.manifest` a SHA-256 and
a size for each of the 191 files taken; a file that does not match is
refused and nothing is installed. The licence is CC0.

**The database is not in this repository and must not be.** `reticle
fetch prjtrellis-db` downloads it into `~/.cache/reticle`, or
`RETICLE_TRELLISDB` names an existing checkout.

### The layout, and what is taken

Everything is plain text checked into git — **nothing is compressed**,
which is the one way this differs from Project Apicula's Gowin database
and the reason `src/fpga/trellis` needs no dependency at all where
`src/fpga/apicula` needs `compcol` for xz. The repository is 705 files
and 81 MB across five device families. What an LFE5U-12F needs is 191
files and 5.8 MB:

| Path | Format | What it is | Read by Reticle |
|---|---|---|---|
| `devices.json` | JSON | every part: IDCODE, frames, bits per frame, pad bits, `max_row`/`max_col`, packages | yes |
| `ECP5/<part>/tilegrid.json` | JSON | every tile: its type, its `R<row>C<col>`, and the rectangle of configuration memory it owns | yes |
| `ECP5/<part>/iodb.json` | JSON | each package: ball → `(row, col, pio)`, and beside them a `pio_metadata` array giving each PIO's **IO bank** | yes, both |
| `ECP5/<part>/globals.json` | JSON | the clock quadrants, spines and taps | yes, and it is the only file that says which tile a clock's wires belong to |
| `ECP5/tiledata/<type>/bits.db` | line-oriented text | one tile type's configuration bits: `.mux`, `.config`, `.config_enum`, `.fixed_conn` | yes, all 185 |
| `ECP5/timing/speed_<n>/*.json` | JSON | cell and interconnect delays | **no** — nothing here does timing |

The `bits.db` files are **shared by the whole family**, one per tile type,
not one per part — which is why the pinned subset has all 185 of them and
only one part's three per-part files.

### `bits.db`, which is the interesting one

Four record kinds, and `libtrellis`' own loader
(`libtrellis/src/BitDatabase.cpp`) accepts exactly these four and throws
on anything else:

```
.mux <sink>                 a programmable connection; then one line per
<source> F1B2 F6B3          source with the bits that select it
<other> -                   (a source with **no** bits is the state the mux
                            is in with none of them set, and that is still
                            a connection — see "A `.mux` source with no
                            bits is a connection" below, which is where
                            this was got wrong)

.config <name> <default>    a multi-bit field, MSB first in the default
!F25B10                     string; then one bit group per bit, **bit 0
!F24B10                     first**, so the first line is the *last*
                            character of the default

.config_enum <name> [dflt]  a field with named settings
0 F25B10
1 F2B3 F4B2
JA0 -                       `-` is the empty group: this value is
                            "none of the other patterns"

.fixed_conn <sink> <source> a connection that is simply there
```

A bit is `F<frame>B<bit>` **within its own tile**, and a leading `!` means
the feature wants it **clear**. There is nothing to *write* for such a bit,
because a bitstream here is assembled by setting bits in a zeroed bitmap —
so the pip does not carry it, and the only thing honouring it can mean is
noticing when something else has set it. "A bit a feature wants clear"
below is where that is done.

Two names in `tilegrid.json` are the other way round from what they look
like, and getting them backwards would move every bit of the part:
**`cols` counts frames and `rows` counts bits within a frame.**
`libtrellis`' `Database.cpp` reads them as `num_frames` and
`bits_per_frame`, and `src/fpga/trellis/parse.rs` follows it.

### The bitstream container

Documented, unusually well, in `docs/architecture/bitstream_format.rst`
in `prjtrellis` itself, and implemented in `libtrellis/src/Bitstream.cpp`.
`src/fpga/ecp5.rs` is Reticle's reader and writer and its header has the
layout; the parts worth naming here are the ones that are not guessable:

- the check word is **CRC-16/BUYPASS** — width 16, polynomial `0x8005`,
  initial value zero, no reflection, no final exclusive-or — written high
  byte first, and a `0xFF` dummy command does not enter it;
- for an ECP5 there is a fresh check word **after every frame**, which is
  what the `0x91` operand byte of the payload command means: check words
  on, per frame rather than at the end, one dummy byte after each;
- **frames are written from the last to the first**;
- **a frame's bits are packed from its last byte upwards**, so a frame is
  a big-endian integer whose least significant bit is bit 0. Getting this
  backwards mirrors every frame and no structural check would notice.

Reticle's reader and writer are checked against three bitstreams Great
Scott Gadgets built for this very board with `ecppack`, and each comes
back **byte for byte identical**:

| File | Bytes | Set bits | Block RAM blocks |
|---|---|---|---|
| `analyzer.bit` | 238 282 | 250 001 | 9 |
| `selftest.bit` | 112 303 | 27 006 | 0 |
| `facedancer.bit` | 399 402 | 424 160 | 44 |

They are not in this repository — they are 750 kB of somebody else's build
output, and what they are for here is a cross-check, not an input. They
ship in the `cynthion` Python package under
`cynthion/assets/CynthionPlatformRev1D4/`; point `RETICLE_ECP5_REF` at a
directory holding them and `tests/fpga_trellis.rs` reads them, or skips
with that sentence if it cannot.

## What the routing graph is, and the three rules that decide it

Project Trellis hands over `.mux` and `.fixed_conn` records per tile
*type*. Turning them into a graph the router can use takes three rules the
files imply and do not state, and each of them is the kind of rule that
produces a bitstream which loads, asserts `DONE` and routes a net through
metal that is not there.

### A wire belongs to the position a prefix points at

`S1E1_JA0` in a `PIOT0`'s `bits.db` is not a wire of that tile. It is the
`JA0` of the tile one row **south** and one column **east**; `N` decreases
the row, `S` increases it, `W` decreases the column and `E` increases it.
`libtrellis`' `RoutingGraph::globalise_net_ecp5` is the rule and
`parse::globalise_ref` follows it, together with the two other cases it
has: a `25K_`/`45K_`/`85K_` prefix means a wire only some dice have, and a
`G_` name is one node for the whole part except for the clock spines and
branches, which are per tile after all.

Two consequences.

**There are no join pips.** `super::xray` spends a third of its graph
edges declaring that one tile's wire is the same metal as its neighbour's,
because a 7-series wire has a different name in every tile it crosses.
Here the mapping is arithmetic, so a wire is declared once by the position
that owns it and every reference resolves straight to it. Every wire has
span `0 0`.

**The loader cannot decide a tile type's wire list by reading that type.**
A `CIB+PICT1` has to declare `JA0` because a `PIOT0` one row north and one
column west says `S1E1_JA0`, and nothing in the `CIB`'s own `bits.db` need
mention the name at all. So the loader walks every reference of every tile
of the die first — about a million and a half of them — and gives each name
to the *composition* the reference lands on. Doing it the other way round
leaves thousands of references resolving to nothing, and a reference that
resolves to nothing is a pip that silently vanishes. 3840 references do
point off the grid, which is what happens at the four edges of the die and
is not an error.

### A `.mux` source with no bits is a connection

This one was got wrong, and it is worth writing down because the mistake
was reasonable and the consequence was total.

```text
.mux F0
F0_SLICE -
F5A_SLICE F8B10
```

`F0` is a routing wire of a logic tile and `F0_SLICE` is a lookup table's
output. The first source has no bits, which reads like "the state of the
mux when nothing drives it" — and that reading is what the loader carried
at first, on the strength of the four `…BOUNCE` sources, which really are
that. It is wrong. With `F8B10` clear the mux carries the lookup table;
`F8B10` set switches it to the carry chain's cascade output. So dropping
the bitless source disconnects **every lookup table in the part** from the
fabric, and a bitless source becomes a pip with an empty bit list, which is
what Reticle's `Arch` already means by a connection that is always there
and what `ecppack` writes for one (`ChipConfig::add_arc` of a bitless arc
sets no bits).

It was found before a line of Rust was written, by walking the graph in a
throwaway script from the button's pad forward and from a LED's pad
backward and intersecting: the backward walk reached `F0` and never
`F0_SLICE`, and no lookup table on the die was in both sets. The
`…BOUNCE` sources stay harmless under the corrected rule because no `.mux`
has a `…BOUNCE` as its *sink*, so nothing drives one and a router can never
route through one.

### A bit a feature wants **clear**, and the only thing that can be done about it

This was the one unsound simplification in this backend, and it is worth
being exact about what was wrong with it, because the fix is not the obvious
one.

A bitstream is assembled by setting bits in a zeroed bitmap, so `!F25B10`
is a bit already in the state the feature wants and there is nothing to
write. The loader therefore does not put it on the pip — a `PipDecl` says
which bits switch a connection *on* and has no room for the ones it needs
off. That is sound only while no two features written into one tile
disagree about a bit, and a `.mux` source with an inverted bit is exactly a
feature that could disagree: taking such an arc leaves five bits that
another source of the same mux wanted clear, and a second feature setting
any of them silently turns the arc into a different one. Nothing structural
would notice.

**There is no way to "honour" such a bit by writing it.** A clear cannot be
written into a map that is already clear. The only meaningful action is to
*notice* when something else has set it — so the fix is a check, and it is
`TrellisFabric::dropped_clear_bits`: the inverted bits are recorded at load
time (4425 of this die's mux sources have some), and every pip a route takes
is asked, against the finished image, whether the bits its source wants
clear are clear. `reticle fpga --bitstream` refuses to write a bitstream
where one is not.

Two things about that check are worth stating because they bound it.

**It is looked up by bits, not by name.** A pip in the graph carries
resolved wire names and the database carries prefixed ones, so the key is
(tile type, the bits the arc sets). Six of this die's mux sources share
their set bits with another source of the same tile type that wants
*different* bits clear; those are indistinguishable in a finished bitstream
whatever is recorded, so only what the two agree about is checked. The
number is asserted in
`tests/fpga_trellis.rs::the_database_describes_one_part_of_the_ecp5_family`
rather than hidden.

**It is shown failing.**
`a_bit_an_arc_needs_clear_is_noticed_when_something_else_sets_it` takes the
clocked design's own bitstream, sets one bit that one of its arcs needs
clear, and asserts the check says which bit in which tile. A green run of
designs that do not collide would prove nothing about whether anything is
looking.

Where the inverted bits are is still worth knowing, and the answer is
clean: **every `.mux` source with an inverted bit is in the clock network's
own tiles.** Over the family's 185 `bits.db` files, 4523 of 85 379 mux
source lines have one, and all 4523 are in `CMUX_LL_0`, `CMUX_LR_0`,
`CMUX_UL_0`, `CMUX_UR_0`, `LMID_0`, `RMID_0`, `ECLK_L`, `ECLK_R`,
`BMID_0H`, `BMID_0V`, `BMID_2`, `BMID_2V`, `TMID_0` or `TMID_1`, and every
one of them drives a clock global (`G_…PCLK…`, `G_…DCC…CLKI`, `ECLKI…`).
The general interconnect — the `CIB*` tiles, the `PLC2`s and the `PIC*`s
that a pad or a lookup table routes through — has none, so a combinational
design cannot reach one.
`tests/fpga_trellis.rs::an_inverted_mux_bit_only_happens_in_the_clock_network`
asserts that from the database rather than from this paragraph. **A clocked
design walks through six of them**, which is why this stopped being
theoretical at the same moment the clock network arrived.

**The same simplification applies to a `.config_enum`'s values, and that
half is not covered by an automatic check.** The flip-flop is where it
shows up: `CLK0.CLKMUX = CLK`, `LSR0.LSRMUX = LSR`, `SLICEA.GSR = ENABLED`
and `SLICEA.REG0.LSRMODE = LSR` are each "one bit, wanted clear".
`dropped_clear_bits` walks pips and does not see them, and neither does
`Decoded::unexplained`, because a stolen bit there is a bit some *other*
value of the same field wants — set `F54B10` and `CLK0.CLKMUX` reads `INV`
rather than leaving a bit unaccounted for. What covers it instead is
weaker and worth naming as weaker: those fields have exactly one writer
(`configure_registers`), nothing else in this backend touches a `PLC2`'s
control muxes, and `a_flip_flops_settings_are_the_ones_lattices_own_packer_writes`
asserts that the decoding of the clocked design reads back the values it
meant. A check as exact as the one for arcs would need the passes to report
what they wrote so the decoding could be compared with it; that has not been
built.

### What it comes to

| | |
|---|---|
| Global wires (one node for the die) | 467 |
| Tile wires | 1 095 958 |
| Graph nodes | 1 096 425 |
| `.mux` sources declared, over 129 tile types | 171 632 |
| `.fixed_conn`s declared | 14 614 |
| Clock network joins the database does not state | 58 928 |
| Graph edges kept | 8 270 828 |
| Graph edges dropped at the edges of the die | 53 632 |
| Distinct bit patterns, interned | 3536 |
| `RoutingGraph::heap_bytes` | 354 MiB |
| Time to build, release | under a second |
| `lut` sites | 24 288 |
| `ff` sites | 24 288 |
| `gb` sites | 56 |
| `io` sites | 120 |

Those are `tests/fpga_trellis.rs::the_database_describes_one_part_of_the_ecp5_family`'s
assertions, so the table cannot drift from the database.

**The whole die fits, and that is the surprise.** `super::xray` needs a
region option and a pip limit because an `xc7a50t` is 30.9 million edges
and 1386 MiB; this is a quarter of the edges and a quarter of the memory,
so there is no region option here and `reticle fpga --bitstream` loads
everything. The reason is the rule above: no join pips, and a wire is one
node however far it reaches.

## What the clock network is, and why it needed a file of its own

Everything else in this backend comes out of `bits.db`, because a wire's
name carries the position it belongs to: `S1E1_JA0` is the `JA0` of the tile
one row south and one column east, and `parse::globalise_ref` resolves it.
**The clock network breaks that rule in one specific way: its wires carry
the same name in every tile they cross, with no prefix.** A logic tile's
branch wire is `G_HPBX0000` and so is its neighbour's, and the tap driver
twenty columns away spells its output `R_HPBX0000`. Nothing in the file says
the three are one piece of metal.

So three hops of a clock's path are connections the database states
nowhere, and `globals.json` is the only thing that says where they go. For
this die it says:

| | |
|---|---|
| Quadrants | `UL` (0,0)–(31,25), `UR` (32,0)–(72,25), `LL` (0,26)–(31,50), `LR` (32,26)–(72,50) |
| Tap columns | 4 (left 0–3, right 4–12), 22 (13–21, 22–31), 42 (32–41, 42–50), 60 (51–59, 60–72) |
| Spines | one per (quadrant, tap): `UL4` at (3,13), `UL22` at (21,13), `UR42` at (41,13), `UR60` at (59,13), and the same four columns at row 37 for `LL` and `LR` |

and the three joins are:

1. the quadrant's primary clock onto the **spine**'s feed wire —
   `G_HPRX<n>00` at the one position `spines` names;
2. the spine's vertical wire as the **tap column** spells it —
   `G_VPTX<n>00` at `(tap, y)` for every row of the quadrant;
3. each tap's two branch drivers onto the **tiles they reach** —
   `L_HPBX<n>00` for the columns left of the tap and `R_HPBX<n>00` for
   those right of it.

58 928 bitless pips in all, which is 0.7% of the graph. They carry **no
bits**: every bit of a clock route is still a `.mux` record charged to the
tile that owns it, so declaring a join is not a claim about the bitstream.

**That is the same walk nextpnr does**, and the difference is where it
happens. `Ecp5GlobalRouter::find_tap_pip` looks up `L_`/`R_HPBX<n>00` at the
tap column of the sink's own row and `find_spine_pip` looks up
`G_VPTX<n>00` at the spine position, both out of band, in a dedicated pass,
because its chipdb has no edge joining them either. Here they are pips of
the graph, so **the ordinary router routes a clock and `Routing::verify`
walks it back**. This is the interesting half of the result: the shape of
`super::xray`'s `enable_global_clocks` — a pass that sets bits belonging to
a whole column — turned out not to be needed, because on this family there
are no such bits. What was needed was three missing edges.

A fourth join would have been the **buffer**, and it is a bel instead. Every
global on this family comes out of a `DCC`, the device file has
`bel DCCA gb port i=CLKI o=CLKO en=CE`, so the flow inserts a cell and the
placer puts it on one of the die's 56. An ungated one costs **no bits at
all**; see the next section.

### Two clocks cannot collide on it, and that had to be checked

`G_HPBX0300` is a separate graph node in every tile although it is one piece
of metal per quadrant and side, so the router's one-signal-per-node rule
does not by itself stop two nets sharing a branch. It does not have to:
every path onto a branch of network *n* goes through its quadrant's single
`G_<quadrant>PCLK<n>` **global** node — one node for the die — and that node
has capacity one. The per-tile naming is therefore conservative rather than
unsound: two nets can use network 3 in two different quadrants, which is
what the hardware allows, and cannot share one quadrant's, which is what it
forbids.

### The router had to be steered, and that is a new knob

A flip-flop's clock mux (`.mux CLK0` of a `PLC2`) offers the sixteen global
branch wires **and** seven ordinary interconnect wires. So the shortest path
from a pad to a clock pin is through general routing — measured on this die,
**seven hops** from `JPADDIB_PIO` on ball A8 to a `CLK0_SLICE`, against
about eighteen through the network — and a router with no preference builds
a clock tree out of data wires. It routes, it verifies, it configures; what
it does not do is control skew.

`RouteOptions::node_base` is the fix and it is the `base(n)` that `route.rs`'
own cost formula always had and never used. `TrellisFabric::clock_node_costs`
gives every network node a base of 0.05, and:

- **it is a preference and not a permission.** Capacity is still one signal
  per node. And a cheap wire a signal has no reason to enter is not entered:
  the network is a one-way funnel whose only exits are flip-flop control
  pins, so the only signal that can traverse it is one that clocks or resets
  something.
- **the distance heuristic had to be scaled by it too.** This is the part
  that was got wrong first. With the base alone, 7 of the clocked design's
  26 flip-flops still came off a data wire, and the reason was A\*: charging
  a full 0.3 per tile of remaining distance for standing on a wire that
  costs 0.05 is twenty times too much, so the search walked past the network
  and found the sink through general routing first. `maze`'s heuristic now
  multiplies the per-tile charge by the node's own base. With a uniform base
  it is exactly what it always was.
- **and it is checked rather than trusted.** `clock_network_use` walks each
  flip-flop's clock pin back through its own path and asks what drives the
  `CLK0` or `CLK1` the pin's `MUXCLK` selected. It has to be per pin: a tile
  shares two clock muxes between four slices, so one flop can be on the
  network while its neighbour is not, which is precisely what happened
  before the heuristic was fixed. A design where one is not is **refused**.

### Everything Lattice's own packer writes for a clock

The question that found `BANK.VCCIO` and `PULLMODE` was not "what differs
from the reference" but "what does `ecppack` write, in full, for one of
these, and do we write all of it?" — because a diff is silent about things
never emitted at all, and both of those misses were of that kind. Asked
again for a clock, against `analyzer.bit`, which is the one of Great Scott
Gadgets' three bitstreams that is clocked and uses two globals:

| Setting | Written here |
|---|---|
| the buffer's input mux, `G_<x>DCC<n>CLKI <- …` | yes, it is a pip |
| the centre mux, `G_<quadrant>PCLK<g> <- …` | yes, it is a pip — but see below |
| the spine, `G_VPTX<g>00 <- G_HPRX<g>00` | yes, it is a pip |
| the tap, `L_`/`R_HPBX<g>00 <- G_VPTX<g>00` | yes, one per row with a sink |
| the tile, `CLK<c> <- G_HPBX<g>00` and `MUXCLK<c> <- CLK<c>` | yes |
| `DCC_<x><n>.MODE` | **nothing, and neither does `ecppack`** |

**The last row is the finding, and a diff could not have produced it.**
There is no `DCC_*.MODE` anywhere in `analyzer.bit`, although two of its
buffers are carrying a global — `G_LDCC0CLKI <- G_JLLCPLL0CLKOS2` and
`G_LDCC6CLKI <- G_JLLCPLL0CLKOS` are in the file. nextpnr's `write_dcc`
explains it: it writes `DCC_<x><n>.MODE = DCCA` **only when the cell has a
clock enable**, `NONE` is the field's default and costs no bits, and an
ungated buffer is therefore a wire. So a `gb` bel here has no
`ConfigEntry` at all, which is not a gap but the whole answer. Had this
been asked as a diff, "we set no bit and neither did they" would have
looked like agreement about nothing.

One row is **deliberately narrower** than nextpnr. `route_onto_global` loops
over all four quadrants whether the design needs them or not, and
`analyzer.bit` has all four centre muxes set for both its globals. This
flow routes the quadrant its flip-flops are in and no others, because the
router routes to sinks and there are none elsewhere. That is one arc instead
of four; the missing three drive branch wires nothing is attached to.

`tests/fpga_trellis.rs::what_lattices_own_packer_writes_for_a_clock`
asserts all of it against the reference file, including that every one of
its branch drivers is in a column `globals.json` calls a tap and every spine
arc is at a position it names. That is the only independent evidence there
is for the geometry tables, since `globals.json` is the one file of the
database whose contents nothing else can be compared with.

### What a flip-flop costs

nextpnr's `write_ff` is the whole of it, and
`TrellisFabric::configure_registers` is a line-for-line answer:

| Setting | Bits on this die | Where the value comes from |
|---|---|---|
| `SLICE<l>.REG<n>.SD = 0` | one | always: the data comes from the fabric's `M` wire, because this flow never packs a lookup table and a flop together |
| `SLICE<l>.REG<n>.REGSET` | one for `RESET` | the cell's `REGSET`; `SET` is the field's default |
| `SLICE<l>.CEMUX` | two for `1` | the cell's `CEMUX` — **and this is the one that matters**, see below |
| `SLICE<l>.GSR` | one for `DISABLED` | the cell's `GSR`; the device file asks for `DISABLED` so a design that did not ask for a global reset does not get one |
| `SLICE<l>.REG<n>.LSRMODE = LSR` | none, it is the default | |
| `CLK<c>.CLKMUX`, `LSR<c>.LSRMUX`, `LSR<c>.SRMODE` | none for the defaults | the cell's, and **only for the mux the route actually took** |

**`CEMUX` is the third `BANK.VCCIO`.** The field's default is `CE` — take
the clock enable from the fabric — so a bitstream that leaves it alone has
every flip-flop gated by a wire nothing drives, and the design is frozen.
Same shape as the bank rail and the pull mode: a database default that is
wrong for the design, in a field the design never mentions, with no symptom
a structural check could see. It is written for every flop, and
`a_flip_flops_settings_are_the_ones_lattices_own_packer_writes` asserts it
is in the decoding.

The last row is why `configure_registers` takes the routing. A tile has two
clock muxes and two reset muxes shared between its four slices, so "which
one is this flop's" is a fact about the route and not about the bel —
nextpnr asks it the same way, by looking at which of `CLK0` and `CLK1`
carries the net. A flop whose parameter needs bits in a mux the routing does
not identify is **refused** rather than written into the wrong one, which
would change every other flop of the tile.

## The part, measured

| | |
|---|---|
| Part | LFE5U-12F, IDCODE `0x21111043` |
| Package | caBGA-256 (`CABGA256` in `iodb.json`) |
| Tile grid | 73 columns × 51 rows, every position occupied |
| Tiles | 4312, of 134 distinct Lattice types |
| Positions holding more than one tile | 480 |
| Most tiles at one position | 6 |
| `Arch` tile types | 129, one per composition of a position |
| Configuration memory | 7562 frames × 592 bits = **4 476 704 bits** |
| Bytes per frame | 74 |
| Balls in the caBGA-256 map | 197 |
| Pads this backend declares | 120: 56 on the top edge and 64 on the right |
| Balls it leaves out | 77, on the left and bottom edges |
| Graph nodes | 1 096 425 |
| Graph edges | 8 270 828 |
| Global clock networks | 16 |
| Clock buffers (`DCC`) | 56: twelve at the top, fourteen on each of the left and right, sixteen at the bottom |
| `lut` sites | 24 288 |
| `ff` sites | 24 288 |

`tests/fpga_trellis.rs::the_database_describes_one_part_of_the_ecp5_family`
asserts every one of those, so the table cannot drift from the database.

**The LFE5U-12F and the LFE5U-25F are the same die.** Their three
per-part files in `prjtrellis-db` are byte-identical, their `devices.json`
entries differ in the `idcode` field and in nothing else, nextpnr loads
one `chipdb-25k.bin` for both, and Lattice's own TN-02039 Table B.4 gives
both 7562 frames of 592 bits. Only the identifier tells them apart, which
is why `TrellisFabric::check_idcode` is not a nicety: a bitstream from the
wrong half of the database would configure the part and assert `DONE`.

## Where a pad's bits are, and how that was established

This is the part of an ECP5 that no amount of reading a database settles,
and it is where a wrong answer is least visible: a bitstream with a pad
configured one column over still loads and still asserts `DONE`.

### The buffer is not where the bits are, and both edges say so differently

Before anything routed, "a pad" meant a set of bits, and the bel was put
wherever they were. A routed design makes the distinction unavoidable,
because a bel's pins resolve in the tile the bel sits in and every
top-edge position has a `PADDOB_PIO` of its own. So:

- **the buffer** — the three wires a PIO presents to the fabric,
  `PADDO<L>_PIO`, `PADDT<L>_PIO` and `JPADDI<L>_PIO` — is at the position
  `iodb.json` gives the ball, and that is where the `io` bel goes;
- **the bits** are wherever the edge's rule puts them, which on the top
  edge is a column east for side B and on the right edge is a row south for
  every side.

| | Top edge | Right edge |
|---|---|---|
| Sides per position | A, B | A, B, C, D |
| Buffer | the ball's `(col, row)`, `PIOT0` | the ball's `(col, row)`, `PICR0*` |
| Pad tile: standard, hysteresis, pull | `(col + [B], 0)`, `PIOT0` / `PIOT1` | `(col, row + 1)`, `PICR1*` |
| Second copy of the standard | `(col + [B], 1)`, `PICT0` / `PICT1` | A, B: `(col, row)`, `PICR0*`; C, D: `(col, row + 2)`, `PICR2*` |
| `CIB` that ties the data and enable | `(col + 1, 1)`, wire `JA0` / `JB0` | `(col - 1, row)` for A, B and `(col - 1, row + 2)` for C, D, wire `JA0`/`JA3` |
| Bank rail | `BANKREF<bank>`, anywhere | same |

The `CIB` row of that table is **not hardcoded**. Nothing in the loader
knows that `JA0` is the top edge's answer: it reads the buffer's own
`.fixed_conn` — `JPADDOB <- S1E1_JA0` on the top edge, `JPADDOD <-
S2W1_JA3` on the right — resolves the direction prefix, and builds the
field name `CIB.<wire>MUX`. That is what nextpnr does too, by walking the
pips uphill of the wire, and it is what lets one piece of code serve both
edges. Nor are the tile *types* hardcoded: the loader asks which tile of a
position declares `PIO<L>.BASE_TYPE`, which gets the right answer through
all twelve spellings of the right edge (`PICR1`, `PICR1_DQS0`,
`PICR1_DQS3`, `PICR2`, `PICR2_DQS1`, `MIB_CIB_LR_A`, …) where nextpnr
carries a set of names per edge, and **refuses** rather than guessing if
two tiles of one position both declare it.

The left and bottom edges are still left out, and their balls are left out
of the ball map rather than placed and configured wrongly. The left edge is
the right edge mirrored and would probably work; "probably" is why it is
not there.

### The top edge, checked against this board's own gateware

All six of this board's FPGA LEDs are outputs in its own analyzer
gateware, so every bit Reticle sets to make one of those balls an output
can be compared, at absolute frame positions, with the bits `ecppack` set
for the same ball. All 48 agree:

| LED | Ball | Side | Buffer | Bits at | Pad tile bits | Second-copy bits |
|---|---|---|---|---|---|---|
| 0 | E13 | B | col 62 | col 63 | F6610B0 F6615B0 F6617B0 F6623B0 F6624B0 F6625B0 | F6662B12 F6663B12 |
| 1 | C13 | B | col 60 | col 61 | F6398B0 F6403B0 F6405B0 F6411B0 F6412B0 F6413B0 | F6450B12 F6451B12 |
| 2 | B14 | A | col 67 | col 67 | F7034B0 F7039B0 F7041B0 F7047B0 F7048B0 F7049B0 | F7086B12 F7087B12 |
| 3 | A15 | B | col 67 | col 68 | F7140B0 F7145B0 F7147B0 F7153B0 F7154B0 F7155B0 | F7192B12 F7193B12 |
| 4 | D12 | A | col 58 | col 58 | F6080B0 F6085B0 F6087B0 F6093B0 F6094B0 F6095B0 | F6132B12 F6133B12 |
| 5 | C11 | B | col 49 | col 50 | F5232B0 F5237B0 F5239B0 F5245B0 F5246B0 F5247B0 | F5284B12 F5285B12 |

That is
`tests/fpga_trellis.rs::the_led_pads_are_where_this_boards_own_gateware_has_them`,
and it is a green test rather than a paragraph.

The **ties** are the one thing that comparison cannot check, because the
gateware routes a signal into the data wire where this backend ties a
constant, so both of its constant-mux bits are clear there. The test
asserts that too, so the comparison stays honest about what it covers.

That comparison also cannot tell a tie from a route, and the reason is
worth knowing: **`CIB.JA0MUX` and the `.mux JA0` that routes into the same
wire are one mux.** `CIB.JA0MUX` offers `0`, `1` and `JA0`, where `JA0`
means "whatever the routing mux selects", so the bits that tie a wire and
the bits that route into it live in the same bit space. A finished
bitstream therefore cannot be asked "is this pad tied?" — the tie's pattern
may be a subset of the route's. What can be asked is what the pass wrote,
and `tests/fpga_trellis.rs` runs `configure_io` into an empty bitmap of its
own to ask exactly that.

### The right edge, checked against the one bitstream that reads the button

Of Great Scott Gadgets' three bitstreams for this board, exactly one has a
design that reads the USER button: `facedancer.bit`, whose top level asks
for `button_user` through its `ButtonProvider`. So it is the one file that
says where an input on the right edge is configured, and it agrees with
this backend bit for bit.

M14 is `(row 32, col 72, PIO D)` in `iodb.json`, bank 3. What Reticle
writes for it, and what `facedancer.bit` has at the same absolute frames:

| Setting | Tile | Position | Bits |
|---|---|---|---|
| `PIOD.BASE_TYPE = INPUT_LVCMOS33` | `PICR1_DQS3` | (72, 33) | F29B394 F30B394 F30B395 F31B394 F34B395 |
| `PIOD.HYSTERESIS = ON` | `PICR1_DQS3` | (72, 33) | F30B395, which the base type also wants |
| `PIOD.PULLMODE = NONE` | `PICR1_DQS3` | (72, 33) | F26B394, with F35B395 clear |
| `PIOD.BASE_TYPE = INPUT_LVCMOS33` | `PICR2` | (72, 34) | **none at all**: the pattern is empty |
| `BANK.VCCIO = 3V3` | `BANKREF3` | (71, 50) | F7474B591, with three others clear |

`tests/fpga_trellis.rs::the_user_buttons_pad_is_where_this_boards_own_gateware_has_it`
asserts all of it, and asserts that `facedancer.bit` ties none of M14's
`CIB` wires, because an input should not.

Two of those rows are things the six-LED milestone never needed.

**`PIOD.BASE_TYPE` in `PICR2` costs nothing**, which is lucky rather than
designed: Lattice's own packer writes the field there and Project Trellis
found no bits for it in that tile. So an input on side C or D of the right
edge is five bits and a bank rail.

**`PULLMODE` is not cosmetic, and this is the second `BANK.VCCIO`.** The
field's default in `bits.db` is `DOWN`: with none of its bits set the pin
has an internal pull-down, which is Lattice's own default for a PIO. On
this board the button is a 10 kΩ pull-up to 3.3 V, a normally-open switch
to ground, and a 33 kΩ series resistor with a 1 µF capacitor into the ball
(R106, SW3, R107, C78 of `indicators_buttons.kicad_sch`). An internal
pull-down of the order the ECP5 datasheet gives would divide the released
level to something like 1.4 V across that 43 kΩ, which is below `VIH` for
LVCMOS33 — so the pin would read **low whether or not anybody pressed
anything**, and the design would look dead in one direction and stuck in
the other. Great Scott Gadgets' own platform file asks for
`PULLMODE="NONE"` on that pin for the same reason, `facedancer.bit` has it,
and `configure_io` writes it for every input.

That is the same shape of omission as the bank rail: a default that is
wrong for the board, in a field the design never mentions, with no symptom
a structural check could see.

### The bank's rail, which is nowhere near the pad

This is the bit that was missing when the LEDs were dark, so it is worth
being exact about where it is and how its place was established.

`BANK.VCCIO` is a `.config_enum` of the `BANKREF<n>` tile types, and there
are seven such tiles on this die: one for each of banks 0, 1, 2, 3, 6 and
7, and `BANKREF8`, which is also where the part's sysconfig settings live.
`BANKREF1` owns all six LEDs and is the single tile at grid **(69, 0)**;
bank 1's top-edge pads have their tiles at columns 33 to 68, so it is
between one and thirty-six columns east of the pad it is configuring. Its `3V3` setting is
one bit, `F18B0` within the tile.

Three things had to be right and each has a source:

1. **Which bank a pad is in.** `iodb.json` has a `pio_metadata` array
   beside `packages`, one entry per `(row, col, pio)` of the die with its
   `bank`. All six LED balls come back bank 1, which is what
   `testdata/fpga/cynthion/leds.rcf` says and what the board's
   `bank0_1.kicad_sch` implies. `parse::pio_banks` reads it; nothing
   infers a bank from a column, and a pad whose bank the database does not
   state is left out of the ball map rather than placed and configured
   incompletely.
2. **Which value.** `bank_voltage` is nextpnr's `get_vccio` table
   (`ecp5/pio.cc`), so `LVCMOS33` is `3V3`. The one oddity is Lattice's:
   a 1.35 V bank is programmed as `1V2`.
3. **That those really are the bits.** Checked the same way the pad tiles
   were: against Lattice's own packer. All three of Great Scott Gadgets'
   bitstreams set exactly that absolute frame position, and the test
   asserts it for each of them. It also asserts that bank 0 — the other
   half of the top edge, with no pad in this design — is **not** written,
   because writing a bank a design does not use is not what `ecppack`
   does either.

### Everything Lattice's own packer writes, asked cell by cell

Finding `BANK.VCCIO` by comparing against three bitstreams is how that gap
was found; it is not a reason to think there is not another. So the
question is asked the other way round as well, for every kind of thing this
backend now writes: **what does nextpnr write for one of these, in full?**
`write_io`, `write_comb`, `tie_cib_signal`, `set_pip` and `init_io_banks` in
its `ecp5/bitstream.cc` are the whole answer.

**An output pad whose data is a constant** — six settings:

| Setting | Tile | Written here |
|---|---|---|
| `PIO<s>.BASE_TYPE = OUTPUT_LVCMOS33` | the pad tile | yes, `output_pad_bits` |
| `PIO<s>.BASE_TYPE = OUTPUT_LVCMOS33` again | the second-copy tile | yes, `output_pic_bits` |
| `CIB.JB0MUX = 0` — the tristate tied low | the `CIB` | yes, `enable_bits` |
| `CIB.JA0MUX = 0` or `1` — the constant | the `CIB` | yes, `low_bits` / `high_bits` |
| `BANK.VCCIO = 3V3` | `BANKREF<bank>` | yes, since 2026-09-26 |
| `PIO<s>.PULLMODE`, `HYSTERESIS`, `SLEWRATE`, `DRIVE`, `OPENDRAIN`, `CLAMP`, `TERMINATION`, `DATAMUX_*`, `TRIMUX_TSREG` | various | **no, and neither does nextpnr**: each is behind an attribute or a direction this design does not have, or defaults to the value it would be given |

**An output pad a route drives** is the same list with the constant left
out, which is the point: `configure_io` ties `CIB.JA0MUX` only when the
netlist gives the pin a constant, because the tie and the route share one
mux and writing both would be writing two drivers.

**An input pad** — `write_io` again, with `dir == "INPUT"`:

| Setting | Written here |
|---|---|
| `PIO<s>.BASE_TYPE = INPUT_LVCMOS33`, in both tiles | yes |
| `PIO<s>.HYSTERESIS = ON` — the attribute's own default | yes |
| `PIO<s>.PULLMODE` — only when an attribute asks | **yes, always**, and the paragraph above says why this is the one place this is deliberately broader than nextpnr |
| the tristate tie | **no**, and nextpnr skips it for an input too |
| `BANK.VCCIO` | yes |

**A combinational lookup table** — `write_comb`:

| Setting | Written here |
|---|---|
| `SLICE<l>.K<n>.INIT` — the truth table | yes, `configure_logic` |
| `SLICE<l>.<X><n>MUX = 1` for every input no pip reaches | yes |
| `SLICE<l>.MODE = LOGIC` | nothing to write: it is the default and costs no bits |
| `SLICE<l>.CCU2.INJECT1_<n> = _NONE_` | nothing to write: `_NONE_` means "no bits", which is what nextpnr uses it for — the bit is shared with the cascade mux and it deliberately leaves it alone |
| `WREMUX`, `CLK1.CLKMUX` | only for `DPRAM` mode, which this does not build |

**An arc** — `set_pip` is two lines: it looks up the tile and calls
`add_arc(sink, source)`. Nothing per wire, nothing per net, no enables.
That is worth stating flatly because the 7 series is not like that: there,
touching a clock-row wire costs a buffer enable that is neither a pip's nor
a bel's, and `super::xray` carries a whole mechanism for it. Here the mux
bits are the whole of an arc.

So there is nothing left that Lattice's own packer writes for any cell this
builds and this does not. That is not the same as a guarantee — two things
it does not cover are named below — but it means the next doubt is not "a
missing bit for a cell we have".

The two things it does not cover:

- **nextpnr's base configuration.** `ecp5/baseconfigs.cc` starts every
  bitstream from a non-empty `ChipConfig`: for this die, 166 settings in
  twenty-one tiles, all of them `VCIB_DCU*`, `CIB_DCU*`, `CIB_PLL3`,
  `CIB_EFB*`, `CMUX_*`, `EFB0_PICB0` or `DSP_SPINE_UL1`. **None of it is in
  a `PIO`, `PICT` or `BANKREF` tile** — it is the SERDES, the PLLs, the
  embedded function block and the clock multiplexers — and this backend
  writes none of it. The part accepts a bitstream without it and asserts
  `DONE`, so nothing there is required to finish configuration; whether any
  of it matters to a pad is unknown and was not assumed either way. (The
  12F has no entry of its own there, because nextpnr treats it as the 25F
  it shares a die with.)
- **The `.config_enum` and `.mux` bit values themselves**, which come from
  Project Trellis' fuzzing rather than from Lattice. Those are as
  trustworthy as `ecppack` is. The settings that could be checked against a
  real `ecppack` output were: the base type on both edges, hysteresis, the
  pull mode, and the bank rail on four banks. **No arc has been checked
  against an `ecppack` output**, because the reference bitstreams route
  different designs and there is nothing to compare an arc against. What
  was checked instead is that every bit of this flow's own bitstream decodes
  back, through the same database, to exactly the arcs the router chose —
  see the top of this file.

### Where the pin numbers came from

Three independent sources, and they agree:

1. **Great Scott Gadgets' own Amaranth platform file**,
   `cynthion/python/src/gateware/platform/cynthion_r1_4.py` in
   `greatscottgadgets/cynthion`:

   ```python
   # FPGA LEDs
   *LEDResources(pins="E13 C13 B14 A15 D12 C11",
                 attrs=Attrs(IO_TYPE="LVCMOS33"), invert=True),

   # Primary, discrete 60MHz oscillator.
   Resource("clk_60MHz", 0, Pins("A8", dir="i"),
       Clock(60e6), Attrs(IO_TYPE="LVCMOS33")),
   ```

   in that order, so `led[0]` is E13. `invert=True` is the polarity:
   Amaranth's `PinsN` is `Pins(..., invert=True)`, so driving one means
   pulling the pad low. The USER button is in the same file:

   ```python
   # USER button
   Resource("button_user", 0, PinsN("M14", dir="i"),
            Attrs(IO_TYPE="LVCMOS33", PULLMODE="NONE")),
   ```

   `PinsN` again, so the button is active low: the ball reads low while it
   is held.
2. **The r1.4.0 KiCad schematics** in
   `greatscottgadgets/cynthion-hardware`, whose `main` is that tag. In
   `indicators_buttons.kicad_sch` all six anodes sit on one wire up to the
   `+3V3` symbol and each cathode goes through a series resistor to nets
   `LED0`..`LED5` (refdes D7 D6 D5 D4 D3 D2) — **anode to the supply,
   cathode to the FPGA, so active low**, confirming `invert=True`
   independently. `clock_misc.kicad_sch` has the oscillator: Y1, a SiTime
   `SIT1602BC-23-33E-60.000000E`, 3.3 V, 60.000000 MHz, to A8. The same
   sheet has the button: `SW3`, valued `BTN_USER`, a normally-open SPST
   between ground and a node that `R106` (10 kΩ) pulls up to `+3V3`, with
   `R107` (33 kΩ) in series and `C78` (1 µF) to ground between that node and
   the hierarchical label `~{BTN_USER}`. So **released is high**, through
   43 kΩ, and the RC is a 33 ms debounce, which is why nothing in the fabric
   has to debounce it. That pull-up is also why `PULLMODE` has to be
   written; the pad section above has the arithmetic.
3. **Project Trellis' `iodb.json`** for the caBGA-256, which puts all six
   LED balls on row 0 — the top edge — and in bank 1, puts A8 on row 0 bank
   0 with the dedicated clock function `PCLKC0_0`, and puts **M14 on the
   right edge**: row 32, column 72, PIO D, bank 3. This is also where the
   grid columns in the tables above come from.

Bank 1's VCCIO is 3.3 V, hence LVCMOS33: `bank0_1.kicad_sch` has only
`+3V3` and `GND` power symbols. Bank 3's is too, and there the evidence is
the bitstreams rather than the schematic: all three of Great Scott Gadgets'
set `BANK.VCCIO` to `3V3` on every bank of the part, `BANKREF3` included.

**Confidence in M14, stated plainly, because a wrong pad is a design that
does nothing with no error.** High. Three sources agree on the ball
(platform file, schematic, and the reference bitstream that configures it);
the reference bitstream also fixes every bit of its configuration at
absolute frame positions, which is the same standard the LEDs were held to
and stronger than anything derived from reading a database. What is *not*
independently confirmed is the silkscreen: that the button labelled `USER`
on the board is the one the schematic calls `SW3`. The schematic's other
two switches are `BTN_RESET` and `BTN_PROGRAM`, and those have their own
labels on the board, so the risk is a mislabelled silkscreen rather than an
ambiguity.

4. **The r1.4.0 PCB layout**, `cynthion.kicad_pcb` in the same
   repository, which is where this file used to have an open question and
   now has an answer. It was opened to settle which end of the LED row is
   `led_n[0]`, and the answer came with a correction: **there are no
   per-LED digits on the board at all.** The front silkscreen near the row
   has one legend, `FPGA LEDs`, at (126.2, 109.4); the LEDs are `D2` to
   `D7` at x = 114.5, 117.5, … 129.5, all at y = 112 on a board spanning
   x 104–160; and the schematic's `LED0`..`LED5` are `D7` down to `D2`. So
   `led_n[0]` is `D7`, the end of the row at the higher x — which is the end
   **away from** the USER button (`SW3` at (106.7, 95)) and away from the
   USB-C receptacle on that same edge (`J1` at (106.65, 109)).

   Two designs in `testdata/fpga/cynthion/` said the LEDs were
   "silkscreened 0 1 2 3 4 5". They are not, and they now say so.

What is still **not** verified: that `D7` really is what Great Scott
Gadgets' own software calls LED 0 — the chain platform file → schematic net
→ refdes → layout position is four files agreeing, not an observation. It
is the thing `button_led.v` asks a person to check, because it is the first
design of the three whose appearance depends on it.

The device file says all of this in `src/fpga/devices/ecp5.dev`, under
`device ecp5-12f-CABGA256`, with `pins partial` set because ten of 256
balls are named. **A device file cannot express polarity**, so the designs
drive zero and say in their headers that zero is lit.

## What was not needed, and why that is the news

The placer and the router needed **no edits at all** to route on this
family, and `src/fpga/bitstream.rs` needed one line of visibility. That is
the same thing `docs/fpga-xray.md` and `docs/fpga-gowin.md` each report for
their vendor, and it held for a routed design as well as a constant one.

Two model changes have been needed in total, each for a reason specific to
this family.

**`NetPin::constant`**, for the six-LED milestone. Before it,
`src/fpga/place.rs` recorded that a pin was driven by a constant rather
than by a signal, and did not record *which* constant. Nothing had needed
to know: neither Gowin nor the 7 series had ever compiled a design with a
constant-driven pin, because both have interconnect and their designs route
something. An ECP5 pad tied to a fixed zero or one is the first case where
the fabric has to be told which, so `NetPin` gained
`constant: Option<Bit>`.

**`bitstream::param_bit` became visible inside `fpga`**, for this one, and
the reason is the interesting half of it: **a lookup table's truth table
cannot be a bel's `ConfigEntry::Param` here.** Every other family puts it
there and `bitstream::generate` copies the cell's parameter into it bit by
bit. On this family it depends on the *routing*.

A `LUT4` arrives with its unused inputs tied to a constant — Reticle's
mapper writes `A=btn, B=0, C=0, D=0` for an inverter — and the slice's own
input mux offers only `1`. There is no `SLICE<l>.A0MUX = 0`. So an input a
design ties to zero cannot be tied at the slice at all. What Lattice's own
packer does is tie every unused input **high** and rely on Yosys having
produced a truth table that ignores them. `configure_logic` does the same
thing but derives it: it specialises the truth table at each untied input to
the constant the netlist gives, so the result genuinely ignores that input
and tying it high is safe whichever constant was meant. For the inverter
that is a no-op — `INIT=0x5555` already ignores B, C and D — and knowing
that it is a no-op is worth more than relying on it.

The rest of the division of labour is unchanged and is
`apicula::Periphery`'s: what belongs to a pip goes through the `Arch`, and
what lives in a tile no bel owns is a pass over the placement
(`configure_io`, `configure_logic`). `src/fpga/bitstream.rs` is still at
zero behavioural edits for three vendors.

One thing that *was* worth changing is shared: `Arch::build_graph` keyed its
wire index by an owned `String`, so resolving a pip allocated two of them.
At sixteen million resolutions for this die that is most of the build time
for nothing, and the names live in the `Arch` for the whole call. The key
borrows now. Every backend gets it.

## What remains

| What | What it needs |
|---|---|
| A clock enable, an inverted clock or an asynchronous reset | `configure_registers` refuses these rather than writing them, and the reason is in "What a flip-flop costs": a tile's two clock muxes and two reset muxes are shared between its four slices, and the routing has to say which one a flop uses. A `CE` also has to be *routed* to `CE<c>_SLICE`, which nothing has done |
| A second clock domain | nothing in principle: sixteen networks are declared and a net reaches one by routing. Nothing has built a design with two, so nothing has seen what the router does when two clocks want the same quadrant's network |
| A clock from a PLL | `EHXPLLL` has no port map in the device file. The path is there: a PLL's outputs are `G_J<quadrant>CPLL0CLKO*` and every buffer's input mux offers them, which is how `analyzer.bit`'s two globals are fed |
| A clock on a **dedicated** clock pad | nothing, and it has never been exercised. A `PCLKT` pad reaches the centre through `G_JPCLKT<q><n> <- JINCK <- JPADDI`, all `.fixed_conn`s already in the graph; a Cynthion's oscillator is on the `PCLKC` half of the pair, so this flow has only ever taken the fabric route |
| The left and bottom edges' pads | the left edge is the right edge mirrored (`PICL0`/`PICL1`/`PICL2` for `PICR*`, and the `CIB` one column *east* instead of west) and could be checked against the reference bitstreams the same way the right edge was, since they use pins on every edge. The bottom edge is different again: `PICB*` puts two PIOs at a position and shares tiles with the `EFB`. Neither has been checked, and `TrellisDatabase::load` leaves those balls out of the ball map rather than placing something it would configure nowhere |
| A carry chain | `CCU2C` has no port map in the device file, on purpose: its two sum bits and internal carry do not match the `(ci, i0, i1) -> co` model Reticle maps carry onto. The `.mux` records for the cascade wires are read already |
| Block RAM | `Ecp5Stream` reads and writes the initialisation blocks — the reference files' 44 blocks round trip — and nothing generates one. The `MIB_EBR*` tiles' wires and pips are in the graph |
| Distributed RAM | `SLICEA.MODE = DPRAM`, `WREMUX`, `CLK1.CLKMUX` and the `WAD`/`WDO` wires, none of which is declared |
| An IO standard other than LVCMOS33 | the bits are in the database and the code takes the standard from the constraints; no other standard has been on a part |
| A bidirectional **bus** on the **right** edge | a resolution rule in `TrellisDatabase::decode`, and nothing in `configure_io`: two bidirectional pads that share a right-edge pad tile cannot be read back, because a pseudo-differential value's pattern spans both halves of the pair and is longer than either. `ecppack`'s own output has the same property, and the flow refuses the design rather than weakening the check. See "What cannot be read back". **On the top edge an eight-bit bus builds and decodes today** — 465 bits, 0 unexplained — so this is the whole of what stands between here and a ULPI data bus |
| A bidirectional bus **on a part** | nothing but somebody looking: one bit has been loaded and watched, not eight |
| `SLEWRATE`, `DRIVE`, `OPENDRAIN`, `CLAMP` or `TERMINATION` on a pad | each is a `.config_enum` of the pad tile, and each is one `ecppack` writes **only when an attribute asks** — so not writing them matches nextpnr exactly for a design that does not ask. `set_io -slew` and `-drive` are parsed and reach the cell, and `configure_io` writes neither, which makes the option a silent no-op in the bitstream. `SLEWRATE=FAST` is the one that will be wanted first: every ULPI pin of `analyzer.bit` has it, and a 60 MHz bus on a 3.3 V bank is what it is for |
| A bidirectional pad with a **registered** tristate | `PIO<s>.TRIMUX_TSREG = IOLTO` and the `IOLOGIC` tristate register, none of which is declared. `fpga::primitives` declines to absorb a tri-state driver on a DDR port rather than moving the enable ahead of the register |
| An ECP5 over an FTDI cable | nothing, in principle: the configuration plans are transport-neutral and `jtag::Scan` encodes them for MPSSE. It is refused because that pairing has never been run |

## The three things worth knowing if you change this

**The order of a position's windows is the order Project Trellis' tile
names sort in.** `parse::tilegrid` sorts by key and
`TrellisDatabase::load` pushes in that order, and a `ConfigBit`'s `row`
addresses them end to end. Change the sort and every bit of a shared
position moves. `CIB_R1C63:CIB` sorts before `MIB_R1C63:PICT1`, which is
why a `PICT1` bit `F54B0` becomes row 106 + 54. The loader now also checks
that every position of one composition has the *same* window layout, not
just the same total size, because that is what every bit of a shared
position hangs off.

**A bel must declare its pins, and must sit where they are.** Two halves of
one lesson, learnt separately.

`Netlist::build` records no `NetPin` for a pin whose role names no wire the
`Arch` declares — it goes in `off_fabric` — so the constant behind an
output pad would never reach `configure_io`. That is how the first half was
found: two designs driving different constants produced the same 60 bits.

The second half is that a pin resolves in the tile the *bel* sits in. While
nothing routed, the pad bel was put where the pad's bits were, one column
east of the ball for side B, and its three wires were declared there by
hand. Both tiles have a `PADDOB_PIO`, so nothing complained — the bel just
had the wrong one, and a design that routed would have routed to a wire the
buffer does not read. The bel is at the ball's own position now and its
wires are the ones `bits.db` says that tile owns; a pad whose tile does not
own all three is left out of the ball map rather than placed.

**A tie and a route can be the same mux.** `CIB.JA0MUX` offers `0`, `1` and
`JA0`, where `JA0` means "whatever the routing mux selects", so the bits
that tie a wire to a constant and the bits that route a signal into it live
in one bit space. Two consequences: `configure_io` must not tie a pin a
signal drives, and no test can ask a finished bitstream whether a pad is
tied. `tests/fpga_trellis.rs` runs `configure_io` into a bitmap of its own
to ask what that pass wrote.

That is the first of **four** places on this part where two features share bit
space, and the shape repeats:

| | Which two | Where it is written down |
|---|---|---|
| 1 | a `CIB`'s constant mux against the routing mux into the same wire | here |
| 2 | a centre mux's six-bit source code, whose other five bits another feature may set | "A bit a feature wants clear" |
| 3 | `PIO<s>.PULLMODE`'s low bit against `OUTPUT_<standard>`'s | "A third place where two features share one bit" |
| 4 | a right-edge `PIO<s>.BASE_TYPE`'s *pseudo-differential* values, whose patterns reach into the neighbouring PIO's bits | "What cannot be read back" |

The first three are handled the same way: the bits go into a zeroed bitmap
with an OR, `dropped_clear_bits` notices a feature whose bits another feature
wanted clear, and a question the finished image cannot answer gets asked of
the pass instead. The fourth is the one that is not handled — it makes a
bidirectional *bus* on the right edge unreadable, and the flow refuses such a
design rather than weakening the check that notices.
