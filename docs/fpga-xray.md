# A real Xilinx 7-series fabric, and a real `.bit`

## Nothing produced by this has been loaded into a part

No bitstream this flow writes has been sent down a JTAG cable by anyone
who wrote it. No LED has been lit. This document says exactly what *is*
established and exactly what is missing, and the missing part is larger
than the finished part.

What is established is structural, and it is checked against a bitstream
Vivado made for the very part in question:

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

What is **not** established is that any of it configures anything. The
gap is named below under *What remains before an LED could light*.

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

20.6 million pips is the scale problem, and it is real. Most of it is
interconnect: an `INT_L` or `INT_R` tile has 3636 features, all of them
pips, and the die has 5650 of those tiles. Reticle's `RoutingGraph`
gives every pip a `Vec<ConfigBit>` of its own; at 20.6 million pips that
is several gigabytes before anything is routed, on a graph the router was
built against at 14 × 18 tiles.

**So the loader does not build it.** `XrayOptions::region` says which
rectangle of tiles gets wires, pips and bels, and `XrayOptions::max_pips`
(four million by default) makes the loader *count first and allocate
after*: a region that will not fit is refused with the numbers rather
than with a dead machine. `reticle fpga` picks the region from the pins
the constraints name, expanded by twelve tiles, because that is where the
design has to be.

A 325-tile region around the Basys 3's switch and LED pins is 96 704
wires, 517 833 pips and 793 bels, and the whole load — including parsing
the 6.6 MB `tilegrid.json` and the 11.9 MB `tileconn.json` — takes well
under a second in a release build.

The frame map always covers the whole part, so the bitstream is a
whole-part bitstream however small the region.

**This is the measurement that decides phase two.** Whole-die routing on
this fabric needs a different pip representation — a pip is a
`(tile_type, index)` pair into a per-type table, not a struct with its own
allocation — or a router that materialises the graph lazily around the
route it is exploring. That is a rewrite of the router's data structures,
not of its algorithm, and it is not phase one's.

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
edge per join per direction: 106 560 of the 282 030 pips in a 195-tile
region are joins.

### 2. A tile's bits live at a frame address, not in a per-tile bitmap

`Arch` has nowhere to put a frame address. The mapping comes out beside
the architecture as an `xc7::FrameMap`, and `XrayFabric` carries the two
together. Nothing is lost, but an `.arch` file written with
`Arch::to_text` cannot carry it, so **a real 7-series architecture does
not round-trip through the text format** the way the synthetic one does.
Adding a `tilebits` directive would fix that; it was not done, because
18 055 such lines is not a file anyone would read.

### 3. The database ships bits, not a wire list

This is the one that matters, and it is a gap in the *data*, not in the
model. `prjxray-db` ships `segbits_<type>.db`, whose lines are
`<TILE_TYPE>.<A>.<B> <bits>`. It does **not** ship the per-tile-type wire
and pip lists; prjxray generates those from Vivado into
`tile_type_*.json`, which is not in the repository. Two things follow.

**Which wire a bel pin reaches is not in the database.** So the loader
declares bels with no pins. A placer can put a cell on a real `SLICEL`
and the flow does; a router cannot get a signal to it, because the bel
has no node to route to.

**Whether `TYPE.A.B` is a pip or a bel feature has to be inferred.** The
rule: it is a pip unless `A` also heads a *longer* feature of the same
tile type, because that is what a site prefix does.
`CLBLL_L.SLICEL_X0.AFF.ZINI` makes `SLICEL_X0` a site, so
`CLBLL_L.SLICEL_X0.CLKINV` is a bel feature and not a pip; `INT_L` has no
feature longer than two components at all, so all 3636 of its features
are pips, which is right. The known place it is wrong is
`LIOB33.DIFF.*`, three features of eighty-three, which describe the
differential pair; the bogus pips they produce name wires that exist
nowhere else and the graph drops them as dangling.

## The milestone design

