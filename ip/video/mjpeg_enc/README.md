# `mjpeg_enc`

Baseline JPEG, on the way to Motion JPEG.

**This package is not finished.** What exists:

| module | what it is |
|---|---|
| `jpeg_fdct` | the 8x8 forward DCT of §A.3.3, with the level shift of §A.3.1 |
| `jpeg_quant` | quantisation and the zig-zag of §A.3.4 and Figure A.6 |
| `jpeg_entropy` | the entropy coder of §F.1.2: DC prediction, run-length, Huffman, byte stuffing |

Still to come: the colour transform, and the framing that turns a
stream of blocks into a file. The version is 0.1.0 and `top` will become `mjpeg_enc` when
there is one.

**Nothing in this package has been run on a board.**

## Why the DCT is not checked the way `adpcm_ima` is

`ip/audio/adpcm_ima` is checked sample for sample against ffmpeg,
because IMA ADPCM's decoder is specified exactly and every
implementation must agree. **JPEG's transform is not like that.** T.81
defines the DCT as an equation over real numbers:

> S(v,u) = ¼ C(u) C(v) Σ Σ s(y,x) cos((2x+1)uπ/16) cos((2y+1)vπ/16)

and says nothing about how to compute it. Every encoder in the world
rounds differently, and two correct encoders given the same image
produce different files. So there is no reference bitstream to compare
against and there never will be.

What can be checked is the **distance from the equation**, and that is
what `jpeg_fdct_is_within_a_count_of_the_transform_t81_defines` does: a
double-precision transform written straight from the formula, against
1003 blocks, with the error bounded.

## No multiplier, and why that is not an optimisation

**Nothing this flow writes a bitstream for can hold a hard
multiplier.** `fpga::trellis::dsp` and `fpga::xray::dsp` each say why
for their family, and the command line turns DSP inference off whenever
a bitstream is asked for, so a `*` here is a multiplier built out of
lookup tables. The first version of this transform used four of them:

| version | LUT4 | depth | flip-flops | block RAM |
|---|---|---|---|---|
| four lookup-table multipliers | 7948 | 56 | 959 | — |
| distributed arithmetic, buffer in flip-flops | 3970 | 12 | 1306 | — |
| **distributed arithmetic, buffer in a block RAM** | **1423** | **10** | **490** | **1** |

Depth 56 is deeper than the 19 that did not meet 60 MHz in
`docs/fpga-trellis.md`, and 7948 LUT4 is six times an iCE40 HX1K's
whole fabric. A fifth of the area and a fifth of the depth is the
difference between a block somebody can use and one nobody should.

### How the multiplies went away

The constants are fixed, so every sum of a subset of them can be worked
out in advance. For each output there is a sixteen-entry table holding
`sum of C[k][n] over the n in the subset`; the four inputs are fed in a
bit at a time, most significant first, and

```text
acc = -table[bits of the sign bit]        then, fourteen times,
acc = 2 * acc + table[the next bits]
```

which is Horner's rule over the binary expansion. The doubling is a
wire, so each cycle is one table lookup and one addition.

**It is exact.** This reproduces the multiplier version bit for bit —
checked over 3000 random blocks and the flat ones before a line of it
was written — so all three correctness tests carried over unchanged,
which is the whole reason to check a rewrite against the thing it
replaces rather than against the specification again.

### Where the rest of the area went

The buffer between the two passes is 64 words of 13 bits. Read eight at
once — a whole row, which is what the second pass wants — and this flow
has to build it out of flip-flops with a multiplexer per read: 1306
flip-flops and more than twice the logic. Read serially, it is one
block RAM and nine cycles a column, which at these rates is nothing.
That one change is 3970 LUT4 down to 1423.

## The arithmetic, and why

The constants are `0.5 C(k) cos((2n+1)kπ/16)` rounded at 13 fractional
bits. The transform is separable, so it is eight one-dimensional
transforms across the rows and eight down the columns, sharing one
arithmetic unit, and the even/odd decomposition makes each output four
products instead of eight: the even-numbered outputs depend only on
`x[n] + x[7-n]` and the odd ones only on `x[n] - x[7-n]`.

