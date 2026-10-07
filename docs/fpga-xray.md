# A real Xilinx 7-series fabric, and a real `.bit`

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
- The `_SING` IO tiles at the ends of a bank, which no table in
  `sites.rs` describes for any direction.

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
`LIOB33_SING` tile, which this flow has no IO table for; constraining a
port to it is refused ("maps to no usable site") rather than guessed at.

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
3. **A carry chain.** This is now the biggest hole in what the loader can
   route, and it is not a missing table — it is packing. Three things
   stand in the way and each is a real piece of work:

   - **`S` has no tile wire.** A `CARRY4`'s four propagate inputs are
     wired inside the slice to the four lookup tables' `O6` outputs and
     reach no wire the interconnect can drive. `ppips_clbll_l.db` lists
     `CLBLL_LL_A.CLBLL_LL_A1 hint`, which is the lookup table used as a
     wire, and taking a `hint` means putting a cell on that lookup table.
     So every bit of the propagate needs a lookup table **in the same
     slice at the same position**, which the placer cannot express: it
     places bels independently and its only relative-placement mechanism,
     the `rloc` macro, works in whole tiles.
   - **The chain must run up one column.** `CIN` comes only from the
     `COUT` of the slice below (`tileconn` joins `CLBLL_LL_CIN` to the
     next tile's `CLBLL_LL_COUT_N`), so a seven-element chain needs seven
     vertically adjacent slices of the same half. Nothing in the placer
     knows that, and annealing for wirelength does not produce it.
   - **`CYINIT` is a constant.** An incrementer's carry in is a one, and
     the bit that says so is `PRECYINIT.C1` — a feature that depends on a
     *pin being tied to a constant*, which `ConfigEntry` has no variant
     for. `PRECYINIT.CIN` for the rest of the chain would fit the
     existing pass-through mechanism; `C1` would not.

   What does work is mapping: `count + 1` becomes seven `CARRY4` with the
   right wiring, `tests/fpga_carry.rs` proves the chain against the
   primitive's own model, and the Vivado export path uses it. It is only
   this loader's placement and routing that cannot. `examples/basys3/blink.v`
   therefore spells its increment out as a toggle chain, which
   `the_blink_designs_toggle_chain_is_an_increment` proves equal to
   `count + 1`, and says so in its header.
4. **A memory, and a `SLICEM`.** Nothing has looked at `RAMB18E1`, so a
   design with one will not route. A `SLICEM`'s **distributed RAM** has now
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

## Where the code is

| | |
|---|---|
| `src/fpga/xc7.rs` | the UG470 container: frames, packets, the frame address register, the CRC, the `.bit` wrapper, a reader |
| `src/fpga/xray/mod.rs` | the loader: the database as an `Arch` plus a `FrameMap`, with the region and the measurement |
| `src/fpga/xray/parse.rs` | one reader per file of the database |
| `src/fpga/xray/lutram.rs` | a `SLICEM`'s distributed RAM: the `RAM64X1D` bel, its pins, its blocks and its bits, with the sources of the quoted part |
| `src/fpga/xray/sites.rs` | the inside of a site: pin names from UG474 and UG471, the wire each sits on, the orientations the database only implies, the IO recipe read off Vivado's own bitstream, and the clock tables — the `BUFGCTRL`, the `BUFHCE` of a clock row, the wires that cost bits to touch and the rebuffer enables |
| `XrayFabric::enable_global_clocks` | the one bit that belongs to no pip: a global clock's rebuffer enables, over the whole column, once the routing is known |
| `src/fpga/devices/xc7.dev` | the device: primitives, pins, and now the IDCODE |
| `tests/fpga_xray.rs` | everything above, against the real database, skipping without it |
| `tests/fpga_xray_lutram.rs` | the distributed RAM, built from `examples/basys3/lutram.v` and decoded |
| `examples/basys3/` | the two designs that have reached a part, the distributed-RAM demo that has not yet, and their constraints |
| `src/fpga/xray/dsp.rs` | the refusal of a `DSP48E1`, and the measured reasons for it |
| `XrayDatabase::decode` | the other direction: a bitstream back into the database's feature names, with an accounting of every bit it could not name |

`src/fpga/arch/synthetic.rs` is untouched and still says what it always
said: that fabric is synthetic, it is not an iCE40, and it programs
nothing. Nothing here changes that.
