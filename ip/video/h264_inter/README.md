# `h264_inter`

Inter prediction for the H.264 decoder: the motion vectors of clause
8.4.1 and the prediction samples of clause 8.4.2, for P slices.

| module | what it is |
|---|---|
| `h264_mvpred` | in the token stream between `h264_parse` and `h264_recon`: turns `MVD` into final `MV` tokens, P_Skip included |
| `h264_inter` | the package's `top`: watches the same stream, fetches reference samples from SDRAM through a cache, and sends each P macroblock's prediction to `h264_recon` |
| `h264_frame_addr_stub` | the tiled frame layout's address, until `ip/memory/sdram_burst`'s `frame_addr` lands |

Everything here was written from ITU-T H.264 (06/2026), clauses 6.4.8 to
6.4.13 and 8.4, and from nothing else: no decoder's source was read.
`docs/h264-decoder.md` has the token format and both blocks' ports.

**Nothing here has been on a board.** What is proved is what
`tests/ip_h264_inter.rs` checks in simulation, listed under *What is
checked*, and what the FPGA flow reports.

## `h264_mvpred`

Every token passes through, in order, except `MVD`, which is consumed.
For each P macroblock, P_Skip included, 32 `MV` tokens — two per 4x4
block, in luma4x4BlkIdx order — go out immediately before its `MB_CBP`.

- **Neighbours** (8.4.1.3.2, 6.4.11.7): A, B and C of a partition are the
  4x4 blocks left of, above and above-right of its corner, C at x +
  predPartWidth, replaced by D (above-left) when C is not available. A
  neighbour inside the macroblock is available once its partition is
  decoded, which a mask of decoded blocks answers exactly; NOTE 2 of
  6.4.11.7 is that case.
- **Availability** (6.4.8, 6.4.9): a neighbouring macroblock is available
  when it is in the picture and in the current slice. Slices are
  contiguous in raster order (no ASO, no FMO), so "in the slice" is "at or
  after the slice's first macroblock", whose position is the first
  `MB_START` after `SLICE_FIRST`; no division by the picture width is
  needed.
- **Prediction** (8.4.1.3, 8.4.1.3.1): the 16x8 and 8x16 directional rules,
  then the median with its two special cases — B and C both unavailable
  with A available make B and C copies of A, and exactly one matching
  refIdx picks that neighbour.
- **P_Skip** (8.4.1.1): refIdx 0, and a zero vector when A or B is
  unavailable or either has refIdx 0 and a zero vector, otherwise the
  16x16 prediction.

The current macroblock's sixteen blocks are a distributed RAM written a
block a cycle; the left column and the six blocks above are registers;
the bottom row of every macroblock goes into a block RAM line buffer of
four entries a macroblock column (320 x 38 bits for 1280 samples). The
above-left block is not read back from the line buffer, because the
previous macroblock's bottom row has already overwritten it there: it is
kept from the previous macroblock instead. A partition takes eight cycles
plus one a block it covers, so a macroblock's vectors take 24 (one 16x16)
to 144 (sixteen 4x4s) cycles, its line-buffer reads and writes 15 more,
and its 32 `MV` tokens one a cycle.

The output is a two-entry queue, so `in_ready` depends on registers only.

**Room for B slices**: the token format, the `list` bit and the `kind`
decode are there; a list-1 `MVD` is consumed and ignored (its `REF_IDX`
passes, as every token but `MVD` does), and B macroblocks get no vectors.
Direct prediction (8.4.1.2) wants the co-located picture's vectors from
SDRAM and a second line buffer.

## `h264_inter`

**Collector.** The `MV` tokens of a macroblock land in one of two vector
banks, each block's vector `{frame, mvy, mvx}` with the frame index already
looked up in the slice's reference map. As each block arrives it is
compared with the blocks that could share a partition with it, and at
`MB_CBP` the macroblock becomes the coarsest set of rectangles with one
vector each: 16x16, 16x8, 8x16, or per 8x8 one of 8x8, 8x4, 4x8, 4x4. That
is the coded partitioning or coarser; equal vectors predict equal samples,
so the merge changes the work and nothing else.

**Tasks.** Each rectangle is three tasks, Y then Cb then Cr. A task's
window is the partition, plus the filter's margin where the vector has a
fraction (2 left and 3 right of luma, 1 right of chroma, the same down),
**clamped into the picture** (8-239, 8-240, 8-262..8-269) — so a vector
pointing anywhere outside the picture, however far, reads the edge
samples and fetches only what is inside.

