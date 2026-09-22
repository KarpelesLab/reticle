# Arithmetic lowering

`reticle::synth::arith` turns the arithmetic cells of a netlist — `add`,
`sub`, `mul`, `div`, `mod`, the shifts and the comparisons — into gates,
with a choice of architecture for each.

Without it, an `add` survives to technology mapping as one generic cell
and the AIG expands it into a ripple carry. That is correct, and on a
small FPGA it is usually what you want, because the carry chain is
dedicated silicon and costs nothing. But it is *one* architecture,
chosen implicitly, and a designer whose adder sits on the critical path
has no way to ask for a different one. This document is how to ask.

It is the user's half of `src/synth/arith/`; the module docs there carry
the derivations, the bit-level formulations and a reference for each
architecture.

## Asking for an architecture

Three ways, in increasing order of locality.

**Options.** `ArithOptions` picks the architecture for each kind of
cell, and `ArithLower` is the pass that applies them:

```rust
use reticle::synth::arith::{
    AdderArch, AdderOptions, ArithLower, ArithOptions, CompareArch,
};
use reticle::synth::{PassManager, SynthOptions, opt, run};

// Generic synthesis first: the optimiser works on whole-word cells.
let options = SynthOptions::default();
run(&mut design, &options, &mut diags);

let mut arith = ArithOptions::default();
arith.adder = AdderOptions::new(AdderArch::KoggeStone);
arith.comparator = CompareArch::Prefix;

let mut pm = PassManager::new(options);
pm.add(ArithLower::new(arith));
pm.add(opt::Merge);      // share what two architectures built alike
pm.add(opt::Dce);        // collect what the rewrite orphaned
pm.run(&mut design, &mut diags);
```

`ArithOptions::fast()` is the whole depth-oriented set at once.

**A module attribute** is a default for one module:

```verilog
(* arith = "brent_kung" *)
module datapath (...);
```

It applies to the cells the name fits and is silently not for the
others, so a module with an adder and a shifter in it can carry an adder
architecture without complaint.

**A cell attribute** is the local override, and the reason the pass
exists:

```verilog
(* arith = "kogge_stone" *) assign sum = a + b;   // this one is fast
assign count = count + 1;                         // this one is small
```

The value is any architecture name from the tables below, or `"none"`
(also `"off"`, `"keep"`, `"generic"`) to leave that one cell generic for
the AIG. A name that means nothing for that kind of cell is a warning,
`S0041`, and the cell keeps the architecture the options chose.

## Where the pass goes, and why it is opt-in

After the optimisation loop and `synth::cellify`, before technology
mapping. The optimiser is most effective on whole-word cells — it folds
a constant operand of an `add`, reduces its width, merges two identical
ones — and none of that survives the expansion into gates. The mapper,
on the other hand, wants gates.

`synth::run` does **not** run it. Lowering arithmetic before the AIG
hands the optimiser a netlist it can rewrite freely but can no longer
recognise as an adder, and whether that trade is a win depends on the
design and the target; until it has been measured on real designs
(picorv32 and NEORV32 are the ones that matter, per `ROADMAP.md`) the
default pipeline is left alone and the pass is something a caller asks
for. The measurements that exist so far are the tables below.

## The architectures

### Adders and subtractors

All five compute the same function of the same inputs and expose the
carry in and the carry out, so they chain.

- **`ripple`** — one full adder per bit, the carry out of each being a
  multiplexer on the propagate. Three cells and one carry level per
  bit. The default.
- **`carry_select`** — blocks of `block_size` bits, each computed for
  both possible incoming carries and selected when the real one
  arrives. The carry crosses one multiplexer per *block*. One and a
  half to two times the cells of a ripple, and the best depth-per-cell
  of the five at moderate widths.
- **`carry_lookahead`** — the classical group lookahead with a
  configurable `group_size` (four by default, which is what the 74x182
  implements in hardware), applied recursively to the groups.
- **`kogge_stone`** — the fully parallel prefix network. The shallowest,
  and the most expensive.
