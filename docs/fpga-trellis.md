# A real Lattice ECP5, and a real `.bit`

## An adder is a carry chain now, and the chain runs east

The ECP5 backend inferred no carry cell. `src/fpga/devices/ecp5.dev` named
`CCU2C` and gave it no port map, so `primitives::Mapper::carry_chains`
declined with a note and every adder on this family became a tree of lookup
tables. Four chained 32-bit additions came out at **logic depth 87** against
5 on an iCE40; `ip/crypto/sha256`'s five-deep T1 chain at **39 against 9**;
`ip/cpu/rv32i` at 34.

That was reported as a performance gap and it was not one. A round put both
crypto cores behind the Cynthion's serial port at 60 MHz — the ULPI rate, so
there is no slower clock to retreat to — and asked for the SHA-256 of the
**empty message**, which is the most deterministic thing the device can be
asked: nothing typed, nothing crossing USB, one block of self-generated
padding. It answered **seven different ways in sixteen runs**, all of them
wrong, and ChaCha20's quarter round two ways in forty. Every command with no
adder in it was exact, forty times out of forty. And the core's throughput on
the part was 0.494 bytes per clock against simulation's 0.496 over 262 144
blocks, so the control path and the cycle count were already right. A
deterministic function that gives a different answer each run is a path that
does not close.

An adder on this part is a carry chain now. `chacha20_core` went from 5834
`LUT4` at depth 87 to 547 `CCU2C` and 3033 `LUT4` at **depth 4**;
`sha256_core` from 3041 at 39 to 314 and 1688 at **depth 7**. Both are now
*shallower on the ECP5 than on the iCE40*, which is worth a sentence of its
own below. **Forty-four** rows of `docs/ip-library.md`'s footprint table
moved, and thirty-five iCE40 rows moved with them by one or two cells and no
depth — which is the other half of this round and is a *correctness* fix
rather than a cost. "A board found what nothing else could" below is that
half, and it is the part worth reading first.

### Everything `ecppack` writes for a `CCU2C`, in full

This is the best-supported table in this document, by weight of evidence.
`RETICLE_ECP5_REF` points at three bitstreams this tree decodes byte for
byte, and **any design with arithmetic in it contains carry cells**: there
are **2131** of them in the three — 1212 in `analyzer.bit`, 6 in
`selftest.bit`, 913 in `facedancer.bit` — over 625 logic tiles. The
distributed RAM round had 111 to work with and the block RAM round 53.
`what_lattices_own_packer_writes_for_a_carry_cell` reads every one back.

| | |
|---|---|
| How many | 1212, 6 and 913; 625 tiles hold them |
| `SLICE<l>.MODE` | `CCU2`, **two bits, and per slice**: four separate fields, `SLICEA.MODE` to `SLICED.MODE`. Set in their files at the frames this flow computes |
| `SLICE<l>.CCU2.INJECT1_0` | `NO` in **all 2131**, without exception. `YES` is the default and gates the generate term away |
| `SLICE<l>.CCU2.INJECT1_1` | `NO` in 1883, left at `YES` in 248 — exactly the slices whose upper half is unused |
| `SLICE<l>.K<n>.INIT` | the lane's truth table. **Bits 15..12 are the propagate and bits 3..0 the generate**, and the two middle quadrants are written to the same value as the top |
| `SLICE<l>.C<n>MUX`, `D<n>MUX` | `= 1` in **every one of the 4262 halves**, without exception |
| `SLICE<l>.A<n>MUX`, `B<n>MUX` | `= 1` for an operand that is a constant, with the constant folded into `INIT` |
| The sum | `F<z> <- F5<l>_SLICE` for a lower half, `FX<l>_SLICE` for an upper one: **all 4014** driven halves, and **not one** on the `F<z>_SLICE` a lookup table uses |
| The chain | `.fixed_conn` throughout with **no mux anywhere**; every one of the 239 maximal runs begins at slice A |
| Entering it | a cell with propagate **zero** at the bottom of every run, so its carry out is its generate whatever `FCI` carried |
| Leaving it | a cell with propagate zero at the top, so its sum *is* the carry handed to it |
| The flip-flops of a carry slice | 419 of them in use. Arithmetic mode takes a slice's lookup tables, **not** its registers |
| Other logic in a carry tile | 384 lookup tables in slices of a carry tile that are not in `CCU2` mode |

Eighteen distinct truth tables appear across the three files and every one of
them is an arithmetic lane under the element's equation, which is what makes
the reading of the nibbles more than a guess:

| `INIT` | propagate | generate | what it is |
|---|---|---|---|
| `0x666A` | `a ^ b` | `a` | a two-operand add — the value this flow writes |
| `0x666C` | `a ^ b` | `b` | the same, generate on the other operand |
| `0x999A`, `0x999C` | `a ~^ b` | `a`, `b` | a subtract |
| `0xAAA0`, `0xCCC0` | `a`, `b` | 0 | one operand and the carry: an increment |
| `0x555A`, `0x333C` | `~a`, `~b` | `a`, `b` | one operand and a constant one |
| `0x5550`, `0x3330`, `0xFFF0` | `~a`, `~b`, 1 | 0 | constants folded in |
| `0xAAAA`, `0xCCCC` | `a`, `b` | `a`, `b` | the same thing as `0xAAA0`/`0xCCC0`: where the propagate is zero the generate is too |
| `0x0000` | 0 | 0 | a carry in of **zero** |
| `0x000A`, `0x000C` | 0 | `a`, `b` | a carry in off a routed wire |
| `0x000E` | 0 | `a \| b` | a carry in of **one**, both pins forced high |
| `0xFFFF` | 1 | 1 | the default, which costs no bits: an unused half, passing the carry through |

So the element's model, which every one of those values satisfies:

```text
propagate[i] = INIT<i>[12 + 2b + a]      the lookup table, C and D high
generate[i]  = INIT<i>[2b + a]           its low nibble, a table of its own
S<i>         = propagate[i] ^ carry[i]
carry[0]     = CIN
carry[i+1]   = propagate[i] ? carry[i] : generate[i]
COUT         = carry[2]
```

The `INIT` bits are `!`-marked, so the default is **all ones** and a truth
table of zeros is sixteen bits *set*. `0xFFFF` costs nothing and passes a
carry through, which is why that is what an unused half is left at.

### The chain's routing, and the direction a guess would have got wrong

This is the part where a mirrored guess would have decoded perfectly and
computed nothing, which is exactly what the left-edge round nearly did. The
`PLC2`'s own `bits.db` says it in seven lines and they are the whole answer:

```text
.fixed_conn FCI_SLICE  FCI            slice A's carry in, from the tile's
.fixed_conn FCI        HFIE0000       which comes off this wire
.fixed_conn FCIB_SLICE FCOA_SLICE     B's, from A's
.fixed_conn FCIC_SLICE FCOB_SLICE
.fixed_conn FCID_SLICE FCOC_SLICE
.fixed_conn FCO        FCO_SLICE      slice D's carry out, to the tile's
.fixed_conn E1_HFIE0000 FCO           and into the tile one column EAST
```

Three things follow and all three decided code:

- **A chain runs A, B, C, D and then east**, one column at a time, in the
  same row. `E1_` is "one column east" by `parse::globalise`'s own rule, and
  `HFIE0000` appears in no other tile type of the database at all. A chain up
  a column — which is where the iCE40's goes, and what an analogy would have
  said — does not exist on this part.
- **Nothing can drive a carry in.** There is no `.mux` anywhere whose sink is
  `FCI`, `FCI_SLICE` or any `FCI<l>_SLICE`. The interconnect cannot reach the
  chain, so a chain cannot be entered by routing a zero to it.
- **The whole chain costs no bits.** Every link is a `.fixed_conn`, which
  this architecture expresses as a pip with an empty bit pattern. The router
  routes a carry like any other net and `Routing::verify` walks it back to
  its driver; it simply leaves no trace in the image.

The vendor's files agree, and they agree in a way that is worth stating
because it is a property rather than a count: **every one of the 239 maximal
runs begins at slice A.** No carry slice anywhere in the three files takes
its carry from a slice in logic mode. That is not a convention — it is the
only arrangement in which the carry entering a chain is defined.

### How a chain is entered, and the cheaper way this flow does it

`ecppack` spends a whole cell at each end of a chain: one at the bottom whose
propagate is zero, so that `COUT = generate` whatever `FCI` carried, and one
at the top whose propagate is zero, so that its *sum* is the carry handed to
it and the chain's carry out can be read on a wire. `0x0000` injects a zero,
`0x000A` and `0x000C` inject a routed signal, and `0x000E` with both pins
forced high injects a one.

This flow spends a **lane**, not a cell, and needs no special `INIT` to do
it. With both operands zero, `0x666A`'s propagate is `0 ^ 0 = 0` and its
generate is `0`, so the lane's carry out is zero however `FCI` arrived — and
`src/fpga/devices/ecp5.dev`'s `param_zero0` / `param_zero1` set that lane's
`INIT` to `0x0000` as well, so the lane needs **no pins connected at all**
and no constant routed to it. A 16-bit add is therefore seventeen lanes and
nine `CCU2C`, and the adder's bit 0 lands on lane 1.

That matters for more than area. Because every chain begins with a lane
whose propagate is zero, **two unrelated chains may sit end to end in one
column** and the first cannot corrupt the second — which is what the vendor's
own files do, and why the leftmost tile of a row, whose `HFIE0000` nothing
drives, is harmless. The carry off the top of a chain is not read at all:
an `add` cell is as wide as its result, and a design that wants the carry
widens its operands.

Nothing in the vendor's files demonstrates the lane arrangement, and nothing
in them contradicts it. What demonstrates it is `tests/fpga_carry.rs`, where
`check_equivalent` **proves** the mapped chain equivalent to the adder it
came from at 16 and 32 bits and at the awkward widths, and
`a_carry_chain_adds_what_it_was_asked_to_add` in `src/fpga/primitives.rs`,
which evaluates every input pattern of a six-bit add against the element's
equation with the dangling `CIN` tried **both** ways and the unconnected
operand pins tried both ways.

### The seventh place where two features share one bit

`SLICEA.CCU2.INJECT1_0 = NO` is `F8B10`. So is the `.mux F0` source
`F5A_SLICE`:

```text
.mux F0
F0_SLICE -
F5A_SLICE F8B10

.config_enum SLICEA.CCU2.INJECT1_0 YES
NO F8B10
YES !F8B10
```

One fuse with two names, and the same pairing in all four slices and both
halves. Read as logic it selects the slice's wide-function output onto the
tile's `F` wire; read as arithmetic it ungates the carry into the lane's XOR
and its generate. It is the same gate either way, which is why a carry
cell's sum comes out on `F5<l>_SLICE` and not on `F<z>_SLICE`.

**The bitstreams measure it rather than merely suggesting it.** 4014 halves
have `INJECT1_<n> = NO` and 4014 sums are driven — the same number, over 4262
halves of 2131 cells in three files. If the two were different fuses that
equality would be a coincidence 248 times over.

The consequence for this flow is small and had to be handled rather than
ignored: a lane whose sum nothing reads — the lane that enters the chain, and
the lane left over at the top of an odd-width adder — still needs the bit
set, so a decoding of the finished image reports a pip the router never
chose. `TrellisFabric::configure_carry` returns those arcs **by name, from
the placement**, and the caller adds them to the router's own set before
comparing. That is an addition and not a relaxation: the comparison is still
an equality, every arc in it is accounted for by a carry cell that is really
there, and a bit no carry cell claims still fails. `reticle fpga --bitstream`
and `chosen_with_carry` in `tests/fpga_trellis.rs` both do it that way.

### What it contends for, and the constraint it forces

**Its slice's two lookup tables, and nothing else.** `BelDecl::blocks` names
`SLICE<l>.K0` and `SLICE<l>.K1` and stops there, and that is the vendor's
answer rather than a cautious guess: 419 flip-flops are in use *inside* carry
slices in their files, and 384 lookup tables are in the other slices of tiles
that hold one. A model that blocked the tile, or the slice's registers, would
have routed and would have been wrong in a way no test of this flow's own
output could have caught — the same trap the distributed RAM round found.

A carry cell takes **no control wire**: no clock, no reset, no clock enable.
So unlike a distributed RAM, which spends the tile's `LSR1`, it does not
touch the control-wire budget `SiteRules` models, and the seventh shared bit
is the only resource it shares with anything.

What it does force is **adjacency**, which nothing else on this part does.
The carry has no routable path, so cell *n* + 1 must be on the very next
carry site or there is no path at all — and "the router refuses after a whole
placement" is the failure this project has twice decided is the wrong one.
**This fabric needed no new placer code for that**, and finding out that it
did not is the useful part. `fpga::place` already had a second kind of group
beside the `rloc` macro — `build_clusters` and `Exact`, written for the
7-series `CARRY4` — and it holds a `CCU2C` chain unmodified. Two rounds built
the mechanism in parallel without seeing each other; the ECP5-specific one
was dropped and this is what is left:

- `build_clusters` reads the relationship **off the routing graph**, and it
  walks *backwards*. For every `(sink kind, sink role, driver kind, driver
  role)` a signal in the netlist uses, it walks back from that sink pin on
  every site of the sink's kind. If the walk closes inside `DEDICATED_WALK`
  (eight wires) on all of them and finds exactly one site of the driver's
  kind each time, then the driver's site **is a function of** the sink's and
  the two cells are tied together. Nothing names a slice, a letter, a
  direction or a family.
- **CHECKED on this die.** `FCI_SLICE` is reached only over
  `FCI <- HFIE0000 <- FCO <- FCO_SLICE` — five wires, every hop a
  `.fixed_conn` — and each of the 12 144 carry sites has exactly one
  candidate driver slice, so the condition holds and the answer is "A, B, C,
  D, then one column east" because that is what the seven `.fixed_conn`s
  say. A left-edge slice A has *no* candidate, which is right: no chain can
  continue into it.
- the tied cells become one rigid group anchored on a **tile**; which site of
  that tile the anchor takes, and therefore where every other member goes, is
  resolved through the relation. The legaliser tries anchor tiles nearest the
  analytic solution first and the annealer moves the group as a unit by
  drawing a new anchor tile, exactly as it moves an `rloc` macro. A group that
  fits nowhere is `PlaceError::NoDedicatedRun` and not "the part is full".
  Every iteration order is over a `Vec` or a `BTreeMap` and the placer stays
  deterministic — `a_carry_chain_places_routes_and_every_bit_of_it_decodes`
  places this design twice in one process and compares every site and every
  bit.
- a chain is still read **off the netlist** and not off the fabric: only
  instances a signal actually joins are grouped, so a `co` nothing reads or a
  `ci` two cells share leaves the cells free, which is right — then there is
  a real net for the router to find a path for and it will say so if it
  cannot.

What the dropped mechanism had and this one does not is a forward walk out of
`co`, which needed the netlist to hand it a chain *order*. This one resolves
a group outwards from any member, so it does not; and it also holds the
7 series' other dedicated connection, a lookup table feeding the carry beside
it, which the chain-shaped one had no shape for.

### What places now, and what it costs

`testdata/fpga/ecp5/carry_chain_16.v` is a 16-bit accumulator and nothing
else, with every port on a top-edge ball of an LFE5U-12F in caBGA-256:

| | |
|---|---|
| Cells | **9 `CCU2C`**, 16 `TRELLIS_FF`, 34 `TRELLIS_IO`, 1 `DCCA` — and **0 `LUT4`** |
| The chain | nine consecutive carry sites: four slices of a tile, then the tile one column east, twice |
| The group | **one** dedicated-wiring macro over all nine cells, which is what the placement report's `dedicated wiring: 1` line says |
| Bits | 1068 set, of which 36 are the carry cells' mode and inject bits |
| Routing | 59 of 60 signals (the carry off the top is driven and read by nothing), 442 pips over 501 wires, every sink walked back to its driver |
| Decoding | **all 1068 bits decode**, 0 unexplained, into 270 arcs, 322 fields and 18 words — and the arcs they select are exactly the 270 the router chose |

`a_carry_chain_places_routes_and_every_bit_of_it_decodes` is that test, and
it checks the chain's shape against the fabric's own pips rather than against
a rule written down in the test: for each consecutive pair it walks the
bitless pips out of one cell's `co` and insists on arriving at the next
cell's `ci`, and it asserts every pip on that walk costs no bits.

`ip/memory/fifo_sync` goes through the same flow with carry chains in it now
— its pointers are three `CCU2C` each — at depths 16, 32 and 64, and the
"every bit decodes" check is not weakened anywhere: the carry chain adds no
unexplained bit. The design also got *smaller*, which moved two numbers that
were pinned: `lutram_reset_64` now occupies 59 logic tiles instead of 65 and
lands 13 flip-flops in RAM tiles instead of 16.

### The ECP5 is shallower than the iCE40 now, and why

| Block | iCE40 HX1K | ECP5 45F, before | ECP5 45F, after |
|---|---|---|---|
| `chacha20_core` | depth 8 | depth **87** | depth **4** |
| `sha256_core` | depth 9 | depth **39** | depth **7** |
| `rv32i` | depth 10 | depth **34** | depth **33** |
| `mos6502` | depth 11 | depth **16** | depth **16** |

The crypto blocks end up shallower here than on the iCE40 because of the
difference between the two carry elements. An `SB_CARRY` computes only the
carry; the sum is `a ^ b ^ carry` in two XOR cells outside it, which the LUT
mapper covers and which therefore *count*. A `CCU2C` is the lookup tables, so
the sum comes out of the element and the adder contributes **nothing** to the
mapped network. `chacha20_qr` says it in one row: 573 `LUT4` at depth 87
before, 68 `CCU2C` and 128 `LUT4` at **depth 1** after.

`rv32i` and `mos6502` moving by one level and none is the honest other half.
They are not adder-bound — their critical paths are instruction decode and
addressing — so they buy area (408 and 126 fewer `LUT4`) and almost no depth.
**A depth figure is not a timing figure**, and those two rows are what say so.

### Whether the equivalence proof sees inside a carry cell

**`every_block_maps_to_the_logic_it_was_mapped_from` does not, and cannot.**
That check proves the *technology mapper* against the AIG it mapped, and a
carry cell is emitted by `fpga::primitives` **before** that pass runs, as a
blackbox the AIG never holds. With respect to a `CCU2C` it is vacuous, and
saying so plainly is better than letting the gate look greener than it is.

Two checks were added that do see inside one, and they share nothing but the
equation quoted above:

- `tests/fpga_carry.rs` evaluates the whole mapped netlist — `LUT4`,
  `CCU2C` and the assignments between them — on **every** input combination
  at 4, 5, 7 and 8 bits in three shapes, with a Rust model of the element
  over `Logic`; and then replaces each primitive with the same equation
  written out of `lut`, `xor` and `mux` cells and hands it to
  `formal::check_equivalent`, which **proves** 16 and 32 bits equivalent, and
  9, 10 and 17, where the lane the chain spends entering itself lands
  differently. Combinationally that is a decision procedure and not a sample.
  It is the only place in this tree where a solver is shown the inside of a
  carry cell.
- `a_carry_chain_adds_what_it_was_asked_to_add` in `src/fpga/primitives.rs`
  does the exhaustive version over the *netlist the mapper produced*, reading
  each lane's propagate and generate out of the `INIT` the mapper wrote, and
  varies the two things the fabric decides rather than the netlist: the
  dangling `CIN` of the first cell, which is whatever the cell to the west
  left there, and the unconnected operand pins, which on this family read as
  a **one**. Both ways, same answer.

### A board found what nothing else could: a constant is not a tie

**Everything above was true and the chain still computed the wrong number.**

`cnt + 1` is an adder whose second operand is a constant, so thirty-one of
its thirty-two `b` pins are a constant **zero**. A constant is not a wire the
router routes — and `CLAUDE.md`'s own first hazard is that **an unrouted
slice input on an ECP5 reads as a one**. So every one of those pins read as a
one and the chain computed `cnt - 1`. Every counter, every pointer, every
`+ 1` in the library.

Nothing off the part could see it, and the list of what passed is the point:

- the design synthesised, placed and routed;
- **every set bit of its bitstream decoded** back through the database into
  the arcs the router chose, with none unexplained;
- `a_carry_chain_adds_what_it_was_asked_to_add` evaluated **every** input
  pattern of a six-bit adder against the element's own equation, with the
  dangling `CIN` and the unconnected pins varied both ways;
- `tests/fpga_carry.rs` **proved** sixteen and thirty-two bits equivalent
  with a SAT solver, over shapes that include a constant operand.

All of them, because **in a netlist a constant pin evaluates to the
constant.** The fabric is the only place it does not, and the three checks
this project calls load-bearing are all netlist checks.

What found it was `testdata/fpga/cynthion/usb_ulpi_device.v` — the design in
`CLAUDE.md`'s own example — refusing to enumerate on a Cynthion:

```text
usb 7-5: device descriptor read/64, error -71      (four times)
usb 7-5: device not accepting address 52, error -71
usb usb7-port5: unable to enumerate USB device
```

And the bisection was as sharp as a board can give: the **same design from
the same tree** with `CCU2C`'s port map stripped out of `ecp5.dev` came out
**byte-identical** to the bitstream that works. One feature, one difference,
one outcome.

`techcells::drive_constant_data` gives those pins a real driver now — the
same shared constant lookup table it was extended to give a distributed RAM's
inputs, for the same reason, which makes this the **third** time this hazard
has been met: a flip-flop's data pin, every input of a `lutram`, and now a
carry element's operands. Two roles are excluded and both are the chain's own
ends: `ci`, which is dedicated metal nothing outside may drive, and `cyinit`,
which is the pin a family *designs* to take a constant.

It costs one or two cells per module — the constant lookup tables are shared —
and no depth at all, which is why thirty-five iCE40 rows of the footprint
table moved by one or two `SB_LUT4` and not one of them by a level. The iCE40
side is the same latent defect closed before anything runs an `SB_CARRY` on a
part.

**The cheaper fix is the one `ecppack` uses**, and this flow does not use it:
fold the constant into the lane's own `INIT` and tie the pin high, which is
what `0xAAA0` and `0xCCC0` are in the table above and costs no net at all. A
driver is correct on every family and the folding would be correct on one;
the folding is worth doing and is not done here.

With the fix, that design enumerates and **two of this tree's board tests
pass on the part with carry chains inside them**, on a Great Scott Gadgets
Cynthion r1.4 (`LFE5U-12F-8CABGA256`, serial
`35L6H2CMGJJVCIBAEA3GCLAN74`) at 60 MHz:

| | |
|---|---|
| `tests/usb_loopback.rs` | `usb_ulpi_device.v`: 962 `LUT4`, **66 `CCU2C` in 14 groups**, 16 distributed RAMs. 256 bytes out through endpoint 1 and back **byte-identical** in packets of 64, 63, 8, 5 and 1, then 16 KiB in 256 round trips at **256 235 bytes/s** each way — the rate the round that wrote that test measured, to a fifth of a per cent. `zero_probe` read ZERO on every packet. The buffers' write pointer and read index are carry chains, so a byte back from the wrong address would be a wrong chain |
| `tests/usb_proxy.rs` | `usb_proxy_target.v` with `VBUS_AUX = 1`: 3245 `LUT4`, **161 `CCU2C`**, 4884 signals of which 4843 routed — the eleven its own header accounts for, plus **thirty carry-outs off the tops of chains**, which nothing reads and which is therefore how many chains it has. The kernel enumerated a **GreatFET through our hub and our proxy** — `1d50:60e6` as a child of `1209:0001` in sysfs — and its device and configuration descriptors came back through the relay byte for byte |

Both bitstreams have every set bit decoding with nothing unexplained and
the arcs they select exactly the arcs the router chose: **39 738** bits into
11 630 arcs for the first, 120 732 into 36 329 for the second.

**Which placer each row was measured under**, because a placement that puts
a chain anywhere but on consecutive carry sites has no path for its carry
and these are the checks that would catch it. The loopback row is
re-measured and the figures above are the new ones: it was first run while
this fabric had a carry-chain mechanism of its own in `fpga::place`, and it
was run again — bitstream rebuilt, part reprogrammed, test re-run — on
`build_clusters`, the generic dedicated-wiring mechanism the tree now has.
Same 14 groups over 66 cells, a different placement (39 738 bits and 11 630
arcs where the first run had 39 900 and 11 715), and the same bytes back.
The proxy row has **not** been re-measured on the new mechanism; its numbers
are the first run's, and re-running it means enabling the AUX VBUS switch,
which the round that reconciled the two mechanisms was not permitted to do.

The proxy is still the better of the two as a check on *placement* —
thirty chains over 161 cells is thirty runs of consecutive carry sites that
all had to be right at once — and the loopback is the one that has been run
both ways.

### What a board would add, and the cheapest experiment

Everything above *except the last section* is off the part. The database, the
vendor's bitstreams, the placer, the router, the "every bit decodes" check,
the solver and the exhaustive simulation are all static, and **not one of
them can tell you whether the clock closes** — nor, as the section above
records, whether a constant is a constant.

A closed clock is the whole point of this change and it is the one thing a
depth figure does not say: depth 4 instead of 87 is a claim
about how many lookup-table levels the mapped network has, and a chain's own
delay is not in it at all. A 32-bit add is seventeen `CIN → COUT` hops and
one lane's `A → S`; Yosys' `cells_sim.v` puts those at 43 ps and 379 ps,
which would be under a nanosecond for the whole adder — but that is
**nextpnr's** timing model quoted, not Lattice's and not anything measured,
and this repository has no vendor timing model at all (`reticle timing` says
in its own help that its device numbers are placeholders).

**The experiment that is still open** is the one written and waiting.
`tests/usb_crypto_console.rs` is on `master`,
`#[ignore]`d, skips with a reason when no board is attached, and **fails on
the board** as of the round that wrote it: it asks a Cynthion at 60 MHz for
the SHA-256 of the empty message and for a ChaCha20 quarter round, and got
seven different answers in sixteen runs. If the diagnosis was right it should
now get `e3b0c442…` every time. That is a far stronger claim than any depth
number — a deterministic function answering deterministically, repeatedly,
from silicon.

**It was attempted here and the obstacle was the router, not the part.**
None of the three builds of that design converged; see "Routing a carry
design is slower" below. What was loaded on a part instead were the two
designs that do route, and both of them work — which establishes that a
carry chain computes and places correctly on this silicon, and leaves
"does a 32-bit chain close 60 MHz" open.

What is known about it as this is written: the design with **both** cores,
which did not route at all before (886 nodes still oversubscribed after 40
iterations and 1 h 50 m), is 7629 `LUT4` and 1018 `CCU2C` at **depth 17**
instead of 11 955 `LUT4` at depth 87, and its placement comes out as **83
dedicated-wiring groups over 1018 cells** — the same 83 under both of the
two mechanisms this fabric's chain has been through, which is one small
piece of evidence that they found the same chains; the single-core build is
4427 `LUT4` and 451 `CCU2C` at **depth 16** instead of 5891 at 39. Neither of those gained a
cell from the constant drivers, because the design already had the two
shared constant lookup tables for its flip-flops and a carry cell's
operand asks for the same two.

### Routing a carry design is slower, and the console did not converge

**This is the gap this round leaves open, and it is a placement-quality one
rather than a correctness one.** All three crypto-console builds were run to
the router's forty-iteration cap and **none of them converged**:

| build | before the chain | with the chain |
|---|---|---|
| `WITH_CIPHER=0` (hash) | 5891 `LUT4`, routed **8353 of 8353 signals in 11 m 29 s** | 4427 `LUT4` + 451 `CCU2C`, **gave up** after 40 iterations and 1 h 45 m, 366 nodes still oversubscribed |
| `WITH_HASH=0` (cipher) | 8375 `LUT4`, routed **10 111 of 10 111 in 4 m 50 s** | 5426 `LUT4` + 683 `CCU2C`, **gave up** after 40 iterations and 1 h 11 m, 72 still oversubscribed |
| both cores | 11 955 `LUT4`, **gave up** after 40 iterations and 1 h 50 m | 7629 `LUT4` + 1018 `CCU2C`, **gave up** after 40 iterations and 4 h 00 m, 5522 nodes still oversubscribed |

The right-hand column is measured on the mechanism the tree ships, which is
not the one the first attempt used; the two agreed about how many chains
there are (83 groups over 1018 cells for the both-cores build) and disagreed
about where they go, and neither converged.

So the design got a third smaller and a factor of five shallower and became
*harder to route*. Two reasons, and the second is actionable in one line:

**A chain concentrates pin demand.** A 32-bit add is five logic tiles, each
wanting sixteen operand wires in and eight sums out of one tile, where the
same adder in lookup tables was spread over twice as many tiles with half the
pins each. The negotiated-congestion router has more to negotiate in the same
interconnect.

**And the annealer may barely be able to move a chain.** `PlaceOptions`'
schedule ends at a **one-tile** move window, and a rigid group is moved by
drawing a new anchor *tile* inside that window — so a group spanning five
tiles can only ever shift by a fifth of its own length per move, and only
when every site the fabric's links then hand its members is free. Everything
else in the design anneals around groups that are nearly where the legaliser
put them, and the legaliser packs tightly and wires long.

### The move-window experiment was run, and it is not the fix

**CHECKED, and the answer is no.** The experiment the paragraph above asks
for — scale a rigid group's `reach` in `propose` by the group's own tile
extent, so that one schedule step means the same *fraction of its own
length* for a group as it does for a cell — was implemented and both
placers were run on the console designs, same seed, same everything else:

| build | one-tile window (shipping) | window × group extent |
|---|---|---|
| `WITH_CIPHER=0` (hash) | wirelength **54 882**, **366** nodes still oversubscribed after 40 iterations, 1 h 45 m | wirelength **54 719**, **302** oversubscribed, 1 h 45 m |
| `WITH_HASH=0` (cipher) | wirelength **81 411**, **72** oversubscribed, 1 h 11 m | wirelength **81 115**, **97** oversubscribed, 1 h 11 m |
| both cores | wirelength **137 708**, **5522** oversubscribed, 4 h 00 m | wirelength **134 397**, **4571** oversubscribed, 3 h 46 m |

So the scaling does what it was meant to do and it does not help: **0.3 % to
2.4 % off the wirelength and not one of the six runs converged**. The
residual overuse does not even move consistently — 64 nodes better on the
hash, 951 better on the both-cores build, and 25 *worse* on the cipher —
and "better" here means 4571 nodes oversubscribed instead of 5522, which is
not a different outcome. The change is therefore **not in the tree**: it
costs a placer behaviour change and every golden that rests on one, and buys
nothing that gets a design routed.