**The two passes round by different amounts** — 10 bits after the first
and 16 after the second — and that is the one decision in here worth
arguing about. Measured against double precision over 2000 random
blocks:

| rounding | peak error | RMS error | flat-block DC |
|---|---|---|---|
| **10 then 16** | **0.616** | **0.291** | **exact** |
| 11 then 15 | 0.725 | 0.298 | off by one |
| 13 then 13 | 1.413 | 0.409 | off by one |
| 14 then 14 (wider constants) | 1.413 | 0.408 | off by one |

Wider constants buy nothing; keeping precision *between the passes*
buys everything. The last column is what decided it.

### Why a flat block has to be exact

A flat block — the sky, a wall, a letterbox bar — must transform to
`DC = 8 × (value − 128)` with all 63 AC coefficients exactly zero.
Round evenly between the passes and the DC comes out one low for some
values, which is a visible step between two flat areas that ought to
match. It costs nothing to avoid, and
`jpeg_fdct_transforms_a_flat_block_exactly` runs all 256 values.

Rounding is to nearest with ties toward positive infinity, which is one
adder and an arithmetic shift. Ties away from zero was measured too and
is identical to three decimal places on every figure above, so the
cheaper one is used.

## What the tests would and would not catch

Three tests, and they were each checked against a deliberately broken
transform rather than assumed to work:

| defect introduced | caught by |
|---|---|
| the second pass's write transposed | the error bound and the basis test; **not** the flat-block test |
| rounding evenly between the passes | the flat-block test and the error bound |
| a cosine constant off by 8 or more | all three |
| a cosine constant off by 4 or fewer | **nothing** |
| a state added to or removed from the sequencer | the cycle-count test |

The last row is the honest one. Four parts in 8192 moves a coefficient
by at most about an eighth of a count, which is below this transform's
own rounding noise — so it is less a gap in the tests than a statement
that an error that small does not matter. A typo large enough to matter
is caught.

The second row is why the basis-function test exists.
`jpeg_fdct_sends_a_basis_function_to_one_coefficient` samples each of
the 64 cosine basis functions into the block and requires the matching
coefficient to dominate all 63 others by four times — a statement about
the transform's *structure*, which a flat block cannot make, because a
flat block is perfect under a transpose error.

## Rate

**648 cycles** from the first sample to the last coefficient, one block
at a time with no overlap. Measured, and pinned by
`jpeg_fdct_takes_the_cycles_its_header_says`, because a quoted rate
that nothing checks drifts the first time the sequencer gains a state —
which this one did twice while the arithmetic was being changed.

At 60 MHz that is 92 000 blocks a second: 4:2:0 video at 320x240 and
60 Hz, or 640x480 and 12 Hz. Fifteen of those cycles a one-dimensional
transform are the bit-serial arithmetic and the rest are moving the
block in and out, so a design that needed more could overlap the output
of one block with the input of the next; this one does not.

## `jpeg_quant`

64 coefficients in, row-major; 64 quantised levels out, in **zig-zag
order**, which is the order the entropy coder wants and the order a
JPEG file stores a table in.

The table is written through a port of its own and is indexed exactly
as `DQT` stores it — in zig-zag order — so output position `p` divides
the coefficient at natural position `zigzag[p]` by `table[p]`, and a
caller hands this block the same 64 bytes it writes into the file
without permuting them. **Getting that round the wrong way is the
classic way to produce a file that decodes to something recognisable
and wrong**, which is why
`jpeg_quant_reorders_into_the_zigzag_of_figure_a6` checks the ordering
on its own, with every divisor set to one so a wrong permutation
cannot hide behind a plausible quotient.

### Division is a loop, and usually a short one

No multiplier means no divider either, so the quotient is built a bit
at a time by restoring division — compare, subtract, shift — eleven
cycles for a quotient that cannot exceed eleven bits.