- **`brent_kung`** — the prefix network as a reduce and a scatter.
  Roughly `2n` prefix nodes against Kogge-Stone's `n log n`, at about
  one and a half times its depth.

A subtractor is the same network over `!b` with the carry in set, so it
costs one inverter per bit more and has the same depth. Its borrow out
is the complement of the adder's carry out.

### Multipliers

Every multiplier is three steps: generate partial products, reduce them
to two rows, add those two rows with one of the adders above (which the
`adder` field of `MultiplierOptions` selects independently).

- **`array`** — one row per multiplier bit, accumulated one row at a
  time. Smallest and deepest.
- **`booth4`** — radix-4 Booth recoding halves the number of rows at the
  cost of recoding logic and a wider row.
- **`wallace`** — a carry-save tree, every column reduced as far as it
  goes at each stage.
- **`dadda`** — the same tree, but each column reduced only as far as
  the next stage needs. Never more adders than Wallace, same depth.

Truncated to the operand width — which is what the generic `mul` cell
is — a product does not care about signedness. A full `2n`-bit product
does, and the multipliers here use the Baugh-Wooley transformation: the
last row and the last column of the matrix are inverted and a constant
is added, which keeps the matrix rectangular instead of sign-extending
every row.

### Comparators

`==` and `<` (and, by complement and exchange, `!=`, `<=`, `>`, `>=`),
signed or unsigned.

- **`ripple`** — a chain from the least significant bit up, one
  multiplexer per bit.
- **`prefix`** — the same associative scan as the prefix adder, reduced
  through a balanced tree. Only the root is needed, so this costs
  `n - 1` prefix nodes and no scatter.

A fast comparator is much cheaper than a fast adder: the tables below
show `prefix` cutting the depth of `<` at 32 bits to a third for about
70% more cells, where `kogge_stone` pays five times the cells of a
ripple for a comparable win. If anything in this module deserves to be non-default,
it is this one.

### Shifters

- **`barrel`** — the radix-2 mux network: one multiplexer per bit per
  amount bit, `⌈log2 n⌉` stages.
- **`barrel4`** — radix 4. **In generic cells this is strictly worse**:
  a 4:1 multiplexer built from two-input primitives is a tree of three,
  two levels deep, so a radix-4 stage costs more cells than the two
  radix-2 stages it replaces at the same depth. It earns its place only
  after technology mapping, where a 4:1 multiplexer is one LUT6 or two
  LUT4s and the stage really is one level. The table says so plainly
  rather than asserting a win that is not there.
- **`funnel`** — an `n`-bit window slid across a `2n`-bit word. For a
  plain shift one half of that word is constant, and the constant folds
  the network back into exactly the barrel shifter — the `funnel` and
  `barrel` rows of the table are identical, which is the folding working
  and not a copy-and-paste error. The funnel earns its keep on
  **rotates**: feed the same operand to both halves and the window wraps
  around, with no arithmetic on the amount.

Rotate amounts must be below the width; mask them to `log2(n)` bits
first when the width is a power of two.

### Dividers

There is one: a combinational restoring array, `w` stages of a `w + 1`
bit subtract and a restore, which is the sequential algorithm unrolled.
The inner adder is selectable like any other.

**A sequential divider is out of scope for this pass**, and that is the
important sentence in this document. A restoring or non-restoring
divider that computes one quotient bit per clock is the right circuit
for nearly every real design, but it has state, a start and a done and a
latency, and a pass that replaces one combinational cell with
combinational logic cannot invent a protocol the surrounding design does
not know about. So `width_thresholds.divider` caps the width at 8 by
default; above it the `div` or `mod` cell is left generic and the pass
says why:

```
warning: `div0` is a 16-bit division, wider than the threshold of 8; it
         stays a generic cell
  = note: a combinational divider grows as the square of the width;
          write the division as a pipelined sequential block, or raise
          `width_thresholds.divider` if the area is really wanted
```

