# `mjpeg_enc`

A baseline **greyscale** JPEG encoder: pixels in, a whole file out,
from its `SOI` to its `EOI`.

| module | what it is |
|---|---|
| `mjpeg_enc` | the whole encoder, and the package's `top` |
| `jpeg_fdct` | the 8x8 forward DCT of §A.3.3, with the level shift of §A.3.1 |
| `jpeg_quant` | quantisation and the zig-zag of §A.3.4 and Figure A.6 |
| `jpeg_entropy` | the entropy coder of §F.1.2: DC prediction, run-length, Huffman, byte stuffing |

**Greyscale, and the version 0.1.0 says so.** Colour wants a DC
predictor and a pair of tables for each of three components, and 4:2:0
wants the subsampling and the macroblock order on top; neither is here.
Nor are restart markers, a JFIF `APP0` segment, progressive mode or
arithmetic coding. Pixels arrive a block at a time, not in raster
order — see *The pixel order is block order* below.

**Nothing in this package has been run on a board.** What is proved is
what `tests/ip_library.rs` checks in simulation and what the FPGA flow
reports; *What is and is not proved* at the end says which is which.

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

## `mjpeg_enc`, the whole thing

### One source of truth for each table

A JPEG carries its quantisation table in `DQT` and its Huffman tables
in `DHT`, and the encoder needs the same numbers. **The caller writes
each table once**, through `cfg_*`, in exactly the form the file
carries it: 64 bytes in zig-zag order for `DQT`, and for each `DHT` the
sixteen counts of codes per length followed by the symbols. This module
puts them in the file *and* derives what the blocks need from them —
the quantiser's divisors verbatim, and the coder's codes by the
canonical procedure of §C.2.

That is the whole reason `jpeg_quant` and `jpeg_entropy` take tables
rather than containing Annex K. **A table written twice is a table that
can disagree with itself**, and a file whose `DHT` does not match the
codes used to write the scan decodes to nothing — which makes it a
defect that no amount of looking at the picture would find, because
there is no picture.

So the §C.2 derivation is exercised by every case in the corpus rather
than tested on its own: the codes in the scan and the counts in the
`DHT` come from the same walk, and ffmpeg read both.

### How a frame goes

`start` begins one.

| phase | what happens | cycles |
|---|---|---|
| prepare | the Huffman codes derived, the divisors copied into the quantiser, the header assembled in memory | about 700 |
| header | the header emitted as one run | 2 a byte |
| data | pixels in, coded bytes out behind them | the blocks' |
| flush | the last code, `EOB`, the padded byte, then `FF D9` | tens |

The coder has no "finished" line, so the flush ends when nothing has
come out for 64 cycles. **That number was measured and not guessed**:
shrinking it, the corpus passes at 16 and fails at 15, so the longest
gap the coder leaves between two bytes is sixteen cycles — a code and
its magnitude bits shifted a bit a cycle. Four times that is the
margin, and a `DHT` with longer codes than Annex K's would eat into it.

**Pixels are not accepted until the header is out.** They could be,
with another buffer; they are not, because the coded bytes would then
queue behind a header that is still going and the buffer would have to
be as deep as the header is long. `in_ready` says so and a caller that
honours the handshake needs to know nothing about it.

`done` rises when the last byte has been taken, and **one more `start`
begins the next frame** — which is what Motion JPEG is. Everything a
frame leaves behind is put back, including the tables, which are
derived again rather than remembered.

### The pixel order is block order

64 samples a block, row-major inside the block, and the blocks in
raster order across the picture. **Not raster order over the whole
picture**, which would need eight lines of it buffered — 15 kilobytes
at 640 wide, four block RAMs, and a module about line buffering rather
than about JPEG. A design with a frame buffer has blocks for the
asking; one with a camera needs those line buffers, and they are not
here. `WIDTH` and `HEIGHT` must both be multiples of eight, because
this encoder does not pad an edge block.

### The configuration memory, and the frame that read its own wreckage

One 2048-byte memory holds all of it: the quantisation table at
`0x000`, the two `DHT`s at `0x100` and `0x200`, and the assembled
header at `0x400`. One read port and **one** write port, the caller's
writes and the builder's multiplexed outside it — written as two write
arms, inference saw two ports and left a generic `memory 2048x8` in the
netlist that neither backend could map.

The header region is 1024 bytes for 310 bytes of header, and the margin
is the point. At `0x300` in a 1024-byte memory the header ran off the
end and wrapped onto the quantisation table. **The first frame was
perfect**: the quantiser had already been loaded from the table before
the builder overwrote it. The second read the wreckage and wrote a file
whose `DQT` was the tail of the AC symbol list. A single-frame test
could not have found that, and
`mjpeg_enc_encodes_a_second_frame_the_same_as_the_first` is there
because of it.

### Three off-by-ones, all the same off-by-one

The memory answers a cycle late, and every reader of it got that wrong
once:

