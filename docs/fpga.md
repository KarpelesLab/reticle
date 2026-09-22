# FPGA placement, routing and bitstreams

Reticle's own back end for open FPGA families, under `src/fpga/`. It
takes the netlist `fpga::synthesize_for` produces — nothing but
primitives the device database declares — and places it, routes it and
writes its bitstream, without calling another program.

```
  .rtl / .v / .vhd
        │
   fpga::synthesize_for          synthesis, primitives, LUTs, device cells
        │
        ├── fpga::export_nextpnr ─► nextpnr-ice40 / nextpnr-ecp5
        ├── fpga::export_vendor  ─► Vivado / Quartus / Diamond
        │
        └── fpga::place_and_route
                 │
              place.rs   analytic (conjugate gradient) + simulated annealing
                 │
              route.rs   PathFinder negotiated congestion, A* maze expansion
                 │
            bitstream.rs   .asc (IceStorm style) and .bin (Reticle's own)
```

`fpga::implement` is synthesis plus all three in one call.

## Read this first: the architecture is synthetic

Placement and routing need to know what wire is where, which
programmable interconnect point (pip) joins which two of them, and which
configuration bit switches each one on. For iCE40 that knowledge is
Project IceStorm's chip database, produced by years of fuzzing real
parts. **It is not in this repository and it has not been guessed.**

What ships instead is an iCE40-*shaped* fabric, built by
`src/fpga/arch/synthetic.rs`, whose header says at length what it borrows
and what it invents. In short:

| Borrowed | Invented |
|---|---|
| the 14 × 18 tile grid of the 1k dice | everything inside a tile |
| an IO ring, RAM in columns 3 and 10, logic elsewhere | which tile holds which bel |
| the tile bitmap sizes (`logic_tile` 16×54, `io_tile` 16×18, `ramb`/`ramt` 16×42) | every bit position in them |
| eight global clock networks | which tiles their buffers sit in |
| the primitive names, pins and parameters (from the `.dev` database) | the package pin to site map |

So a placement, a routing and a bitstream produced here are *structurally*
correct and *electrically* meaningless: the `.asc` has the right shape and
reads back to the same bits, and it will not program a part. What the
tests prove is that the algorithms are right — the placer respects the
constraints, the router reaches every sink from its driver, the bitstream
round-trips — and that the flow is reproducible byte for byte.

The synthetic fabric also differs from the real part in four ways that a
design must not lean on, all listed in `arch::synthetic`: two logic cells
per tile rather than eight; the LUT, the flip-flop and the carry as three
separate bels on three separate sites rather than one packed logic cell;
per-flip-flop clock, enable and reset wires rather than tile-wide shared
ones; and a carry chain that can escape onto the local tracks.

## The `.arch` format

A routing architecture is data, in the same line-oriented text format the
`.dev` and `.rcf` files use: one directive per line, whitespace
separated, `#` or `//` comments. The grammar is deliberately flat — an
`arch` block has exactly one `end`, and a tile type's members name their
tile type instead of nesting — so one file may hold `device` blocks and
`arch` blocks side by side and either parser skips the other's.

`testdata/fpga/tiny.arch` is a complete hand-written example. The
directives:

| Directive | Meaning |
|---|---|
| `arch <name>` … `end` | one architecture |
| `family <name>` | the device family it belongs to |
| `part <device>` | a device name this architecture serves; repeatable |
| `grid <width> <height>` | the tile grid |
| `asc_device <word>` | what the `.asc` `.device` line carries (`1k`) |
| `global <wire>` | a wire that reaches the whole die; one node for the part |
| `tiletype <name> asc <keyword> bits <rows> <cols>` | a tile type and its configuration bitmap |
| `wire <tiletype> <name> span <dx> <dy>` | a wire that starts in a tile and reaches `(dx, dy)` tiles away |
| `bel <tiletype> <name> <kind> pin <role>=<wire> …` | a placeable element and the wire each pin reaches |
| `config <tiletype> <bel> cell <PRIMITIVE> bits <r>.<c>,…` | bits set when a cell of that primitive sits on the bel |
| `config <tiletype> <bel> param <NAME> <index> <r>.<c>` | the bit carrying bit `index` of the cell's parameter |
| `pip <tiletype> <from> <to> bits <r>.<c>,…` | a directed programmable connection and its bits |
| `tile <x> <y> <tiletype>` | one tile |
| `tiles <tiletype> rect <x0> <y0> <x1> <y1>` | a rectangle of them |
| `pinmap <package pin> <site>` | which site a package pin reaches |