That is `S0043`. The cost is real: the table below shows an 8-bit signed
divider at 405 cells and 98 levels deep, and both numbers grow as the
square of the width. At 32 bits it would be some six thousand cells on a
path no clock will close.

`S0042` is the same idea for multipliers, at a default threshold of 32,
as a note rather than a warning — a wide multiply is expensive but it is
not usually a mistake, and on a device with DSP blocks it should never
have become gates in the first place. See "DSP blocks" below.

## Defaults

Area-oriented, because a small FPGA is the common case and because the
architecture that wins on depth loses on cells by a factor of two to
five:

| Cells | Default | Why |
|-------|---------|-----|
| `add`, `sub` | `ripple` | smallest, and an FPGA's carry chain is free silicon |
| `mul` | `array` | smallest; a design that needs a fast multiplier usually wants a DSP block, not a Dadda tree |
| `eq`, `lt`, … | `ripple` | smallest |
| `shl`, `shr`, `sshr` | `barrel` (radix 2) | smallest *and* shallowest in generic cells |
| `div`, `mod` | restoring array up to 8 bits | see above |

`width_thresholds.min_width` (1 by default) leaves narrow cells generic
if you would rather the AIG had them; it handles a handful of bits at
least as well as any architecture here.

## Measurements

Cells and estimated combinational depth (`synth::report`), for a module
containing nothing but the architecture. Depth counts generic cells on
the longest combinational path, so it is a proxy for delay before
technology mapping, not a delay. Counts are as built: no common
subexpression merging has been run over them, and `opt::Merge` in a real
pipeline will share a little more.

These tables are generated by `measurements_match_the_documentation` in
`tests/synth_arith.rs` and compared byte for byte, so they cannot drift.
Run the tests with `UPDATE_EXPECT=1` to refresh them after an intended
change, and read the diff.

<!-- measurements: generated by tests/synth_arith.rs -->
#### Adders (`a + b + cin`, with the carry out)

| Architecture | 8-bit cells | depth | 16-bit cells | depth | 32-bit cells | depth |
|---|---|---|---|---|---|---|
| ripple | 24 | 9 | 48 | 17 | 96 | 33 |
| carry_select | 36 | 7 | 84 | 9 | 180 | 13 |
| carry_lookahead | 87 | 9 | 190 | 13 | 383 | 15 |
| kogge_stone | 79 | 8 | 194 | 10 | 469 | 12 |
| brent_kung | 52 | 10 | 113 | 14 | 238 | 18 |

#### Subtractors (`a - b - bin`, with the borrow out)

| Architecture | 8-bit cells | depth | 16-bit cells | depth | 32-bit cells | depth |
|---|---|---|---|---|---|---|
| ripple | 34 | 11 | 66 | 19 | 130 | 35 |
| carry_select | 46 | 9 | 102 | 11 | 214 | 15 |
| carry_lookahead | 97 | 10 | 208 | 14 | 417 | 16 |
| kogge_stone | 89 | 10 | 212 | 12 | 503 | 14 |
| brent_kung | 62 | 11 | 131 | 15 | 272 | 19 |

#### Multipliers, truncated (`n x n -> n`, the `mul` cell)

| Architecture | 8-bit cells | depth | 16-bit cells | depth | 32-bit cells | depth |
|---|---|---|---|---|---|---|
| array | 120 | 28 | 496 | 60 | 2016 | 124 |
| booth4 | 151 | 18 | 531 | 34 | 1963 | 66 |
| wallace | 121 | 13 | 519 | 22 | 2073 | 38 |
| dadda | 114 | 10 | 482 | 18 | 1986 | 34 |

#### Multipliers, full signed product (`n x n -> 2n`)

| Architecture | 8-bit cells | depth | 16-bit cells | depth | 32-bit cells | depth |
|---|---|---|---|---|---|---|
| array | 246 | 36 | 1006 | 76 | 4062 | 156 |
| booth4 | 367 | 26 | 1411 | 50 | 5515 | 98 |
| wallace | 273 | 21 | 1110 | 38 | 4385 | 70 |
| dadda | 240 | 18 | 993 | 34 | 4032 | 66 |