**Check.** Every 8-sample line of the window's rows is looked up in the
reference cache. Words the line lacks are fetched; the lacking words of
consecutive lines of a row are one burst while they are contiguous and in
one 32-sample tile row, so a burst never leaves its SDRAM row.

**Interpolate.** Once a task's fills are in, its window is streamed one
9-sample row segment a cycle, in columns of four outputs. Each pushed row
gives the 6-tap horizontal intermediates b1 (unrounded, 16 bits) and the
integer samples; six rows deep, that is every name of Figure 8-4 for the
four outputs: b and s from b1, h and m vertically from the samples, j from
the b1 column (8-246) with its (x + 512) >> 10, each clipped. Every
position of Table 8-12 is then `(A + B + 1) >> 1` of two of those names, a
whole or half position being the same name twice. Chroma is 8-270 done
as a horizontal blend and then a vertical one, which is the same integer
sum with no intermediate rounding. Neither uses a multiplier: the taps are
shifts and adds, and an eighth-sample weight is three of them.

**Overlap.** The check of one task runs while the previous one
interpolates. Consecutive tasks are always of different planes, and the
planes have disjoint parts of the cache, so a check can never evict a line
the interpolation in front of it is reading.

**Stream.** The prediction buffer holds two macroblocks; one is sent while
the next is computed.

### The reference cache

24 KB of data in eight block RAMs, two banks (even and odd lines) of four
16-bit lanes (the four SDRAM words of a line), so that any 9-sample segment
is one read of each bank and any SDRAM word is one write. Direct mapped:
the index is frame[1:0] and the low bits of the row and of the line — luma
32 rows by 16 lines (128 samples wide), each chroma plane 16 by 8 — for
four frames; the tag is the rest, 12 bits, with **a valid bit per word**:
it is a sector cache, so a window that wants two words of a line fetches
two words and not eight. It is flushed at every picture, since the DPB
reuses a frame index for a new picture and a line that outlived its
picture would be a sample of the wrong one.

Why it is this shape, and why it is not larger, is the measurement below.

### The reference map and the memory port

`map_frames` is sixteen 4-bit frame indices, refIdxL0 0 in bits 3:0. After
each `SLICE_HDR` this block raises `map_ready` and takes no token until
`map_valid`; the DPB logic therefore holds `map_valid` low until the table
for that slice is on the wires.

The `sdram_burst` client port reads only; `cmd_*` come from registers.
`rd_*` has no ready, and none is needed: every word is written into the
cache as it arrives. `h264_frame_addr_stub` is a placeholder for
`frame_addr` with the same idea of ports — frame, plane, sample column and
row in, word address out — so swapping in the real one is changing an
instance. Its layout (eight frames of 2^21 words, no bank swizzle) is only
what the test bench's memory model mirrors.

**Integration.** `h264_inter` and `h264_recon` both take the stream behind
`h264_mvpred`; a token moves when both are ready. That is deadlock-free if
`h264_recon` takes a macroblock's tokens up to its `MB_CBP` without waiting
for its prediction, which it can, since the prediction is needed only to
add the residual.

## Measured: SDRAM traffic and cycles

The question this answers is whether one 16-bit SDR SDRAM is enough. At
720p30 a decoder handles 108 000 macroblocks a second; the contract's
estimate leaves motion compensation 80 to 125 MB/s of the SDRAM's 200 to
250 usable, which is **750 to 1150 bytes per macroblock**, and the decoder
has about **925 cycles per macroblock** at 100 MHz.

Five 720p streams, made with ffmpeg's libx264 encoder as a black box
(Baseline profile, CRF 23 unless said), decoded by the model agent's
`tools/h264_model.py` into vectors and predictions; the streams are too
large to commit, and `inter_traffic_on_real_streams` (ignored, with
`RETICLE_H264_DUMPS`) prints this table again from them:

| stream | what moves | refs | P MBs inter | partitions |
|---|---|---|---|---|
| `pan720` | a 2560x1440 still panned 211 x 67 samples/s, plus grain (`noise=alls=6`) | 4, `partitions=all` | 39141 of 39600 | 65% 16x16, 32% skip |
| `pan720r1` | the same | 1 | 38982 of 39600 | 57% 16x16, 41% skip |
| `zoom720` | ffmpeg's `mandelbrot` zooming, plus grain | 4, `partitions=all` | 39359 of 39600 | 56% 16x16, 3% 8x8 |
| `zoom720hq` | the same at CRF 12, `-preset slower` | 4, `partitions=all` | 8190 of 25200 | 35% P_8x8, 38% 16x8 or 8x16, no skip |
| `tsrc720` | ffmpeg's `testsrc2`, nearly static | 4 | 6994 of 7200 | 87% skip |