A **wire reference** in a pip or a bel pin says which node is meant:
`local0` is the wire of that name starting in this tile, `sp4e@-4,0` the
one starting four tiles west (so it arrives here), and `*glb0` the global
wire `glb0`.

`Arch::to_text` writes the directives in that order and
`parse(to_text(a)) == a` holds, so an architecture can be generated,
diffed and reviewed like source. The one normalisation is that
`tiles … rect` comes back as one `tile` line per position.

`Arch::build_graph` expands an architecture into a `RoutingGraph`: a
`Vec<Wire>` of nodes, a `Vec<Pip>` of edges (each with the tile it lives
in and its configuration bits), a `Vec<ArchSite>` of placeable locations
named `X<x>Y<y>/<bel>`, and compressed-sparse-row neighbour lists in both
directions. Ids are handed out in a fixed order — tiles row-major, each
tile's wires in declaration order — so the same architecture always gives
the same graph.

A pip's bits are normally a **multiplexer code**, not a bit of its own:
every wire is driven by one multiplexer, and the pips into it share a
small field, with code 0 meaning off. That is how a real database encodes
them, and the bitstream writer does not care either way — it sets
whatever bits the pip lists.

## Dropping in a real iCE40 database

Everything the flow needs is in the format above, so a converter from
Project IceStorm's chip files replaces the built-in architecture with no
code change: parse it with `Arch::parse` and pass it as
`PnrOptions::arch`. What such a converter has to emit:

1. **The grid and the tiles.** Icebox's chip database gives the die size
   and the type of every tile: one `grid` line and one `tile` line each.
2. **The tile types** with IceStorm's real bitmap sizes: `logic_tile`
   16 × 54, `io_tile` 16 × 18, `ramb_tile` and `ramt_tile` 16 × 42 on the
   1k dice.
3. **Every wire** icebox names for that tile type — `local_g0_0`,
   `sp4_h_r_0`, `sp12_v_b_0`, `lutff_0/in_0`, `glb_netwk_0`, … — with the
   span its segments imply, as a `wire` line.
4. **Every pip**: each entry of icebox's `.buffer` and `.routing` blocks
   becomes a `pip` line, with the `B<row>[<col>]` positions it lists as
   that pip's configuration bits.
5. **Every bel**: one per `lutff_*`, `io_*`, `ram` and `gb`, with the wire
   each pin reaches, plus `config` lines for `LC_<n>` (the sixteen
   `LUT_INIT` bits, the flip-flop enable and set/reset bits, the carry
   enable) and for `IoCtrl` / `IOB_<n>` (`PIN_TYPE`, `PULLUP`).
6. **The package**: one `pinmap` line per package pin, from the package
   file.

With that in place, `Bitstream::write_asc` produces an `.asc` that
`icepack` turns into a real bitstream and `icebox_vlog` can explain. The
bel kinds a bel must use are the device database's role keywords —
`lut`, `ff`, `carry`, `io`, `gb`, `bram` — and the pin roles are derived
from the `.dev` port maps: a role with several port names (a LUT's
`i=I0,I1,I2,I3`) takes that role plus its position (`i0`…`i3`), a block
RAM's signals take their physical port's index as a prefix (`p0_clk`,
`p1_addr3`), and a port connected to more than one bit takes the bit
number (`p0_dout7`).

## Placement

`fpga::place` runs two passes.

**Analytic.** The netlist becomes a weighted graph — every signal a clique
over the instances it touches, weighted so that a wide net does not
outvote a two-pin one — and the placement minimising total squared wire
length is the solution of two sparse symmetric systems, one for `x` and
one for `y`. They are solved by a hand-written conjugate-gradient
iteration (`place::solve`). Instances tied to a package pin are the fixed
boundary that makes the systems non-singular; a small pull towards the
middle of the die keeps a design with no fixed pins bounded. The result
is a cloud of real numbers, so every instance is then assigned the
nearest free site of the kind it needs.

**Annealing.** Simulated annealing over a half-perimeter wirelength cost,
with a move set of swaps, moves to a free site and macro relocations. The
starting temperature is twenty times the spread of the cost changes a
random walk sees; the schedule is geometric, and it stops when the
temperature falls below `0.005 * cost / nets` or after
`PlaceOptions::max_temperatures` steps. The best placement seen is kept,
so the pass can only improve on what legalisation produced.

Constraints (`fpga::Constraints`) are honoured as follows:

- a **pin constraint** (`set_io`) is hard: the cell lands on the site the
  architecture's `pinmap` gives for that package pin and never moves; two
  cells on one pin is an error;