#### Comparators

| Architecture | 8-bit cells | depth | 16-bit cells | depth | 32-bit cells | depth |
|---|---|---|---|---|---|---|
| ripple `==` | 16 | 9 | 32 | 17 | 64 | 33 |
| ripple `<` | 32 | 10 | 64 | 18 | 128 | 34 |
| ripple `<` and `==` | 41 | 11 | 81 | 19 | 161 | 35 |
| prefix `==` | 16 | 5 | 32 | 6 | 64 | 7 |
| prefix `<` | 50 | 8 | 105 | 10 | 216 | 12 |
| prefix `<` and `==` | 54 | 9 | 110 | 11 | 222 | 13 |

#### Shifters (amount as wide as the operand)

| Architecture | 8-bit cells | depth | 16-bit cells | depth | 32-bit cells | depth |
|---|---|---|---|---|---|---|
| barrel `shl` | 40 | 5 | 96 | 6 | 224 | 7 |
| barrel `shr` | 40 | 5 | 96 | 6 | 224 | 7 |
| barrel `sshr` | 32 | 4 | 86 | 5 | 212 | 6 |
| barrel4 `shl` | 54 | 6 | 118 | 6 | 310 | 8 |
| barrel4 `shr` | 54 | 6 | 118 | 6 | 310 | 8 |
| barrel4 `sshr` | 43 | 5 | 106 | 5 | 294 | 7 |
| funnel `shl` | 40 | 5 | 96 | 6 | 224 | 7 |
| funnel `shr` | 40 | 5 | 96 | 6 | 224 | 7 |
| funnel `sshr` | 32 | 4 | 86 | 5 | 212 | 6 |

#### Rotates (funnel network, amount masked to the width)

| Architecture | 8-bit cells | depth | 16-bit cells | depth | 32-bit cells | depth |
|---|---|---|---|---|---|---|
| rotate left | 51 | 4 | 132 | 5 | 325 | 6 |
| rotate right | 51 | 4 | 132 | 5 | 325 | 6 |

#### Dividers (restoring array, `q` and `r` together)

| Architecture | 4-bit cells | depth | 8-bit cells | depth |
|---|---|---|---|---|
| ripple inner adder, unsigned | 78 | 25 | 286 | 81 |
| ripple inner adder, signed | 133 | 34 | 405 | 98 |
| kogge_stone inner adder, unsigned | 160 | 27 | 734 | 70 |
| kogge_stone inner adder, signed | 224 | 35 | 894 | 81 |
<!-- end measurements -->

## Which to pick

**Leave the defaults alone** unless something told you not to. The
ripple carry is a third of the cells of anything else, and on an FPGA
the carry chain it maps to is free.

**If a timing report names an adder**, take `carry_select` first: at 16
bits it is half the depth of the ripple for 1.75× the cells, which is
the best trade in the adder table. `kogge_stone` is the answer when
that is still not enough — at 32 bits it is 12 levels against the
ripple's 33 — and it costs five times the cells. `brent_kung` sits
between the two on cells but only beats a ripple from 16 bits up, and
`carry_lookahead` likewise: at 8 bits both of them lose, because the two
levels of hierarchy they pay for cost, in two-input gates, about what
eight bits of ripple carry do. That is why neither is a default and why
the test suite asserts the ordering only from 16 bits.

**Comparators are the cheap win.** `prefix` costs 55–70% more cells
than `ripple` for `<` and cuts the depth from `n + 2` to about
`2 log n`; for `==` it is the *same* cells at a fraction of the depth,
since both spend one XOR per bit and differ only in how the results are
collected. Any
comparison wider than a few bits should be `prefix` if depth matters at
all.

**Multipliers: `dadda`, or a DSP block.** Dadda is both smaller and
shallower than Wallace at every width measured, which is what the theory
says and a useful check that the reduction is right. Against the array
it is a third of the depth for roughly the same cells at 8 and 16 bits —
there is very little reason to choose `array` except that its regular
structure places well. `booth4` halves the rows but the recoding and
the wider partial products eat the saving in generic cells; it is in the
table because it is the architecture a standard-cell flow reaches for,
where the row count drives the area.