* **the quantiser's divisors.** `tbl_we` is registered, so the write
  lands the cycle after the state that asks for it — by which time
  `q_addr` has moved on but `cfg_q` has not. Divisor `k` went into
  entry `k + 1`, entry 0 was never written at all, the DC coefficient
  was divided by an undefined value, and the coder's table lookup
  never resolved. The whole encoder stalled with a quarter of a
  million bytes of nothing out.
* **the header's copied runs.** The read issued when a copy is
  dispatched lands two cycles later, so going straight to the first
  write copied whatever the previous read had left behind and shifted
  the run by a byte: a header of exactly the right length with every
  table one entry out.
* **and then the same run one cycle the other way.** `rd_addr` is a
  register too, so the address has to be *assigned* two cycles before
  the byte is wanted, not one. One ahead made every byte of a run the
  byte before it.

They are listed because the shape recurs: in this flow a memory read is
`cfg_q` a cycle after `rd_addr`, and `rd_addr` a cycle after what was
assigned to it, so **a value is wanted two assignments early**. The
emitting path got it right by accident — its armed/unarmed alternation
is exactly two cycles a byte.

### Against ffmpeg, whole files

`testdata/ip/mjpeg_enc_corpus.txt` holds six pictures, and every `file`
in it was **decoded by ffmpeg** when it was generated: no warning, no
error, and the PSNR against the pixels that went in is on the case
line.

| case | size | what it is for | PSNR |
|---|---|---|---|
| `flat16` | 16x16 | the degenerate scan: one DC code and `EOB` | 99.00 |
| `ramp16` | 16x16 | a gradient, so the DC predictor differences | 46.37 |
| `edges16` | 16x16 | hard block edges, so the AC categories spread | 51.14 |
| `sine24` | 24x16 | a width that is not a multiple of sixteen | 34.96 |
| `noise16q1` | 16x16 | a divisor of one everywhere: the largest categories Annex K can code | 60.45 |
| `sine16qv` | 16x16 | a varied table, so the divisors are not all equal | 33.40 |

This is **the strongest check in the package and the reason the other
three exist**. A wrong marker, a wrong length field, a `DHT` that
disagrees with the scan, a missing `EOB` or absent byte stuffing does
not make a worse picture: it makes a file nothing opens. A case that
survives that round trip says the file is really a JPEG, and the corpus
is what lets the gate say it again without ffmpeg installed.

### What the tests would and would not catch

Each was checked against a deliberately broken encoder:

| defect introduced | caught by |
|---|---|
| the quantiser's divisors written one entry out | the corpus — it stalls outright |
| the copy's read one cycle late | the corpus, at the first `DHT` |
| a byte of `SOF0` wrong (the width's low byte) | the corpus, at that byte |
| a `DHT` length field wrong | the corpus, at that byte |
| the header region overrunning the memory | **only** the second-frame test |
| the header pointer not reset between frames | both |

The fifth row is the one worth keeping in view: a defect that leaves
the first frame byte-perfect is invisible to any test that encodes one
frame.

What none of them would catch: colour, which this encoder has not got;
anything about line buffering, since the pixels arrive a block at a
time; a `DHT` giving some symbol no code, which makes an unreadable
file and which nothing here checks; and whether the picture *looks*
right, which is the PSNR on the case line and was ffmpeg's to judge.

### Area, and the depth to watch

Whole, at 640x480, from `docs/ip-library.md`:

| | LUT4 | depth | flip-flops | block RAM |
|---|---|---|---|---|
| ECP5 45F | 3042 | 14 | 886 | 7 x DP16KD |
| iCE40 HX1K | 3473 | 19 | 888 | 10 x SB_RAM40_4K |

**19 on a four-input fabric is the depth that did not meet 60 MHz** in
`docs/fpga-trellis.md`, and it is a statement about four-input parts
rather than about this design: an HX1K has 1280 lookup tables and this
needs 3473, so it will not go on one anyway. On the ECP5 it is 14. No
part has been asked to run it.

Of the seven ECP5 blocks, one is the transform's transpose buffer,
three are the coder's symbol table, two are the quantiser's divisors
and natural-order block, and one is the configuration memory. Giving
the header a kilobyte it does not need costs nothing there, because
2048 bytes and 1024 are both one `DP16KD`; on the iCE40, whose blocks
are 4 kilobits, it is two more.

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

Proved for `mjpeg_enc`: every byte of six whole files that ffmpeg
decoded, at four picture sizes and three quantisation tables; and a
second frame from the same instance identical to the first, on five of
the six, which is the only check that sees what a frame leaves behind.

Not proved, and not claimed: that the transform agrees with any other
encoder, which it will not and need not; that the quantiser's rounding
or any particular table is a *good* choice, which is a question about
pictures and not about correctness; anything about colour, which does
not exist here; anything about line buffering, since pixels arrive a
block at a time; and anything on a part — no bitstream containing this
encoder has been built or loaded.