- a **region** confines every cell whose name matches the pattern, in
  legalisation and in every annealing move;
- **`keep_hierarchy`** gives each matched group its own region: the box
  its members legalised into, grown by one tile, so the group can improve
  inside it but cannot be scattered;
- an **`rloc` macro** is rigid — its members keep the exact tile offsets
  the constraint states, and the annealer moves the macro, not its
  members.

A design that does not fit is reported, not approximated: `PlaceError`
says which kind of site ran out and by how much, which two cells want one
package pin, or which region cannot hold what was assigned to it.

## Routing

`fpga::route` is PathFinder (McMurchie and Ebeling, 1995). One iteration
rips up every signal and routes it again through an A\* maze expansion
over the routing graph. The cost of entering a node is

```
cost(n) = (1 + present_factor * overuse(n)) * (1 + history(n))
```

where `overuse(n)` counts the signals beyond its capacity, `history(n)`
accumulates every iteration in which the node was oversubscribed, and
`present_factor` grows each iteration. History is what makes it converge
rather than oscillate: a node that has been fought over stays expensive
once it is free, so the loser looks elsewhere instead of taking it back.

A multi-sink signal is routed sink by sink onto a growing tree, with the
nodes already in the tree free, which is how the trunk comes out shared.

`RoutingReport` keeps the overuse of every iteration, so the convergence
is visible rather than asserted; the golden `.route` files in
`testdata/fpga/` are exactly that history. A design the fabric cannot
carry is reported — `RouteError::Unroutable` names the signal and the
sink for a path that does not exist at any price,
`RouteError::Congested` names the worst contended wires when the
iteration cap is reached — rather than looped on.

`Routing::verify` walks every sink backwards through the pips its signal
was given until it reaches the source, which is the property that
matters; `tests/fpga_pnr.rs` does the same walk independently.

## Bitstreams

`fpga::bitstream::generate` sets, for each routed pip, the configuration
bits that pip lists in the tile that holds it, and for each placed cell
the bits its bel's `config` entries give: a `cell` entry for the
primitive it is (which is how twenty `SB_DFF*` variants become one mode
field) and a `param` entry per bit of a parameter it carries (`LUT_INIT`,
`PIN_TYPE`, `READ_MODE`). Nothing else. Every position comes from the
architecture file; no bit number appears in Rust.

Two output formats:

- **`.asc`** — the IceStorm-style text: a `.device` line, then one block
  per tile headed `.<keyword>_tile <x> <y>` followed by one line of
  `0`/`1` per bitmap row. `Bitstream::parse_asc` reads it back, so the
  writer and the reader check each other.
- **`.bin`** — *Reticle's own* container, documented on
  `Bitstream::write_bin`: magic `RTBS`, a version, the device word, then
  each tile's position, keyword, bitmap size and packed bits. **It is not
  an iCE40 configuration image**; producing one of those from an `.asc`
  is what `icepack` does, with the database this crate does not ship.

`Bitstream::to_summary` gives the compact, diffable view a reviewer
wants: the totals and the set bits of each tile that has any.

## Determinism

Golden files are compared byte for byte, on three platforms, so every
choice is pinned:

- the single source of randomness is `place::Rng`, an xorshift seeded
  from `PlaceOptions::seed` (default `0x5EED_1CE4_0000_0001`);
- every iteration order is over a `Vec` or a `BTreeMap`, never a
  `HashMap`;
- the router is not random at all: signals go in netlist order, sinks in
  pin order, and the priority queue breaks ties by node id;
- the annealer avoids the two library calls whose last bit is not
  guaranteed to match across platforms. The acceptance probability of an
  uphill move is the Cauchy rule `T / (T + Δ)` rather than `exp(−Δ/T)`,
  and the moves per temperature use an integer cube root rather than
  `powf`. Both are monotone in exactly the same way.

## Tests

- Unit tests next to the code: the graph expansion and the `.arch`
  reader, the placer's legalisation and each kind of constraint, the
  router on small fabrics with known-routable, known-congested and
  known-disconnected cases, and the bitstream round trips.
- `tests/fpga_pnr.rs`: the three iCE40 designs of `testdata/fpga/`
  through the whole flow, with `<name>.place`, `<name>.route` and
  `<name>.bits` golden files and one full `<name>.asc`. Rewrite them with
  `UPDATE_EXPECT=1` and read the diff.
- `tests/fpga_pnr.rs` also walks every routed sink back to its driver
  independently of `Routing::verify`, and runs `icepack` over the result
  when one happens to be installed, reporting what it said rather than
  failing: a synthetic architecture is not one `icepack` has any reason
  to accept.