**What the router itself says the obstacle is, which is not the annealer.**
Every one of the runs above fails the same way, and the message names it:

```text
X37Y11/V02S0201 is a control wire of X37Y11: 21 of that tile's bels have a
pin that can only be reached through it, and 2 signals were routed onto it
... The placer rejects this arrangement before it is made (`SiteRules` in
`src/fpga/place.rs`), so a design that gets here has found a control wire
the architecture does not describe yet
```

That is the same diagnosis `ip/crypto/sha256/README.md` §8 recorded for the
both-cores build before the chain existed, and it is a **fabric-description**
fact rather than a placement-quality one: `SiteRules` believes it already
forbids the arrangement the router ended up in, so some wire of a logic tile
that 21 of its bels can only be reached through is not in the architecture's
control-wire set. Naming that wire class and adding it is the next thing to
do, and it is a `src/fpga/trellis` change rather than a `src/fpga/place` one.
The chain did not cause it — it made it easier to hit, by packing more logic
into fewer tiles.

Nothing about this is a correctness claim either way: the two designs that
*do* route are on a part and work, and a design that does not converge is
refused rather than written.

Two cheaper things that are *not* substitutes, and it is worth being clear
about why: the `0.494 bytes/clock` throughput figure the earlier round
measured was already correct **while the digests were wrong**, because the
control path closed and the data path did not; and `DONE` asserting says
nothing at all, as four earlier sections of this document record.

If the digests come out right, what it will have established is that this
chain's direction is the right way round on silicon and not merely
self-consistent — the one claim the vendor's bitstreams cannot settle for a
flow that places its own chains. If they come out wrong, the diagnosis was
wrong, and that is worth more than a green depth number.

## A block RAM is on the fabric, and its bits are in three tiles

`ip/memory/fifo_sync` taught this backend that a memory needs a *site*, and the
round that modelled a distributed RAM taught it that the site can be
several slices of one tile. A `DP16KD` is neither, and the lesson does not
transfer:

```
error: the design needs 2 `bram` site(s) and the part has 0
```

`src/fpga/trellis` created `lut`, `ff`, `io` and `lutram` bels and no
`bram`, so a design whose memory inferred a block RAM died at the placer
even though `src/fpga/devices/ecp5.dev` declared the primitive and the
technology mapper emitted it. That was a ceiling on everything built for
this board: `examples/mos6502_monitor` uses a **512-byte logic ROM**, about
one lookup table per byte, and says in its own header that it does so
because a writable memory of any size was unavailable. 56 block RAMs of
18 kbit each — 1008 kbit — were sitting unused.

They are not now. A 2 KiB writable memory and a 2 KiB ROM place, route and
come out as a bitstream every bit of which decodes.

### The tile rule, and it is not the slice's rule

A distributed RAM is one tile, one bit and three slices. Guessing a block
RAM by analogy would have been wrong in the way this file exists to catch,
so the rule was read out of `bits.db` and the tilegrid instead:

| | |
|---|---|
| The EBR rows | **two**, at Y25 and Y37, and each is seven groups of **nine** tile types — `MIB_EBR0` to `MIB_EBR8` — which hold **four** blocks between them |
| Which tile owns a block | the four of the nine that carry its 116 `.fixed_conn` records: `MIB_EBR0`, `MIB_EBR2`, `MIB_EBR4` and `MIB_EBR6`, at the group's columns +0, +2, +4 and +6. The bel goes there and nowhere else; the other five hold *bits* and have no pins at all |
| Which of the four a tile's block is | read off the field names. A tile that owns a block declares `EBR<n>.<field>` for its own `n` and, where the fuzzer found a leftover bit, for `n - 1` as well, so the block is the **highest** index the tile names |
| Where its bits are | its own tile and the **two east of it**, and a single field straddles the boundary. `EBR1.DP16KD.DATA_WIDTH_B` is three bits of `MIB_EBR2` and a fourth of `MIB_EBR4`, and **without that fourth bit the 9-bit and the 18-bit mode are the same pattern** |
| `MODE = DP16KD` | five bits over **two** tiles: one in the block's own and four in the tile east of it for the first and second block of a group, and all five east of it for the third and fourth |
| The four edge cases | four of the 56 blocks have an `EBR_SPINE_*` or an `EBR_CMUX_*` where `MIB_EBR8` would be, and one group's first block has `EBR_CMUX_LR_25K` instead of `MIB_EBR0`. Those tiles declare the same fields, so the rule covers them without naming them |

That last row is why a block RAM is the **first feature of this backend
whose bits are a property of its position rather than of its tile type**.
Every other one — a lookup table's truth table, a flip-flop's settings, a
distributed RAM's mode — is a `BTreeMap` keyed by `Arch` tile type index,
because the same composition has the same frame offsets everywhere. A
block's three tiles do not: the easternmost is `MIB_EBR8` for 52 of the 56
and something else for the other four, and a frame offset computed for one
is wrong for the other. So `TrellisFabric::brams` is a `Vec<BramSite>`,
one per `DP16KD` of the part, and every bit in it carries the position it
belongs to.

### How the bitstreams established it, which is the part that matters

A tile rule that is wrong produces a bitstream that loads, asserts `DONE`
and addresses the wrong memory, and nothing in this flow's own output can
tell. So the rule was read back through Great Scott Gadgets' own
bitstreams, at the absolute frame positions it computes, by
`what_lattices_own_packer_writes_for_a_block_ram`.

**Reading `MODE` back through the model finds nine blocks in use in
`analyzer.bit`, none in `selftest.bit` and forty-four in
`facedancer.bit`** — the same counts
`what_lattices_own_packer_writes_for_a_distributed_ram`'s header had
already recorded from a completely different route, the per-tile field
decoder. A wrong tile rule would not produce the same numbers, because
**at least** four of each block's five mode bits are in a tile that is not
the block's own, and for half of them it is all five.

**One correction to this file's record.** That header calls all 53 of them
`DP16KD`. They are not: **two are `PDPW16KD`**, the pseudo dual-port mode
that reads 36 bits. `PDPW16KD`'s bit pattern is a superset of `DP16KD`'s,
which is why counting "are `DP16KD`'s bits set" gets 53 and asking which
mode it is gets 51 and 2. This flow builds no `PDPW16KD`.

### Everything `ecppack` writes for a block RAM, in full

| | How many of the 53 |
|---|---|
| `EBR<n>.MODE = DP16KD` | 51; the other two are `PDPW16KD` |
| `EBR<n>.GSR = DISABLED` | 53 |
| `EBR<n>.RESETMODE = ASYNC` and `ASYNC_RESET_RELEASE = ASYNC` | 53 each |
| `EBR<n>.RSTAMUX = INV`, `RSTBMUX = INV` | 53 each |
| `EBR<n>.WEBMUX = INV` | 51 — exactly the `DP16KD` ones |
| `EBR<n>.WEAMUX = INV` | **2** — exactly the two `PDPW16KD` ones |
| `EBR<n>.CEBMUX = INV`, paired with `CLKBMUX = INV` | 32 — the blocks whose B port is idle |
| `EBR<n>.DP16KD.WRITEMODE_A`, `..._B` | `READBEFOREWRITE` for all 51 |
| `EBR<n>.DP16KD.DATA_WIDTH_A` | 1 for 32 of them, 4 for four, 9 for fourteen, and every bit clear — the default of 18 — for three |
| `EBR<n>.WID` | 53: nine bits, and the number they spell is an initialisation block the same file carries |
| `CSDECODE_A`, `CSDECODE_B`, `CEAMUX`, `OCEBMUX`, `CLKAMUX`, every `AD<x>MUX`, `REGMODE_A`, `REGMODE_B` | **0**. Never written, in any of the 53 |
| The contents | one `LSC_EBR_WRITE` block of 2048 nine-bit words **per block RAM, written even when it is all zeros** — and it is all zeros in all 53 |

**Three of those rows are about a pin nothing drives reading as a one**,
which is the hazard this file opens with, and together they are the
clearest statement of it anywhere in the vendor's output.
`RSTAMUX = INV` and `WEBMUX = INV` are how their packer holds a reset and
an unused write enable **low** — it inverts the wire, because the wire is
already a one. `CEAMUX` and `OCEBMUX` being absent is how it holds an
enable **high**: it does nothing. And the chip selects say it from the
third side: all six are left unrouted and `CSDECODE_A` and `CSDECODE_B`
are left at `111`, which is exactly what three unrouted wires read. So
this flow leaves the same six pins off its bel — they are not in
`bram_pins()` — and ties the same pins with the same muxes.

`CEBMUX = INV` on the 32 idle ports is theirs too, and it is worth
copying: a port whose clock enable is held low can neither read nor write
whatever its other pins float to. `configure_bram` writes it for any port
a design does not use, which is how the ROM below makes its unused port A
inert rather than relying on "no clock edge, no write".

### A defect the database's own account caused, and the one-line fix

`WEAMUX` being written for **no** true dual-port block is not a curiosity.
It is why `src/fpga/devices/ecp5.dev` now lists `DP16KD`'s **B** port
first.

`fpga::primitives` gives a memory's read ports the earliest `port rw`
lines and its write ports the ones after them, so with the A side listed
first a one-read one-write memory read on port A and wrote on port B.
That needs `WEA` held low, which needs `WEAMUX = INV` — and Project
Trellis' `EBR<n>.MODE` record claims `WEAMUX`'s own bit as one that
`DP16KD` wants **clear**. Setting both is a contradiction the database
cannot express, and the result was exact:

```
error: 1 of this bitstream's 1016 set bit(s) belong to no feature the database names
  F99B0 of MIB_EBR2 at (col 24, row 25)
```

That is `MODE = DP16KD`'s own bit, unreadable because `WEAMUX = INV` had
set a bit the mode wanted clear. **The "every bit decodes" check caught it
on the first run**, which is the seventh real defect it has found in this
backend, and the fix is to do what `ecppack` does: write on port A, read
on port B. Then nothing this flow writes for a block collides with
anything else it writes for the same block.

Worth saying plainly what the alternative would have been. The bits are
*independent in silicon* — `WEAMUX` is a mux and `MODE` is a mode — and a
flow that simply dropped the clear bits from `MODE`'s record would have
produced a working bitstream that no longer decoded. `docs/fpga-trellis.md`
already has a section on why a bit a feature wants clear can only be
*noticed* and not honoured; this is the same simplification meeting a
`.config_enum` whose record the fuzzer polluted, and the only sound answer
is not to write both.

### The contents are not in the configuration memory

This is the row that is unlike everything else in this file. A block RAM's
words do not go into the CRAM: they arrive as their own commands —
`LSC_EBR_ADDRESS` then `LSC_EBR_WRITE`, 2048 nine-bit words packed nine to
nine bytes — and `src/fpga/ecp5.rs` has read and written them since the
container was first modelled. What was missing was the tie between a block
and its data, and that is the nine-bit `EBR<n>.WID` field.

**That tie is measured, not read.**
`what_lattices_own_packer_writes_for_a_block_ram` reads the nine bits of
every block out of the CRAM and asserts that the numbers are exactly the
set of initialisation-block indices the same file carries: **3 to 11** for
analyzer's nine and **3 to 46** for facedancer's forty-four, with no gaps
and no duplicates. That settles three things at once from a vendor
artefact — that row 0 of a `.config` record is the **most significant**
bit, that `WID` *is* the block index, and that `ecppack` numbers from 3
(which is also what nextpnr's `pack_ebr` says in a comment, and now does
not have to be taken on trust).

### The ordering of the contents, and the honest answer about it

**It has not been verified against a vendor bitstream, and it cannot be.**
All 53 of their initialisation blocks are **zero** — exactly as all 111 of
their distributed RAMs are empty — so they cannot tell a right ordering
from a wrong one. The test asserts the zeros rather than skipping them, so
a future reference bitstream with contents would fail there and say so.

What the ordering rests on instead is `src/fpga/devices/ecp5.dev`'s own
layout plus one step: 1024 rows of 18 bits in 20-bit slots, sixteen rows
to an `INITVAL_<nn>` parameter, and **two nine-bit words of the stream to
a row, the low word in bits 8..0**. The one thing that makes that
self-consistent rather than merely plausible is that the block's 9-bit
mode addresses the array in exactly those units — 2048 words of nine bits
is the whole 18 kbit — so word *w* of the stream is address *w* of a 9-bit
port, and the device file's `init low 0-8 9-17` says which half of a row
each is. **A part is the only thing that can settle it**, and the
experiment is named below.

This is the same gap the distributed RAM has, one size up and one step
closer to closing: `dpram_init_word`'s permutation is untested because
`fpga::primitives` declines to lower an initialised memory onto a
`TRELLIS_DPR16X4` at all. A **block** RAM's initial contents do reach the
bitstream, so the experiment is now buildable.

### The control wires, and what a block RAM takes from something else

The distributed-RAM round found that a RAM's write enable takes the tile's
`LSR1`, which the placer had to learn or the router failed. The same
question asked of a `DP16KD` has a different answer, and it is a better
one: **a block RAM contends with nothing in its own tile**, because an EBR
tile has no other bel. `CIB_EBR+MIB_EBR0` contributes one `bram` bel and
nothing else — no lookup table, no flip-flop, no pad — so there is no
budget to share and `SiteRules` has nothing to say.

What it does take is **interconnect in two neighbouring columns**. Its 116
pins do not fit in one column's `JA*`/`JB*`/`JC*`/`JD*`/`JF*`/`JQ*`: port
A's low pins are in the block's own column, port B's clock and enable and
both ports' high address bits are one column east, and the **two top data
bits of each port** are two columns east. That last column is the next
block's own column, so in this model an 18-bit block and the block two
columns east of it cannot both be used, and `configure_bram` refuses the
arrangement by name.

**The evidence is mixed and it is recorded rather than resolved.** Of the
51 true `DP16KD` blocks in these files exactly one is 18 bits wide, and
the block two columns east of it is **not** in use, which is what the
refusal expects. But both `PDPW16KD` blocks read 36 bits — which needs the
same four wires — and both **do** have an in-use block two columns east.
So either the overlap is not what this reading says, or those designs rely
on something this model does not express. This flow builds no `PDPW16KD`,
refusing is the conservative direction, and the experiment that would
settle it is named below.

**That refusal has not been shown failing**, which by this file's own
standard makes it weaker than the checks around it: no test builds an
18-bit block beside another, because nothing this flow maps chooses the
18-bit mode at a depth where two blocks land two columns apart. A green run
of designs that do not collide proves nothing about whether anything is
looking, and the experiment below is also what would let the check be
exercised.

The clock is the other control wire, and it needed a fix at each end.
`configure_bram`'s companion check is `clock_network_use`, which now asks
about `p0_clk` and `p1_clk` of every block RAM as well as every flip-flop's
`clk` and every distributed RAM's `wclk`: a block RAM's clock pin is joined
straight to its tile's `JCLK0` with no mux of its own, so the walk stops
there instead of at a slice's `CLK0`. And
`primitives::Mapper::clock_buffers` learned that **a block RAM is two clock
pins, not one** — `CLKA` and `CLKB`, usually the same net — found by
primitive name out of the device file's own `port rw` lines, exactly as the
distributed RAM's `WCK` is. Until that, a block RAM's clock was not counted
towards a buffer's fanout and was not rewired onto it, which is the defect
the distributed-RAM round found one size down.

**A memory's clock can still lose to the global-buffer threshold, and that
is a gap.** A clock earns a buffer by driving at least eight clock pins,
and two block RAMs are **three**; this backend then refuses the bitstream,
because a memory clocked through general routing has skew nobody has a
model for. Bypassing the threshold for a memory was tried and is wrong in
general: on the iCE40 architecture this flow has, an `SB_RAM40_4K`'s `RCLK`
has no path from an `SB_GB`'s output at all, so buffering it makes a design
that used to route stop routing. So the threshold stands, the design below
carries sixteen pipeline registers to get over it, and the real fix — a
per-family statement that this family's memories must be clocked globally
— is in "What remains". The same gap is why `logicram_ecp5`, two
distributed RAMs and nothing else, has its clock on local routing: a
bitstream for it would be refused today.

### What places now, and what it costs

`testdata/fpga/ecp5/block_ram_2048.v` on an LFE5U-12F in caBGA-256: a
2048×8 writable memory with a registered read port, and a 2048×8 ROM whose
contents are a function of the address, with all 37 of its ports on
top-edge balls.

| | |
|---|---|
| `DP16KD` | 2, both in the **9-bit** mode — one block each, which is why the memories are 2048 deep: a shallower one ties on block count and the mapper breaks a tie by preferring the *widest* mode |
| Configuration bits | **1274**, of which **33** are the two blocks' own |
| Initialisation blocks | 2, numbered 3 and 4, of 2048 nine-bit words each |
| Clock pins on a global network | 19 — both ports of the writable memory, the ROM's read port, and sixteen pipeline registers |
| Unexplained bits | **0**, and the 460 arcs the bits select are exactly the 460 the router chose |

The 33 bits break down as twelve per block that every block gets — five
for the mode, one for each port's 9-bit width, one for each port's write
mode, one for `GSR`, one for each of the two reset fields — plus the ties
and the identifier: a reset mux per port, a write-enable mux per port that
is **in use** and does not write, a clock-enable mux per port the design
does not use at all, and one bit per set bit of the block's own number.

That "in use" is the second half of the `WEAMUX` story. A port the design
does not touch at all — the ROM's port A — is held off by its **enable**,
and giving its write enable a tie as well would set the very bit
`EBR<n>.MODE` wants clear and make the mode undecodable again. A port
whose enable is low cannot write, so one tie is the right number, and it
is the one `ecppack` writes.

Three things about the Verilog are load-bearing. **The memory read is
registered with no reset**: `q <= mem[a]` inside a plain
`always @(posedge clk)` is what `src/synth/proc` turns into a *clocked*
read port, and a clocked read port is the only kind a block RAM can serve.
Add a reset to that register and the promotion does not happen, the memory
has an asynchronous read port, and `fpga::primitives` puts it in
distributed RAM instead — which is exactly what `ip/memory/fifo_sync` does, and
why **no `ip/memory/fifo_sync` footprint moved**: both of its variants read
asynchronously, so none of them has ever been a block-RAM candidate and
`docs/ip-library.md`'s table is unchanged. **The ROM has contents and no
write port**, which is the only case in which a block RAM's `INITVAL`
parameters reach a bitstream at all. And **both outputs are registered a
second time**, which is not for timing: sixteen flip-flops take the
clock's pin count from three to nineteen, over the global-buffer threshold
of eight, without which the clock stays on local routing and this flow
refuses the bitstream.

### What a board would have added, and the two cheapest experiments

**A board would have added something real here, and this round did not
touch one.** Everything above is the database, the vendor's own bitstreams
and this flow's own output; a *wrong* block RAM decodes perfectly against
itself and addresses the wrong memory, which is the exact shape of defect
that cost the distributed-RAM round a section of this file.

Two things rest on reading rather than on measurement, and there is one
experiment for each.

**The contents' ordering.** Whether word *n* of an initialisation block is
address *n* of the memory the silicon reads. The check in
`a_block_ram_places_routes_and_every_bit_of_it_decodes` reads back what
`bram_init_words` wrote, so a permutation that was wrong in a
self-consistent way would pass it, and the vendor's blocks are all empty.
*The cheapest experiment:* `usb_bulk_ep`'s packet buffers are already two
RAMs on a Cynthion with a host reading every byte back through endpoint 1,
and `tests/usb_loopback.rs` is already the instrument. Add a third memory
beside them — a 2048×8 ROM whose contents are `(a ^ 0x5a) | 1`, the same
function `block_ram_2048.v` uses — and return a window of it on endpoint 1
instead of the loopback bytes. If the host reads back the function it
expects, at the addresses it asked for, the ordering becomes a
measurement. It needs no new gateware beyond the endpoint's address
decode, and it would settle `dpram_init_word`'s sibling question by
analogy rather than by assumption.

**The 18-bit overlap.** Whether an 18-bit block really sterilises the
block two columns east of it. The reading says yes, the vendor's one
18-bit `DP16KD` is consistent with it, and their two 36-bit `PDPW16KD`s
contradict it. *The cheapest experiment:* build a 1024×16 memory — which
chooses the 18-bit mode — and a second 2048×8 memory, with an `rloc` macro
or a region putting the two blocks two columns apart, then read both back
through endpoint 1. If both memories return their own bytes, the refusal
is too strict and should be dropped; if the wide one's top two bits come
back as the narrow one's bottom two, it is a fabric fact worth a section of
its own. Until then `configure_bram` refuses, which costs the 18-bit mode
nothing a design cannot get from two 9-bit blocks.

### One gap this round found and did not fix

A clock buffer can be placed where its clock **pad** cannot reach it.
`G_CLKI_<name>` is a global wire, so a `DCC` anywhere on the die looks
equally good to a cost function made of distances, and the one nearest a
block RAM at Y25 is `LMID_0` at X3Y25 — which a **top**-edge pad has no
path to at all. Left to itself the placer puts the buffer there and the
router then says:

```
error: no path exists from the driver of `clk` to `clk$gbuf.CLKI[0]`:
the architecture has no wire joining them
```

`testdata/fpga/ecp5/block_ram_2048.rcf` works around it with a region
pinning the buffer to `TMID_0`/`TMID_1`, and the workaround is marked as
one in the file. It is not about block RAM — any design whose clock sinks
sit far from its clock pad can hit it — and the fix belongs in
`fpga::place`, which would have to know which buffers a pad can reach.
Nothing before this round had a clock sink far enough from its pad for it
to matter.

## The left edge is on a part now, and the witness is a transceiver's own vendor ID

The section below this one describes all four edges of this die and ends by
saying what it cannot say: **that the ball is the ball**. A wrong tile rule
is the failure that decodes perfectly and drives the wrong pin, and this
board has no LED, no button and nothing else observable on the left edge at
all. It names the cheapest experiment that would close the gap — *the
transceiver itself is the instrument; it will not raise `nxt` without the
clock the FPGA drives out of `T4`* — and says it has not been run.

It has been run. `testdata/fpga/cynthion/usb_host_target.v`, loaded onto a
Cynthion r1.4, prints this on the USB serial console on its **AUX** port:

```
== CYNTHION TARGET HOST
VIDL=24   VIDH=04      00h, 01h — the vendor ID pair: 0424, Microchip
PIDL=09   PIDH=00      02h, 03h
FUNC=45                04h — XcvrSelect=01, TermSelect=1, SuspendM=1
OTGC=06                0Ah — DpPulldown and DmPulldown, which is a host
INTS=18                13h
DBUG=00                15h — LineState SE0
IOPM=06                39h, read back as this design wrote it
RXCM=40   SEEN=01      a receive command arrived; its ID bit is set
LINE=00   VBUS=00      out of that command: SE0, below SessEnd
PHYR=01   RFAL=00      ready, and not one register read failed
STGE=01   FLAG=00      waiting for an attach; no VBUS, so nothing attaches
VBEN=00                no VBUS switch is closed
```

**Why that settles the edge.** Those nine answers came out of a Microchip
USB3343 over a real ULPI bus, and every one of the sixteen left-edge balls
this design drives has to be the right ball for them to arrive:

| Ball | What it carries | What a wrong ball would do |
|---|---|---|
| `T4` | the 60 MHz interface clock, FPGA → transceiver | the part has no clock, never drives `dir`, answers nothing |
| `R4` | `rst`, active low | the part never leaves reset |
| `R3` | `dir`, transceiver → FPGA | the Link never believes a byte; every read times out |
| `T2` | `nxt`, transceiver → FPGA | no command is ever accepted |
| `T3` | `stp`, FPGA → transceiver | writes never end; `IOPM` could not read back `06h` |
| `R2 R1 P2 P1 N3 N1 M2 M1` | the eight-bit bidirectional bus | a register byte comes back wrong or not at all |
| `K5 L1 L2` | the three VBUS switches, driven low | `VBEN=00` is what a decode says, and a part says the same |

So the two rules the section below measured against Lattice's own packer —
the pad tile one row *south* and the C/D second copy two rows south, with
the column mirrored and the rows not — are **checked against a part**, for
four PIO sides, two directions and a bidirectional bus, on the edge that had
none. 197 of 197 balls of this package are pads and the two long edges no
longer differ in confidence.

### The console said the opposite first, and the report was the thing that was wrong

This is the part worth keeping. The first reading of that console was:

```
PHYR=00  RFAL=01  STGE=01
VIDL=00  VIDH=00  PIDL=00  PIDH=00
FUNC=00  OTGC=00  RXCM=00  VBUS=00  LINE=00
```

A transceiver that answers zero to its own vendor ID and has never latched
a receive command, with the AUX transceiver in the same bitstream working —
which is as clean a signature of "the left-edge mapping drives the wrong
pin" as a board can produce, and it set up a round of work to decide between
that and a hardware-only defect in `ip/usb/usb_host_ulpi`.

Both were wrong. `usb_host_target.v`'s report has a four-character label per
item in a `LABELS` concatenation written back to front, and it indexed that
table **the way it indexes its banner**. The banner is a table of single
bytes, where an element index and a byte index are the same number, so one
subtraction reverses it. `LABELS` is a table of four-byte elements, and the
same subtraction reverses the elements *and the characters inside each
element* — so item 0's value was printed under item 29's name and the whole
report came out end for end. `0424` was on the console the whole time, under
the name `FRML`:

```
FRML=24     ← VIDL, Vendor ID Low
LIN2=04     ← VIDH, Vendor ID High
VBEN=01     ← PHYR, phy_ready
SEEN=00     ← RFAL, probe_fail
```

and `VBEN=01` is the tell that no amount of reasoning about ULPI would have
produced: `VBEN` is three constant zeros in the design, so a `01` under that
name cannot be the value that belongs to it. Two more were arithmetically
impossible — `FSTA=40` from a three-bit field zero-extended to eight, and
`LIN2=04` from a two-bit one.

The fix is one expression: the element counted from the low end, because the
concatenation is back to front, and the character counted from the high end,
because a string literal's first character is its most significant byte.
`every_label_of_the_target_hosts_report_names_the_value_beside_it` in
`tests/fpga_trellis.rs` drives the printer on its own — the nine probe slots
written through the simulator's array handle, the host's outputs forced to
distinct values, `con_in_ready` forced high because nothing in a simulation
enumerates the console — and compares the byte stream character for
character. Against the old index it prints `FRML=24 … VIDL=a5`, which is the
shape the board printed. What it does **not** cover is anything about the
bus, the probe or the enumeration: all of those are forced, so a host that
never read a register would print the same report. Only the part can say
otherwise, and now it has.

**What this cost, and the lesson that is not about Verilog.** The experiment
lined up to decide between a backend bug and an IP bug — put
`ip/usb/usb_device_ulpi`'s known-good link on the TARGET balls and see whether
*it* reads `0424` — was a good experiment and was never needed, because the
answer was already in the bytes on the console. Three of the printed values
were impossible for the names they carried, and that is checkable without a
board, without a build and without a hypothesis. **A report is a piece of
gateware and it can be wrong in a way that looks like the thing it is
reporting on.** The two checks this file will not weaken are about the
bitstream; this one is about the instrument, and the new test is the
instrument's.

### What is still not measured

- **Whether the TARGET port crosses DP and DM** the way AUX does. `LINE`
  reads SE0 because nothing is attached, and SE0 is the same on both wires.
  It takes a full-speed device pulling one of them up, which takes power on
  the socket. `usb_host_ulpi`'s `FS_LINE` stays a parameter.
- **Everything above the ULPI bus**: no token, no frame, no bus reset and no
  enumeration has run on silicon. `STGE=01` is stage 1 waiting for an
  attach and it will stay there while no VBUS switch is closed, which is
  `CLAUDE.md`'s rule and is what `VBEN=00` reports.
- **The bottom edge.** It is described from the same evidence and every ball
  of it on a caBGA-256 is one of bank 8's configuration pins, so nothing
  here drives them and nothing should.

The build that was loaded:

```
128583 configuration bit(s) set, 36 pad(s), 3168 lookup table(s), 1108
flip-flop(s) and 28 distributed RAM(s) configured, 36/197 io
routed 4408 of 4410 signal(s) with 63301 pip(s) over 67709 wire(s), and
every sink was walked back to its driver
all 128583 set bit(s) decode back through the database into 42037 arc(s),
6948 field(s) and 3335 word(s), with 0 unexplained, and the arcs they
select are exactly the 42037 the router chose
```

The "4408 of 4410" is the accounting the section below explains:
`Netlist::is_routable` wants a driver and a sink, and a pad driven by a
constant has neither, of which this design has three. The numbers differ
from the 128 408 bits and 3183 lookup tables quoted below because the label
index was fixed between the two builds and a different expression places
differently; nothing on the ULPI side changed.

## All four edges of the die, and the port that was blocked on two of them

`Edge::of` in `src/fpga/trellis` described the **top** and **right** edges
of this die and not the left or the bottom, and its own doc comment said
why: the rule is nextpnr's `get_pio_tile` / `get_pic_tile`, and "a rule
that has not been checked against a part produces a bitstream that loads,
asserts `DONE` and drives the wrong ball". The top edge had been checked
against all six of this board's LEDs and the right edge against its USER
button. That was a cost nothing had been willing to pay for, because
everything this flow had been asked to build lived on those two edges: the
oscillator and the LEDs on the top, the AUX ULPI transceiver and the button
on the right.

Then `ip/usb/usb_host_ulpi` arrived — a USB host for this board's **TARGET**
port — and every ball of that port is at column 0 of the caBGA-256 in
`iodb.json`, which is the left edge:

| What | Balls | Where |
|---|---|---|
| `target_phy` data | R2 R1 P2 P1 N3 N1 M2 M1 | column 0, rows 26-38 |
| `target_phy` clock, dir, nxt, stp, rst | T4 R3 T2 T3 R4 | column 0, rows 38-44 |
| the three VBUS switches onto TARGET A | K5 L1 L2 | column 0, rows 26-29 |
| `target_a_discharge` | K4 | column 0, row 29 |
| the TARGET D+/D- sniffer pair | N4 P3 | column 0, row 38 |
| *for comparison*, `aux_phy` | F16 … J13 | column **72** — the right edge |
| *for comparison*, the LEDs and the oscillator | E13 … C11, A8 | row **0** — the top edge |

so the flow refused the design, with the error that says exactly what is
wrong:

```
error: `tgt_data$io0` is constrained to package pin `R2`, which the
       architecture maps to no usable site
```