**Shifters: `barrel`.** `barrel4` is a LUT-mapping bet, not a
generic-cell one, and `funnel` folds back into `barrel` for anything
that is not a rotate.

**Dividers: think again.** If the width is above a handful of bits,
pipeline it.

## DSP blocks

`arith::dsp_candidates(module)` is the recognition half of DSP block
inference: it walks a module and returns a `DspCandidate` per `mul`
cell, saying whether it is a bare multiply, a multiply-add or a
multiply-accumulate; what each operand's width and signedness is; and
which registers a block could absorb into its pipeline stages. A
register is only listed when the block could really swallow it — its
output must feed nothing but the multiplier — because a register
someone else reads has to stay where it is.

It maps nothing to any device. That is the target's job, and for the
families Reticle knows it lives in `src/fpga/primitives.rs`; nothing in
`synth::arith` mentions a device, a primitive name or a port. The point
of the split is that recognition is the same on every device and mapping
is not.

Not recognised, deliberately: a product that is subtracted (`a*b - c`, as
opposed to `c - a*b`, which is), a pre-adder feeding the multiplier, and
cascades between blocks. Each is a shape real blocks support and each is
an addition to make when a target asks for it; leaving them out costs a
missed inference, never a wrong one.

## Correctness

Every architecture is **proved** equivalent to the generic cell it
replaces, not tested against it. `tests/synth_arith.rs` builds each
operation twice — once out of the generic cells (`add`, `mul`, `lt`,
`shl`, `div`, …) and once out of the architecture — and hands the pair
to `formal::check_equivalent`. Both modules are combinational, so the
checker decides the miter outright, by SAT, SAT sweeping or exhaustive
simulation (see `docs/equivalence.md`), and `Equivalent` means *for
every input vector*, not sampled.

The widths are 1, 2, 3, 7, 8, 13 and 16: one bit, powers of two, one
below a power of two, and two odd widths, which between them catch the
off-by-one in a block boundary, the empty final group and the sign bit
that is also the only bit. Signed and unsigned are separate cases
wherever the operation has them. The carry in and carry out of the
adders and the borrow of the subtractors are ports of both modules, so
the chaining interface is proved and not merely the sum. The block and
group sizes of the carry-select and lookahead adders are proved at 1, 2,
3, 5 and 8 as well as their defaults, and each multiplier is proved over
each of the five final adders.

Underneath that, the unit tests next to each architecture check it
*exhaustively* against a model written in Rust — every input
combination, evaluated through the gate netlist — at widths up to five.
That is the same completeness by a different route, it needs no feature
beyond `synth`, and it runs in a fraction of a second, so a broken
architecture fails immediately rather than in the integration suite.

### Two limits, stated rather than hidden

**The 16-bit Booth and tree multipliers are proved in optimised builds
only.** The miter of two structurally *different* multipliers is the
textbook hard instance for SAT. Until the equivalence checker learned
SAT sweeping, the multiplier proofs at 13 and 16 bits, and the full
`2n`-bit product at 7 and 8, sat in an `#[ignore]`d test: the full
8-bit product took about ten minutes a pair (unoptimised, by the look of
it: 5 to 8 seconds optimised), and the 16-bit truncated product did not
finish in forty.

SAT sweeping proves internal equivalences bottom-up and merges them, and
it turns out not to be what settles these pairs. A Wallace or Dadda
tree, or a Booth recoding, shares no internal signal with the
reference's row-at-a-time array beyond the partial products and the low
output bits. The sweep merges those, and what is left is the whole
problem. Measured in an optimised build, the sweep alone against the
monolithic miter, truncated product:

| Pair | Monolithic | SAT sweep alone | Default engine |
|---|---|---|---|
| Booth, 10 bits | 1.7 s | 2.2 s | 4 ms |
| Booth, 11 bits | 17 s | 14 s | 14 ms |
| Wallace, 11 bits | 38 s | 40 s | 13 ms |
| Wallace, 12 bits | not run | 343 s | 62 ms |
| Dadda, 11 bits | 66 s | 53 s | 14 ms |
| Dadda, full signed 8-bit product | 8.2 s | 11 s | 0.6 ms |
| Booth, 13 bits | not run | not run | 0.30 s |
| Dadda, 16 bits | did not finish in 30 min | not run | 28 s |

Both SAT engines grow by five to eight times per bit, so neither
reaches 16 bits in hours. That is the problem, not the implementation:
the tools that prove multipliers at 32 bits and beyond use algebraic
reasoning over the adder structure, which Reticle does not have. What
the sweep *does* do for multipliers is prove one against a
*restructured version of itself*: a 16-bit multiplier of every
architecture against its own AIG-optimised netlist, the question
post-synthesis verification asks, in milliseconds, where the monolithic
miter took 18 seconds at 12 bits, four minutes at 14 and ten at 16
(`multipliers_match_their_optimised_netlists`).

What settles the pairs in the table is the checker's other complete
method: **exhaustive simulation**, used when every input pattern can be
tried within `SweepOptions::exhaustive_budget`. Sixteen inputs (a full
8-bit product) are 1024 words of bit-parallel simulation, a
millisecond. Twenty-six inputs (a 13-bit multiplier) are `2^20` words, a
third of a second optimised and some fifteen seconds unoptimised, still
within the default budget. So every run of the test suite now proves:

- every architecture at 1, 2, 3, 7 and 8 bits, truncated;
- every architecture's full product at 1 to 8 bits, signed and
  unsigned (7 and 8 by simulation);
- every architecture at 13 bits, truncated (Booth and trees by
  simulation, the array by structural hashing: its accumulation is the
  generic cell's own, so the miter folds to a constant);
- the array at 16 bits.

Thirty-two inputs are `2^26` words: about 28 seconds a pair optimised
and half an hour unoptimised. So the 16-bit Booth, Wallace and Dadda
proofs (`*_multipliers_at_sixteen_bits_match_the_generic_cell`) run in
optimised test builds and are skipped in unoptimised ones:

```sh
cargo test --release --features synth,formal --test synth_arith sixteen
```

The six pairs take under three minutes there. An unoptimised run still
covers those widths with `wide_multipliers_match_over_random_vectors`,
which feeds every corner case and two hundred random vectors through
the netlist. That is a check and not a proof, which is why this
paragraph exists.

**Dividers are proved to 8 bits**, which is exactly the default width
threshold. Above it the pass leaves the cell generic, so there is
nothing there to prove; raise `width_thresholds.divider` and the proof
for that width is one line of the test away, but it will be slow for
the same reason.

The semantics being matched are the ones the rest of the toolchain
already implements, and the awkward corners are part of the proof:

- arithmetic is modulo `2^n`, so a truncated product has no sign;
- a shift by an amount at or above the width shifts everything out, and
  the amount is read as unsigned however wide it is;
- `sshr` fills with the sign bit only when its **left** operand is
  signed, which is what the bit-blaster does;
- division by zero yields an all-ones quotient and the dividend as
  remainder — the standard says `x`, and every trial subtraction against
  a zero divisor succeeds, so the array says all-ones and so does
  `formal::blast`.

Beyond the proofs, `architectures_are_shallower_than_ripple` asserts the
*shape* of the measured numbers. A Kogge-Stone adder that is not
shallower than a ripple carry would mean a broken network rather than a
surprising result, and it is better to fail a test than to publish a
table saying so.

## Diagnostics

| Code | Meaning |
|------|---------|
| `S0041` | An `arith` attribute on a cell names no architecture for that kind of cell |
| `S0042` | A multiply is wider than `width_thresholds.multiplier` and was left generic |
| `S0043` | A division is wider than `width_thresholds.divider` and was left generic; pipeline it |