**Most coefficients take two cycles instead of eleven.** After the
transform and a realistic table most of the 63 AC coefficients
quantise to zero, and a magnitude below its divisor is recognised in
one comparison. A block with ten non-zero coefficients takes about 240
cycles rather than 830, so the timing is data-dependent and a caller
should pace itself by the handshake rather than by a cycle count.
There is no cycle-count test for this module for that reason, where
`jpeg_fdct` has one.

### Rounding is the encoder's choice

`round(coefficient / q)` with ties away from zero, done by adding
`q / 2` to the magnitude before dividing. **A decoder multiplies by
`q` and cannot tell**, so this is a choice exactly as the nibble is in
`ip/audio/adpcm_ima`: any rule produces a legal file and the
difference is quality. Truncation, which a bare divide gives, is
measurably worse and this costs one adder.

A zero in the table divides by one. `DQT` forbids zero and nothing
here can police what a caller writes, so the only question is what
happens when one arrives — and a divider left to run on a zero divisor
takes every quotient bit and answers with all ones, which is worse
than a lie because it looks like data.

### What the tests would and would not catch

Each was checked against a deliberately broken quantiser:

| defect introduced | tests failing |
|---|---|
| the table indexed naturally instead of in zig-zag order | all three |
| the inverse permutation applied | all three |
| truncation instead of rounding | two |
| the zero-divisor guard removed | two |

## `jpeg_entropy`, and the one part with a right answer

**This module is checked byte for byte, and the other two are not.**
That is not a difference in rigour, it is a difference in what the
standard says. T.81 leaves the transform's arithmetic to the
implementer, so no two encoders agree and there is nothing to compare
against. §F.1.2 leaves nothing open: given the levels and the code
tables, every bit of the entropy-coded data is determined. So there is
a right answer, and `testdata/ip/jpeg_entropy_corpus.txt` holds it.

Three things happen to a block's 64 levels:

* the **DC level is differenced** against the previous block's
  (§F.1.2.1), which `start` clears at the beginning of a scan;
* the **AC levels are run-length coded** (§F.1.2.2) into symbols
  `(run << 4) | size` — sixteen or more zeros emit `ZRL` first, as
  many times as needed, and a tail of zeros emits `EOB`;
* each symbol becomes a **Huffman code** followed by the magnitude bits
  of §F.1.2.1.1: the low `size` bits of the value, or of the value
  **minus one** when it is negative.

Then §F.1.2.3's byte stuffing: an `FF` in the data is followed by a
`00`, because `FF` begins a marker, and the final partial byte is
padded with ones.

### The tables are the caller's, and so is Annex K

Nothing in this package contains Annex K. The DC and AC code tables
are written through a port, a code and a length per symbol, so a caller
writes the same table it puts in the file's `DHT`. That is the same
choice `jpeg_quant` makes about the quantisation table and for the same
reason: one copy, in the caller, that goes to both places.

The corpus holds the tables in the form a file carries them — a count
of codes per length, then the symbols — and the test derives the codes
by §C.2, so **that derivation is checked too**. They were taken from
the `DHT` segments of a file ffmpeg wrote with `-huffman default`
rather than typed out here: 162 AC symbols is a great deal to copy by
hand, and a copy is not a second opinion.

### How the corpus was validated

The expected bytes come from a Python model of §F.1.2 — and a model is
only worth what checked it. That one was used to build **whole
greyscale JPEG files**, which **ffmpeg decoded**: 40.5 dB against the
original at a flat quantisation table of 16, with ffmpeg reporting
nothing. A wrong code, a wrong run, a missing `EOB` or absent stuffing
does not decode to a slightly worse picture; it does not decode.

Nine cases, each a whole scan with a flush so the byte stream is
self-contained: an empty block, DC only, three blocks exercising the
DC predictor, one `ZRL`, two `ZRL`s in a row, a block with all 64
levels non-zero, the largest categories baseline allows, four sparse
blocks, and one chosen by search because its bytes contain an `FF`.
**Four of the nine contain an `FF`**, so the stuffing is exercised and
not merely hoped for.

### What baseline JPEG cannot code