**All four edges are described now**, and 197 of 197 balls of this package
are pads where 120 were. The two new rules and the measurement each rests
on are in "Where a pad's bits are, and how that was established"; the three
things worth knowing up here are what it took, what it corrected and what
it cannot say.

**What it took was the vendor's own packer, asked in full.** All three of
Great Scott Gadgets' bitstreams use the TARGET port, so between them they
configure 58, 52 and 40 left-edge pads whose ball names the platform file
gives — against the single ball (M14) that settled the right edge. 477 bits
of the sixteen balls of the TARGET transceiver and the VBUS switches were
compared at absolute frame positions and all 477 agree, and **every**
`PIO<s>.BASE_TYPE` those files set anywhere in column 0 belongs to a pad
tile or a second-copy tile of a ball the map now names, with no orphan.
`tests/fpga_trellis.rs::what_lattices_own_packer_writes_for_a_left_edge_pad`
is that comparison, and `..._for_a_bottom_edge_pad` is the bottom edge's.

**What it corrected is the sentence this section used to end with.** The
left edge was expected to be "the right edge mirrored", and half of that was
wrong. The column mirrors — the tiles are at column 0 and the `CIB` that
ties the pad's data is one column *east* where the right edge's is west —
and **the rows do not**: the pad tile is one row *south* and the C/D second
copy two rows south on both long edges, because the die's rows do not
reverse. A mirror written in good faith would have put every left-edge
pad's bits in the tile of a different ball of the same edge, and it would
have decoded perfectly against itself. That is the whole reason the rule had
to be measured rather than derived.

The bottom edge turned out to be a different shape again, and it is
described because the evidence covered it: one tile per pad instead of two,
7 bits for an output and 10 for a bidirectional pad where every other edge
spends 6 and 8. **Every ball of that edge on a caBGA-256 is one of bank 8's
thirteen configuration pins**, so nothing here drives them and nothing
should; the pad section has the warning in full.

**What it could not say is that the ball is the ball.** A wrong tile rule is
exactly the failure that decodes perfectly and drives the wrong pin, so a
part is the only thing that could settle `X0Y38/PIOC` being the ball wired
to the transceiver's `DATA0` rather than the ball one row away — and **this
board has no LED, no button and nothing else observable on the left edge at
all**, which is why there is no "what a person should look for" section for
this round. "A design on left-edge balls, and what a board would have
added" names the two cheapest experiments that would close it.

**One of them has since been run, and it closes it.** The section above this
one has the console: a USB3343 on the TARGET port answering `0424` to its
vendor-ID pair, which needs the clock out of `T4`, the reset off `R4`, `dir`
on `R3`, `nxt` on `T2`, `stp` on `T3` and all eight data balls. Read that
section before trusting the paragraph above, which is kept as the state of
the evidence at the time the edges were described.

### What now builds, and what is still not on a part

`testdata/fpga/cynthion/target_ulpi_loopback.v` is the design the edge was
described for and is confined to it on purpose: the thirteen TARGET
transceiver balls and the three VBUS switches, with nothing but the
oscillator off column 0.

It places, routes and decodes:

```
17 pad(s), 74 lookup table(s), 26 flip-flop(s)
routed 112 of 112 signal(s) with 1217 pip(s) over 1329 wire(s)
all 2729 set bit(s) decode back through the database into 751 arc(s),
265 field(s) and 74 word(s), with 0 unexplained, and the arcs they select
are exactly the 751 the router chose
```

**And so does the design this section was written about.** The same
`testdata/fpga/cynthion/usb_host_target.v` that this section used to quote
an error for — the USB host behind the TARGET transceiver, with the AUX
console beside it — now builds with its TARGET balls **constrained**, which
is the thing it could not do:

```
128408 configuration bit(s) set, 36 pad(s), 3183 lookup table(s), 1108
flip-flop(s) and 28 distributed RAM(s) configured, 36/197 io
routed 4423 of 4425 signal(s) with 63153 pip(s) over 67576 wire(s), and
every sink was walked back to its driver
all 128408 set bit(s) decode back through the database into 41882 arc(s),
6918 field(s) and 3350 word(s), with 0 unexplained, and the arcs they
select are exactly the 41882 the router chose
```

The "4423 of 4425" is the same accounting as before — `Netlist::is_routable`
wants a driver and a sink, and a pad driven by a constant has neither, of
which this design has three. The bit count differs from the 128632 this
section used to quote because that run left the TARGET balls unconstrained
and this one does not: sixteen more pads are configured and the placement
is not the same one. Nothing in that design or its constraints was changed
for this; it is the same two files, built against a backend that now knows
where a left-edge pad's bits are.

**One consequence is worth stating on its own**, because it is about power
rather than about pins: the three bidirectional VBUS switches onto the
TARGET A node are on this edge too, so until now **no design this flow
could build was able to put power on the TARGET A socket**. It can now, and
`CLAUDE.md`'s rule is the one that decides what happens next: enable no VBUS
switch. Nothing built here closes one. `target_ulpi_loopback.v` drives all
three to a constant zero, which is off, and that is also what makes four of
its output pads a `CIB` tie rather than a route;
`testdata/fpga/cynthion/usb_host_target.v` has a `VBUS_AUX` parameter that
would raise `aux_vbus_en`, defaulted to **0**, and its own header says to
read its account of that node before changing it.

`ip/usb/usb_host_ulpi/README.md` §9 is the same account from the block's side.
One thing about it has not changed: `pins partial` is still set for
`ecp5-12f-CABGA256` in `src/fpga/devices/ecp5.dev` with only ten balls
listed, so every TARGET ball still draws a cosmetic `F0202` warning about
Reticle's own partial list — the pin map the placer uses comes from
`iodb.json`. The other thing — that nothing about any of this had been on a
part — has changed, and the section above this one is what changed it.

## A control wire is a budget, and a distributed RAM spends one of the tile's two

The round below this one modelled a distributed RAM's **lookup tables** and
got them exactly right, down to the six bels it takes of the tile's eight. It
did not model the RAM's **control wire**, and a design that used many of them
therefore placed and then would not route:

```
routing did not converge: 1 node(s) are still oversubscribed after 40 iteration(s), worst at X24Y3/LSR1 (2 signals)
```

That message names a tile and not a cause, which was the other half of the
problem. Both halves are fixed: the placer refuses the arrangement now, and
when a control wire is oversubscribed anyway the router says what contended
for it and what the two ways out are.

### Which wire, and it is per tile

Read out of `PLC2`'s own `bits.db` rather than reasoned about:

| Record | What it says |
|---|---|
| `.mux MUXLSR0` … `.mux MUXLSR3` | four muxes, one per slice, each choosing between exactly two sources: `LSR0` and `LSR1` |
| `.fixed_conn LSR<s>_SLICE MUXLSR<s>` | a flip-flop's reset pin is its slice's mux output and nothing else |
| `.fixed_conn WRE0_SLICE LSR1`, `.fixed_conn WRE1_SLICE LSR1` | a distributed RAM's write enable is joined **straight to `LSR1`**, with no mux to choose with |

So it is `LSR1`, it is **per tile and not per half-tile or per slice pair**,
and it does not depend on which slices the RAM occupies — it always occupies
A, B and C, and both of its write-enable wires land on the same `LSR1`. Two
wires for four slices is a budget of two distinct reset signals per tile, and
a RAM has already spent one of them.

The **clock** is the same shape: `.mux MUXCLK0`…`MUXCLK3` over `CLK0` and
`CLK1`, and `.fixed_conn WCK0_SLICE CLK1`, `.fixed_conn WCK1_SLICE CLK1`.

The **clock enable is not**, and that is worth stating because it looks like
it should be. `CE0`, `CE1`, `CE2` and `CE3` are **four** wires with
`.fixed_conn CE<s>_SLICE CE<s>`, one per slice, each driven by a mux of its
own — and a `TRELLIS_DPR16X4` has no enable pin at all. A distributed RAM
contends for no clock enable and the clock enable needs no rule. What it does
need is the rule it already had: the *two flip-flops of one slice* share
`CE<s>_SLICE`, which is the fifth shared-resource case at the end of this
file.

### What `ecppack` does with the wires a RAM leaves, in full

The question is not "does a RAM share a tile with a flip-flop" — the round
below measured that it does, in 79 of the 111 distributed RAMs of
`analyzer.bit`, `selftest.bit` and `facedancer.bit`. The question is **what
those flip-flops do with the two wires the RAM did not leave them**, and
their own `MUXLSR<s>` and `MUXCLK<s>` settings answer it. All of this is
decoded by `what_lattices_own_packer_writes_for_a_distributed_ram`, which
asserts every number below:

| | analyzer | facedancer | all three |
|---|---|---|---|
| Distributed RAMs | 22 | 89 | **111** (`selftest.bit` has none) |
| …whose tile also holds a flip-flop | 9 | 70 | **79** |
| Slices of a RAM's tile driving a reset | 1 | 50 | **51** |
| …of those on `LSR0` | 1 | 50 | **51** |
| …of those on `LSR1`, the wire the RAM spent | 0 | 0 | **0** |
| Slices of a RAM's tile taking a clock | 17 | 149 | **166** |
| …of those on `CLK1`, the RAM's own write-clock wire | 13 | 145 | **158** |
| …of those on `CLK0` | 4 | 4 | **8** |
| Slices of a RAM's tile with a clock enable | 1 | 113 | **114** |

The two rows in bold say opposite things and both are right. A **reset** is a
signal of its own, so it takes the free wire: not one of 51 goes on `LSR1`. A
**clock** is usually the *same net* as the write clock, so it takes the RAM's
own wire: 158 of 166 go on `CLK1`. A wire that already carries a signal is
free to the signal it carries, and expensive to every other one.

### The weak rule is the right rule, and the strong one would have been wrong

Two rules were available:

- **strong** — a flip-flop may not share a tile with a distributed RAM (or,
  weaker but still strong, not one with a reset);
- **weak** — a flip-flop needing a *different* signal on the wire may not
  share it.

The strong rule is stricter than what `ecppack` itself writes, which by this
project's standard makes it a wrong rule: 79 RAM tiles with flip-flops in
them, 51 slices driving a reset and 158 sharing the write clock are the
counter-examples, and they are in files this tree already verifies byte for
byte. So the weak one it is, and a flip-flop with **no** reset — which is
most of them — is unconstrained either way.

### How the placer knows, without anything naming a wire

`SiteRules` in `src/fpga/place.rs` already had two kinds of rule: pins of two
bels that are *one wire* must want the same signal, and bels a cell makes
*unusable* are excluded. A control wire is a third kind and it is a **budget**
rather than either.

For every pin of every bel, the placer walks backwards from the pin's wire
while every way into the wire it is standing on comes from inside the tile,
and stops at the frontier. The set it stops at is a **cut**: every path from
any driver to that pin crosses exactly one of its wires. For an ECP5:

| Pin | The walk | The pool |
|---|---|---|
| a flip-flop's `rst` | `LSR<s>_SLICE` → `MUXLSR<s>` → stop, because `LSR0` and `LSR1` are fed from the global network and from neighbouring tiles | `{LSR0, LSR1}` |
| a distributed RAM's `we` | `WRE<n>_SLICE` → stop at `LSR1` | `{LSR1}` |
| a flip-flop's `clk` | `CLK<s>_SLICE` → `MUXCLK<s>` → stop | `{CLK0, CLK1}` |
| a RAM's `wclk` | `WCK<n>_SLICE` → stop | `{CLK1}` |
| a flip-flop's `en` | `CE<s>_SLICE` → stop at `CE<s>` | `{CE<s>}`, one per slice |
| a lookup table's input | `A0_SLICE` → `A0`, whose own mux has sources in four neighbouring tiles, so the walk stops there | `{A0}`, which nothing else in the tile draws on, so it is dropped |

A pool no other bel of the tile draws on is dropped, which is every ordinary
interconnect wire on every family — so on the 7 series, on Gowin and on a
tile with one bel there are no pools at all and the check costs one
`is_empty`. Nothing in `place.rs` names `LSR`, or a slice, or an ECP5.

Legality is then a **system of distinct representatives**: each distinct
signal wanting a pool needs a wire of its own, and the answer is a bipartite
matching between signals and wires, run over the sites of one tile whenever a
move touches it. Two pins wanting the *same* signal cost one wire between
them, which is exactly how a flip-flop comes to share `CLK1` with the RAM
beside it; a pin with no signal on that role — a flip-flop with no reset —
asks for nothing.

Two things fall out of this that were not the point and are worth having.
The rule is not about distributed RAM: **three distinct reset nets in one
logic tile** were always illegal and were always placed, and now they are
not. And it is not about the ECP5: any family whose architecture has the
shape gets it.

### What it cost, measured

`testdata/fpga/ecp5/lutram_reset_64.v` is the reproducer — four 64-word
FIFOs, 32 `TRELLIS_DPR16X4`, 226 lookup tables, 88 flip-flops and **two**
reset nets — and `a_distributed_ram_and_two_reset_domains_share_a_die` is the
test. Without the rule it is the failure at the top of this section; with it
the design places, routes, and **all 15057 set bits of its image decode**
back through the database into a feature they name, with **nothing
unexplained**, and the 4393 arcs those bits select are exactly the arcs the
router chose.

Three FIFOs is not enough and sixteen RAMs at depth 32 is not enough: both of
those build without the rule, because the placer has room to keep the two
domains apart by accident. That is worth knowing about the file — it is at
the size where accident stops working.

Placement quality, with the strong rule measured on the same design rather
than argued about:

| | weak rule | strong rule |
|---|---|---|
| Logic tiles occupied | **57** | **57** |
| Pips | 6790 | 6698 |
| Flip-flops in a RAM's tile | 41 of 88 | 0 |

**So on this design the weak rule buys nothing in tiles, and saying so is the
honest answer.** This design is bound by its 226 lookup tables, and a RAM's
tile keeps two of those either way, so the flip-flops the strong rule evicts
land in tiles the lookup tables had already claimed. What the weak rule buys
is **capacity** — the eight flip-flop sites of every tile that holds a RAM,
24288 of them on this part — which a design bound by its *registers* spends
and this one does not. The design this defect was found on has about 530
distributed RAMs and a thousand flip-flops; the strong rule would have
sterilised 4240 flip-flop sites, a sixth of the part's registers, to no
purpose.

### The error says what it means now

When a node that will not converge turns out to be a wire several of a tile's
bels have to agree about, `RouteError::Congested` says so. The test for
"is this a control wire" names nothing: it asks whether two or more bels of
the node's own tile have a pin reachable only through it.

This is the real thing, from the reproducer with the placer's rule switched
off — nine bels because a logic tile has eight flip-flops and one
distributed RAM:

```
error: routing did not converge: 1 node(s) are still oversubscribed after 40 iteration(s), worst at X24Y3/LSR1 (2 signals)
  X24Y3/LSR1 is a control wire of X24Y3: 9 of that tile's bels have a pin that can only be
  reached through it, and 2 signals were routed onto it — `mem$we$13` for `b1.mem$dpr0_1_0`'s
  `we` pin and `rst_a_n` for `a0.g_registered.rd_data_q$ff$ff7`'s `rst` pin. A wire carries one
  signal, so there are two ways out and a placement has to take one of them: give the cells that
  share the tile the **same** signal on that pin, or put the ones that disagree in **different
  tiles**. The placer rejects this arrangement before it is made (`SiteRules` in
  `src/fpga/place.rs`), so a design that gets here has found a control wire the architecture does
  not describe yet — say which one, it is a fabric fact and not a budget.
```

The last sentence is the actionable one, and it is what the message that
started this section could not say: the placer refuses this arrangement
before it is made, so a design that reaches the message has found a control
wire the architecture does not describe yet.

### One hazard the weak rule creates, and it is refused

Letting a flip-flop share a RAM's wire has a consequence the strong rule
would not have had. `CLK1.CLKMUX = INV` and `LSR1.LSRMUX = INV` invert the
**wire**, not the flip-flop, and a RAM's write clock and write enable hang
off those wires with no mux of their own. A flip-flop on `CLK1` asking for a
falling edge would therefore invert the write clock of the RAM beside it, and
the memory would write on the wrong edge with every structural check
passing — the exact shape of defect this file exists to catch.