**Bytes fetched per macroblock**, over every macroblock of every P
picture (intra ones fetch nothing), by the fetch model — which the random
test holds to the hardware's own count, burst for burst:

| stream | with the cache | without (each window exactly) | saved | bursts with / without | MB/s at 720p30, with / without |
|---|---|---|---|---|---|
| `pan720` | 470.8 | 549.5 | 14% | 48.6 / 52.6 | 50.8 / 59.3 |
| `pan720r1` | 434.1 | 537.6 | 19% | 45.6 / 50.1 | 46.9 / 58.1 |
| `zoom720` | 492.3 | 564.0 | 13% | 51.3 / 55.4 | 53.2 / 60.9 |
| `zoom720hq` | 197.9 | 241.8 | 18% | 26.6 / 29.7 | 21.4 / 26.1 |
| `tsrc720` | 381.6 | 397.8 | 4% | 37.3 / 38.2 | 41.2 / 43.0 |

"Without" is `CACHE=0`: every lookup misses, so each window is fetched
word for word with nothing reused, which is what a design with no cache
at all would fetch (and slightly less than one fetching whole lines).

**Per inter macroblock, by the hardware**, simulated over the first 300
inter macroblocks of every P picture (so the top edge, with its clamped
windows, is over-represented), against an SDRAM that answers after 12
cycles and then gives a word a cycle, and one that also refuses half its
cycles at random:

| stream | bytes, with / without | cycles, idle SDRAM, with / without | cycles, SDRAM busy half the time, with / without |
|---|---|---|---|
| `pan720` | 481 / 556 | 311 / 343 | 516 / 583 |
| `pan720r1` | 446 / 543 | 297 / 338 | 487 / 571 |
| `zoom720` | 502 / 571 | 322 / 352 | 537 / 598 |
| `zoom720hq` | 622 / 755 | 443 / 489 | 709 / 816 |
| `tsrc720` | 382 / 389 | 255 / 258 | 417 / 422 |

These cycle counts were measured before the task generator became two
stages (see *Resources*); the final version, measured again on `pan720`,
takes one cycle a macroblock more (311.5 against 310.5 with the cache and
the SDRAM idle), and fetches exactly the same.

What these say:

- **One SDR SDRAM is enough for motion compensation on this content, with
  room.** The worst stream fetches 53 MB/s averaged over its pictures,
  under half the low end of the 80-125 MB/s estimate. The hardest
  macroblocks measured, `zoom720hq`'s, are 622 bytes each with the cache:
  a whole picture of them would be 67 MB/s.
- **The cache saves 13 to 19%, and no more is there to be had.** An
  *infinite* cache with the same word granularity, flushed per picture,
  fetches 428 bytes/MB on `pan720` against this one's 471: reuse between
  macroblocks is mostly the five columns two horizontal neighbours share,
  and with four references x264 often gives neighbours different ones.
  Reuse between macroblock rows would need a whole row of reference
  windows per frame: a cache eight times this size (192 KB) fetches 457
  instead of 471.
- **A line-granular cache would have been worse than none.** The first
  version of this cache fetched whole 8-sample lines (four words): on the
  four-reference streams it fetched 548 to 594 bytes/MB, *more* than the
  549 to 564 of fetching every window exactly, because a 21-sample luma row
  overhangs its lines on both sides. Sixteen-sample lines were worse still
  (602 to 743 over the sizes tried). That is why a tag carries a valid bit
  per word.
- **Cycles fit**: 255 to 443 per inter macroblock with the SDRAM to itself,
  417 to 709 when half its cycles are refused, against 925. They are
  dominated by waiting for fills; the filters stream a row a cycle.
- **What is not covered** is the analytic worst case, every macroblock
  sixteen 4x4 partitions with fractional vectors pointing in unrelated
  directions: about 1800 bytes a macroblock, which is 900 cycles of fill
  at a word a cycle before any filtering, and twice the byte budget. No
  real stream measured comes near it, and the decoder has nothing to fall
  back on if one does, except that `h264_recon` waits.

Bursts are reported because a burst costs the controller time beyond its
words; how much depends on `ip/memory/sdram_burst`'s controller (row
activations, CAS latency, whether bursts to an open row chain), which is
not written yet. Every burst here stays inside one tile row, so a
partition's window opens at most four SDRAM rows.

## Resources

`reticle fpga ... --device xc7a100t-ftg256 --speed-grade 2` (the
contract's die, at the board's grade -2), each block on its own:

| block | LUTs | of them memory | flip-flops | RAMB36 | DSP48 | routed estimate |
|---|---|---|---|---|---|---|
| `h264_mvpred` | 2406 | 24 RAM32 cells (the current macroblock) | 1035 | 1 (line buffer) | 0 | 6.86 ns |
| `h264_inter` | 5139 | 66 RAM32/RAM64 cells (vectors, request queues, prediction buffer) | 2360 | 10 (8 data, 2 tags) | 0 | 9.31 ns |

The routed figures are the flow's own timing model (`--timing-driven`,
fitted to Vivado's timer at grade 2), for each block behind a four-pin
harness that shifts its inputs in and XORs its registered outputs out, so
they time the block's own register-to-register paths and nothing of a
real neighbour. Both are under 10 ns. `h264_inter`'s worst path is the
request queue's count; the previous version failed at 11.18 ns, from
the vector RAM through the window arithmetic in one cycle, which is why
the task generator is two stages. One placement seed of the final version
left the clock unroutable in the harness's region and another (7) routed
it; that is the flow and the harness, not the design, but it is why the
seed is written down.

On the XC7A100T (63 400 LUTs, 135 RAMB36, 240 DSP48) the two together
are 12% of the logic, 8% of the block RAM and no DSP: the filters are
shifts and adds.

## What is checked

All in `tests/ip_h264_inter.rs`, on the compiled simulator. "Bit-exact"
below means every output word equal, not a tolerance.

| test | against | what it proves |
|---|---|---|
| `mvpred_matches_the_model_on_random_streams` | a model of 8.4.1 in the test file | the whole output token stream, token for token, over random P and I pictures 1..8 by 1..5 macroblocks, random slices, every partition and sub-partition shape, P_Skip, intra among inter, 1..4 references, vectors up to ±3000, under random gaps on both sides |
| `mvpred_matches_the_model_dump` | the model agent's dump of a real stream | bit-exact: every vector and refIdx of a P_8x8 stream over two references, and every other token passed unchanged |
| `mvpred_matches_large_dumps` (ignored) | the same, for 720p | bit-exact over 17 738 inter macroblocks of `pan720` and 5 797 of `zoom720hq` (5 P pictures each) |
| `inter_matches_the_model_on_random_vectors` | a model of 8.4.2.2 in the test file | bit-exact prediction over random pictures, every partition shape, every fraction, vectors far outside the picture, a reference map that changes per picture, and an SDRAM with random latency and stalls; and the hardware's bursts and words equal to the fetch model's |
| `inter_matches_the_model_dump` | the model agent's dump | bit-exact prediction for a real stream's P_8x8 macroblocks |
| `inter_traffic_on_real_streams` (ignored) | the 720p dumps | bit-exact prediction of 12 600 inter macroblocks of five streams, with and without the cache, and the tables above |
| `handshakes_are_functions_of_registers` | the timing graph | `in_ready`, `out_valid`, `tok_ready`, `map_ready`, `cmd_valid` and `pred_valid` depend on no input combinationally; checked by breaking `in_ready`, which fails it |

The two models in the test file were written from the Recommendation by
the same hand as the hardware, but deliberately in another shape: the
vector model keeps the whole picture's vectors and decides availability
by macroblock address and slice, where the hardware keeps a line buffer
and decides it by position; the sample model computes every sample from
the equations of Figure 8-4 one at a time, with clamping on every
coordinate, and computes j both ways 8-245 and 8-246 allow and asserts they
agree. A misreading the two share would pass them both, which is what the
model agent's dumps are for: they come from a decoder written separately
and checked against ffmpeg's decoded pictures.

What none of this covers: B slices, which are not implemented; anything
about a real SDRAM controller, since the memory here is a behavioural
model of the contract's port; and the board.

## What is not done

- **B slices.** List 1, bi-prediction's average and direct prediction
  (8.4.1.2), which needs the co-located picture's vectors.
- **Weighted prediction** (8.4.2.3.2): Baseline forbids it. Main allows it
  in P slices, and it is a multiply and a shift after this datapath.
- **`frame_addr`.** `h264_frame_addr_stub` stands in for the SDRAM agent's
  module; swapping it is one instance. Its eight frame slots are the
  stub's, not a limit of this block, which carries 4-bit frame indices.
- **The worst case** above: nothing here bounds the time of a picture of
  unrelated 4x4 vectors.
- The per-picture flush costs 3 072 cycles, under one a macroblock at
  720p.
- A vertical vector component beyond 13 bits, which no level allows,
  arrives truncated in the `MV` token.