`examples/basys3/sw_led.v`: two slide switches through one LUT to LED 0.
Combinational, three IO buffers, one LUT, no clock, no global buffer.
Verified — when it can be verified — by a human flipping a switch.

A plain `assign led = sw` would not serve. Synthesis turns that into a
wire, no LUT survives, and nothing carries a truth table into the
bitstream; the exclusive-or needs a lookup table and is still a one-glance
test.

What the flow does with it today:

```
loaded: 325 tile(s) of 20 type(s), 96704 wire(s), 517833 pip(s), 793 bel(s)
wrote sw_led.bit, 2192115 byte(s), 5420 frame(s), 32 configuration bit(s) set
4 instance(s) placed on real sites, 3 held at a package pin
0 of 0 signal(s) routed
warning: the design is NOT FULLY ROUTED. ...
```

The 32 bits are the `LUT6`'s `INIT` for `a ^ b`, half of a 64-entry
table, and they land in frames 32 to 35 of configuration column 2 of the
bottom-half CLB row — real frames at real addresses, eight bits each.
Every one of those positions came from `segbits_clbll_l.db`; no line of
Rust knows what a LUT is.

## What remains before an LED could light

In rough order of how much stands behind each.

1. **Bel pins.** Nothing routes without them. The wire a `SLICEL`'s
   `A1` input or its `AMUX` output reaches has to come from somewhere:
   either prjxray's `tile_type_*.json` (which means asking the user for a
   second database, or generating it, which needs Vivado), or from the
   `ppips_<type>.db` files that name site-pin connections, or from a
   hand-written table for the handful of site types a small design
   touches. The third is the only one that needs nothing new, and it is
   the obvious next step.
2. **Routing at this scale.** Even with pins, the route from an IOB
   through the IO interconnect to a CLB crosses tiles well outside a
   small region, and the region can only grow so far before the pip
   count does. See the measurement above.
3. **IO buffer configuration.** An `IBUF` and an `OBUF` are not just
   sites: `LIOB33` has eighty features for drive strength, slew,
   termination and standard, and which of them an `LVCMOS33` output at
   12 mA needs is a mapping from Reticle's `IoAttrs` onto the database's
   feature names that does not exist yet. The loader records the features
   as `ConfigEntry::Cell` entries keyed on the database's own names, so
   the mapping is a table, not a code change.
4. **The `IOI3` path.** A pad does not reach the interconnect directly;
   it goes through `ILOGICE3` / `OLOGICE3` in the neighbouring `LIOI3`
   tile, and those have their own features and their own routing.
5. **Feature-name to primitive-name mapping.** The loader emits
   `ConfigEntry::Cell { primitive: "ZINI", .. }` because `ZINI` is what
   the database calls that flip-flop feature. Reticle's primitive is
   `FDRE` with `INIT=1'b0`. Nothing connects the two yet.
6. **A programmer.** None is installed. When one is, the first thing to
   do is not to load a Reticle bitstream: it is to load
   `artix7/harness/basys3/swbut/design.bit` and confirm the board and the
   cable work, then to load a Reticle bitstream and watch nothing happen,
   then to work through the list above.

Until every one of those is done, what this flow writes is a
structurally valid Xilinx bitstream that configures nothing.

## Where the code is

| | |
|---|---|
| `src/fpga/xc7.rs` | the UG470 container: frames, packets, the frame address register, the CRC, the `.bit` wrapper, a reader |
| `src/fpga/xray/mod.rs` | the loader: the database as an `Arch` plus a `FrameMap`, with the region and the measurement |
| `src/fpga/xray/parse.rs` | one reader per file of the database |
| `src/fpga/devices/xc7.dev` | the device: primitives, pins, and now the IDCODE |
| `tests/fpga_xray.rs` | everything above, against the real database, skipping without it |
| `examples/basys3/` | the milestone design and its constraints |

`src/fpga/arch/synthetic.rs` is untouched and still says what it always
said: that fabric is synthetic, it is not an iCE40, and it programs
nothing. Nothing here changes that.