`configure_registers` refuses it, naming the cause and the two ways out
(move the polarity into logic, or keep the flip-flop out of a RAM's tile).
`SRMODE = ASYNC` is **not** refused and should not be: it decides whether a
*register* takes its reset on the clock edge, so a RAM in the tile is not
affected by it.

### What a board would have added, and the cheapest experiment

Nothing in this section needed one, and that is the honest summary: the
constraint is a fabric fact in `bits.db`, the evidence for the weak rule is
in vendor bitstreams this tree already verifies, and the reproducer's
bitstream decodes completely off the part. But two things rest on reading
rather than on measurement, and both are the same experiment:

- **that a flip-flop may take its reset from `LSR1` when that wire already
  carries a RAM's write enable.** The model allows it because the wire
  carries one signal either way; `ecppack` never does it, in 0 of 51 cases,
  so there is no vendor artefact to agree with. It cannot arise in an
  ordinary design — a write enable is not a reset — which is why it has not
  been chased.
- **that a flip-flop clocking off `CLK1` beside a RAM really clocks
  together with it.** This one `ecppack` does 158 times, so it is as well
  evidenced as anything in this file; what no file can show is the timing.

The cheapest experiment is one design and it settles both. Take the packet
buffers of `usb_bulk_ep`, which are already eight `TRELLIS_DPR16X4` on a
Cynthion with a host reading every byte back, and add an `rloc` macro that
puts a resettable flip-flop in one RAM's own tile with its reset tied to the
RAM's write-enable net and its output XORed into a byte endpoint 1 returns.
If the bytes still come back identical and the flip-flop reads the reset when
the RAM is written, both readings become measurements. It needs no new
gateware and no new instrument: the trace path `tests/usb_loopback.rs` uses
is already there. That is how "A word written does come back" closed the gap
the round below it named, and it is the same shape of experiment.

## A distributed RAM is on the fabric, and it is three slices held together by one bit

`ip/memory/fifo_sync` could not be placed on an ECP5 at any depth. The
message was exact and it was not about a budget:

```
error: the design needs 2 `lutram` site(s) and the part has 0
```

`synthesize_for` succeeded, which is why the library test passed and nobody
noticed for several rounds. The round before this one established where the
fault was not: not a missing `count` on `ecp5.dev`'s `TRELLIS_DPR16X4` line,
because a `.dev` `count` is a resource budget and `fpga::place` counts
`RoutingGraph::sites`, which come from the `Arch` — and an ECP5's `Arch` is
built by `src/fpga/trellis`, whose bels were `lut`, `ff` and `io` and nothing
else. There was no distributed-RAM site on the die for the placer to find.

There is one now, one per logic tile, and `ip/memory/fifo_sync` places,
routes and comes out as a bitstream every bit of which decodes, at depths
16, 32 and 64.

### It was expected to be the weak case, and it is the strong one

This file distinguishes two kinds of claim: what a **vendor artefact** proves,
and what the database plus nextpnr's stated intent merely suggest. A slew rate
and a constant driver are the first kind — read out of Great Scott Gadgets'
own `ecppack` output at absolute frame positions. A distributed RAM looked
like the second, because nothing had ever looked for one in those files.

Somebody looked. **`analyzer.bit` has 22 distributed RAMs and
`facedancer.bit` has 89**; `selftest.bit` has none. 111 of them, in files
this tree already verifies byte for byte, and
`what_lattices_own_packer_writes_for_a_distributed_ram` reads every one back.

Two of the rows below are that test's findings and not nextpnr's, and both
of them decided code rather than prose.

### Everything `ecppack` writes for a distributed RAM, in full

| | |
|---|---|
| How many | 22 in `analyzer.bit`, **none** in `selftest.bit`, 89 in `facedancer.bit` |
| `SLICEA.MODE`, `SLICEB.MODE`, `SLICEC.MODE` | `DPRAM`, `DPRAM`, `RAMW` — and in all 111 tiles **all three or none** |
| Which bit | **`F50B11` of the `PLC2`, one bit**, because Project Trellis' database gives all three settings that same bit. Set in their files at the frame this flow computes for it |
| `SLICEA.K0/K1.INIT`, `SLICEB.K0/K1.INIT` | the contents, one 16-bit word per bit of the four-bit word — and sixteen **zeros** in all 111: every distributed RAM on this board starts empty |
| `SLICEC.K0.INIT`, `SLICEC.K1.INIT` | sixteen zeros each, in all 111. The `RAMW` slice's two lookup tables hold nothing and are written anyway: 32 bits that nothing reads |
| `SLICEA.WREMUX` | **never written**, in any of the three files. `WRE` is the default and costs nothing |
| `CLK1.CLKMUX` | never written either. `CLK` is the default, and the write clock's polarity is the only thing it could say |
| `SLICE<l>.CCU2.INJECT1_<n>` | nextpnr writes `_NONE_`, which means "no bits"; a bitstream assembled from a zeroed bitmap already has them clear |
| An unused lookup-table input | `SLICE<l>.<X><n>MUX = 1`. It cannot arise: a `DPRAM` slice uses all four inputs as its read address |
| The write clock | tile net **`CLK1`**, and in all 111 it arrives on a **global clock network** (`G_HPBX<n>00`). This flow did not, until that row was turned into a check — see below |
| The write enable | tile net **`LSR1`**, and in all 111 it arrives on **general routing** |
| Slice D | **nothing, and it is still in use**: ordinary logic in 18 of analyzer's 22 tiles and 82 of facedancer's 89 |
| The flip-flops of slices A, B and C | **nothing, and they are still in use**: 8 of analyzer's tiles and 60 of facedancer's have one. `DPRAM` mode takes a slice's lookup tables, not its registers |

So a distributed RAM with no initial contents costs **97 bits**: one for the
mode, 64 for the four zeroed content words, 32 for the `RAMW` slice's two.
That is what this flow writes, and it is `ecppack`'s number too.

The `INIT` bits are `!`-marked in `bits.db`, so a content bit of *zero* is a
bit **set** in the bitstream and all-ones is free. An empty RAM is the
expensive case, which is the opposite of the intuition and is why 97 rather
than 1.

### The slice relationship, and why ignoring it is worse than not placing

`SLICEA.MODE = DPRAM`, `SLICEB.MODE = DPRAM` and `SLICEC.MODE = RAMW` are one
bit. There is no way to ask for two of the three, and `SLICED.MODE` has
neither value at all. So:

- a `TRELLIS_DPR16X4` is **slices A, B and C of one logic tile**, and a tile
  holds exactly one;
- slices A and B *are* the RAM. `WD0A_SLICE` takes `WD0` and `WD0B_SLICE`
  takes `WD2`, so slice A holds bits 0 and 1 of the word and slice B bits 2
  and 3, and their four `K<n>.INIT` words are the contents;
- slice C *is* the write-port register. Its two lookup tables are given up:
  `WAD[0..3]` arrive on the first one's `D`, `B`, `C`, `A` inputs and
  `DI[0..3]` on the second's `C`, `A`, `D`, `B`, and its `WADO<n>C_SLICE` and
  `WDO<n>C_SLICE` outputs reach slices A and B over `.fixed_conn`s **inside
  the tile**. There is no routable wire between a distributed RAM's halves;
- slice D is untouched.

That is six of a logic tile's eight lookup tables. Not eight, and not the
whole tile — and that is the part the reference bitstreams decided. A model
that blocked the tile would have been wrong in a way no test of this flow's
own output could have caught, because this flow would simply have used fewer
tiles and everything would still have decoded. `ecppack` puts ordinary logic
in slice D of 100 of the 111 and a flip-flop in the RAM's own slices in 68 of
them, so the exclusion is exactly six bels wide.

A placer that ignored it would produce a bitstream that loads, asserts `DONE`
and computes nothing: a lookup table's truth table and a distributed RAM's
contents are the **same `INIT` words**, so the second cell to be written
silently replaces the first.

This is the **sixth** place on this part where two features share one
resource, and it is handled the way the fifth was — as a legality constraint
the placer reads off the architecture, not as a thing to remember.
`BelDecl::blocks` names the bels of one tile a cell makes unusable,
`SiteRules` in `src/fpga/place.rs` makes it symmetric, and the legaliser and
every annealing move honour it. Nothing in `place.rs` names a slice.

### Two things the routing graph could not express, and now can

**A pin can be several wires.** A distributed RAM's read address is one
address read by **four** lookup tables, so `raddr0` is the `D` input of all
four of them — four separate pieces of metal. `BelDecl::pins` may now repeat
a role, and the router treats every wire of a *sink* role as a sink of the
same net, which is what the fabric says they are. A driver role still names
one wire: a cell output that drove two would be two nets. `wclk` and `we` are
two wires each for the same reason, slice A's and slice B's, both fixed to
`CLK1` and `LSR1`.

**A constant on a RAM's input needs a driver.** An unrouted lookup-table
input on this family is **tied high** (`SLICE<l>.<X><n>MUX = 1`), so an
address bit left as a constant zero reads as a one and the memory is
addressed one word off — which nothing structural would notice.
`techcells::drive_constant_data` was written for a flip-flop's data pin and
now covers every input of a `lutram` too, giving them the shared constant
lookup table that "The constant is built now" describes. That is nextpnr's
`pack_constants` applied to one more pin, and it changed the 7 series as
well: `logicram_xc7` now has one `LUT6` making a zero instead of 32 literal
constants on the unused address bits of its eight `RAM64X1D`s.

### A defect the vendor's own files found: the write clock was on data wires

Two rows of that table are not description, they are what a check was built
out of — and one of them failed the first time it ran.

`ecppack` puts a distributed RAM's write clock on a **global clock network**
in all 111 cases: `CLK1 <- G_HPBX<n>00`, every time. This flow refuses a
*flip-flop* whose clock arrived through general routing, because a clock off
data wires routes, verifies, configures and has skew nobody has a model for.
Extending that check to a `lutram`'s `wclk` took four lines, and it
immediately said no:

```
depth 16: ["fifo.mem$dpr0_0_0 on X13Y2/DPR16X4", "fifo.mem$dpr0_0_1 on X16Y2/DPR16X4"]
```

Dumping the net showed it exactly: every one of the design's eighteen
flip-flops had `CLK0 <- G_HPBX0000`, and the two RAMs had
`CLK1 <- V00B0100` and `CLK1 <- V00B0000` — vertical **data** wires, coming
straight off the clock pad without going through the `DCCA` buffer at all.

The cause is an ordering one and it was there before the RAM existed.
`primitives::Mapper::clock_buffers` finds a clock's sinks by looking for
`CellKind::Dff` and for a clocked `MemRdPort`/`MemWrPort` — but `block_rams`
runs **first**, so by the time the buffer is inserted the memory is no longer
a `MemWrPort`: it is a `TRELLIS_DPR16X4` blackbox with a `WCK` port, which
that pass did not recognise, did not count towards the buffer's fanout and
did not rewire. It does now, by primitive name out of the device file, and
`CLK1 <- G_HPBX0000` at every RAM tile afterwards.

Worth saying plainly: **nothing structural was wrong before the fix.** The
design placed, every signal routed, every sink walked back to its driver,
every bit of the image decoded, and the arcs matched the router's exactly.
What told the two apart was asking what the vendor writes and then checking
it, which is the same thing that found the five defects before this one.

### The read address is scrambled, and getting it wrong is silent

`RAD[0]` is the `D` input, `RAD[1]` the `B`, `RAD[2]` the `C` and `RAD[3]`
the `A`. Nothing in `bits.db` says so — it is nextpnr's `dram_to_comb`, and
the same permutation has to be applied to the contents, which is what
`trellis::dpram_init_word` is. A flow that ignored it would place, route,
decode and read every word from the wrong address.

That is the one part of this that rests on nextpnr alone rather than on the
vendor's bitstreams: all 111 of their RAMs are empty, so the permutation
leaves no trace in them. A bitstream with non-zero contents would pin it, and
`what_lattices_own_packer_writes_for_a_distributed_ram` would notice one
arriving — it asserts the words are zero rather than skipping them.

The half of it that reaches a **run-time** address is settled in silicon now:
`usb_bulk_ep`'s packet buffers are two 64-deep distributed RAMs that a real host
writes and reads through endpoint 1, and every byte comes back from the address
it was written to. "A word written does come back" below has the measurement.
What is still on nextpnr's word alone is the `INITVAL` side — a word the
*bitstream* places rather than the fabric — because this flow builds no RAM with
initial contents.

### What places now, and what it costs

`ip/memory/fifo_sync` with `WIDTH = 8`, through `synthesize_for`, `place`,
`route` and `stream` on an LFE5U-12F in caBGA-256, with all of its ports on
top-edge balls:

| Depth | `TRELLIS_DPR16X4` | Logic tiles they take | Bits they cost |
|---|---|---|---|
| 16 | 2 | 2 | 194 |
| 32 | 4 | 4 | 388 |
| 64 | 8 | 8 | 776 |

At each depth: every signal routed, every sink walked back to its driver,
every flip-flop's clock **and every write clock** on a global network, no
bit an arc needs clear set by something else, **every set bit of the image
decoding back through the database into a feature it names with nothing
unexplained**, and the arcs those bits select exactly the arcs the router
chose. The "every bit decodes" check has found five real defects in this
backend and it is not weakened anywhere here; the distributed RAM adds no
unexplained bit.

`a_distributed_ram_places_routes_and_every_bit_of_it_decodes` is that test,
and it also asserts the exclusion from both sides: no lookup table shares a
tile with a RAM except on slice D, and every one of the six bels a RAM
consumes is empty in every tile that holds one.

### A word written does come back, and half of the gap is closed

**This was written as an open gap and it is worth keeping the question it
asked.** Nothing in the round that modelled a slice's distributed-RAM mode had
run on a part: the database, the reference bitstreams, the router, the "every
bit decodes" check and the simulator were all off the part. What none of them
could tell you was whether the silicon stores and returns a byte — whether the
write address really arrives on `WADO<n>C_SLICE`, whether `WCK` is the edge
this flow thinks it is, and whether `dpram_init_word`'s permutation is the
right way round rather than merely self-consistent. The contents this flow
writes are all zeros, so every word reads the same, and a permutation that was
wrong in a self-consistent way would have been invisible to every check in this
tree.

**The experiment has been run, by the second of the two routes named below.**
`usb_bulk_ep`'s two packet buffers are arrays now — eight `TRELLIS_DPR16X4`
each, 64 words of 8 bits — and `testdata/fpga/cynthion/usb_ulpi_device.v` puts
both of them on the AUX port's endpoint 1, where `tests/usb_loopback.rs` reads
every byte back. That is the first non-trivial contents this project has put in
a distributed RAM on silicon. Loaded on a Cynthion, 256 bytes went out in
packets of 64, 63, 8, 5 and 1 byte and came back **byte-identical**, through one
RAM on the way in and another on the way out, at all 64 addresses of each, with
the write pointer and the read index counting independently.

So: the silicon stores and returns a byte, the write address arrives where this
flow puts it, `WCK` is the right edge, and the write-port and read-port address
decodings agree with each other at every address — including across the four
16-word banks the depth expansion builds, which a wrong bank select on one port
would have scrambled. `docs/ip-library.md`'s "The endpoint buffers are arrays
now" has the numbers and both bitstreams.

**What is still open is `dpram_init_word` itself.** That function is about
*initial* contents, and this design has none: `fpga::primitives` still declines
to lower a memory with initial contents, so every RAM this flow builds still
starts empty. Nothing above says that an `initial` block's word *n* ends up at
address *n*, only that a word this design **writes** at address *n* is read back
from address *n* — and those are two different claims, because a write goes
through the fabric's own address wires and an initialiser goes through
`dpram_init_word`. The remaining experiment is the one this section named first:
a design whose RAM has contents the bitstream puts there, read back where it can
be seen. It needs `fpga::primitives` to lower an initialised memory before it
can be built at all.

## The constant is built now, and the vendor's own bitstreams said how

The round before this one found a flip-flop whose data input is the constant
zero, coming up as a **one**, and **refused** to write it. Refusing was honest
and it was a capability regression: `reg [2:0] stage` for four values is legal
Verilog and legal silicon, and a compiler should build it. It builds now, and
what it builds was read out of Lattice's own bitstreams rather than reasoned
about.

### What `ecppack` writes for a constant on a flip-flop's data pin, in full

The question is the one this file is built on — "what does the vendor write,
**in full**?" — and the three Great Scott Gadgets bitstreams this tree already
verifies against can answer it, because a flip-flop's data wire is one of the
few things a bitstream states outright: `SLICE<l>.REG<n>.SD = 0` says the data
comes off the fabric's `M<z>` wire, and whether anything is routed to that wire
is a question about the file. So every one of their flip-flops was asked, and
where the answer was yes the arcs were walked **backwards** to whatever drives
them.

| | `analyzer.bit` | `selftest.bit` | `facedancer.bit` |
|---|---|---|---|
| Flip-flops taking data from the fabric (`REG<n>.SD = 0`) | 1135 | 215 | 3132 |
| **Of those, with nothing routed to their `M` wire** | **0** | **0** | **0** |
| Fed by a lookup table with `INIT` all **zeros** | 12, from `SLICEA.K0` at X38Y27 | — | 8, from `SLICEA.K0` at X11Y6 |
| Fed by a lookup table with `INIT` all **ones** | 18, from `SLICEA.K0` at X5Y27 | — | 32, from `SLICEA.K0` at X64Y8 |
| `SLICE<l>.M<n>MUX` written anywhere | never | never | never |

Four constant drivers over three files, and they are the same cell four times:
a lookup table in `SLICEA.K0`, `MODE` left at `LOGIC`, **all four inputs tied
high** (`A0MUX`/`B0MUX`/`C0MUX`/`D0MUX = 1`), one `INIT` of all zeros or all
ones, and an output routed across the whole die — X38Y27's reaches 891 sinks
and X11Y6's 965, from X2 to X70 and Y5 to Y29. That is **one driver per
constant per design, shared**, which is nextpnr's `pack_constants` exactly: a
`$PACKER_GND` and a `$PACKER_VCC` `LUT4` with `INIT` 0 and 0xFFFF, made once
and routed everywhere. `selftest.bit` needs no constant on a flip-flop and has
none, which is the case worth stating too: the vendor does not write a driver
nothing asks for.

Two things in that table are more interesting than the headline.

**A constant one is free and a constant zero costs sixteen bits.**
`SLICE<l>.K<n>.INIT` defaults to all ones in `bits.db` — every group is a
single `!F<x>B<y>` — so the `$PACKER_VCC` LUT leaves *no* `INIT` bits in the
image at all and is visible only by its four input ties and its routed output.
Which is why the all-ones case had to be found by walking arcs rather than by
looking for a pattern: there is no pattern to look for.

**There is no `CIB` tie for an `M` wire, and there is a slice one nobody
uses.** The previous round's note — "neither a constant driver nor a `CIB` tie
for `M` is declared here" — is now checked against the database rather than
asserted: the `CIB` tile declares `CIB.J<x><n>MUX` tie-offs for the lookup
table's `A`–`D` inputs, for `JCE0`–`JCE3`, `JCLK0`/`JCLK1` and
`JLSR0`/`JLSR1`, and **nothing for `JM<n>`**. The `PLC2` tile itself, though,
declares `SLICE<l>.M<n>MUX` with a `1` value worth two bits, which would tie a
flip-flop's data wire high for two bits and no lookup table at all. Not one of
the three reference bitstreams sets it anywhere. So it is left alone here too:
it would only ever serve a constant *one*, it is two bits against a LUT's four
tie bits plus a route, and nothing has ever exercised it. The test asserts the
vendor's silence about it, so if that ever changes the assertion says so.

### What this flow emits, and what it costs

`techcells::drive_constant_data` runs at the end of the device-cell mapping —
the same pass that turns a generic `dff` into a `TRELLIS_FF` and inserts one
inverter per net whose polarity the family lacks — and gives every flip-flop
whose data input is a constant a lookup table to take it from: **one per
constant, shared**, with a truth table that is all zeros or all ones and
therefore ignores every input, so the value does not depend on what an
unrouted input reads as. It is not ECP5 code: the pass asks the device file
which primitive is the LUT and which port is the flip-flop's data pin, so an
iCE40 gets the same thing (nextpnr-ice40's `pack_constants` builds the same
`SB_LUT4`), and the fourteen library blocks that have such a flip-flop each
grew by exactly one LUT on both families in `docs/ip-library.md`'s footprint
table.

**The spare lookup table cannot collide with one the placer used, because this
does not choose one.** The driver goes into the netlist as an ordinary `lut`
cell and `fpga::place` allocates a site for it like any other; a design with
no site left is the placer's `NotEnoughSites` error with the message it always
had. Nothing here reads or reserves a site.

What it costs, measured on `testdata/fpga/cynthion/wide_state.v` — three bits,
four values, so `state[2]` is the constant — by building the same design with
the pass off and diffing the two images bit by bit:

| | Without a driver | With one |
|---|---|---|
| Set bits | 171 | **199** |
| Arcs that cost bits | 45 | 47 |
| `.config` words | 4 | 5 |
| Enumerated fields | 34 | 38 |
| Bits **unexplained** | 0 | 0 |

**Twenty-eight bits, every one of them accounted for, and not one bit of the
old image moved** — the raw bit difference is 28 added and **0 removed**:

- **16** for `SLICEA.K0.INIT = 0000000000000000` at X40Y2, one bit per entry of
  a truth table that is all zeros;
- **8** for the four two-bit ties `SLICEA.{A,B,C,D}0MUX = 1` that hold its
  inputs high;
- **4** for the two arcs that carry its output to the flip-flop's data wire,
  `H00L0000 <- F0` and `M1 <- H00L0000`, two bits each.

The constant landed in the same *tile* as the flop it feeds and went out into
the interconnect and back, which is what this flow does with every lookup
table: it never packs one with a flip-flop, so `SD` is always `0` and the data
always arrives over the fabric.

On the design this all came from, `usb_ulpi_device.v`, the accounting is a net
figure rather than an itemised one, because one more cell moves the placer's
assignment and 7240 of its arcs change: 26 916 set bits and 728 lookup tables
before, **27 015 and 729** after, 1036 signals to 1037, and **0 unexplained
either way**. The constant it now builds is a *one* — `usb_ulpi_link`'s
`rst_q`, the set-once register that releases the transceiver's reset pin, whose
data input is the literal `1'b1` — so it costs no `INIT` bits, which is why the
word count is 728 in both. **That register has worked on this board since the
first ULPI bitstream by accident**: the untied wire read as a one and the one
was what it wanted. It is built now instead of being right by luck.

### What is still refused, and what a board would have added

`configure_registers` still refuses a flip-flop whose data input nothing
drives, and the case is narrower than "nothing drives the data input": a
constant is built, so what is left is a data pin with **nothing at all** on it,
a pin whose constant is an `x` or a `z` — there is no wire value for either —
or a flow run with `FpgaOptions::device_cells` off, or on a device whose file
declares no LUT to make a constant out of. The message says which of those it
is. A pin tied *high* is still allowed through, because the untied wire is a
one and the flop loads what the design asked for; after this pass nothing
should reach that path, and it stays so that a netlist built without the pass
is not refused for a case that does work.

**Both halves of this are now confirmed on a part.** The round that built the
driver went nowhere near the board — it was in use — so what follows was
measured afterwards, in two goes, by building `usb_ulpi_device.v` from this
document's own commands and loading it.

A **constant one** holds in silicon, and the observation is stronger than a
lamp. `usb_ulpi_link`'s `rst_q` is the constant-one flip-flop of that design,
and what it drives is `ulpi_rst_n`, the transceiver's reset pin. Had the new
lookup table handed it a zero, the transceiver would stay in reset and no host
would see anything at all. The part enumerates as `1209:0001` and loops 256
bytes through endpoint 1, so the flop loads the one the design asked for —
through a driver this time rather than by the accident above. 43 299 bits, 0
unexplained.

A **constant zero** is now confirmed too, in the next section, which is the
experiment this paragraph used to ask for.

### A constant zero is on a part, in a byte a host reads back

`usb_ulpi_device.v` holds one flip-flop, `zero_probe`, whose data input is the
literal `1'b0`, and the byte endpoint 1 hands back is XORed with it. Right, the
XOR is with zero and the loopback is **byte-identical**, so the
already-written `tests/usb_loopback.rs` passes unchanged; wrong, and **every
returned byte is its own complement**, which nothing else in that design can
do. It has been run both ways.

```
$ cargo test --features program -- --ignored usb_endpoint_one_loops
[00, 01, 02, 03, 04, 05, 06, 07] -> [00, 01, 02, 03, 04, 05, 06, 07]
[de, ad, be, ef, ff] -> [de, ad, be, ef, ff]
[5a] -> [5a]
[00, 25, 4a, 6f, 94, b9, de, 03] -> [00, 25, 4a, 6f, 94, b9, de, 03]
256 bytes through endpoint 1 and back, in 32 packets of at most 8
`zero_probe` read ZERO on every one of them: a flip-flop whose data input is
the constant zero holds zero in silicon on this family
```

**And the same design with the probe fed a constant one instead**, built and
loaded onto the same board, which is the control that says the check is not
vacuous — one line changed in a scratch copy that is deliberately not committed,
and the working bitstream was put back afterwards:

```
[00, 01, 02, 03, 04, 05, 06, 07] -> [ff, fe, fd, fc, fb, fa, f9, f8]
every one of 8 returned byte(s) is the complement of the byte sent, which is
`zero_probe` holding a ONE: a flip-flop whose data input is the constant zero
came up set.
```

**The register has to survive synthesis, and that is the hard part of the
experiment, not the board.** A compiler may delete a flip-flop whose value is a
known constant and `synth::opt::FfOpt` does — so an experiment built out of one
can quietly stop being an experiment and pass whatever the backend writes.
`(* keep *)` did not help when this was built: `FfOpt`'s constant rule read the
*cell's* attributes, an attribute on a `reg` lands on the net, and the register
folded to `assign %z = 1'd0` with the XOR gone with it. That is fixed —
`synth::keep` now answers the question from both places, and a `(* keep *)` on a
register keeps the flip-flop — but the design here still does not rely on it,
and what it relies on instead is the semantics.
`zero_probe` is initialised to **one** and clocked to **zero**, so its value
before the first edge differs from its data and folding it away would change
what the design means; `FfOpt`'s rule is exactly that — a constant `d`
collapses only when the initial value agrees with it. On the part the
initialiser is a fiction and does not matter: an ECP5 releases every flip-flop
into its `REGSET` state, which is `RESET` here, so the register starts at zero
and the **first clock has to keep it there**, which is the thing being
measured.

Nothing was believed off the board until the bitstream had been read, and both
halves are tests rather than a note:

| | |
|---|---|
| `the_usb_devices_constant_zero_probe_survives_synthesis` | no database, no board, seconds. `zero_probe$ff` is a `TRELLIS_FF` with `DI=%const0` in the mapped netlist, `const0$lut` is an all-zeros `LUT4`, and there is exactly one of it. This is what stops a future optimisation from turning the hardware test green for ever |
| `the_usb_devices_constant_zero_probe_reaches_the_bitstream` | `#[ignore]`d: it places and routes the whole device. One all-zeros `INIT` word in the image **that is not a distributed RAM's storage**, its four inputs tied high at absolute frame positions, all **408** flip-flops taking data from the fabric with **none floating**, and exactly one of them walking back through the file's own arcs to that lookup table's output. The sixteen distributed RAMs of the endpoint buffers put 96 more all-zero `INIT` words in the image — six each, empty at configuration — and the test separates them by `SLICEA.MODE = DPRAM` and asserts that total too |

What the probe costs, and what moved:

| | Before | After |
|---|---|---|
| Set bits | 43 299 | **43 487** |
| `.config` words | 1083 | **1084** |
| Lookup tables | 1084 | 1085 |
| Flip-flops | 490 | 491 |
| Signals | 1586 | 1588 |
| Arcs that cost bits | 14 323 | 14 396 |
| Bits **unexplained** | 0 | **0** |

**Those two columns are of the eight-byte device**, which is what
`usb_ulpi_device.v` was when the probe was added; its endpoints carry 64 bytes
now and the design is bigger, which changes every number above except the one
that matters. Measured again on the same design with 64-byte packets:

| | 8 bytes, shift | 64 bytes, shift | 64 bytes, **array** |
|---|---|---|---|
| Set bits | 43 577 | 86 845 | **44 355** |
| Lookup tables | 1086 | 2004 | **1028** |
| Flip-flops | 491 | 1432 | **408** |
| `TRELLIS_DPR16X4` | 0 | 0 | **16** |
| Signals | 1589 | 3448 | **1512** |
| Arcs that cost bits | 14 449 | 31 028 | **14 361** |
| Bits **unexplained** | 0 | 0 | **0** |

The middle column is what 64 bytes cost while both buffers were shift
registers: 949 more flip-flops than eight bytes — 1024 of them where eight
bytes needed 128 — and 918 more lookup tables, which are the 64-to-1 byte
multiplexer over each buffer, 63 of them per bit on LUT4. The right-hand column
is the same design with the buffers as **arrays**, which is `usb_bulk_ep`'s
`BUF_RAM = 1` and what it does by default now: the multiplexer is not built, the
1024 bits are in sixteen distributed RAMs, and the image is **half the bits**.
`docs/ip-library.md` has that accounting and what the size bought, measured on
this same board.

**Nothing is unexplained in any of the three columns**, which is the statement
this table is for; each was placed, routed, written and decoded to get its
numbers. It is a stronger statement in the right-hand one than in the other two,
because those sixteen RAMs are bits of a kind no earlier image here had: 97 of
them each, the six `INIT` words and the mode. The `#[ignore]`d test above adds
to that by following every flip-flop back to a driver in the fabric with none
floating; it was run over the middle column in 562 seconds and over the
right-hand one in 124, and it asserts the flip-flop count, so it tracks whatever
`usb_bulk_ep` does by default rather than both shapes at once.

**`zero_probe` reaches eight lookup tables and not sixty-four**, which is a
sentence in `usb_ulpi_device.v` that no round has been able to update: that file
is frozen as the hardware-verified reference design. `in_data` used to be
written into one of eight byte registers by a `case`, so the XOR's output
reached sixty-four flip-flops' worth of write logic. It has been eight ever
since, and it was counted in the mapped netlist of **both** shapes to be sure
the array did not change it again: as a shift register `in_data` feeds one byte,
the top of the register, and as an array it feeds the eight `DI` pins of a
distributed RAM. Eight `LUT4` carry the probe in each, and in each the XOR folds
into a cover the endpoint already needed. The probe is **not** weakened either
way: it is still on the byte the host reads, every byte still passes through the
XOR, and `tests/usb_loopback.rs` still reads a few hundred bytes back
byte-identical with a constant-zero flip-flop in the middle of each one.

The one new `.config` word is the constant's `INIT` and nothing else. The rest
is a net figure rather than an itemised one, because one more cell moves the
placer's assignment and thousands of arcs change — which is what this design's
accounting has always been; `wide_state.v` above is where the same 28 bits are
itemised one by one. `zero_probe` costs **one** extra lookup table and not
nine, because the eight XORs fold into inputs the endpoint's own cover was not
using.

**Why this went into the reference design rather than beside it.** The cost is
real: `usb_ulpi_device.v` is what everything else is compared against and its
footprint moves. A variant would have cost the same flip-flop with none of
that — and it would have been built once, measured once, and then stopped being
measured, because the design that gets loaded onto this board in the ordinary
course is this one, and a property nobody re-checks is a property that rots.
The deciding argument is that the register is **not dead**: `zero_probe` fans
out into sixty-four lookup tables of the IN data path and its value leaves the
part in every byte the host reads, so it is a signal this design carries and
not a stub bolted to the side of it.

**What the check catches and what it does not.** It catches a flip-flop whose
data input is the constant zero coming up, or being clocked to, a one — on
every one of the 293 bytes the test moves, so it cannot pass by accident. It
does **not** catch a broken constant *one*: that holds `rst_q` low, the
transceiver stays in reset, no device appears on the bus, and the test *skips*
with "no 1209:0001 is attached" rather than failing. It says nothing about a
flow run with `device_cells` off, and nothing about a partial inversion, which
the probe cannot cause — it is one wire into all eight bits.

### The unused reset wire, which turned out to be a different shape entirely

The round that built the constant left a note beside it: the vendor writes
`CIB.JLSR<n>MUX = 0` for slices whose flip-flops do not use their reset, this
flow writes nothing, every clocked design built here works, "so an unrouted
`LSR` evidently does not hold a flop in reset the way an unrouted `M` holds its
data at one — but *why* is unmeasured, and a register that resets itself every
clock would look exactly like the `stage` fault did".

It is measured now, from the database and the three reference bitstreams, in
`what_lattices_own_packer_writes_for_an_unused_reset`. **The two pins are not
the same shape at all.**

**A reset wire has a constant zero and a data wire has no constant.**
`CIB.JLSR0MUX = 0` is not a field beside the routing: it is a twentieth code
point of the *same nineteen-source mux* that `PLC2`'s `.mux LSR0` describes —
identical bits, source for source, at the same position, because a logic
position's `CIB` and `PLC2` tiles overlap. "Tie the reset low" and "route
something to the reset" are one mux, and one of its settings is a zero. There
is no `CIB.JM<n>MUX` at all, and the only constant the slice offers on a data
pin is `SLICE<l>.M<n>MUX = 1`, a **one** — which is what an unrouted wire
already reads as, so it buys nothing.

| Pin | Constants the database offers |
|---|---|
| `JCE<n>`, the enable | `1` only — an unused enable must be high or the flop never clocks |
| `JCLK<n>`, the clock | `0` only |
| `JLSR<n>`, the reset | `0` only |
| `JD<n>`, a lookup table's inputs | `0` **and** `1` |
| `JM<n>`, a flip-flop's data | **none**, and the slice's own tie is a `1` |

Every control pin is offered exactly the level that is safe for it. A
flip-flop's *data* has no safe level — either one is a value some design does
not want — so the hardware offers none, and a lookup table is the only way to
put a zero there. That is the whole asymmetry, and it is the correct one.

**And Lattice's own bitstreams leave hundreds of flip-flops on a reset wire
that nothing drives and nothing ties.** Of the logic tiles holding flip-flops,
the ones with no arc *and* no tie on either of the tile's two `LSR` wires:

| | `analyzer.bit` | `selftest.bit` | `facedancer.bit` |
|---|---|---|---|
| Tiles holding flip-flops | 438 | 91 | 1159 |
| **Of those, nothing at all on either `LSR`** | **84** | 3 | **551** |
| Flip-flops in `PRLD` mode, which would explain it another way | 0 | 0 | 0 |
| `CIB.JLSR<n>MUX = 0` written | 30 | 0 | 153 |
| **Of those, at a position holding a flip-flop** | **0** | 0 | **0** |

So an unrouted `LSR` cannot hold a flop in reset: `analyzer.bit` would be a
logic analyser with 84 tiles of dead registers, and it is Great Scott Gadgets'
shipped gateware. And the ties the vendor *does* write are **never at a
position that holds a flip-flop** — not one of the 183 — so they are not about
flip-flops either, and the note's guess that they were is wrong. This flow's
silence about an unused `LSR` is what the vendor's flow does too.

What that does **not** settle is a voltage. Nothing in it measures what an
unselected mux output reads as, on `LSR` or on `M`. The `M` case was measured
the expensive way, by a register coming up set on a part and eight rounds of
looking somewhere else; the `LSR` case is settled the cheap way instead, by
three of the vendor's own working files not doing it in a thousand places.

One thing is still recorded and deliberately not acted on: `SLICE<l>.M<n>MUX =
1`, the two-bit tie above, which would make a constant one without a lookup
table and which no reference bitstream exercises anywhere.

## And then on a block RAM's address, which was wrong on silicon

**CHECKED, 8 October 2026.** The round above gave a flip-flop's data pin a
driver, and the rounds before it a distributed RAM's inputs and a carry
cell's operands. Three roles, each added after something broke. Asking
*why the list is a list* found a fourth pin, and this one was wrong on the
part rather than merely fragile.

`src/fpga/devices/ecp5.dev` says, for the widest mode of a block RAM:

```
mode 18 1024 param DATA_WIDTH_A=18 DATA_WIDTH_B=18 addr 4 pad 4'b0011 init low 0-17
```

A `DP16KD` in 18-bit mode addresses its words through `AD[13:4]`, and the
four bits below are **not** don't-care: they must read `0011`. Two of those
four are therefore a constant **zero** — and an unrouted input on this
family reads as a **one**, which is the whole subject of this document.
Nothing routed them, and `configure_bram` ties only `RST`, `WE` and `CE`,
so `AD[3:0]` read `1111` in every 18-bit block RAM this flow has ever
built.

Nothing off the part could see it. The netlist holds the zeros, so
simulation reads zeros; every bit of the image decoded; placement and
routing were untouched by it. It is the carry-operand fault again, in the
hard block, found this time by asking the general question instead of
waiting for a board — which is the point of
`tests/fpga_constants.rs::no_primitive_input_is_left_holding_a_constant`.

`techcells::drive_constant_data` now gives those bits a driver, and
`a_block_ram_places_routes_and_every_bit_of_it_decodes` went from 1259 set
bits to **1352**: one constant lookup table and the arcs that carry it to
eight address pins over two blocks. The blocks' own thirty-three bits did
not move.

**Not measured on a part.** No design in this tree puts a `DP16KD` in
18-bit mode on the board, so what is established is that the netlist no
longer holds the constant and that `AD[3:0]` has a driver on every bit.
What those bits do to the block's addressing in silicon is the device
file's statement, taken from nextpnr's own `DATA_WIDTH` handling, and is
not something this round measured.

A family that states **no** `pad` value is read as not reading those bits,
and that is a reading rather than a measurement: a `RAMB18E1` at 18 bits
uses `ADDRARDADDR[13:4]` and an `SB_RAM40_4K` at 8 bits `ADDR[8:0]`, with
the rest unused, and that is their libraries speaking. So only the ECP5's
address is driven, which is also what keeps the cost where the defect is.

### What `ecppack` writes for a constant on an output pad, and why this flow still does not

The other pin the same question raised was an **output pad's data**, and
here the answer went the other way — which is worth writing down, because
the evidence pointed one way and the part pointed the other.

On the 7 series a constant on an `OBUF`'s input gets nothing at all:
`assign led = 1'b1` leaves the pad's input unrouted and the pad came out
**low** on a Basys 3 (`docs/fpga-xray.md`). This family is not in that
position. A pad's data wire can be **tied** in the `CIB` tile beside it —
`CIB.J<x>MUX = 0` or `= 1`, [`DATA_MUX`] — which
`TrellisFabric::configure_io` has always written, and which is in every
bitstream this flow has put in a Cynthion.

Their own files do not use it. Walking backwards from every output and
bidirectional pad of the three reference bitstreams:

| | `analyzer.bit` | `selftest.bit` | `facedancer.bit` |
|---|---|---|---|
| Output/bidirectional pads with a drive strength | 109 | 102 | 107 |
| **Of those, data wire tied through `CIB.J<x>MUX`** | **0** | **0** | **0** |
| **…with nothing on the data wire at all** | **0** | **0** | **0** |
| Data from a lookup table with `INIT` all **ones** | 32 | 32 | 31 |
| …all **zeros** | 21 | 21 | 20 |
| …from ordinary logic | 56 | 49 | 56 |
| **Tristate wire tied through `CIB.J<x>MUX`** | **0** | **0** | **0** |
| Tristate from the all-zeros lookup table | 76 | 69 | 78 |
| …from ordinary logic | 33 | 33 | 29 |
| Which lookup tables | zero `F0` at X38Y27 (97 wires), one `F0` at X5Y27 (32) | zero `F4` at X63Y23 (90), one `F0` at X3Y10 (32) | zero `F0` at X11Y6 (98), one `F0` at X64Y8 (31) |

The last row is the strong part: those are **the same four slices** the
flip-flop table above names, found that time by walking back from every
`M` wire instead. Two readings of different pins landing on the same cells
is what makes this a measurement. It also finishes a sentence that table
left open — `selftest.bit` has no constant on a flip-flop's data pin and
**does** have both constant lookup tables, at X63Y23 and X3Y10; they are
there for its *pads*.

The reason is mechanical rather than a preference: these files were built
with nextpnr and `ecppack`, nextpnr's `pack_constants` runs before its
`tie_cib_signal`, and after `pack_constants` no pad's data pin is a
constant any more. So the tie is a path their toolchain never reaches.

**And this flow keeps writing it anyway.** The tie is measured on
**silicon**: `testdata/fpga/cynthion/leds.v` lights six LEDs through six
`CIB.J<x>MUX = 0`, and `usb_ulpi_device.v`'s `R4` holds a constant zero
through the same field in the bitstream a USB host enumerates. The
vendor's answer is measured in somebody else's file. Replacing something
that works on a part with something that works on a part is churn, and the
part is the better instrument — so `src/fpga/devices/ecp5.dev` says

```
bel TRELLIS_IO io absorbs dout port pad=B din=O dout=I oen=T …
```

and the constant pass leaves that pin alone on this family and builds it
on every other. `absorbs` is a new clause and it exists for exactly this:
a pin one family has a mechanism for and another has not. If a reason ever
appears to prefer the vendor's shape here — a `CIB` tie that misbehaves,
or a board to re-check `leds.v` on — deleting two words from a device file
is the whole change.

### What the pass is now, and what it is allowed to skip

It is no longer a list of three roles. Every input of every primitive the
device declares gets a driver, and the exceptions are a short list each of
which names the thing that absorbs the constant:

| Pin | What absorbs it |
|---|---|
| any input of a `lut` | the truth table — `map_lut` widens a narrow one, and `ecppack` ties every unused input with `<X><n>MUX = 1` and folds the value into `INIT`, in all 4262 carry halves of these files |
| a `ff`'s `clk`, `en`, `rst` | a mux bit the backend writes: `SLICE<l>.CEMUX = 1` and `LSR<c>.LSRMUX` here, a cleared `CEUSEDMUX`/`SRUSEDMUX` on the 7 series |
| a `carry`'s `ci` | dedicated metal from the cell below |
| a `carry`'s `cyinit` | the pin a family designs as its way in |
| a wide `carry`'s `p` and `di` | the lane's own truth table. These two roles exist only on the 7-series `CARRY4` shape, and `src/fpga/xray/carry.rs` already puts a constant propagate into a lookup table of that lane and a constant generate into its lower half, because `DI` is reached from the slice's own `O5` and not from general routing. The narrow shapes' operands — `a`, `b`, `i0`, `i1` — are **not** here: those are the pins that cost a board |
| an `io`'s `oe`/`oen` | the direction recipe, plus the `CIB.JB0MUX` tie |
| an `io`'s `pad` | it reaches the outside world, not the fabric |
| anything named `*clk` | a clock is not a data pin, and a lookup table on one would be a clock off general routing, which both backends refuse by name |
| every pin of a `gb` | its `i` is a clock and its `en` is a constant this flow puts there on purpose: `DCCA`'s `CE` is tied high because nothing gates a clock here |
| a `bram`'s pins other than its address | the backends' own tie passes, which is what those were written for. What that leaves open is named in `tests/fpga_constants.rs`, and the sharpest part of it is a write-data bit a *design* ties to zero inside the mode's width: `mem[a] <= {4'b0, x}` on an eight-bit memory would read ones back here |
| whatever a device file's `absorbs` clause names | the family's own mechanism; `TRELLIS_IO`'s `dout` is the one |

A `dsp` and a `pll` are not `bel` lines but shapes of their own, so they
never reach the pass; the pins they hold at a constant are the ones their
`tie` clause names, which is a statement that the silicon drives them.

One more thing had to change for the block RAM: the pass now works **per
bit**. A `DP16KD`'s address is one fourteen-bit pin of which four bits are
the constant, and the old code only looked at pins that were one bit wide
— which is why listing the role would not have been enough on its own, and
also why a `CARRY4`'s four-bit `S` and `DI` had never reached the pass
before.

## It enumerates, and the fault was one bit of a register this backend brings up wrong

On 2026-09-27 a host on the other end of a USB cable read an eighteen-byte
device descriptor out of a Great Scott Gadgets Cynthion whose ECP5 was running
a bitstream this compiler wrote.

```
$ lsusb -d 1209:0001 -v
Bus 007 Device 073: ID 1209:0001 Generic pid.codes Test PID
Negotiated speed: Full Speed (12Mbps)
Device Descriptor:
  bLength                18
  bDescriptorType         1
  bcdUSB               2.00
  bDeviceClass          255 Vendor Specific Class
  bDeviceSubClass         0 [unknown]
  bDeviceProtocol         0
  bMaxPacketSize0         8
  idVendor           0x1209 Generic
  idProduct          0x0001 pid.codes Test PID
  bcdDevice            1.00
  iManufacturer           0
  iProduct                0
  iSerial                 0
  bNumConfigurations      1
  Configuration Descriptor:
    bLength                 9
    bDescriptorType         2
    wTotalLength       0x0012
    bNumInterfaces          1
    bConfigurationValue     1
    iConfiguration          0
    bmAttributes         0x80
      (Bus Powered)
    MaxPower              100mA
    Interface Descriptor:
      bLength                 9
      bDescriptorType         4
      bInterfaceNumber        0
      bAlternateSetting       0
      bNumEndpoints           0
      bInterfaceClass       255 Vendor Specific Class
      bInterfaceSubClass      0 [unknown]
      bInterfaceProtocol      0
      iInterface              0