Writing the corpus turned this up and it is worth stating: **Annex K's
AC table stops at size 10 and its DC table at 11**, which is Tables
F.1 and F.2. An AC level needing eleven bits has no code at all, and a
test case asking for one simply failed to encode. With 8-bit samples
the transform's DC reaches −1024 and its AC about 1020, so a
quantisation table of all ones is the worst case and it fits — but only
just. The generator now checks every symbol it needs has a code, so an
impossible case is reported rather than crashing.

### A bit a cycle, and the depth

The bits go out one at a time rather than through a barrel shifter,
which is what the rest of this package does for want of a multiplier.
A code is at most sixteen bits and a magnitude at most eleven, so a
block of ten non-zero levels is some ninety cycles — less than the
quantiser spends on it and far less than the transform.

**713 LUT4 at depth 13 and three block RAMs** on an ECP5 45F. Two
measurements got it there, and neither was visible from the Verilog:

* **Depth 16 → 13.** The magnitude's category was thirteen `else if`s,
  which is thirteen levels of logic in front of the register holding
  the symbol. Halving the field four times instead — four ORs and four
  multiplexers — costs 23 lookup tables and brings it under the 19
  that did not meet 60 MHz in `docs/fpga-trellis.md`.
* **The tables were not memory.** A DC and an AC table, each with a
  code and a length, is the obvious four arrays — and read that way
  two arrays drive each output register, which is not the bare
  `q <= mem[a]` this flow builds a memory from. It cost 102
  distributed RAMs on an ECP5 and, on an iCE40, **5712 flip-flops and
  11617 lookup tables**: the tables became registers, nine times the
  whole part. One 512-word memory of 21 bits, addressed by the table
  bit and the symbol, is three block RAMs and 728 lookup tables there.

| | ECP5 LUT4 | ECP5 depth | iCE40 LUT4 | iCE40 flip-flops |
|---|---|---|---|---|
| four arrays, priority category | 1028 | 16 | 11617 | 5712 |
| **one memory, balanced category** | **713** | **13** | **728** | **115** |

The DC half uses sixteen of its 256 words and the waste is not worth a
second memory and a multiplexer to save.

### What the tests would and would not catch

Each was checked against a deliberately broken coder, and all six fail
it:

| defect introduced |
|---|
| magnitude bits taken from the value instead of the value minus one |
| the byte stuffing of §F.1.2.3 removed |
| the final byte padded with zeros instead of ones |
| `EOB` never emitted |
| one `ZRL` where two were needed |
| the DC level coded without differencing |

What it would not catch: anything about the tables themselves, which
are the caller's to write into both this block and the file; and a
`DHT` that gives a symbol no code at all, which this block cannot
detect and which makes a file nothing can read.

## What is and is not proved

Proved, by `tests/ip_library.rs`:

* peak error below one count and RMS below a third of a count against a
  double-precision transform, over 1000 pseudo-random blocks plus an
  all-zero, an all-255 and a checkerboard block — the extremes being
  where an accumulator one bit too narrow shows up and a random block
  never does;
* `DC = 8 × (value − 128)` and all 63 AC coefficients exactly zero, for
  every one of the 256 flat blocks;
* each of the 64 basis functions landing on its own coefficient;
* 648 cycles a block, twice over, so a sequencer that does not return
  cleanly to its start is caught too;
* the block synthesises with no diagnostic and no inferred latch, and
  maps for the iCE40 and the ECP5 with the footprint
  `docs/ip-library.md` records.

Proved for `jpeg_quant`: the zig-zag permutation, all 64 positions
pinned by distinct coefficients and a table of ones; division and
rounding against the documented rule over 44 blocks, every divisor
from 1 to 64, both twelve-bit extremes and the exact ties where the
rounding shows; and a zero divisor treated as a one.

Proved for `jpeg_entropy`: every byte of all nine corpus cases, exactly
— against a model that whole-file decoding by ffmpeg validated — with
the codes derived in the test from the `DHT` form by §C.2.

Not proved, and not claimed: that the transform agrees with any other
encoder, which it will not and need not; that the quantiser's rounding
or any particular table is a *good* choice, which is a question about
pictures and not about correctness; anything about colour or framing,
neither of which exists yet; and anything on a part.
