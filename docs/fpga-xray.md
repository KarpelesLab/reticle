# A real Xilinx 7-series fabric, and a real `.bit`

## One design produced by this has run on a real part

On 2026-09-24 the milestone design — two slide switches through one
lookup table to one LED — was built by this flow from Verilog, loaded
onto a Digilent Basys 3 (XC7A35T-1CPG236C) by `reticle program`, and
**confirmed working by a person flipping the switches**: LED 0 followed
the exclusive-or of SW0 and SW1 through all four combinations. No vendor
tool took part at any step.

That is one design, of one lookup table and three pins, on one board. It
establishes that the chain from Verilog to configured silicon closes.
**It does not establish that anything larger works**, and the list at the
end of this document of what is untried on a part — anything with a
clock, a flip-flop, a memory, a carry chain, or an IO standard other than
LVCMOS33 — is unchanged by it.

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

## Getting the database

Reticle never fetches anything. The library is sans-I/O: the database
reaches it through a `FileProvider` the caller supplies, and the caller
says where it is. The database is not in this repository and must not be.

It is [`f4pga/prjxray-db`](https://github.com/f4pga/prjxray-db), Project
X-Ray's chip database, **CC0-1.0 — public domain**. The whole repository
is large; about 41 MB of it is needed for the Artix-7:

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
```

`--chipdb <dir>` names the database explicitly; `RETICLE_CHIPDB` is the
fallback. **CI has neither**, and every test that needs the database
skips with a line saying what is missing. A missing database never fails
the build.

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

## What remains before an LED could light

In rough order of how much stands behind each.

1. **A programmer, and a board.** This is now the top of the list rather
   than the bottom, which is the real change. None is installed here.
   When one is: load `artix7/harness/basys3/swbut/design.bit` first and
   confirm the board and the cable work, then load a Reticle bitstream
   and find out.
2. **Everything outside the tiles that were compared.** The oracle
   covers the IO path. It says nothing about the 5420-frame image as a
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
3. **Anything with a clock.** A flip-flop has no `D` pin in these tables
   — its `D` is fed from inside the slice and has no tile wire of its
   own — and there is no global buffer, no clock tree, no `BUFGCTRL`
   and no `CLK_HROW` routing. A sequential design will not route.
4. **Feature-name to primitive-name mapping.** The loader still emits
   `ConfigEntry::Cell { primitive: "ZINI", .. }` for a flip-flop feature
   because `ZINI` is what the database calls it; Reticle's primitive is
   `FDRE` with `INIT=1'b0`. Nothing connects the two. The IO buffers
   work because `sites.rs` gathers their features under the names `IBUF`
   and `OBUF` explicitly; nothing else does.
5. **IO standards other than LVCMOS33, and tristate.** `OBUFT` and
   `IOBUF` need `OLOGIC` `T` features that have not been measured, so
   the `io` bel declares no `oe` pin and a tristate design will not
   route. Other standards are refused rather than approximated.
6. **Six million `String`s.** The routing graph holds the whole die in
   1386 MiB, most of it wire names. Interning those is what makes
   whole-die routing comfortable rather than merely possible.

Until at least 1 and 2 are done, what this flow writes is a bitstream
whose IO configuration matches a working one and which has never
configured anything.

## Where the code is

| | |
|---|---|
| `src/fpga/xc7.rs` | the UG470 container: frames, packets, the frame address register, the CRC, the `.bit` wrapper, a reader |
| `src/fpga/xray/mod.rs` | the loader: the database as an `Arch` plus a `FrameMap`, with the region and the measurement |
| `src/fpga/xray/parse.rs` | one reader per file of the database |
| `src/fpga/xray/sites.rs` | the inside of a site: pin names from UG474 and UG471, the wire each sits on, the orientations the database only implies, and the IO recipe read off Vivado's own bitstream |
| `src/fpga/devices/xc7.dev` | the device: primitives, pins, and now the IDCODE |
| `tests/fpga_xray.rs` | everything above, against the real database, skipping without it |
| `examples/basys3/` | the milestone design and its constraints |
| `XrayDatabase::decode` | the other direction: a bitstream back into the database's feature names, with an accounting of every bit it could not name |

`src/fpga/arch/synthetic.rs` is untouched and still says what it always
said: that fabric is synthetic, it is not an iCE40, and it programs
nothing. Nothing here changes that.