```

```
usb 7-5: new full-speed USB device number 71 using xhci_hcd
usb 7-5: New USB device found, idVendor=1209, idProduct=0001, bcdDevice= 1.00
usb 7-5: New USB device strings: Mfr=0, Product=0, SerialNumber=0
```

### The fault, in one sentence

**`ip/usb/usb_device_fs/rtl/usb_ctrl_ep.v` declared `reg [2:0] stage` for four
states, so `stage[2]` was a flip-flop whose data input is the constant zero, and
on this backend such a flip-flop comes up holding a *one*.** `stage` read **5**,
every label of `case (stage)` missed, every IN token the host sent was answered
from the `default` arm with a **NAK**, and the host retried for five seconds and
gave up: `device descriptor read/64, error -110`.

Why a one. `SLICE<l>.REG<n>.SD = 0` takes a flip-flop's data from the fabric's
`M` wire; nothing is routed to that wire when the netlist gives the pin a
constant; and an unrouted slice input on this family is **high**, because
Lattice's own packer ties every unused lookup-table input high and
`TrellisFabric::configure_logic` does the same for the same reason. A LUT can
absorb a constant input into its truth table. A flip-flop's data input cannot
absorb anything: it needs a driver, and nextpnr makes one — `pack_constants`
builds a lookup table with `INIT` all zeros or all ones and routes it — where
this backend made nothing at all.

**The fix in the backend is a refusal, not a driver.**
`TrellisFabric::configure_registers` now names the cell and stops:

```
error: flip-flop `state$ff$ff2` (`SLICEA.FF1` at X40Y2) has nothing driving its
data input and is not tied high, and an unrouted slice input on this family
reads as a **one** — so it would come up set rather than at the constant the
design gives it. Give the register only as many bits as its values need, or
drive that bit
```

`testdata/fpga/cynthion/wide_state.v` is a three-line design that does it on
purpose. **That refusal lasted one round.** The round after it built the
constant instead — a lookup table with `INIT` all zeros and every input tied
high, which is what Lattice's own bitstreams for this board turned out to
contain — and
`tests/fpga_trellis.rs::a_register_bit_nothing_drives_is_built_from_a_constant`
is what pins that. See "The constant is built now, and the vendor's own
bitstreams said how"; what is still refused is a data pin with nothing at
all on it.

**A data pin the netlist ties *high* is allowed through, and that is not
laziness.** The untied wire is a one, so the flop loads the one the design asked
for. `usb_ulpi_link`'s `rst_q` — the set-once register that releases the
transceiver's reset pin — is exactly that, and it has worked on this board since
the first ULPI bitstream. It worked **by accident**, and the accident is now
written down in the code rather than left to be rediscovered.

### The fix in the design, and it is one character

```diff
-    localparam [2:0] C_IDLE       = 3'd0;
+    localparam [1:0] C_IDLE       = 2'd0;
...
-    reg [2:0]  stage;
+    reg [1:0]  stage;
```

Nothing else. The rest of `usb_ctrl_ep`, the whole of `usb_ulpi_link`,
`TURNAROUND` at its original `9`, `SLEWRATE`, the register sequence and the
board's `SwapDP/DM` are all as they were, and with those two lines the
device enumerates. That was checked the only way it can be: by building the
**original** `usb_ulpi_link.v` and the **original** `usb_ctrl_ep.v` out of
git with nothing but the width changed. It enumerated.

Three other changes were made while the fault was still hidden — `line_idle` not
testing LineState, the turnaround counting consecutive quiet cycles, and a bus
reset confirmed by a Debug-register read — and **all three are reverted**,
because the measurement says they were not needed and a shipped block should not
carry changes argued from reasoning when the reasoning turned out to be about
something else. What was learnt from them is in "Two things this part does that
ULPI forbids", which is measurement and not change.

### How it was found, which is the part worth keeping

Eight rounds of experiments had reached one bit of information per bitstream —
an attach or a silence in `dmesg` — and the answer needed hundreds. Three
instruments, each built on the last, and the first two are committed:
`testdata/fpga/cynthion/usb_ulpi_trace.{v,rcf}` is the console and the trace
together, with the safety argument for the pin in its header and the host-side
recipe beside it, and `usb_ulpi_trace_tb.v` asserts its output character for
character.

**1. A console, on the pin the debug microcontroller shares with JTAG.** Great
Scott Gadgets' platform file gives the FPGA a `uart` resource on `R14`/`T14`,
and Apollo bridges it to `/dev/ttyACM0` on the machine the CONTROL port is
plugged into — so the FPGA can talk to the *same host* over a channel that has
nothing to do with the port under test. The catch is in their own file: "UART
pins R14 and T14 are connected to JTAG pins R11 (TDI) and T11 (TMS)
respectively". The PCB has no series resistor; `/Debugger/FPGA_JTAG.TMS` carries
FPGA balls T11 **and** T14 and the microcontroller's pin. So a gateware driving
T14 while Apollo shifts JTAG is two drivers on one wire. What makes it safe:

- the pad drives only inside a **window**, a counter and two latches, from about
  0.28 s after configuration to about 17.9 s, and is high impedance before and
  after — once the window shuts nothing can reopen it but a reconfiguration;
- `PULLMODE=UP`, which is what the platform file asks for on that pin;
- Apollo's own firmware is the other half of the interlock: `jtag_init` calls
  `uart_release_pinmux` and `jtag_deinit` calls `uart_configure_pinmux`.

Two host-side details cost an hour each and are worth writing down. Apollo only
initialises its SERCOM when the host **changes** the CDC line coding
(`tud_cdc_line_coding_cb`); `tud_cdc_line_state_cb` skips it when the firmware
already thinks the UART is active, which it does after every JTAG transaction.
So the reader asks for 9600 first and makes 115200 a change. And the console was
silent at first for a reason that belongs in this file: **`if (e) q <= 1'b1`
infers a clock enable and the pad never drove; `q <= q | e` worked.**
`usb_ulpi_device.v`'s header had already said so about its LED latches, and this
is the second time it has been paid for.

**2. A ULPI trace.** Ninety-six entries of
`{gap, usb_reset, dir, nxt, stp, data}` in one shift register, triggered on
the first packet whose PID and whose *second byte* match — the second byte
matters because a root hub repeats every downstream packet to every enabled
port, so this device sees the host's whole conversation with all twenty
devices on that bus and a token's PID alone does not say whose it is. `00h`
there is address zero, endpoint zero, which is this device and nothing else.
Two filters make a whole control transfer fit in ninety-six entries: a
receive command whose only news is LineState is dropped, and so is an empty
turnaround cycle.

Its own bugs are the lesson. The trace printed its entries rotated by four
because a dump that began before the buffer was full rotated partway through it;
and the address match fired on the cycle *after* the PID instead of on the next
byte, which a stub transceiver handing bytes over back to back could not
catch — the testbench's stub now spaces them forty clocks apart, as a
full-speed line does.

**3. A debug port on the endpoint.** When the trace had shown the NAK and
the bus was exonerated, `{stage, expect, toggle, await_ack, pending}` was
brought out of `usb_ctrl_ep` in a scratch copy of the IP and latched at the
trigger. It read `A4`: `toggle = 1`, so the SETUP had been processed;
`expect = none`, as it should be; `pending = 0`, as it should be; and
`stage = 5`, which the RTL cannot produce. That is the whole answer, and
nothing short of reading the register was going to give it.

### What the trace settled on the way, and what it corrected

Every one of these is one line of a decoded trace taken on the part.

| Question | Answer |
|---|---|
| Does the host's SETUP arrive? | **Yes, byte-exact**: `2D 00 10`, then a DATA0 of `C3 80 06 00 01 00 00 40 00 DD 94` — `GET_DESCRIPTOR(DEVICE, wLength=64)` with its CRC16 |
| At what rate? | five clocks a bit, exactly 12 Mbit/s against the board's 60 MHz |
| Does the device answer it? | **Yes**: `TX CMD pid=2 (ACK)` six clocks after the packet ended, `nxt` three clocks later, `stp` with `nxt` still high — which is the USB334x datasheet's own condition for ending a transmit |
| Is that ACK well formed on the pair? | **Yes, symbol by symbol.** The transceiver hears its own transmission and reports it: K J K J K J K J K J K, then SE0, then J — which is SYNC, then `D2h` LSB-first in NRZI, then the EOP, exactly |
| Does the host understand it? | **Yes.** It goes on to send an IN token to address zero, which is what a host does only when the setup stage succeeded |
| What does the device answer the IN with? | **`TX CMD pid=A (NAK)`**, six clocks after the token, for ever |
| Why? | `stage` is 5 |

**That corrects the previous account of this file in its central claim.**
"The host understands none of this link's transmissions" was wrong, and so
was the measurement it rested on: a build whose every answer is a STALL
produced the same `dmesg` as a build that answers nothing because **a STALL
in answer to a SETUP's data packet is not the STALL a host reports `-32`
for** — that one belongs to the data stage — and because, as it turned out,
`stage` being 5 meant neither build ever had the endpoint state its own
experiment assumed. Every transmit-side experiment of the previous rounds,
`SLEWRATE` and the `TURNAROUND` sweep and the `stp` timing included, was run
against an endpoint that was going to answer NAK whatever the wire did. That
is the shape of the mistake: a sound instrument pointed at a device whose
state nobody had read.

### Two things this part does that ULPI forbids, measured and not fixed

Both are real, both are in the traces, and **neither is why it failed** — which
is exactly why they are recorded here as facts about the part rather than as
changes to the block.

1. **Receive commands report LineState late.** Between a host's SETUP token
   and its DATA0 packet, with the pair idle at J the whole way, the
   transceiver sent seven single receive commands three to five clocks apart
   reporting J, K, J, K, J, K and finally **K**. Those are the bit
   transitions of the packet that had already finished, arriving after it:
   one receive command per transition, a bus that carries one at a time, and
   a backlog that outlives the packet. ULPI 1.1 §3.8.1.3 says a queued
   receive command "must always convey the current RX CMD values, not a
   previous or old value". This part does not.

   It is worth knowing because two things in this block read LineState: the
   turnaround `line_idle` waits for, and the SE0 count that makes a bus reset.
   Neither has been seen to break — the closing receive command of a packet, the
   one that clears RxActive, carries the **right** LineState, and DS00002646A
   §6.3.2 says the part does not send it until the pair is idle — so nothing is
   changed. `tests/ip_library.rs`'s transceiver model can be told to do it
   (`reporting_stale_line`) and the block enumerates through it.

2. **A device answers an IN token six clocks after it ends**, and ULPI 1.1
   Table 10 asks for seven to eighteen. That is `TURNAROUND` at 2, which was one
   of the things tried while the fault was hidden; at its shipped value of 9 the
   answer lands at thirteen, inside the window, and that is the build that
   enumerated.

### What this settles

It settles the thing this backend was built for: **a design of 728 lookup
tables and 296 flip-flops, compiled from Verilog by this crate alone — no Yosys,
no nextpnr, no `ecppack` — placed, routed, written into a `.bit` whose every one
of 26 916 set bits decodes back through Project Trellis' database with nothing
unexplained, loaded into a real ECP5 over a real JTAG transport this crate also
wrote, is a USB 2.0 full-speed device that a Linux host enumerates.**

It settles that the flow's structural checks are not enough on their own,
and says exactly where the gap was: every one of them passed on the broken
bitstream. 100% of the bits decoded, every sink walked back to its driver,
every clock was on a global network, and the device did not work, because
none of those checks has an opinion about a flip-flop whose data input
nothing drives. That check exists now.

It settles nothing about the rest of `usb_ctrl_ep`'s reach: one control
endpoint, eight-byte packets, two descriptors, no interfaces with endpoints
on them.

## An edge rate on every ULPI pin, and it changed nothing

On 2026-09-27 `PIO<side>.SLEWRATE` became the first pad attribute this backend
writes that is not a direction or a pull, which closes the last gap between
what Great Scott Gadgets' `ULPIResource` asks for and what this flow puts in a
bitstream. It was the leading suspect for the USB device. **It is not the
cause**: the device still reports `device descriptor read/64, error -110`, with
`new full-speed USB device` in the same line as before.

### What Lattice's own packer writes for it, asked in full

The method is the one that has found five defects on this part — ask what
`ecppack` writes, in full, rather than diffing against what this flow writes —
and the answer is short:

| | |
|---|---|
| Where | the **pad** tile, the one `HYSTERESIS` and `PULLMODE` are in |
| How much | **one bit** for `FAST`; `SLOW` is the field's default and is that bit clear, so it costs nothing and cannot be written |
| For which pads | every direction. nextpnr's condition is the *attribute*, not the direction: `if (ci->attrs.count(id_SLEWRATE) && !is_referenced(...)) cc.tiles[pio_tile].add_enum(pio + ".SLEWRATE", ...)` in `ecp5/bitstream.cc`'s `write_io` |
| Of the base type's bits | **none**. It is a setting of its own, like the pull mode and unlike hysteresis, whose bits `BIDIR_LVCMOS33` already contains |
| When this writes it | only when a constraint asks — `set_io -slew fast` — which is nextpnr's condition exactly, so a design that does not ask comes out bit for bit as it did before |

`tests/fpga_trellis.rs::what_lattices_own_packer_writes_for_a_slew_rate` is
that table as assertions, and it is asked of **all thirteen pins of the
auxiliary ULPI resource in all three of Great Scott Gadgets' own bitstreams** —
the eight bidirectional data balls, `dir` and `nxt` (inputs), `stp`, `rst` and
`clk` (outputs). All thirty-nine bits are set in their files at the absolute
frame position this crate computes, and all thirty-nine read back through the
database as `PIO<s>.SLEWRATE = FAST`. That is the whole of it: one bit per pin,
in the pad tile, for inputs as much as outputs.

### What it cost in the bitstream, and what it bought on the part

`usb_ulpi_device.rcf` now asks for `-slew fast` on all thirteen ULPI pins. The
bitstream grew by **exactly thirteen bits** — 27 169 to 27 182 set bits, 1 455
to 1 468 decoded fields — and **all 27 182 still decode with nothing
unexplained**, which is the standard every other bitstream here was held to.

On the part: loaded, the host saw the attach, called it a full-speed device,
and failed the same way at the same place.

```
usb 7-5: new full-speed USB device number 87 using xhci_hcd
usb 7-5: device descriptor read/64, error -110
```

So the edge rate is now written because the board's own platform file asks for
it and because a silently dropped constraint is a defect on its own terms. It
is **not** evidence about anything, and the thing it was the leading suspect
for is still open.

## The board crosses D+ and D-, and one register says so

On 2026-09-27 the question "why does a host call this full-speed device a
low-speed one" was answered, and the answer was not a cable, not a plug and
not the Type-C controller. **Great Scott Gadgets' Cynthion r1.4 exchanges
D+ and D- between each ULPI transceiver and its connector on purpose, and
the transceiver has a bit whose documented purpose is to undo it.** Writing
that bit changed the one line of `dmesg` this whole exercise is measured by:

```
-usb 7-5: new low-speed  USB device number 91 using xhci_hcd
+usb 7-5: new full-speed USB device number 92 using xhci_hcd
```

It does not enumerate yet. What is settled, what is newly measured and where
it now stops are below, and the three defects the search found in this
project's own ULPI core are in "Three things the datasheet says that ULPI
does not".

### What the part is, and how the board wires it

The transceiver had already been made to name itself over its own bus: Vendor
ID Low reads `24h`, the low byte of `0424h`, Microchip. The rest came out of
Great Scott Gadgets' published design and the part's datasheet, and the two
of them together are what nobody here had put beside each other:

| Fact | Source | Confidence |
|---|---|---|
| The AUX transceiver is `U11`, a **USB3343-CP** in a 24-pin VQFN | the r1.4 PCB's own component list | HIGH |
| **Pin 13 is DP and pin 14 is DM** | *USB334x Data Sheet* DS00002646A, Figure 2-2 and Table 2-2, confirmed by the SMSC Rev. 1.2 edition and by the EVB-USB3343 schematic | HIGH |
| Pin 13 goes to the receptacle's **D-** (`A7`/`B7`) and pin 14 to its **D+** (`A6`/`B6`), through the common-mode choke `FL1`, whose pads 1-2 and 3-4 are the two windings | the r1.4 PCB's nets | HIGH |
| The receptacle's **two data pairs are tied together** — `A6` to `B6` and `A7` to `B7` — so plug orientation cannot cross anything | the same | HIGH |
| Register **`39h`** "USB IO & Power Management", bit 1 **`SwapDP/DM`**: *"When asserted, the DP and DM pins of the USB transceiver are swapped. This bit can be used to prevent crossing the DP/DM traces on the board."* Reset value `04h` | DS00002646A §7.1.3.5 | HIGH |
| Great Scott Gadgets' own gateware writes exactly that byte to exactly that address, for every port | `cynthion/python/src/gateware/platform/core.py`: `ulpi_extra_registers = {0x39: 0b000110}  # USB3343: swap D+ and D- to match the hardware design` | HIGH |

So the crossing is deliberate — a layout that would otherwise have had the
pair cross over — and the bit is the compensation the part provides for it.
`0b000110` is `06h`, which is the register's reset value `04h` with bit 1
set; bits 3:2 are the UART-mode regulator setting and are left at their
default.

**This also disposes of the previous conclusion.** "The board's Type-C data
pair must be wired crossed" was right about the symptom and wrong about the
cause: the board is crossed, deliberately, between the transceiver and the
connector, and not between the connector and the world. Every earlier
measurement stands — the transceiver really did report its own D+ high while
the host reported low speed — and now has a mechanism.

### What was ruled out on the way, and how

- **The Type-C controller cannot be it.** The AUX port's controller is `U12`,
  a **FUSB302B** in a WQFN-14. Its fourteen pins are CC1 ×2, CC2 ×2, VCONN
  ×2, VDD ×2, GND ×2, VBUS, SCL, SDA and `INT_N` (onsemi FUSB302B/D Rev. 5,
  Figure 5 and Table 3): **it has no pin that touches D+ or D-**, no data
  mux and no orientation switch for the data pair. Its reference schematic
  draws the receptacle's `A6`/`A7` and `B6`/`B7` going past it to the
  transceiver. HIGH, and it is why `ip/bus/i2c_master` was not needed after all:
  there was nothing on that bus worth writing.
- **It needs no configuration to be attached to, either.** `SWITCHES0`
  (`02h`) resets to `03h`, which is `PDWN1` and `PDWN2` set: a 5.1 kOhm Rd
  on both CC pins, i.e. a Type-C sink, with no I²C traffic at all. The part
  even presents Rd with no supply — "Dead Battery Support" — so a host
  applies VBUS and calls the port attached whatever the FPGA does. HIGH.
- **The plug's orientation cannot matter**, because both of the
  receptacle's data pairs land on the same two nets. HIGH, from the PCB.
- **The VBUS switches were left alone** and are not implicated. Nothing here
  enabled `aux_vbus_in_en` or `aux_vbus_en`, and nothing needed to.

### What the host says now, and what it does not

With `39h` = `06h` written before `TermSelect`, on the part:

| What the gateware did | What the host reported |
|---|---|
| nothing written to `39h` (every earlier run) | `new **low-speed** USB device` |
| `39h` = `06h` written after the transceiver's own reset and before `TermSelect` | `new **full-speed** USB device` |

The speed is now right, which is most of the test, and the pull-up is
therefore on the wire the host calls D+. The device still does not answer:
`device descriptor read/64, error -110`.

### Where it stops now, and the measurement that says so

> **Superseded, and in its central claim wrong.** It does not stop here and the
> host understands this link's transmissions perfectly well: it acknowledges the
> host's SETUP with a packet whose every symbol on the pair has since been read
> back, and the host goes on to send an IN token. What was wrong was one bit of
> a register in `usb_ctrl_ep`, which this backend brought up as a one — "It
> enumerates, and the fault was one bit of a register this backend brings up
> wrong" at the top of this file has the whole of it, including why the STALL
> experiment below could not have said what it was read as saying. The
> measurements in this section stand; the conclusion drawn from them does not.


The instrument for this round was not a bitstream per question but a
**staircase probe**: `ip/usb/usb_device_ulpi`'s two blocks wired up by hand in a
top level that also latches what it saw, and then reports one number by
**detaching for a measured length of time** — the link's own reset held, so
the transceiver's registers go with it and the start-up runs again on the way
back. The number is read out of
`/sys/bus/usb/devices/usb7/7-0:1.0/usb7-port5/state` polled at 50 Hz, which
says `powered` whenever the pull-up is there. `dmesg` cannot be used for it:
a host that is failing to enumerate keeps the port's device object alive for
its whole eighty-second retry cycle and prints nothing about a device that
comes and goes inside it. Two experiments were lost to that before it was
understood.

What the probe reported, in order, each figure one bitstream and one
detachment timed to a fifth of a second:

| Question | Answer |
|---|---|
| How far does the conversation get? | `phy_ready`, LineState J, a bus reset, whole received packets, SETUP tokens, a transmit started, a transmit finished with `stp`, **an IN token arrived** — and never an ACK from the host |
| How many bytes of a data-carrying transmit does the transceiver take? | **three**, and then `stp`: a whole zero-length data packet, so the packet leaves the link entire |
| Was `nxt` high in the first cycle of the transmit command? | **yes** — which ULPI 1.1 Table 2 forbids twice, and which a link that believes it advances a byte early on |
| Was the transfer paced at the wire's rate? | yes, over 32 clocks for three bytes, so the transceiver was serialising it |
| Did the packet reach the wire? | **yes**: twelve or more receive commands follow it inside 8.5 us, which is what §6.3.1 says a transmitted full-speed packet leaves behind |
| How many of the host's SETUP data packets went unacknowledged? | **none**. Every one passed its CRC16 and was answered |
| Does the host's ACK reach the link? | **`D2h` arrives on the data bus**, and `rx_active` inside the link was once high for more than 1024 clocks — twenty times the longest packet this host sends |
| Are **any** of this link's packets understood by the host? | **no.** A build whose every answer is a STALL handshake produces `error -71` in a third of a second, exactly like a build that answers nothing at all, and never the `error -32` (EPIPE) a host reports when it hears a STALL |

That last row is the one that matters, and it was worth the four rounds it
took to think of. A STALL is the one answer whose arrival a host names in
`dmesg` with an errno of its own, so it turns "did anything get through" into
one line of kernel log. Nothing did. **The host understands none of this
link's transmissions**, which means the IN tokens the earlier rows saw are
the host's own blind sequence and not evidence that it heard the ACK before
them, and that every conclusion drawn from them — including two of the
fixes below — was reasoning from a premise that had not been tested.

So the state of it is:

- **Out** is wrong: nothing this device sends is understood, in any slot, at
  any answer delay between **1 and 11.7 bit times**. `TURNAROUND` was swept
  to 5 and 15 — the extremes of ULPI Table 10's 7-to-18 clocks — and then, by
  widening the counter from four bits to seven, to **40 and 70 clocks**,
  which is past the window ULPI allows and well inside the **16 bit times** a
  host waits before calling a device's response a timeout (USB 2.0
  §7.1.19.1). Four bitstreams, four identical `error -110`s. The timing axis
  is excluded.
- **In** is right: tokens arrive with their CRC5 correct — an IN token is
  only answered at all when `token_ok` passes — and the SETUP's eight-byte
  data packet arrives with its **CRC16 correct**, every time, which is a
  byte-exact eleven-byte receive.
- **The terminations** are right: the host calls it full speed.

What that leaves is the shape of the transmitted packet on the wire, and the
`SLEWRATE=FAST` gap is now the only difference between what this flow writes
for these pins and what Great Scott Gadgets ask for on all of them. It is an
edge rate on the transmit path: invisible to a register readback that lands
byte-exact on a 60 MHz bus, invisible to a receiver at this end, and the one
thing that a receiver at the far end of a cable could care about. It has been
demoted twice, both times for the wrong reason — the first for an argument
about which wire a resistor is on, the second because the bus reads back
cleanly — and neither argument touches it.

Three things are consistent with that shape and are not yet distinguished.
The first is that `SwapDP/DM` swaps the terminations and the receiver but
**not the transmitter**, which would leave every transmitted symbol inverted
while leaving everything measured above intact; the datasheet's wording ("the
DP and DM pins of the USB transceiver are swapped") is against it and no
measurement here is. The second is the transmit command's PID field, which
is `01_00_pppp` with `P3` the most significant bit (DS00002646A §6.2.1) and
is what this core sends. The third is `SLEWRATE=FAST`, which this backend
still does not write for any pin and which Great Scott Gadgets ask for on
every ULPI pin of this board — an edge rate on the transmit path is the one
attribute that could plausibly matter to a receiver at the far end of a
cable and not to a register readback on a 60 MHz bus at all, and it has been
demoted twice for the wrong reason. It is now first on the list.

### Three things the datasheet says that ULPI does not

Reading the transceiver's own datasheet beside ULPI 1.1 found three places
where this project's core was wrong. None of them is why it fails to
enumerate — the STALL measurement above rules them out as the cause — and all
three are real, so they are fixed and modelled.

1. **The link must drive the bus in the cycle `dir` falls.** This core left
   the falling turnaround undriven, on the strength of ULPI §2.3.1's "data
   during the turnaround cycle is undefined". The part says what that costs:

   > "When the USB334x sends a RXCMD the Link is required to drive the data
   > bus back to idle at the end of the turn around cycle. If the Link does
   > not drive the databus to idle the USB334x may take the information on
   > the data bus as a TXCMD and transmit data on DP and DM until the Link
   > asserts stop." ... "The pull downs are not strong enough to pull the
   > data bus low after a ULPI RXCMD, the Link must drive the data bus to
   > idle after DIR is de-asserted." — DS00002646A §6.5.4.1

   A receive command whose ID or `alt_int` bit is set is a byte with bit 6 or
   bit 7 high, which is exactly a transmit command or a register command;
   left floating for one cycle it is read back as one. `ulpi_data_oe` is now
   `~dir`, which is also what §2.3.1's "dir is wired straight to the output
   buffers" says in the first place, and the transceiver model now **requires**
   it and complains if the cycle is left floating or driven with anything but
   `00h`.

2. **`dir` de-asserting ends a packet, whatever the link thought.** §3.8.2.4
   says so — "or `dir` is de-asserted, whichever occurs first" — and this
   core wrote `dir_fell && rx_active_q`, which reads the register's *old*
   value. A receive command that reports RxActive out of an idle bus sets
   `rx_active` on the very edge `dir` falls on, so the old condition cleared
   nothing, and nothing could afterwards: there was no packet left to close.

3. **A packet is bytes.** `rx_active` now reaches the endpoint only once a
   byte of the packet has been delivered. ULPI's RxActive is a statement
   about the line; what an endpoint needs is a statement about the bus. The
   two come apart on a transceiver whose full-speed receiver is not squelched
   while it transmits, and this one reports RxActive with nothing on the pair
   but the link's own answer: measured, `rx_active` high for over 1024
   clocks. A phantom packet like that swallows the next real one — its PID
   byte is filed as the *k*th byte of the phantom instead of the first byte
   of a handshake — and ULPI permits the transceiver's half of it, since
   §3.8.1.3 says only that a receive command "contains the status that is
   current at the time the RX CMD is sent".

A fourth was tried and **reverted**, and the reasoning is kept because both
readings are quoted from the same page. DS00002646A §6.4.2 says "The USB
Transmit ends when the Link asserts STP while NXT is asserted" and, as a
note, "The Link cannot assert STP with NXT de-asserted since the USB334x is
expecting to fetch another byte from the Link". At full speed `nxt` is one
pulse in forty clocks, so a `stp` in the cycle after it lands with `nxt` low,
and holding `stp` until the next `nxt` looked like the fix. It changed
nothing on the part, and the same datasheet says a held `stp` has a cost of
its own — "If the Link has held STP high the USB334x will hold DIR high until
STP is de-asserted" — so the one-cycle `stp` of ULPI Table 2 is what the core
does. The other reading of that note is ULPI's own "the Link must not assert
`stp` before the first byte has been consumed", which this core already
obeyed.

### What the model had wrong, and still has

Two defects in `tests/ip_library.rs`'s transceiver model, both of the shape
§11 of `ip/usb/usb_device_ulpi/README.md` warns about — the document, the core
and the model agreeing about something no device had been asked:

- **It kept the ULPI bus silent for the whole of the link's own packet and
  then sent one receive command, already at J.** The part sends a stream:
  "after STP is asserted each FS/LS bit transition will generate a RXCMD
  since the bit times are relatively slow" (§6.3.1). A link cannot find its
  own end of packet in one report that has already moved past it, which is
  why the model could not falsify a link that read every receive command as
  news about a host. The model's line transmitter now runs alongside its bus
  state machine, because a packet on a wire does not pause while a receive
  command goes out.
- **It had no way to be a transceiver that hears itself.** It has one now
  (`hearing_itself`), and a test enumerates through it.

**And it still cannot reproduce the board's failure.** The new test passes
with the fixed core and passes with the old one, so it is a new case covered
and not a regression test for defects 2 and 3 above, which rest on the
specification's words and on one measurement instead. That is stated here
rather than papered over, because a test that cannot fail is worth exactly
what it says and no more.

## An eight-bit bidirectional bus has been built, and a host has seen it

On 2026-09-27 `testdata/fpga/cynthion/bidir_bus.v` — **eight** pads that
drive, release and read themselves back on the die's **right** edge, the
auxiliary ULPI transceiver's own data balls — was compiled by this flow and
loaded into the LFE5U-12F of the Great Scott Gadgets Cynthion r1.4 attached
to the machine this was written on. So was
`testdata/fpga/cynthion/usb_ulpi_device.v`, which is `ip/usb/usb_device_ulpi`
behind those same eight pads: a USB full-speed device.

What the flow reports for the bus:

```
3736 configuration bit(s) set, 20 pad(s), 103 lookup table(s) and 31
flip-flop(s) configured, 31/24288 ff, 1/56 gb, 20/120 io, 103/24288 lut
routed 146 of 146 signal(s) with 1668 pip(s) over 1814 wire(s), and every
sink was walked back to its driver
all 3736 set bit(s) decode back through the database into 1060 arc(s), 314
field(s) and 103 word(s), with 0 unexplained, and the arcs they select are
exactly the 1060 the router chose
```

and for the device:

```
24458 configuration bit(s) set, 20 pad(s), 658 lookup table(s) and 275
flip-flop(s) configured, 275/24288 ff, 1/56 gb, 20/120 io, 658/24288 lut
routed 945 of 945 signal(s) with 11784 pip(s) over 12729 wire(s)
all 24458 set bit(s) decode back through the database into 7827 arc(s),
1363 field(s) and 658 word(s), with 0 unexplained
```

**100% of both bitstreams decodes, with nothing left over**, which is the
same standard `button_led` (114 bits), `clock_blink` (2491) and
`bidir_loopback` (2557) were held to — and is the standard that used to
refuse this design. The part accepted both and asserted `DONE` with no fault
bit: status `0x00200100`, as every run before it, which is exactly why
`DONE` is not the claim being made. The board went on enumerating at the
same Apollo serial number on the same bus and device number afterwards.

### The claim, and what is not being claimed

This milestone has a result that needs no eye, and it is worth stating
before anything else because it is the first of its kind here:

> **A host on the other end of a USB cable saw a device appear on this
> board's AUX port, and saw it appear and disappear on command.** The
> transceiver was reset, configured over the eight-bit bidirectional bus,
> **read back over the same eight pads**, and only then told to present the
> 1.5 kOhm pull-up that makes a host notice a device. The host noticed.
> `dmesg` says so, and "What the host saw" below has every line of it.

And what is not being claimed: **it does not enumerate.** `lsusb` does not
show `1209:0001`. The host detects the device as **low speed** where a
full-speed peripheral was asked for, and then talks to it at 1.5 Mbit/s,
which a full-speed device cannot answer.

**Why is now known, and it is not in this compiler.** The transceiver's own
Debug register reports its **D+ high** at the moment the host reports a
low-speed device, which is the host's **D-**; and in low-speed mode the
transceiver reports its D- high while the host reports full speed. The two
ends of the cable name opposite wires, so D+ and D- are exchanged between
them, and no register value and no attribute of a pad can undo a crossed
differential pair. Getting there needed the eight-bit bus to be proved exact
rather than merely turning around, and it is: a register nobody had written
was read off the part and came back as the `41h` the specification gives it.
"Where it stops, and it is not the bus" is the whole of it — fourteen
experiments on the part, each one bitstream and one `dmesg` window — and it
ends with the three physical things left to try.

### What a person should look for, on the bus design

`bidir_bus.v` is the design with a human observable; the USB device's
observable is `lsusb`. **Press and hold the button silkscreened `USER` for
three seconds and watch LED 1 come on and stay on while LED 2 never does.**
The other two tactile buttons end the experiment rather than perform it:
`PROG` reloads the FPGA from flash and `RESET` resets the debug
microcontroller.

The six FPGA LEDs are the row beside the legend `FPGA LEDs`, counted from
the end farthest from that button (`led_n[0]`, the diode `D7`):

| | LED 0 | LED 1 | LED 2 | LED 3 | LED 4 | LED 5 |
|---|---|---|---|---|---|---|
| nothing touched | dark | dark | dark | blinking | **lit** | dark |
| `USER` held three seconds | **lit** | **lit, and stays** | dark | blinking | lit | dark |

**LED 1 is the milestone.** It latches high on the first clock edge on which
all eight pins read back exactly the eight bits the pads are driving; LED 2
latches high if any pin ever reads back something else. They are latches and
not levels, so what a person sees a second later is what happened rather
than what is happening — LED 1 on and LED 2 off cannot be produced by a
glimpse. LED 4 says the transceiver has let go of the bus at all, which is
the precondition for the pads ever being asked to drive.

Each way of being wrong is a different picture, and `bidir_bus.v`'s header
has the table: LED 4 dark means the transceiver never released the bus (the
clock is not reaching its ball); LED 2 lit means a pin did not read back
what it drove; LED 3 frozen means the clock stopped and nothing else means
anything.

### Why driving these eight balls is safe, on a bus with another driver

These are the first pins this project drives whose net has a driver that is
not the FPGA: a USB transceiver chip. `bidir_loopback.v`'s E13 had a
resistor and an LED on it and nothing else. So the reason this is safe is
written out, and it is the bus's own arbitration rather than a workaround:

1. **`dir` is the interlock, and it is combinational.** A ULPI data bus
   belongs to the transceiver while `dir` is asserted and to the Link
   otherwise (ULPI 1.1 §3.3). Both designs release all eight pins the
   instant `ulpi_dir` goes high, with no register in the way, and
   `bidir_bus.v` additionally drives nothing at all unless a finger is on
   the USER button. With nobody at the board the FPGA never drives these
   pins.
2. **`bidir_bus.v` tells the transceiver to ignore the bus.** It holds `stp`
   high throughout, and a transceiver "must stop interpreting `data`" while
   `stp` is unexpectedly high — the specification's own protection for a
   Link that is not driving the bus properly yet (§3.12). So its walking
   pattern is not read as a transmit command or a register write.
3. **The transceiver is clocked and out of reset**, which is what the board
   asks for: `clk_dir='o'` in Great Scott Gadgets' platform file means the
   FPGA drives the 60 MHz to the transceiver's clock ball, and
   `rst_invert=True` means its reset is active low there.
4. **Great Scott Gadgets' own gateware drives these same eight balls.**
   `analyzer.bit` has all eight as `BIDIR_LVCMOS33`, which
   `tests/fpga_trellis.rs` reads back out of their file bit by bit, and
   `default_usb_connection = "aux_phy"` in the platform file means the AUX
   port is where their own designs put a USB device.
5. **Nothing else is touched.** The Type-C controllers, the VBUS switches
   and the pseudo-supply pins are left alone, which is what every gateware
   in the Cynthion repository does with them — they are declared in the
   platform file and used by none of it. The `CONTROL` port, where the
   Apollo debugger this board is programmed over lives, is a different
   transceiver on different balls and is not mentioned by either design.

**And the pin map was checked against the board and not only against the
platform file**, because a self-loopback cannot check it: a pad reads its own
pin, so two balls exchanged in the constraints are invisible to any amount
of driving and reading back. `cynthion.kicad_pcb`'s netlist gives, pad by
pad, FPGA ball → series resistor → transceiver pin:

| Signal | FPGA ball | transceiver pin |
|---|---|---|
| `DATA0` | F16 | 4 |
| `DATA1` | G15 | 5 |
| `DATA2` | G16 | 6 |
| `DATA3` | H15 | 7 |
| `DATA4` | J15 | 8 |
| `DATA5` | J16 | 10 |
| `DATA6` | K15 | 11 |
| `DATA7` | K16 | 12 |
| `DIR` | E16 | 1 |
| `NXT` | F15 | 3 |
| `STP` | E15 | 24 |
| `RESET` | J13 | 22 |
| `CLK` | D16 | 21 (`REFCLK/XI`) |

The three resistor arrays are straight through — element *n* joins
`AUX_PHY.DATAn` to `AUX PHY/DATAn` and nothing crosses — and the
transceiver's footprint labels its own pins `DATA0`…`DATA7`, so the map is
one-to-one from `ulpi_data[0]` to the transceiver's bit 0. That table also
rules out one explanation of the low-speed detection below, which is what it
was gathered for.

### Three pad tiles, not eight, and why that was the whole obstacle

The eight balls do not get a pad tile each. `iodb.json` puts them at:

```
F16 (72, 14, A)   G15 (72, 14, B)   ->  pad tile (72, 15)
G16 (72, 20, A)   H15 (72, 20, B)   ->  pad tile (72, 21)
J16 (72, 23, A)   J15 (72, 23, B)   ->  pad tile (72, 24)
K16 (72, 23, C)   K15 (72, 23, D)   ->  the same tile
```

So **three** pad tiles, and every one of the three holds more than one
bidirectional pad: two, two, and all four sides at once. That is exactly the
case "What could not be read back" described and the flow refused, because a
pseudo-differential `PIOA.BASE_TYPE` spells four of its ten bits in PIOB's
frames and a `PIOC.BASE_TYPE` spells four of its own in PIOD's. It is fixed,
and how is the next section.

### What fixed it: fewest bits unexplained, not the longest match

`TrellisDatabase::decode` resolved a field by the **longest** matching
pattern, which is what `libtrellis`' own `Tile::get_config` does. It now
resolves by two rules in order:

1. the reading that leaves **fewest of the tile's set bits unexplained**;
2. among those, the longest match — which is still what picks
   `OUTPUT_LVCMOS33` over the `NONE` whose single bit it contains.

Rule 1 is applied as a **fixed point** and not as a ranking, because "how
much does this reading leave unexplained" is a question about the whole tile
and not about one field: every field and mux sink takes the longest match
first, and then each in turn may change its reading for one that explains
strictly more, until none will. Each change strictly raises the number of
explained bits, which is bounded, so it terminates.

Two properties are what make this safe to do to the most load-bearing check
in this backend, and both are asserted rather than argued:

- **a tile with nothing left over is never touched.** There is nothing to
  improve, so the loop does not run and every reading is the longest match,
  exactly as before. Every bitstream this flow writes is of that kind —
  `unexplained == 0` is what `reticle fpga --bitstream` refuses to write
  without — so this cannot change how any of them is read.
- **it is not a licence to explain a bit twice, or to invent one.** A
  reading is still only ever one of the values the image's bits allow.

**Why this and not something else.** Two other rules were considered and
are worse. Scoring a value by how many of its bits no *other field* of the
tile could claim — a static property of the tile type, cheap and
image-independent — gets this case right for the same reason, but it is a
proxy: it answers "could another field explain this bit" where the question
is "does one". And restricting the new rule to enumerated fields, leaving
muxes on the longest match, was considered because a mux's reading is checked
against the router's own arcs while a field's is not, so giving a mux more
freedom trades a check away. The measurement settled it: across 701 167 set
bits of three vendor bitstreams **no mux ever moves**, so the exception
would buy nothing and cost a second rule to remember.

### What it was proved against, which is `ecppack`'s own output

The rule is judged on Great Scott Gadgets' own `ecppack` bitstreams for this
board, and the numbers are the point:

| | set bits | unexplained before | after |
|---|---|---|---|
| `analyzer.bit` | 250 001 | 34 | **5** |
| `selftest.bit` | 27 006 | 33 | **5** |
| `facedancer.bit` | 424 160 | 25 | **5** |

and the five are **the same five bits of one `DSP_SPINE_UL1` tile at
(col 3, row 13) in all three** — an unrelated gap this does not touch, and
one worth naming: the same five bits in three unrelated designs is a
property of the tile rather than of a design, most likely a feature the
fuzzers never named.
`tests/fpga_trellis.rs::the_only_bits_of_lattices_own_bitstreams_this_database_cannot_name`
asserts that list exactly rather than as a bound, because a rule that
explained *more* bits than it should would pass a bound and fail this.

Every reading that changed is a `PIO<s>.BASE_TYPE`, fifteen of them in
`analyzer.bit`, each going from a pseudo-differential output to `BIDIR_*` —
on the **left** edge's HyperRAM bus as well as the right edge's ULPI pins.
**Not one arc, word or other field changes anywhere**, in 250 001, 27 006 or
424 160 set bits.

And the check the fix was supposed to pass, which is the one
`docs/fpga-trellis.md` named in advance: `analyzer.bit`'s eight aux ULPI data
balls now read back `BIDIR_LVCMOS33` on **both** halves of every pair, and
**no bit of any of their pad tiles is left over**. Both halves of that are in
`what_lattices_own_packer_writes_for_a_bidirectional_pad`.

The other direction was checked too, because it is the one that matters more:
`button_led`, `clock_blink`, `bidir_loopback` and an eight-bit **top**-edge
bus decode to a report that is **byte-identical** to the one the old rule
produced — whole `Decoded::to_text()` compared, not just the counts.

### The case it does not fix, and cannot

Two ordinary **outputs** on one right-edge pair also set all ten bits of
side A's `OUTPUT_LVCMOS33D`, and there the longer reading leaves nothing
over either: the image is equally consistent with both readings, so no
accounting of bits can tell them apart and the differential one wins on
length. `analyzer.bit` has one at (col 72, row 18), whose side C is the aux
transceiver's own reset pin.
`src/fpga/trellis/mod.rs`'s
`a_field_is_read_as_the_value_that_leaves_fewest_bits_unexplained` asserts
that case as well as the three that work, on a fixture carrying the real
`PICR1` patterns for sides A and B, so the day it becomes resolvable the
test says so.

### And the other obstacle, which was not in the bitstream at all

Fixing `decode` made the bus build. It did not make the **USB device** build:
`ip/usb/usb_device_ulpi` failed after seven and a half minutes with

```
error: routing did not converge: 52 node(s) are still oversubscribed after
40 iteration(s), worst at X65Y14/CE0 (2 signals), X65Y14/CE0_SLICE
(2 signals), X66Y14/CE2 (2 signals), …
```

Every one of the 52 was a `CE`. The cause is that **a fabric does not always
give every bel its own pins**: an ECP5 slice is two flip-flops, and its
`CLK<c>_SLICE`, `CE<c>_SLICE` and `LSR<c>_SLICE` are one wire each for the
pair, which `src/fpga/trellis/sites.rs` declares faithfully. `fpga::place`
did not know, so it put two flip-flops with different clock enables in one
slice and handed the router a node with two signals on it.

`SiteRules` in `src/fpga/place.rs` is the whole fix, and it is read off the
architecture rather than hard-coded: **any two pins of any two bels in one
tile that resolve to one node are a constraint**, on every family. Two things
about it are worth stating:

- **agreeing means the same signal, including no signal at all.** A flip-flop
  with no enable is not indifferent to the enable wire: the mux that ignores
  it, `SLICE<l>.CEMUX`, is a setting of the *slice* and not of the
  flip-flop, so `configure_registers` cannot write a pair that disagrees
  either way round — one half would silently lose its enable. Equality is
  what both halves can be configured to do.
- **it is a legality constraint and not a cost**, so it belongs to
  `nearest_free`, to `macro_sites` and to every move `propose` offers, and
  the annealer never has to undo an illegal placement.

Nothing that placed before places differently, and that is checked rather
than argued: `clock_blink.bit` and `bidir_bus.bit` come out **byte for byte
identical** to the bitstreams built before the change. A design whose cells
share no pin — every family but the ECP5, today — takes the trivial path,
and a design that already agreed everywhere never has a move rejected, so
the search is the same search.

With it, the device places and routes in well under a minute where it had
failed after seven and a half: 31 seconds for a first top level of 578 lookup
tables, 40 for the one that shipped.

Two things "What remains" said were not possible turned out to be possible
once it was there, and both are in the device now: a **clock enable**, whose
tie `configure_registers` already skips when a signal reaches the pin — so a
routed `CE` and `CEMUX` at its `CE` default were always written correctly,
and only the placement was wrong — and an **asynchronous reset**, which 40
of this design's slices carry as a routed `LSR` with its `SRMODE`, because
which of a tile's two reset muxes carries a signal is exactly what the
routing can only say once the pair agrees.

### What the host saw

The observable for the device is `lsusb`, and this is the whole of what it
and `dmesg` say. The board's AUX port is on this machine's root port 7-5,
and **that is established rather than assumed**: a variant of the design that
detaches and re-attaches every 2.24 s, by releasing the core's reset from a
counter bit, produced attach events on 7-5 at exactly that period.

```
usb 7-5: new low-speed USB device number 96 using xhci_hcd
usb 7-5: device descriptor read/64, error -71
usb 7-5: device descriptor read/64, error -71
usb usb7-port5: attempt power cycle
usb 7-5: new low-speed USB device number 97 using xhci_hcd
usb 7-5: Device not responding to setup address.
usb 7-5: device not accepting address 97, error -71
usb usb7-port5: unable to enumerate USB device
```

So: **the host sees a device attach.** That is not a small thing. A host
notices a device when it sees the 1.5 kOhm pull-up on `D+` or `D-`, and on
this board that pull-up is not a pin the FPGA can drive — it is a bit of the
transceiver's Function Control register, reached only by writing that
register over the eight-bit bidirectional bus. An attach is therefore
**evidence that a register write crossed the bus**, from a witness on the
other side of a USB cable.

### That the bus turns around was measured, not inferred

The attach shows the bus works outwards. One experiment shows it works
**both** ways, and it is the sharpest thing this milestone has:

`usb_ulpi_link`'s start-up writes Function Control, reads it back, and only
continues if the readback is what it wrote. A variant was built in which
**every write before the readback leaves `TermSelect` clear**, so nothing a
host can see happens until the readback has matched, and the byte that turns
the pull-up on is written only afterwards. The host saw the attach.

So the transceiver drove the eight pads, the FPGA's input buffers read the
value off them, it was compared with what had been driven out of the same
eight pads, and the comparison passed — on real silicon, with a host as the
witness. That is an eight-bit bidirectional bus turning around, and it is
what this milestone is about.

What it does **not** prove is that no *permutation* of the eight lines is
involved: writing `X` and reading `π(X)` back through the inverse of the same
permutation matches for any `π`. That is why the pin map was traced through
the board's own netlist in the table above — and it is why the section below
reads a register **before writing anything to it**, which is the one reading
a permutation cannot survive. Function Control came back as the `41h` the
specification gives it, so there is no permutation: `π` is the identity.

### Where it stops, and it is not the bus

The device does not enumerate. The host detects **low speed**; a full-speed
peripheral was asked for, so the host then talks at 1.5 Mbit/s and nothing
the device says can be understood. Every experiment below was run on the
part, each is one bitstream and one `dmesg` window, and the reading it all
comes to is that **the transceiver and the host disagree about which of the
two data wires the pull-up is on**. Nothing in this compiler, in the ULPI
core or in its register conversation is between them.

#### What the transceiver was asked, over the real bus

The instrument for all of this is one idea: the gateware **decides whether to
attach** from a value it has read out of the transceiver, so a register's
contents reach a host as one bit of `dmesg`. A design that attaches read what
it was looking for; one that stays silent did not, and silence is not an
accident — it is `TermSelect` never written, so the transceiver never presents
a pull-up and the host's log stays empty. Both halves were simulated first
(`tests/ip_library.rs`'s transceiver model, which reports `writes 0` and
Function Control untouched on the silent path), and both halves were then
confirmed against the part.

| What was read, and when | What the gate was set to | What happened | What that says |
|---|---|---|---|
| Function Control (`04h`) **before any write** | `== 41h` | **attach** | the read path is exact, and the transceiver holds ULPI 1.1 Table 22's reset value |
| the same | `== 42h` | **silence for 45 s** | the negative control: the gate discriminates, so the reading above is `41h` and not "anything at all" |
| Vendor ID Low (`00h`) before any write | `== 24h` | **attach** | a second address, a read-only value **nobody wrote**, and the low byte of `0424h` — **Microchip (formerly SMSC)**. The register map is ULPI's, and this is the first time anything here has learnt what the part is from the part |
| USB Interrupt Status (`13h`) before any write | `VbusValid` set | silence | — |
| the same | `SessEnd` set | silence | — |
| the same | `== 00h` | silence | so `13h` is none of those, five microseconds after the reset pin is released; the VBUS comparators are analogue and this is too early to conclude anything, which is why nothing is concluded from it |
| Debug (`15h`), **once**, ~5 us after `TermSelect` | `LineState == 01` (J) | silence | — |
| the same | `LineState == 10` (K) | silence | — |
| the same | `LineState == 00` (SE0) | **attach** | five microseconds after the pull-up is connected the pair is still at **SE0**. A single read of LineState is a reading of a line on its way up |
| Debug (`15h`), **re-read for ~5 ms** | `LineState == 01` (J) | **attach** | given time, the transceiver reports its **D+ high and D- low** — the pull-up is where ULPI says `XcvrSelect = 01, TermSelect = 1` puts it |
| the same, with `XcvrSelect = 10` | `LineState == 10` (K) | **attach**, and the host says **full speed** | in low-speed mode the transceiver reports its **D- high** — again where ULPI says |

**So the first hypothesis is dead, and so is the reading that motivated it.**
Bits 0 and 1 of the data byte are not exchanged, the byte is not bit-reversed
and it is not off by a bit position: a read of Function Control *before
anything was written* came back as `41h`, whose bits 0 and 1 differ, and the
control set to `42h` — that same byte with those two bits swapped — produced
nothing for forty-five seconds. The command cycle and the data cycle are both
exact, in both directions, and so is the address field. Whatever is wrong,
**the eight-bit bus is not it**.

#### The two ends name opposite wires

Put the last two rows of that table beside what the host says, and there is
nothing left to interpret:

| Function Control | `XcvrSelect` | where ULPI puts the 1.5 kOhm pull-up | what the **transceiver** reports | what the **host** reports | so the host's high line is |
|---|---|---|---|---|---|
| `45h` | `01` full speed | D+ | LineState `01` — its D+ | **low speed** | its D- |
| `46h` | `10` low speed | D- | LineState `10` — its D- | **full speed** | its D+ |
| `44h` | `00` HS transceiver, FS termination | D+ | — | **low speed** | its D- |
| `47h` | `11` FS transceiver for LS packets | (nothing, on this part) | — | **no attach at all** | — |

Every row: the wire the transceiver pulls up and sees go high is the wire the
host calls the *other* one. `44h` is worth its own sentence, because bits 0
and 1 of it are both zero — no confusion in those two bits can touch it — and
"HS transceiver with FS termination" is the one state a high-speed-capable
device is required to sit in with the pull-up on **D+**. The host called it
low speed.

**D+ and D- are exchanged somewhere between the transceiver's data pins and
the host.** That is the finding. It is not a register value, because all four
were tried; it is not the bus, because the bus reads back exactly; and it is
not an edge rate, because no edge rate decides which wire a resistor is tied
to.

#### Why no register value can rescue it, and what would

USB is differential. With the pair exchanged, every J on the wire is a K at
the other end, so the SYNC field, the NRZI and every bit after them invert.
The two polarities the transceiver can be put in do not help either, and the
reason is worth writing out because it looks at first as though one of them
should:

- `XcvrSelect = 01`: the transceiver signals at 12 Mbit/s and its idle J is
  its D+ high, which is the host's D- high, which is a **low-speed** J. The
  polarities actually agree — and the host therefore talks at **1.5** Mbit/s
  to a transceiver running at 12.
- `XcvrSelect = 10`: the transceiver signals at 1.5 Mbit/s and its idle J is
  its D- high, which is the host's D+ high, a **full-speed** J. The
  polarities agree again — and the host talks at **12** Mbit/s to a
  transceiver running at 1.5.

In both directions the polarity is fine and the **rate** is wrong, and
nothing in Function Control sets one without the other. `XcvrSelect = 11`
would be the combination that does, and this part connects no pull-up for it
at all: `47h` produced no attach.

So what is left is physical, and it is a minute with the board rather than a
day with the compiler:

1. **Turn the AUX plug over**, which costs one second and is the cheapest
   thing that could possibly explain this. A Type-C receptacle has **two**
   D+/D- pairs, `Dp1`/`Dn1` and `Dp2`/`Dn2`, and a plug uses whichever of them
   its orientation puts it against; a board therefore has to wire both to the
   transceiver's one pair, and a board that wires the second one the wrong way
   round is crossed in exactly one of the two orientations and fine in the
   other. That is the shape of what was measured. **This is a guess about a
   board nothing here has read the schematic of** — LOW, in this file's own
   scale — and it is first on the list only because it is free.
2. **Try another cable**, and another port on the host.
3. If neither changes it, the exchange is on the board or in whatever the AUX
   pair passes through, and the next step is a different transceiver — the
   `TARGET` port's — rather than a different bitstream.

Any of those is decidable by exactly the software this used: load
`testdata/fpga/cynthion/usb_ulpi_device.v` and read `dmesg`. **`full-speed`
in that line instead of `low-speed` is the whole test**, and `lsusb -d
1209:0001 -v` is the answer.

#### What the earlier account had wrong

Two claims in the version of this section written before these measurements
should be read with what replaced them:

- "**the data cycle of a register write is sampled wrongly where the command
  cycle is not**" — no. Both cycles are exact. The evidence for it was that
  `XcvrSelect` behaved as though its two bits were exchanged; the register was
  never read back on hardware from a state nobody had written, which is what
  would have settled it, and when it was, it read `41h`.
- "**`SLEWRATE=FAST` is now the first thing to try**" — it is **demoted**, and
  this is the reason: the eight-bit bus reads back byte-exact in both
  directions at 60 MHz, so its edges are good enough, and no edge rate can
  change which of two wires a 1.5 kOhm resistor is connected to. It remains
  the one attribute of the platform file's ULPI resource this backend does not
  write, and that is still a gap — see "What remains" — but it is a gap and no
  longer a suspect.

One earlier experiment is also contradicted and should not be relied on: the
row that reported "command address `05h` instead of `04h` → attach" was used
to rule out an exchange of bits 0 and 1 of the bus. The pre-write readback
rules that out far better, and the `05h` variant is gone, so what it actually
did cannot be checked. **The pre-write readback is the one to cite.**

#### Two defects this found on the way

Neither is why it fails to enumerate; both are real and both are fixed.

- **The programmer refused bitstreams according to the first byte of their
  compression dictionary.** `src/program/lattice.rs`'s header walk skipped
  `LSC_INIT_ADDRESS` (`0x46`) as though it carried a four-byte word. It
  carries three reserved bytes and nothing else, so the walk landed four bytes
  into `LSC_WRITE_COMP_DIC`'s operand — the eight-byte dictionary — and read a
  dictionary byte as an opcode. `0x22` there happened to put it back on the
  payload command, which is why every bitstream this project had ever loaded
  worked; anything else sent it into the frames, where the first stray `0xe2`
  became a nonsense IDCODE and the part was refused as the wrong device. Two
  designs differing only in a Verilog parameter behaved differently, which is
  how it was found. All 256 first bytes are now a test.
- **The ULPI core believed one read of LineState, and the model could not
  produce the answer that makes that wrong.** Measured above: five
  microseconds after `TermSelect`, Debug reads `00h`; milliseconds later it
  reads `01h`. The core took the first answer, so `line_state` was SE0 with
  `phy_ready` high — `line_idle` false, and `se0_cnt` counting towards a
  **bus reset that nothing would end**, since a transceiver reports LineState
  only when it *changes*. The core now re-reads while it says SE0, and a bus
  reset now requires the pair to have been seen somewhere other than SE0.
  What the model had wrong is in `ip/usb/usb_device_ulpi/README.md` §11.

#### Putting a board back to a quiet state

`testdata/fpga/cynthion/quiesce.{v,rcf}` is now committed, because this is
the third time it has been needed. **A transceiver's registers outlive the
FPGA's configuration**: reconfiguring away from a design that set
`TermSelect` leaves the pull-up connected, and the host goes on seeing a
device that cannot answer and retrying the port every few seconds
indefinitely. `quiesce` holds `J13` — the transceiver's own reset, active low
at the ball — at zero, which puts Function Control back to `41h`, and lights
**LED 5 alone**, the end of the row nearest the `USER` button and deliberately
the opposite end from every other design here. It leaves the eight data balls
and `stp` out of the design entirely; `stp` because ULPI 1.1 §3.12 gives a
transceiver a weak pull-up on it and requires it to stop interpreting `data`
while it is unexpectedly high, so a pad that drives nothing leaves `stp` in
its protective state. Seventy-seven configuration bits, seven pads, nothing
clocked, and all seventy-seven decode.

The loop every experiment above used is: load `quiesce`, which makes the host
see a disconnect and re-arm the port, then load the experiment. Without the
disconnect a host that has printed `unable to enumerate USB device` will not
look again.

### What this settles, and what it does not

It settles that an eight-bit bidirectional bus builds, routes and decodes
completely on the edge where a ULPI bus lives, that the bits of three pads
sharing one tile can be read back through Project Trellis' database, that a
clock leaves the part on a pin of its own, that a clock enable and an
asynchronous reset can be placed and written, and that a design of 658
lookup tables and 275 flip-flops — nine times the largest thing this flow had
built before, which was `clock_blink`'s 72 — places and routes and decodes
with nothing left over.

It settles that a **host** saw this board present a device, and that the
eight-bit bus turned around: a value the transceiver drove came back through
the same eight pads and was checked against what had gone out, before
anything a host could see happened.

It settles that the eight-bit bus is **byte-exact in both directions**, which
the turnaround alone did not: a register nobody had written was read off the
part and came back as the value the specification gives it, `41h`, and the
same gate set to that byte with two bits exchanged produced nothing at all. It
settles that the transceiver is a **Microchip** part, read out of its own
Vendor ID register rather than off a platform file, and that its register map
and its `XcvrSelect` and `TermSelect` are ULPI 1.1's as
`ip/usb/usb_device_ulpi/README.md` reads them — the transceiver's own LineState
says the pull-up lands on the wire ULPI names, in both full-speed and
low-speed mode.

It does not settle enumeration, and it now says why: **the transceiver and the
host name opposite wires**, so D+ and D- are exchanged between them, and no
register value and no attribute of a pad can undo that. "Where it stops, and
it is not the bus" has the measurements and the three physical things to try.

It settles nothing about `SLEWRATE`, which is no longer a suspect but is still
unwritten, and nothing about VBUS: the one reading taken of the transceiver's
VBUS status was five microseconds after its reset pin was released, which is
too early for an analogue comparator to be believed.


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

**Somebody pressed the button, and it does what the table says.** On
2026-09-27 the board's owner reported: nothing touched, LED 2 lit and LED 3
blinking; `USER` held, LED 2 blinking and — in the half of the cycle where
LED 2 is off — **LEDs 0, 1 and 3 all on**.

That last clause is the whole milestone. LED 0 is lit exactly while the pad
drives its own pin low; LED 1 is lit exactly while the **input buffer of
that same pin** reads low. Seeing them on together is the turnaround
observed rather than inferred: the pad drove, and the pad read back what it
drove, through the same ball.

Everything under "What a person should look for" was written down before the
observation, which is the order the previous three milestones were done in
and the only order in which the observation is worth anything.

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

For the record, the only ball of the whole package that the platform file
never mentions at all is **B3** (top edge, column 4, side B, bank 0) — when
this was written only two edges were described and the sentence said "of the
two edges this backend describes"; all four are described now and the answer
is the same ball. It is a worse choice for the same reason turned around:
the file's silence is not a statement that the ball is free.

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

### What could not be read back: two bidirectional pads on one right-edge tile

> **Fixed on 2026-09-27, in the milestone at the top of this file.** This
> section is left as it was written, because the diagnosis is still the
> right one and the section it points forward to is the fix. Only the last
> paragraph has been brought up to date. `decode` now resolves a field by
> the reading that leaves fewest of the tile's bits unexplained, with the
> longest match as the tie-break, and an eight-bit bidirectional bus builds
> and decodes on the right edge.

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

So this was a limit of reading a `bits.db` back, not a wrong bitstream — the
silicon decodes bits and `F5B0` set is not a state `OUTPUT_LVCMOS33D`
produces; what the **longest match** cannot do is partition the tile's bits
between two PIOs of one pair. The check was left in place rather than
weakened, and the flow refused such a design, because "every bit decodes" is
the strongest thing this backend has and weakening it to admit a case would
weaken it for every case.

**The fix was in `decode` and not in `configure_io`, and it was the rule this
paragraph predicted**: "the longest match" became "the match that leaves
fewest bits unexplained", which on this tile picks `BIDIR_LVCMOS33` for side A
because `F0B3`, `F1B3`, `F8B3` and `F9B4` are covered by PIOB's own fields
either way. It got its own milestone, at the top of this file, and it was
held to the two conditions named here: every design in
`testdata/fpga/cynthion/` decodes to a byte-identical report afterwards, and
`analyzer.bit`'s eight ULPI pads read back `BIDIR_LVCMOS33` on **both** halves
of every pair with no bit of their tiles left over. What was not predicted
here is how much else it fixed: the same fifteen readings were wrong on the
**left** edge's HyperRAM bus too, and the vendor's own bitstreams went from
34, 33 and 25 unexplained bits to five each.

One prediction in this section was **wrong** and is worth leaving visible: it
said a ULPI data bus "is blocked on this and not on anything about the pad".
It was blocked on this *and* on something else entirely — two flip-flops of
one slice sharing a `CE` wire, which is a placement constraint and not a pad
or a bit. "And the other obstacle, which was not in the bitstream at all" at
the top of this file has it. A named obstacle being cleared is not the same
as the road being clear.

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

And it would settle all of that only as far as `DONE`, which the first
milestone in this file proved is worth nothing on its own: the LEDs were dark
and `DONE` was high. This one was then **watched**, and the pattern it showed
was the one written down in advance, including LEDs 0 and 1 lit together.

It settles nothing about a bidirectional **bus on a part**: one bit has been
loaded, not eight. An eight-bit bus *builds* on the top edge —
465 bits, 0 unexplained — and is refused on the right one, for a reason that
is about reading a bitstream back and not about writing one; "What could not
be read back" above has it, and it is the thing standing between this and a
ULPI data bus. It settles nothing about `SLEWRATE`, which every ULPI pin of
the reference asks for and this writes for none. Both are in "What remains".

> Both of those were settled the next day, and the milestone at the top of
> this file is where: eight bidirectional pads on the **right** edge have
> been loaded into the part. `SLEWRATE` was the leading suspect for why the
> USB device behind them does not enumerate for about a day, and is a loose
> end again — the bus reads back byte-exact at 60 MHz and the fault is a
> crossed pair on the far side of the transceiver.

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

**A design that instantiates library IP needs its sources named too**, because
there is no search path: `reticle fpga` takes a list of files and nothing will
go looking for the rest. Forgetting one now says so, at the instantiation,
before anything is mapped:

```text
error[I0034]: no module named `usb_device_ulpi` is defined
   --> testdata/fpga/cynthion/usb_ulpi_device.v:235:7
    |
235 |     ) u_dev (
    |       ^^^^^^^ `u_dev` instantiates it
    |
    = note: Reticle has no library search path, so nothing will find
      `usb_device_ulpi` later: add the file that defines it to this build
```

It used to be a *warning* that the instance had become a black box, followed by
a hundred errors of the form `reads … which nothing drives` naming signals of
the **top level** — every one of them an output of the missing instance, and
every one of them in the file that was correct. That cost real time twice: once
as a bogus `FAIL: LED 1 is lit and no host has configured anything` from a
healthy design, and once at the command line an hour later. The check is
`ir::Design::check_instance_targets`, called by `fpga::synthesize_for` and
`asic::synthesize_asic` before they map anything, and by `sim::Simulator` before
it runs; `src/ir/hier.rs` documents where the line falls.

**A black box on purpose still works**, because something declares it: a
`blackbox module` in the `.rtl` text form, an IP package whose sources are
`encrypted` (`docs/ip.md`), or a primitive the *device* declares — the check is
handed `Device::primitive_names`, so `EHXPLLL` and `TRELLIS_IO` are never
mistaken for a file left off the build. What is refused is a name that nothing
at all declares, which is always the same mistake.

The USB device is the one here that has dependencies:

```sh
reticle fpga testdata/fpga/cynthion/usb_ulpi_device.v \
    ip/usb/usb_device_ulpi/rtl/usb_ulpi_link.v \
    ip/usb/usb_device_ulpi/rtl/usb_device_ulpi.v \
    ip/usb/usb_device_fs/rtl/usb_ctrl_ep.v \
    --device ecp5-12f-CABGA256 \
    --constraints testdata/fpga/cynthion/usb_ulpi_device.rcf \
    --bitstream /tmp/usb_ulpi_device.bit
reticle program --device <serial> /tmp/usb_ulpi_device.bit
cargo test --features program -- --ignored usb_endpoint_one_loops
```

That last line is the observable: `tests/usb_loopback.rs` claims endpoint 1 and
sends bytes back and forth, and it skips with a printed reason when no such
device is attached. `rtl/usb_ctrl_ep.v` carries four modules at the moment, not
one, which is why three names are enough for a device that has seven.

**`RETICLE_TRELLISDB`, if set, names the database itself** — the directory
holding `ECP5/LFE5U-12F/tilegrid.json`. `reticle fetch` stores it under a commit
hash inside `~/.cache/reticle/prjtrellis-db/`, so the variable wants that inner
directory and not the one the fetch created; leaving it unset finds the cached
copy on its own, which is the easier way to be right.

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
half is not covered by an automatic check.** The flip-flop is where it shows
up: `CLK0.CLKMUX = CLK`, `LSR0.LSRMUX = LSR`, `SLICEA.GSR = ENABLED` and
`SLICEA.REG0.LSRMODE = LSR` are each "one bit, wanted clear".
`dropped_clear_bits` walks pips and does not see them, and neither does
`Decoded::unexplained`, because a stolen bit there is a bit some *other*
value of the same field wants — set `F54B10` and `CLK0.CLKMUX` reads `INV`
rather than leaving a bit unaccounted for. What covers it instead is weaker
and worth naming as weaker: those fields have exactly one writer
(`configure_registers`), nothing else in this backend touches a `PLC2`'s
control muxes, and
`a_flip_flops_settings_are_the_ones_lattices_own_packer_writes` asserts that
the decoding of the clocked design reads back the values it meant. A check
as exact as the one for arcs would need the passes to report what they wrote
so the decoding could be compared with it; that has not been built.

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

Those are
`tests/fpga_trellis.rs::the_database_describes_one_part_of_the_ecp5_family`'s
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

## What a build costs, and where it was being spent

Every hardware iteration pays for this, so it is worth the same treatment
as a tile rule: measure, then explain. The numbers below are one idle-ish
64-core Linux box, release build, `ecp5-12f-CABGA256`, and they are
**wall-clock**, so they belong here and never in an assertion. What *can*
be asserted is beside them: `PlacementReport::work` and
`RoutingReport::visited`/`queued` count work, not seconds, and mean the
same thing on a slower machine.

`reticle fpga --timing` prints one line per stage, which is where the
tables come from.

### First: it was a debug build

The complaint that started this was a **thirteen second floor** on a design
with no lookup tables in it. That number is `cargo build` without
`--release`:

| `leds.v` (0 lookup tables) | debug | release |
|---|---|---|
| whole flow | 15.3 s | 2.06 s |

So the floor was 2 s, not 13. It is **1.17 s** now, and the rest of this
section is what the 2 s was. The debug figure is kept because it is the
first thing to check when a build feels an order of magnitude too slow:
this flow is five hundred million array accesses with bounds checks on.

### The fixed cost: 1.84 s of 2.06 s, before the design was looked at

| stage | before | after |
|---|---|---|
| read the database | 0.03 s | 0.03 s |
| **expand the fabric** (`TrellisDatabase::load`) | 0.67 s | **0.06 s** |
| **build the graph** (`Arch::build_graph`) | 1.17 s | **0.80 s** |
| place (`SiteRules::find`, mostly) | 0.16 s | 0.14 s |
| whole flow | 2.06 s | 1.17 s |

Callgrind on that run: 31.4 G instructions, 98% of them inside
`write_ecp5_bitstream`, and `BTreeMap<&str, _>::insert` alone was **32%**
of the program.

**The expansion walked the die once per position.** "What the routing graph
is" explains why the loader has to collect every reference of every tile
before it declares anything — a `CIB+PICT1` owns `JA0` because a `PIOT0`
elsewhere says `S1E1_JA0` — and there are about a million and a half of
those references. It was walking them per *position*, calling
`TileDatabase::wire_names` (which builds a `BTreeSet`) once per position
per tile and `composition_key` (which joins a type list into a fresh
`String`) once per reference. But `bits.db` belongs to the **family**: one
tile type spells the same names wherever it sits, and the only thing that
varies with the position is which composition an offset lands on. So the
names are classified once per type, grouped by the offset they point at,
and a group is handed to a neighbouring composition the first time that
(type, offset, composition) triple appears — about a hundred thousand set
insertions over this die instead of three million. The sets are the same,
because a set does not count how often something was inserted, and
`references_off_the_grid` is still 3840.

**The graph build asked a linear question a million times.** `is_global`
was a scan of this die's 467 global wires, asked once per wire of the die:
half a billion string comparisons for an answer that is almost always no.
And the wire index hashed sixteen million keys with SipHash-1-3, which is
the right default for a map that may be fed hostile keys and the wrong one
for a map whose keys came out of a chip database. A set and a
multiply-rotate mixer (`arch::FastHasher`) between them took hashing from
28% of the build to noise.

**What is left, and the obvious next step.** 0.80 s of the remaining 1.17 s
is still `build_graph`, and most of it is now the irreducible-looking part:
1.1 million `Wire`s each owning a `String`, 8.3 million `Pip`s, and a
`HashMap` lookup per resolution. nextpnr does not pay this at all because
it loads a **prebuilt binary chipdb**. The same move is available here and
has an obvious home and an obvious invalidation key: `reticle fetch` already
stores the database under `~/.cache/reticle/<name>/<commit hash>/`, so a
derived `graph-<part>-<package>.bin` beside it is keyed on exactly the thing
that can make it stale. It has not been built, because at 1.17 s the fixed
cost is no longer what a build waits for.

### The per-cell cost: a thousand-sink net, rescanned on every move

`usb_host_target.v` is 3168 lookup tables and 1108 flip-flops, and it took
**694 s**. `--timing` put 489 s of that in placement and 202 s in routing,
and `PlacementReport::work` said what the 489 s was in one number:

```
annealing: 103 temperature(s), 70693890 move(s), 32026441 accepted
work: 72428913 legality test(s) over 1070412963 step(s),
      84532947396 cost pin read(s), 681 snapshot(s)
```

**84 532 947 396 pin reads for 70 693 890 moves: 1196 per move.** The cost
of a move is the change in half-perimeter wirelength of the signals it
touches, and `cost_of` worked that out by reading every pin of every one of
those signals, twice — once before the move and once after. This design's
clock net has 1136 sinks. A move of one cell cannot change where the other
1135 pins are, and every one of those reads was a
`graph.sites[site].tile`: a random probe into a five-megabyte array of site
records. 84.5 billion of those at about seventeen cycles each is 489
seconds, near enough.

So the growth was never mysterious. The move count is the textbook
`10 · n^(4/3)` per temperature, which is 155× more moves for 44× the cells;
the extra factor is that the cost of *one* move grows with the design's
largest net. `n^(4/3)` × fan-out is the superlinearity.

`place::Spans` is the structure that was missing: per signal, **how many of
its pins are in each tile column and in each tile row**, with the lowest and
highest occupied index of each. Moving a pin is two decrements, two
increments and a walk of as many columns as the box really shrank by —
nothing in the net's fan-out at all. It is **exactly** equal to what
`signal_hpwl` computes, because a bounding box is decided by which columns
and rows hold a pin and by nothing else, so the annealer accepts the same
moves in the same order and the placement does not change.

Two smaller things in the same loop, found the same way:

- `SiteRules::allows` read **418** entries per candidate move on
  `clock_blink.v`. `control_fits` asked `after(site)` — a scan of the move
  and a look in the placement — once per *pool per site*, six times over
  for an ECP5 logic tile, and `per_site` was one entry per shared *pin*, so
  a check read a hundred and fifty of them because the distributed-RAM bel
  shares pins with all eight of its tile's lookup tables. Occupants worked
  out once, and the entries grouped by the other site, make it 13.5.
- `matchable`, Kuhn's algorithm over a control pool, walked all thirty-two
  bits of its mask. A pool has **two** wires.

| | before | after |
|---|---|---|
| legality steps, `clock_blink.v` | 199 067 369 | 6 448 799 |
| cost pin reads, `usb_host_target.v` | 84 532 947 396 | 1 111 051 172 |
| `place`, `usb_host_target.v` | 489.4 s | 135.7 s |

### The router's inner loop, which was the same mistake one size down

```
routed 4408 signal(s) with 63301 pips over 67709 node(s)
work: 532127182 node(s) visited, 1287538849 queued
```

Half a billion nodes popped and 1.3 billion edges relaxed, and each one
loaded a `Wire` — fifty-odd bytes with a `String` in it — to ask for two
coordinates, then chased a twenty-byte `Pip` to find out where the edge
went. `route::Geometry` is the four numbers the distance estimate uses plus
the node's base cost; a per-node table of pip *targets* means the inner loop
walks two sequential arrays; a node's estimate is cached beside its cost so
a stale queue entry costs one comparison; and the queue's key is
`f64::to_bits`, which for a non-negative cost orders exactly as `total_cmp`
does and compares in one instruction. Same routes, same counts, and
`route` on this design went from 202 s to 114 s.

### Doing less annealing is a loss, not a trade

**This section is superseded and kept for its measurement.** Lowering the
effort *was* a loss, under the schedule described below; it is now a
trade, because the schedule changed. `docs/fpga-placement.md` has what
replaced it and what it is worth — in short, the annealer improved
nothing at all because its moves had no range limit, and effort 1 is now
a 39% faster whole flow than the old default was, to a shorter placement.
What follows is the measurement that said the cheap version of the fix
does not work, which is still true.

What is left of a big build is the **number of moves**, and the obvious lever
is the multiplier in front of them. The schedule is VPR's — `effort · n^(4/3)`
moves per temperature — and this flow's effort is 10, VPR's high-quality
setting; VPR's own default is 1. At 4304 movable cells, 10 is 70.7 million
moves and 135 s. `--place-effort <n>` exposes it, and the answer it gives is
worth having in writing, because it is the opposite of what a speed knob is
supposed to do.

| `usb_host_target.v` | effort 1 | effort 3 | **effort 10** |
|---|---|---|---|
| moves | 6 864 042 | 20 797 269 | 70 693 890 |
| `place` | 36.8 s | 92.5 s | 135.7 s |
| wirelength, before → after | 28699 → **28699** | 28699 → **28699** | 28699 → **23770** |
| nodes the router then visited | — | 1 110 252 586 | **532 127 182** |
| `route` | — | 409.7 s | **113.5 s** |
| pips | — | 62 166 | 63 301 |
| whole flow | — | 505.9 s | **251.1 s** |

**At 1 and at 3 the annealer improves nothing at all.** Not a little less —
nothing: it ends at exactly the wirelength legalisation handed it, having
tried seven and twenty-one million moves to get there. The reason is the
start temperature, `20 ×` the spread of a random walk's cost changes, which
is hot enough to scatter the placement; recovering from that takes a certain
number of moves per temperature and below it the walk never gets back under
where it began. 103 temperatures at effort 10 do get back, and 21% further.

And the saving is not a saving, because the router pays for the placement it
is given. Effort 3 saves 43 s of placement and spends **296 s** more of
routing, on twice the nodes visited, for 1135 fewer pips over a worse
spread. The whole build doubles.

So the move count is not where the next win is; the **schedule** is. VPR
does not use a fixed `cooling` and a fixed cap either — it adapts the
temperature step to the acceptance rate and stops when the acceptance rate
says the walk is done, which is what makes a smaller `inner_num` usable
there. This placer cools by 0.9 a hundred and twenty times whatever happens.
That is a real piece of work and nothing here has done it; what this section
records is that the cheap version of it does not work, and the measurement
that says so.

**It has been done since, and the diagnosis above was half right.** The
start temperature was one of two causes; the other, and the larger, was
that a move could send a cell to any site of its kind on the die, so
there was no temperature at which a move was both acceptable and
informative. With VPR's range limit and its acceptance-rate cooling,
`usb_host_target.v` places to a wirelength of 13 188 rather than 23 770 in
the same placement time, and at effort 1 the whole flow is 118.5 s
against 193.0 s — which is this build re-measured in the same sitting, not
the 251.1 s in the table above, since the two sittings are not comparable.
`docs/fpga-placement.md` is the account.

### A sharper distance estimate was tried and rejected

This is worth leaving here, because the reasoning looked airtight and the
part of it that was wrong is the interesting part.

`RouteOptions::astar_weight` charges 0.3 per tile of Manhattan distance
still to cover, and its own documentation explains that an estimate above
what a tile really costs makes the search stop at the first path it finds
rather than the cheapest: "on a fabric whose span-4 lines cross four tiles
for one node, a tile costs about a quarter of a node". **Every wire of this
die has span `0 0`** — see "What the routing graph is" — so on this family
every pip that crosses a tile boundary costs a whole node, one tile of
distance costs at least one node, and the admissible estimate is 1.0 rather
than 0.3. The search was being asked to look three times further than the
fabric requires.

It measures exactly as predicted, and on `clock_blink.v` the routes come out
**better**: 2 442 457 nodes visited instead of 5 953 689, 1084 pips instead
of 1089, 2481 set bits instead of 2491, every one decoding. On
`usb_host_target.v` the router drops from 114 s to 57 s.

And it is still wrong, for a reason that is not about admissibility at all.
The estimate is scaled per node by the node's own base cost, because
`CLOCK_PREFERENCE` makes a clock-network node cost a twentieth of a data
wire; a clock's cheapest route is eighteen hops of cheap wire against seven
hops of expensive wire, and whether the expansion finds it depends on how
far past the greedy answer it looks. Sharpen the estimate and it stops
looking. `wide_state.v` then routes its clock through general routing, and
`clock_network_use` refuses the design — correctly — with *3 of 3
flip-flop(s) have a clock that did not arrive on a global clock network*. On
`usb_host_target.v`, which keeps its network, the routes get 795 pips
**longer**.

So the weight and the clock preference are one knob with two ends, and
moving either without the other costs a clock network. What the measurement
leaves behind is the shape of the real fix: the estimate has to be
admissible against the *cheapest* class of wire a path might use, not
against the class of the node it is standing on — which is a change to how
`node_base` and `astar_weight` compose, not a number. Nothing here has
built that.

### Where a build's time goes now

| design | lookup tables | before | after |
|---|---|---|---|
| `leds.v` | 0 | 2.18 s | 1.17 s |
| `quiesce.v` | 0 | 2.13 s | 1.38 s |
| `button_led.v` | 1 | 2.11 s | 1.42 s |
| `clock_blink.v` | 72 | 4.46 s | 2.31 s |
| `bidir_loopback.v` | 72 | 4.41 s | 2.89 s |
| `usb_host_target.v` | 3168 | 694 s | 252 s |

Those are best-of-three, and the "after" column for the small designs was
taken while something else on the machine was using most of it, so the
smallest designs are flattered least — `leds.v` measures 1.17 s on a quiet
box and 1.44 s on a busy one, which is the size of the noise on a
one-second number and the reason the work counters exist. Every one of the
six produces the **same bitstream it produced before**: 61, 77, 114, 2491,
2557 and 128 583 set bits, 0 unexplained in each, and the arcs the bits
select are the arcs the router chose.

`usb_host_target.v` is 1.0 s of fixed cost, 135.7 s of placement and
113.5 s of routing, so the two sections above are now the whole of a large
build and the database is 0.4% of it.

Nothing here is parallel and nothing here was made parallel: a constant
factor of three is not what sixty-four threads are for, and the three
structures above are smaller to parallelise than what they replaced. What
*would* parallelise, for whoever does it: `Arch::build_graph`'s per-tile
expansion is independent once node ids are handed out, `SiteRules::find` is
per tile, and the router's rip-up-and-reroute iteration is the classical
PathFinder parallel target — one signal per thread with the congestion
arrays shared, which is what nextpnr does. The annealer is the hard one and
the usual answer is spatial partitioning rather than locks.

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
| Pads this backend declares | **197**: 56 on the top edge, 64 each on the left and the right, 13 on the bottom |
| Balls it leaves out | **none.** Every ball `iodb.json` names for this package is on an edge this describes, has its tiles where that edge's rule says, and has a bank in `pio_metadata` |
| Banks with a rail | 7: 0 and 1 (top), 2 and 3 (right), 6 and 7 (left), 8 (bottom). This die has no bank 4 or 5 |
| Graph nodes | 1 096 425 |
| Graph edges | 8 270 828 |
| Global clock networks | 16 |
| Clock buffers (`DCC`) | 56: twelve at the top, fourteen on each of the left and right, sixteen at the bottom |
| `lut` sites | 24 288 |
| `ff` sites | 24 288 |
| `lutram` sites | 3036 |
| `bram` sites | 56 |
| **What Lattice specifies for this part number** | **12 144 LUT4, 12 144 registers, 1518 distributed RAMs, 32 block RAMs** — half the die for logic, 32 of 56 for block RAM. The next section is about that |

`tests/fpga_trellis.rs::the_database_describes_one_part_of_the_ecp5_family`
asserts every one of those, so the table cannot drift from the database.

**The LFE5U-12F and the LFE5U-25F are the same die.** Their three
per-part files in `prjtrellis-db` are not merely byte-identical — at
commit `015e033` the two directories are **the same tree object**,
`76b742b1e18d8a3a37a9f6c1c9a51547b6f6c683` — their `devices.json` entries
differ in the `idcode` field and in nothing else, nextpnr loads one
`chipdb-25k.bin` for both, and Lattice's own TN-02039 Table B.4 gives
both 7562 frames of 592 bits. Only the identifier tells them apart, which
is why `TrellisFabric::check_idcode` is not a nicety: a bitstream from the
wrong half of the database would configure the part and assert `DONE`.

## The part number is specified at half the die, and the flow says so now

The section above says the LFE5U-12F and the LFE5U-25F are the same die
and treats that as a fact about *identifiers*. It is also a fact about
**capacity**, and for a long time nothing in this flow said so: a 12F
design placed on all 24 288 lookup-table sites of the die, routed, got a
bitstream in which every bit decoded, and was reported as using "half the
part". It was using all of a part Lattice sells as half that size.

### What each source actually says

**CHECKED — Lattice.** Table 1.1 of the *ECP5 and ECP5-5G Family Data
Sheet*, `FPGA-DS-02012-3.4`, page 12, read from the document rather than
remembered:

| | LFE5U-12 | LFE5U-25 | LFE5U-45 |
|---|---|---|---|
| LUTs (k) | **12** | **24** | 44 |
| sysMEM Blocks (18 kb) | **32** | **56** | 108 |
| Embedded Memory (kb) | 576 | 1,008 | 1944 |
| Distributed RAM Bits (kb) | **97** | **194** | 351 |
| 18 × 18 Multipliers | 28 | 28 | 72 |
| PLLs/DLLs | 2/2 | 2/2 | 4/4 |
| 256 caBGA (SerDes/I/O) | 0/197 | 0/197 | 0/197 |

So the derating is **not uniform**: logic and distributed RAM are exactly
half, block RAM is 32 of 56, and the multipliers, the PLLs and the 197
balls of the caBGA-256 are the *same on both part numbers*. Nothing
geometric explains a set of factors like that. §2 of the same document
says "Each slice contains two LUT4s feeding two registers", which is how
the register count is known: it follows the lookup tables, so 12 144.

**CHECKED — Project Trellis.** The database draws no distinction at all.
At commit `015e033` of `YosysHQ/prjtrellis-db`, the git **tree object**
for `ECP5/LFE5U-12F` and for `ECP5/LFE5U-25F` is the same object,
`76b742b1e18d8a3a37a9f6c1c9a51547b6f6c683` — not two identical copies,
one tree listed twice. (`LFE5UM-25F` and `LFE5UM5G-25F` likewise share
`04caf2fb74c0`, and the three 45Fs share `7486f1c6798c`.) `tilegrid.json`
fetched for the 12F is 2 806 980 bytes with MD5
`f96e14359ba5a53cc541e676fbe37a2e`, and so is the 25F's. Counted out of
that one file: **3036 `PLC2` tiles, 12 144 `SLICE` sites** — two LUT4 to a
slice, so 24 288 — and `devices.json` gives both part numbers
`max_row` 50, `max_col` 72, 7562 frames of 592 bits, differing only in
`idcode`.

**CHECKED — this backend.** `graph.site_counts()` for the 12F is 24 288
`lut`, 24 288 `ff`, 3036 `lutram`, 56 `bram`, 56 `gb`, 197 `io`, pinned by
`the_database_describes_one_part_of_the_ecp5_family`. The 56 block RAMs
are the 25F's figure, which that test already said in a comment.

**CHECKED — nextpnr.** `ecp5/arch.cc` upstream loads
`ecp5/chipdb-25k.bin` when the chip is `LFE5U_12F`, together with the
25F, the UM-25F and the UM5G-25F, and applies no restriction of any kind;
`ecp5/main.cc` mentions the 12F exactly once, to offer `--12k`. **So the
state of the art does not solve this problem — it ignores it.** A design
built with yosys and nextpnr for a 12F may use the whole die and
routinely does, which is also why this is a warning here and not a
refusal.

**NOT ESTABLISHED — which sites.** *Nothing read here says which 12 144
of the 24 288 Lattice covers*, and the shape of the derating argues that
the question may not have a geometric answer: a contiguous half of the
die would take half the DSP columns and half the I/O with it, and the
data sheet gives the 12F all of both. Candidate explanations not
distinguished by anything available — speed binning on a fully present
fabric, a per-part test program that only guarantees part of it, a
fuse-free marketing split — all predict the same open database. Lattice
Diamond presumably has the answer in its own device data; that is not
something this project can read.

### What the flow does about it

`count` in a `.dev` file now means **what the part number is specified to
have**, and it is a different question from what the fabric enumerates.
`Device::specified_count` answers the first, `graph.site_counts()` the
second, and `Device::over_specification` is the only place that compares
them. A design over the specified figure is `F0400`, a **warning**:

```text
warning[F0400]: this design needs 33 `DP16KD` and `ecp5-12f-CABGA256` is specified to have 32
 --> wide_store.rtl:1:8
  = note: the fabric database describes the die, which may serve several part numbers; the
          sites beyond the specified count exist and this build will use them, but which of
          them the part number covers is not stated in any database Reticle reads, so
          placement cannot be confined to them
```

It is a warning and not an error for one reason, and it is the reason
above: **the covered region cannot be identified, so nothing can be done
about the overrun except say it.** Refusing would also refuse builds that
nextpnr makes every day and that work. What the diagnostic buys is that
the number stops being invisible.

The counts the ECP5 file now carries, from the table above:

| device | `LUT4` | `TRELLIS_FF` | `TRELLIS_DPR16X4` | `DP16KD` |
|---|---|---|---|---|
| `ecp5-12f-CABGA256` | 12 144 | 12 144 | 1518 | 32 |
| `ecp5-25f-CABGA381` | 24 288 | 24 288 | 3036 | 56 |
| `ecp5-45f-CABGA381` | 43 848 | 43 848 | 5481 | 108 |

The `TRELLIS_DPR16X4` figures are derived and exact rather than quoted:
Lattice counts eight bits of distributed RAM per LUT4 (24 288 × 8 =
194 304, which is the 25F's "194 kb"), a `TRELLIS_DPR16X4` is 64 bits, so
the number of them is the LUT4 count over eight.

### What this changes for the designs already here

Every design under `testdata/fpga/cynthion/`, mapped for
`ecp5-12f-CABGA256`, against the 12 144 the part number is specified to
have — CHECKED, by running `reticle fpga --report` over each:

| design | `LUT4` | of 12 144 | `TRELLIS_FF` | `TRELLIS_DPR16X4` |
|---|---|---|---|---|
| `usb_crypto_console` | **11 955** | **98.4 %** | 2999 | 18 |
| `crypto_console_ulpi` | **11 888** | **97.9 %** | 2956 | 18 |
| `crypto_console` | 10 679 | 87.9 % | 2477 | 0 |
| `usb_proxy_target` | 3449 | 28.4 % | 1137 | 10 |
| `usb_host_target` | 3181 | 26.2 % | 1109 | 28 |
| `usb_hub_target` | 2531 | 20.8 % | 979 | 2 |
| `usb_cdc_uart` | 1877 | 15.5 % | 845 | 18 |
| `usb_ulpi_trace` | 1321 | 10.9 % | 2029 | 8 |
| `usb_ulpi_device` | 1063 | 8.8 % | 417 | 16 |
| `bidir_bus` | 103 | 0.8 % | 31 | 0 |
| everything else | ≤ 74 | | | |

`examples/mos6502_monitor` is 6183, 50.9 %, which its README already
states against 12 144.

**Nothing here is over the line, and two designs are within two per cent
of it.** That is the result worth having: `usb_crypto_console` at 11 955
of 12 144 has been reported for months as "11 955 / 24 288", which reads
as a part half empty, and it is a part at 98.4 %. The 11 955 figure is
not new — `ip/crypto/sha256/README.md` recorded it, and running the
design again reproduced it exactly, which is what makes this table a
measurement rather than a restatement.

It is worth saying what this does **not** settle. `ip/crypto/sha256`
records that `usb_crypto_console` *does not route*: 886 nodes still
oversubscribed after 40 iterations, with a named control wire the
architecture does not describe. A design at 98 % of its part number is
exactly where congestion is expected, so the utilisation is a second
reading of that failure — but the router was given all 24 288 sites and
still gave up, so it is not an explanation, and the named wire is still
the thing to act on. The honest statement is that the two facts are
consistent and neither implies the other.

## Where a pad's bits are, and how that was established

This is the part of an ECP5 that no amount of reading a database settles,
and it is where a wrong answer is least visible: a bitstream with a pad
configured one column over still loads and still asserts `DONE`.

### The buffer is not where the bits are, and all four edges say so differently

Before anything routed, "a pad" meant a set of bits, and the bel was put
wherever they were. A routed design makes the distinction unavoidable,
because a bel's pins resolve in the tile the bel sits in and every
top-edge position has a `PADDOB_PIO` of its own. So:

- **the buffer** — the three wires a PIO presents to the fabric,
  `PADDO<L>_PIO`, `PADDT<L>_PIO` and `JPADDI<L>_PIO` — is at the position
  `iodb.json` gives the ball, and that is where the `io` bel goes;
- **the bits** are wherever the edge's rule puts them, which on the top and
  bottom edges is a column east for side B and on the two long edges is a
  row south for every side.

| | Top edge | Right edge | Left edge | Bottom edge |
|---|---|---|---|---|
| Sides per position | A, B | A, B, C, D | A, B, C, D | A, B |
| Buffer | the ball's `(col, row)`, `PIOT0` | the ball's `(col, row)`, `PICR0*` | the ball's `(col, row)`, `PICL0*` | the ball's `(col, row)`, `PICB0` — **for both sides** |
| Pad tile: standard, hysteresis, pull, slew | `(col + [B], 0)`, `PIOT0` / `PIOT1` | `(col, row + 1)`, `PICR1*` | `(col, row + 1)`, `PICL1*` or `MIB_CIB_LR` | `(col + [B], 50)`, `PICB0` / `PICB1` |
| Second copy of the standard | `(col + [B], 1)`, `PICT0` / `PICT1` | A, B: `(col, row)`, `PICR0*`; C, D: `(col, row + 2)`, `PICR2*` | A, B: `(col, row)`, `PICL0*`; C, D: `(col, row + 2)`, `PICL2*` or `MIB_CIB_LR` | **none: the pad tile is the only tile** |
| `CIB` that ties the data and enable | `(col + 1, 1)`, wire `JA0` / `JB0` | `(col - 1, row)` for A, B and `(col - 1, row + 2)` for C, D, wire `JA0`/`JA3` | `(col + 1, row)` for A, B and `(col + 1, row + 2)` for C, D, same wires | `(col + [B], row - 1)`, wire `JA0` / `JB0` |
| What `OUTPUT_LVCMOS33` costs in the pad tile | 6 bits | 6 | 6 | **7** |
| What `BIDIR_LVCMOS33` costs | 8 | 8 | 8 | **10** |
| What `INPUT_LVCMOS33` costs | 5 | 5 | 5 | 5 |
| Second copy: output / bidir / input | 2 / 2 / 0 | 2 / 2 / 0 | 2 / 2 / 0 | the same tile again |
| Hysteresis, pull mode, slew rate | one bit each | one bit each | one bit each | one bit each |
| Bank rail | `BANKREF<bank>`, anywhere | same | same | same |

The `CIB` row of that table is **not hardcoded**. Nothing in the loader
knows that `JA0` is the top edge's answer: it reads the buffer's own
`.fixed_conn` — `JPADDOB <- S1E1_JA0` on the top edge, `JPADDOD <-
S2W1_JA3` on the right, `JPADDOD <- S2E1_JA3` on the left, `JPADDOB <-
N1E1_JA0` on the bottom — resolves the direction prefix, and builds the
field name `CIB.<wire>MUX`. That is what nextpnr does too, by walking the
pips uphill of the wire, and it is what lets one piece of code serve all
four edges. Nor are the tile *types* hardcoded: the loader asks which tile
of a position declares `PIO<L>.BASE_TYPE`, which gets the right answer
through all twelve spellings of the right edge (`PICR1`, `PICR1_DQS0`,
`PICR1_DQS3`, `PICR2`, `PICR2_DQS1`, `MIB_CIB_LR_A`, …), the eight of the
left (`PICL0`, `PICL0_DQS2`, `PICL1`, `PICL1_DQS0`, `PICL1_DQS3`, `PICL2`,
`PICL2_DQS1`, `MIB_CIB_LR`) and the seven of the bottom (`PICB0`, `PICB1`,
`EFB0_PICB0`, `EFB1_PICB1`, `EFB2_PICB0`, `EFB3_PICB1`, `SPICB0`) where
nextpnr carries a set of names per edge, and **refuses** rather than
guessing if two tiles of one position both declare it. At three positions
of the left edge the ambiguity is real and the refusal is what keeps it
honest: `(0, 13)`, `(0, 25)` and `(0, 37)` each hold a `MIB_CIB_LR` *and*
a `MIB_CIB_LRC`, and only the first declares the field.

**This table used to say that the left edge was "the right edge mirrored
and would probably work", and the reading was half wrong**, which is why
the correction is worth keeping visible. What mirrors is the column: the
tiles are at column 0 and the `CIB` is one column *east* where the right
edge's is west. What does **not** mirror is the row arithmetic — the pad
tile is one row *south* and the C/D second copy two rows south on **both**
long edges, because the die's rows do not reverse. A mirror written in good
faith would have put every left-edge pad's bits one row the wrong way, in
the tile of a different ball of the same edge, and it would have decoded
perfectly against itself. The next two sections are the measurement that
settles it.

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

### The left edge, checked against the port it unblocks

This backend described two edges of four for a long time, and the cost was
concrete: **every ball of a Cynthion's TARGET USB port is on the left edge,
and so are all three of the board's VBUS switches.** A design naming one of
them was refused with `constrained to package pin R2, which the architecture
maps to no usable site`, so a USB host for that port could not be built at
all and the board's power switches could not be driven.

The evidence for the left edge is unusually good and it is why the rule is
now a measurement rather than a symmetry. **All three of Great Scott
Gadgets' bitstreams use that port**, and two of the three also drive the
HyperRAM, the pseudo-supply pins and the direct `D+`/`D-` taps, which are
on the same edge. So between them they configure 58, 52 and 40 left-edge
pads whose ball names are known from the platform file. The right edge's
oracle was one ball — the USER button, in one of the three files — so this
is a different order of evidence.

What the database says, and what those files confirm at absolute frame
positions:

| | |
|---|---|
| Where the pad tile is | one row **south** of the ball: the `PICL1*` whose four sides it declares, or the `MIB_CIB_LR` at rows 13, 25, 37 and 49 |
| Where the second `BASE_TYPE` is | the ball's own row for sides A and B, two rows south for C and D |
| Which `CIB` ties the data and enable | column 1, at the ball's row for A and B and two rows south for C and D |
| How many PIO sides | **four.** `PICL1*` declares `PIOA` to `PIOD`, the `PICL0*`/`PICL2*` pair splits A B from C D, and the sixteen balls of the TARGET port and the switches use all four |
| What differs from the right edge | the column, and the `CIB`'s direction prefix. **Nothing else.** |
| What differs from the top edge | two more sides, and the pad tile a row south instead of the same row |

The row arithmetic is the part that had to be measured, and the measurement
is `tests/fpga_trellis.rs::what_lattices_own_packer_writes_for_a_left_edge_pad`.
It takes the thirteen pins of `target_phy` in all three files and the three
switches in `analyzer.bit`, and checks 477 bits:

| Ball | PIO | Buffer | Pad tile | Second copy | What it is |
|---|---|---|---|---|---|
| R2 | C | (0, 38) | (0, 39) | (0, 40) | `ulpi_data[0]`, bidirectional |
| R1 | B | (0, 35) | (0, 36) | (0, 35) | `ulpi_data[1]`, bidirectional |
| P2 | B | (0, 32) | (0, 33) | (0, 32) | `ulpi_data[2]`, bidirectional |
| P1 | A | (0, 35) | (0, 36) | (0, 35) | `ulpi_data[3]`, bidirectional |
| N3 | D | (0, 35) | (0, 36) | (0, 37) | `ulpi_data[4]`, bidirectional |
| N1 | A | (0, 32) | (0, 33) | (0, 32) | `ulpi_data[5]`, bidirectional |
| M2 | D | (0, 26) | (0, 27) | (0, 28) | `ulpi_data[6]`, bidirectional |
| M1 | C | (0, 26) | (0, 27) | (0, 28) | `ulpi_data[7]`, bidirectional |
| T4 | B | (0, 44) | (0, 45) | (0, 44) | `clk`, an output |
| R3 | B | (0, 41) | (0, 42) | — | `dir`, an input |
| T2 | D | (0, 38) | (0, 39) | — | `nxt`, an input |
| T3 | D | (0, 41) | (0, 42) | (0, 43) | `stp`, an output |
| R4 | C | (0, 41) | (0, 42) | (0, 43) | `rst`, an output |
| K5 | B | (0, 29) | (0, 30) | (0, 29) | `target_c_vbus_en`, an output |
| L1 | A | (0, 26) | (0, 27) | (0, 26) | `control_vbus_en`, an output |
| L2 | B | (0, 26) | (0, 27) | (0, 26) | `aux_vbus_en`, an output |

An input has **no** second copy — its pattern in that tile is empty, which
is the same oddity the right edge's USER button has — and that is why two
rows of the table have a dash rather than a position.

**Everything in the pad's own fields is the same as the right edge**, and
that is a finding and not an absence of one, because the widths are what a
wrong tile rule would most plausibly have changed: `BIDIR_LVCMOS33` is
eight bits here too, `OUTPUT_LVCMOS33` six, `INPUT_LVCMOS33` five, the
second copy two for either of the first two and nothing for an input, and
hysteresis, the pull mode and the slew rate one bit each. What the two long
edges have that the top and bottom do not is `TERMINATION_1V35/1V5/1V8` and
`DIFFRESISTOR`; what the top and bottom have that they do not is `CLAMP`.
This backend writes none of the four.

**Three negative controls**, because a test that only looks for bits that
are set cannot tell a rule from a coincidence:

- **hysteresis is clear on an output.** `ecppack` writes `HYSTERESIS = ON`
  for `dir`, `nxt` and the eight data balls and leaves it clear on `clk`,
  `stp`, `rst` and the three switches — which is exactly what
  `configure_io` does;
- **the slew rate is clear on the three switches.** All thirteen ULPI pins
  carry `SLEWRATE="FAST"` in the `ULPIResource`'s attributes and have the
  bit; the three `Resource` lines for the switches ask for nothing and have
  it clear. So the field is written on request, not by direction;
- **no `CIB` tie on any of the sixteen.** Every one has a signal routed
  into its data wire, and the tie and the route are one mux.

And the honest direction, which is the one that would catch a rule that is
right for sixteen balls and wrong for the rest: **every**
`PIO<s>.BASE_TYPE` these three files set anywhere in column 0 is either a
pad tile or a second-copy tile of a ball the map now names. No orphans, in
any of the three.

Two things measured on the way that are pinned rather than smoothed over.
`R4` reads back as the pseudo-differential `OUTPUT_LVCMOS33D` rather than
`OUTPUT_LVCMOS33`, because its C/D partner `T3` is an output too and that
spelling's pattern spans the pair — the same ambiguity "What could not be
read back" describes on the right edge, and all six of the plain value's
bits are inside it. And a switch that a design does **not** drive still has
one of its six output bits set, for the same cross-pair reason: `K5` is
side B of a pair whose side A (`K4`, `target_a_discharge`) both other files
do drive. So the assertion there is "the pattern is not complete", not "no
bit of it is set", and the pull mode — a field of its own that nothing
shares — is the clean negative control.

### The bottom edge, which is one tile and is nothing but configuration pins

The bottom edge came with the same evidence, so it is described too, and it
is the one edge that is genuinely a different shape.

**There is no second tile.** The top edge keeps the pad's own fields in a
`PIOT<n>` on row 0 and a second `BASE_TYPE` plus the `DATAMUX_*` in a
`PICT<n>` on row 1. The bottom edge has only `PICB0` and `PICB1`, on row 50,
and they hold both: `BASE_TYPE`, `CLAMP`, `DRIVE`, `HYSTERESIS`,
`OPENDRAIN`, `PULLMODE`, `SLEWRATE`, `DATAMUX_ODDR`, `DATAMUX_OREG` and
`TRIMUX_TSREG` in one tile. Row 49 is ordinary `CIB`. So `IoSite::pic_at`
is `IoSite::pad_at` there, the second copy is the same bits, and writing it
twice writes it once. Nothing in the loader special-cases that beyond the
tile rule itself.

**And the patterns are wider.** `OUTPUT_LVCMOS33` is **seven** bits in a
`PICB<n>` where it is six everywhere else, and `BIDIR_LVCMOS33` is **ten**
where it is eight. `INPUT_LVCMOS33` is five, the same as everywhere.
Hysteresis, the pull mode and the slew rate are one bit each. Those numbers
are the database's and they are confirmed set, bit for bit, in the
reference files — which is the reason to report them: a model that assumed
one edge's widths for all four would have written six of seven bits and
produced a pad in a standard nobody asked for.

The column rule *is* the top edge's: side A in the ball's own column, side
B one column east, with the buffer for both sides in side A's `PICB0`. The
`CIB` is one row north, from `JPADDOA <- N1_JA0` and `JPADDOB <-
N1E1_JA0`.

The oracle is smaller than the left edge's but it covers both sides and
several columns: `analyzer.bit` configures five bottom-edge pads,
`facedancer.bit` seven and `selftest.bit` one, and
`what_lattices_own_packer_writes_for_a_bottom_edge_pad` checks every bit of
all thirteen of those. `T6` is the interrupt line to the debug
microcontroller and `R6` a pseudo-supply pin; `T8`, `T7`, `M7` and `N7` are
the SPI flash, an output and an input in `analyzer.bit` and four
bidirectional pads in `facedancer.bit`'s quad-mode spelling; `N8` is the
flash's chip select. Side A's tiles are at columns 4, 11 and 15 and side
B's at 5, 10 and 12, so the `+1` for side B is measured and not assumed.

**A hazard that has to be said plainly.** On a caBGA-256 the bottom edge is
**thirteen balls and all thirteen are bank 8's configuration pins** —
`D0`..`D7`, `CSN`, `CS1N`, `HOLDN`, `DOUT` and `WRITEN` — and `BANKREF8` is
also where the part's sysconfig settings live. A design that drives them is
driving the pins the part loads itself through and, on a Cynthion, the pins
its configuration flash is on. Nothing in this repository places a design
there, nothing should be loaded onto a board that does, and the backend does
not police it: a constraints file naming `T8` will place and route exactly
as one naming `R2` does. The edge is described because the alternative —
leaving thirteen balls of the package out of the fabric — hides a tile rule
the evidence settles, not because anything here wants to drive them.

### A design on left-edge balls, and what a board would have added

`testdata/fpga/cynthion/target_ulpi_loopback.v` is the design the left edge
was described for: the thirteen balls of the TARGET transceiver plus the
three VBUS switches, and nothing else except the oscillator on A8. Eight
bidirectional pads whose tristate a *route* drives, two inputs, two routed
outputs, four outputs whose data is a `CIB` tie, one bank rail, over seven
pad tiles — one of which holds all four PIO sides at once, because `L1`,
`L2`, `M1` and `M2` are sides A, B, C and D of `(col 0, row 26)`.

It places, routes and decodes:

```
17 pad(s), 74 lookup table(s), 26 flip-flop(s)
routed 112 of 112 signal(s) with 1217 pip(s) over 1329 wire(s)
all 2729 set bit(s) decode back through the database into 751 arc(s),
265 field(s) and 74 word(s), with 0 unexplained, and the arcs they select
are exactly the 751 the router chose
```

`the_target_ulpi_design_routes_and_configures_what_its_header_promises`
asserts that, and the things a board could not have shown either: that a
routed tristate is **not** tied and a constant output's **is**, that
`-slew fast` reaches thirteen pads and not the three switches, that
`-pullup yes` reaches two of them and that nothing is left at the
database's default pull-*down*, and that banks 0 and 6 have their rail
written while banks 1, 2, 3, 7 and 8 do not.

**Nothing was loaded onto a part, and this is the one milestone of this
file where that is not merely caution.** A wrong tile rule is precisely the
failure that decodes perfectly and drives the wrong pin, so a board would
have added something real here: it would be the only evidence that
`X0Y38/PIOC` is the ball silkscreened nothing and wired to the TARGET
transceiver's `DATA0`, rather than the ball one row away.

**And this board cannot give it *directly*.** Its six FPGA LEDs are on the
**top** edge and its USER button is on the **right**; the left edge of the
die carries the TARGET transceiver, the HyperRAM, the Type-C controllers,
the power monitor and the pseudo-supply pins, and **not one thing a person
can see or press**. A design confined to this edge therefore has no
on-board observable at all, which is worth saying plainly rather than
leaving a reader to wonder why there is no "what a person should look for"
section above. What it *can* give is the second experiment below — a
transceiver as the instrument and a console on another edge — and that is
the one that was run.

So the cheapest experiment, named exactly:

1. **One wire and one LED.** Drive a left-edge output — a pseudo-supply
   ball is the safe choice, because the platform file says what is on it
   and it is a supply rail rather than a driver — with a counter bit, and
   route the *same* counter bit to a top-edge LED. If the left-edge pad is
   configured in the wrong tile, the LED blinks and the left-edge ball does
   not, and a scope probe or a multimeter on the ball settles it in one
   look. This needs no instrumentation in the fabric and no host software.
2. **Cheaper still, with no probe at all: the TARGET transceiver's own
   clock.** `clk_dir='o'` means the FPGA drives `T4` and a USB3343 will not
   produce `dir` or `nxt` transitions without it. So a design that drives
   `T4` with the 60 MHz, brings `rst` out of reset, and lights a top-edge
   LED when it has seen `nxt` toggle, is a one-bit answer to "did the
   left-edge output pad really reach `T4` and did the left-edge input pad
   really read `R3`" — a transceiver as the instrument, with the LED on
   the edge that is already proved. It is the same trick
   `usb_ulpi_trace.v` plays with a serial console on a spare pin, and it
   would need the TARGET port's VBUS left alone, which is the rule anyway.

**The second one has been run, and it answered with more than one bit.**
What was loaded was not a new design but `usb_host_target.v`, which already
drives `T4` with the 60 MHz, already releases `R4`, and already reads nine
of the transceiver's registers and prints them on the AUX console — so the
"one-bit answer" above came back as nine bytes, and the first two of them
are `24h` and `04h`: the vendor ID **0424**, which is Microchip's. The
section at the top of this file has the whole console and what each line
needs in order to be non-zero. So the thing this section said it could not
settle is settled, and the first experiment — a wire, an LED and a
multimeter — is not needed.

It took one more thing to get there, and it is the reason this paragraph
is not three weeks older: the console said `VIDL=00 VIDH=00 PHYR=00` at
first, because that design's report printed every label beside another
item's value. The bytes were right and the names were not.

**One thing to expect when reading the older transcripts above.** Every
`reticle fpga` run quoted earlier in this file ends with a line like
`20/120 io`, and the denominator is now **197**: it is the number of `io`
sites the fabric declares, which was 120 while two edges of four were
described. The transcripts are dated records of runs on a part and are left
as they were. The same figure appears in `ip/usb/usb_cdc_acm/README.md` and
`examples/mos6502_monitor/README.md`, which are dated records too; the
numerators — how many pads each design uses — are unchanged, and so is
everything in `docs/ip-library.md`'s footprint table, because a footprint
counts the cells a design instantiates and not the sites a part has.

### The bank's rail, which is nowhere near the pad

This is the bit that was missing when the LEDs were dark, so it is worth
being exact about where it is and how its place was established.

`BANK.VCCIO` is a `.config_enum` of the `BANKREF<n>` tile types, and there
are seven such tiles on this die: one for each of banks 0, 1, 2, 3, 6 and
7, and `BANKREF8`, which is also where the part's sysconfig settings live.
`BANKREF1` owns all six LEDs and is the single tile at grid **(69, 0)**;
bank 1's top-edge pads have their tiles at columns 33 to 68, so it is
between one and thirty-six columns east of the pad it is configuring. Its
`3V3` setting is one bit, `F18B0` within the tile.

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
| `WREMUX`, `CLK1.CLKMUX` | only for `DPRAM` mode, and their defaults cost nothing; `ecppack` writes neither in any of its 111 distributed RAMs on this board |

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
  real `ecppack` output were: the base type on all four edges, hysteresis,
  the pull mode, the slew rate, and the bank rail on seven banks. **No arc
  has been checked against an `ecppack` output**, because the reference
  bitstreams route different designs and there is nothing to compare an arc
  against. What was checked instead is that every bit of this flow's own
  bitstream decodes back, through the same database, to exactly the arcs the
  router chose — see the top of this file.

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

> That stayed true until a design with more than one **clock enable** —
> the ULPI USB device, on 2026-09-27. The placer needed one thing after all,
> and it is a thing no family had needed because no other family's bels share
> a pin: two cells in one tile whose pins are one wire must want the same
> signal on it. `SiteRules` in `src/fpga/place.rs` is the edit, it is read
> off the architecture rather than written for the ECP5, and every bitstream
> built before it comes out byte for byte identical. "And the other
> obstacle, which was not in the bitstream at all" at the top of this file
> is the account.

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
| An **inverted** clock | `configure_registers` refuses `CLKMUX=INV` unless the routing says which of the tile's two clock muxes carries the signal, and nothing has built a design with one. A clock **enable** and an **asynchronous reset** are no longer here: both are in `usb_ulpi_device.v`, which has 40 slices carrying a routed `LSR` with its `SRMODE` and flip-flops whose `CEMUX` is left at its `CE` default with a signal routed to `CE<c>_SLICE`. Both became possible when `fpga::place` learnt that the two flip-flops of a slice share those wires and must agree about them; before that they placed and did not route. `CLKMUX=INV` is refused a second way now: on `CLK1` in a tile that holds a distributed RAM it would invert the RAM's write clock, since that wire is the RAM's too |
| A second clock domain | nothing in principle: sixteen networks are declared and a net reaches one by routing. Nothing has built a design with two, so nothing has seen what the router does when two clocks want the same quadrant's network |
| A clock from a PLL | `EHXPLLL` has no port map in the device file. The path is there: a PLL's outputs are `G_J<quadrant>CPLL0CLKO*` and every buffer's input mux offers them, which is how `analyzer.bit`'s two globals are fed |
| A clock on a **dedicated** clock pad | nothing, and it has never been exercised. A `PCLKT` pad reaches the centre through `G_JPCLKT<q><n> <- JINCK <- JPADDI`, all `.fixed_conn`s already in the graph; a Cynthion's oscillator is on the `PCLKC` half of the pair, so this flow has only ever taken the fabric route |
| A ball of a package whose edge is not described | **nothing on a caBGA-256**: all four edges are described and all 197 balls the package names are pads. The row used to say the left edge was "the right edge mirrored" and half of that was wrong; see "The buffer is not where the bits are". What is still untried is a *package* whose edges this die does not have — the caBGA-381 and the TQFP144 are in `iodb.json` and no design has been built for either |
| A carry chain | `CCU2C` has no port map in the device file, on purpose: its two sum bits and internal carry do not match the `(ci, i0, i1) -> co` model Reticle maps carry onto. The `.mux` records for the cascade wires are read already |
| A memory's clock **below the buffer threshold** | a clock earns a global buffer by driving eight clock pins and two block RAMs are three, and this backend refuses a memory clocked through general routing — so a small design has to carry registers it does not need. Bypassing the threshold for memories breaks the iCE40, whose `SB_RAM40_4K` clock has no path from an `SB_GB` in this flow's architecture, so the fix is a per-family statement in the `.dev` file that this family's memories must be clocked globally, and that does not exist yet. `logicram_ecp5` is the design this already refuses |
| A block RAM's **contents on a part** | the bits are written and the stream carries them, and whether word *n* is address *n* has no evidence either way: all 53 of the vendor's initialisation blocks are empty. See "What a board would have added" in the block-RAM section, which names the one experiment |
| A block RAM in **18-bit mode beside another** | `configure_bram` refuses it, because the two top data bits of each port are the next block's bottom two in this reading, and the vendor's two 36-bit blocks contradict that reading. The same section names the experiment |
| A `PDPW16KD` | the pseudo dual-port mode, 36 bits on one side. Two of the 53 blocks in this board's own bitstreams are one; `fpga::primitives` has no shape for it and `configure_bram` writes `MODE = DP16KD` |
| A block RAM's **output register** | `REGMODE_A`/`REGMODE_B` are `NOREG`, which is the default and what `ecppack` writes for all 53. A pipelined read would be `OUTREG` and a cycle later, and nothing models the extra cycle |
| A distributed RAM's **contents** | `configure_lutram` writes them from an `INITVAL` parameter and nothing produces one: `fpga::primitives` declines to lower a memory with initial contents, so every RAM this flow builds starts empty. All 111 of the vendor's do too, so `dpram_init_word`'s permutation still has no evidence either way. What *is* settled now is the pair of **run-time** address decodings, which agree at every address of a 64-deep RAM on a real part — see the first section |
| A distributed RAM **on a part** | see "What a board would have added" in the distributed-RAM section. Everything else about it is measured; that a written word reads back is not |
| A flip-flop **sharing a RAM's `LSR1`** | the placer allows it when the reset *is* the RAM's write-enable net, because a wire carries one signal either way. `ecppack` never does it — 0 of 51 — so it rests on the database alone. "What a board would have added" at the end of the first section names the one experiment that settles it and the `CLK1` reading with it |
| An **inverting** write clock or write enable | `WCKMUX = INV` is `CLK1.CLKMUX = INV` and `WREMUX = INV` is a field of its own, both in the database. `configure_lutram` writes neither, and neither appears in any of the three reference bitstreams |
| An IO standard other than LVCMOS33 | the bits are in the database and the code takes the standard from the constraints; no other standard has been on a part |
| `DRIVE`, `OPENDRAIN`, `CLAMP` or `TERMINATION` on a pad | each is a `.config_enum` of the pad tile, and each is one `ecppack` writes **only when an attribute asks** — so not writing them matches nextpnr exactly for a design that does not ask. `set_io -drive` is parsed and reaches the cell, and `configure_io` writes nothing for it, which makes the option a silent no-op in the bitstream. **`SLEWRATE` has left this row**: it is written now, see "An edge rate on every ULPI pin" |
| A bidirectional pad with a **registered** tristate | `PIO<s>.TRIMUX_TSREG = IOLTO` and the `IOLOGIC` tristate register, none of which is declared. `fpga::primitives` declines to absorb a tri-state driver on a DDR port rather than moving the enable ahead of the register |
| An ECP5 over an FTDI cable | nothing, in principle: the configuration plans are transport-neutral and `jtag::Scan` encodes them for MPSSE. It is refused because that pairing has never been run |

**Two rows have left this table**, and both were about a flip-flop's control
pins. A constant **zero** driving a flip-flop **on a part** is measured now —
`usb_ulpi_device.v`'s `zero_probe` is XORed into the byte endpoint 1 hands
back, and it reads zero over 293 bytes; see "A constant zero is on a part, in a
byte a host reads back". So is the **CIB tie for an unused `LSR`**, from the
database and the vendor's own files rather than from a board: a reset wire's
mux has a constant-zero source and a data wire's has none at all, and Lattice's
bitstreams leave 84 of `analyzer.bit`'s 438 flop-bearing tiles with nothing on
either reset wire. See "The unused reset wire, which turned out to be a
different shape entirely".

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
space — seven, if the three placement constraints at the end of this section
are counted, and they deserve to be — and the shape repeats:

| | Which two | Where it is written down |
|---|---|---|
| 1 | a `CIB`'s constant mux against the routing mux into the same wire | here |
| 2 | a centre mux's six-bit source code, whose other five bits another feature may set | "A bit a feature wants clear" |
| 3 | `PIO<s>.PULLMODE`'s low bit against `OUTPUT_<standard>`'s | "A third place where two features share one bit" |
| 4 | a right-edge `PIO<s>.BASE_TYPE`'s *pseudo-differential* values, whose patterns reach into the neighbouring PIO's bits | "What could not be read back", and the fix at the top of this file |

The first three are handled the same way: the bits go into a zeroed bitmap
with an OR, `dropped_clear_bits` notices a feature whose bits another feature
wanted clear, and a question the finished image cannot answer gets asked of
the pass instead. The fourth needed a fourth way, because it is the only one
where two features' patterns make a *reading* ambiguous rather than a
*writing* unsafe: `decode` resolves a field by the reading that leaves fewest
of the tile's bits unexplained, with the longest match as the tie-break. It is
the one of the four that leaves a residue — two ordinary outputs on one pair
still read back as a differential output, because there both readings explain
the image equally well and nothing in the bits can tell them apart.

**And there is a fifth of these, which is not about bits at all.** The two
flip-flops of a slice share one `CE`, one `CLK` and one `LSR` **wire**, so
two cells the placer puts there must want the same signal on each of them.
That is the same shape — two features, one resource, and nothing complains
until something is silently wrong — and it is handled the same way, by making
it a legality constraint the placer enforces rather than a thing to remember.
`SiteRules` in `src/fpga/place.rs` reads it off the architecture, so it is
true of any family whose bels share a pin.

**And a sixth, which is the sharper version of the fifth.** Sometimes two
bels are not two pieces of silicon at all. A slice in `DPRAM` mode *is* its
two lookup tables and the slice beside it in `RAMW` mode has given up both of
its own, so a distributed RAM is six of a logic tile's eight lookup tables
and they are not "shared" but gone. `BelDecl::blocks` says which, `SiteRules`
makes it symmetric, and the legaliser and every annealing move refuse a
placement that breaks it. "A distributed RAM is on the fabric" has the
measurement that fixed the number at six rather than eight or sixteen.

**And a seventh, which is neither of those: a budget.** A logic tile has
`LSR0` and `LSR1` for its four slices and `CLK0` and `CLK1` for the same
four, and a distributed RAM's write enable and write clock are joined to
`LSR1` and `CLK1` with no mux of their own. So a RAM does not exclude a
flip-flop and does not share a *pin* with one — it **spends one of two**, and
what is legal is decided by counting distinct signals rather than by either
of the two rules above. `SiteRules` reads the pools off the routing graph and
answers the question as a bipartite matching, which is also why three
distinct reset nets no longer place in one tile. The first section of this
file has the measurement, the reproducer and what a board would still add.
