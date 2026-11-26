# `h264_recon`

The reconstruction back end of the H.264 decoder: the syntax token stream
(after `h264_mvpred`) and the inter prediction in, the decoded and
deblocked picture out to SDRAM in the tiled frame layout.

| module | what it is |
|---|---|
| `h264_recon` | the whole back end, and the package's `top` |
| `h264_tokin` | stage A: tokens to coefficient levels and one descriptor a macroblock — boundary strengths, neighbour availability, QPs |
| `h264_residual` | stage B1: scaling and the inverse transforms of §8.5, levels to residuals |
| `h264_itrans4` | the 4x4 inverse transform of §8.5.12.2, which is also the Hadamard of 8-320 |
| `h264_intra` | stage B2: intra prediction (§8.3) or the inter prediction stream, plus the residual |
| `h264_deblock` | stage C: the deblocking filter of §8.7, and what is final leaves as row records |
| `h264_dbf_filter`, `h264_dbf_tables` | one line of eight samples across an edge, and Tables 8-16 and 8-17 |
| `h264_frame_wr` | stage D: row records to `sdram_burst` write bursts |
| `h264_frame_addr` | (frame, plane, x, y) to a word address — **a stand-in** for `sdram_burst`'s `frame_addr` |

Everything here was written from ITU-T H.264 (V16, 06/2026). No other
decoder's source was read. Clause and equation numbers are that edition's.

**Nothing in this package has been run on a board.** In simulation it
reproduces, sample for sample, every picture of seventeen libx264
streams as the model agent's decoder (`tools/h264_model.py`, itself
bit-exact with ffmpeg) decodes them — see *What is and is not checked*.

## The pipeline

```text
tokens ─► A h264_tokin ─► B1 h264_residual ─► B2 h264_intra ─► C h264_deblock ─► D h264_frame_wr ─► SDRAM
             │  levels, masks   (2 halves)       ▲ (4 slots)       │ line buffer
             └─ descriptor ring (8) ─────────────┼─────────────────┘
                                  inter prediction
```

Each of A, B1, B2 and C counts the macroblocks it has finished, modulo 8,
and starts macroblock *m* when the stage before it has finished *m* and the
buffer *m* goes into is free: A fills coefficient half *m* % 2 once B1 has
finished *m* − 2, B1 fills residual half *m* % 2 once B2 has finished
*m* − 2, and B2 fills slot *m* % 4 once C has finished *m* − 3, because C
uses slot *m* − 1 as the left macroblock (and sends it) while it works on
*m*. Up to seven macroblocks are in flight.

### A — tokens

`h264_tokin` reads the tokens of `docs/h264-decoder.md` §1. Levels go to
their raster position inside their block — the inverse zig-zag of Table
8-13 — with a mask per block of the positions written, so the coefficient
memory is never cleared: a position whose mask bit is clear reads as zero,
and a block with no `COEF_BLK` this macroblock is skipped whole.

On `MB_END` it derives, for the descriptor:

- **bS** for all 32 luma edge segments (8.7.2.1, frames): 4 on a
  macroblock edge with an intra macroblock (I_PCM counts) on either side, 3
  inside an intra macroblock, 2 where either 4x4 luma block has non-zero
  coefficients (a LumaLevel4x4 `COEF_BLK` with TotalCoeff > 0), 1 where the
  references differ or a motion vector component differs by 4 quarter
  samples or more, else 0. It keeps the right column of the macroblock to
  the left and, in a line buffer, the bottom row of every macroblock of the
  row above.
- **mbAddrA..D availability** for intra prediction: inside the picture, not
  below first_mb_in_slice (6.4.9 for one slice group and no ASO), and under
  constrained_intra_pred_flag not inter (8.3.1.2 step 3, 8.3.3, 8.3.4).
- **filterLeftMbEdgeFlag, filterTopMbEdgeFlag, filterInternalEdgesFlag**
  (8.7 step 2), with disable_deblocking_filter_idc 1 and 2.
- **QPs**: chroma QP from Table 8-15 with chroma_qp_index_offset for Cb and
  second_chroma_qp_index_offset for Cr (8.5.8); and for the filter each
  macroblock's QPY with I_PCM's taken as 0 (8.7.2's qPz), for this
  macroblock and the ones to the left and above.

### B1 — residuals

`h264_residual` takes a coefficient a cycle: the scaling of 8.5.12.1 with
LevelScale4x4 = weightScale4x4 × normAdjust4x4 (8-313 to 8-315), the 4x4
transform of 8.5.12.2 rows first, then columns, then (h + 32) >> 6. Before
the blocks, an Intra_16x16 macroblock's DC block goes through the Hadamard
(8-320, the same butterflies with the two `>> 1`s removed) and 8-321 /
8-322, and each component's chroma DC through 8-324 and 8-326; each block
then takes its DC from there, unscaled again (8-335). An uncoded block costs
one cycle; a coded one sixteen. I_PCM samples are copied through (8.3.5).

**Scaling matrices are the flat 16 of Flat_4x4_16**, which is all that
Baseline and Main streams use. `weight` in `h264_residual.v` is the one
place ScalingList4x4 would enter; the 8x8 transform (8.5.13) and
LevelScale8x8 are not written. The token format already carries both
(`docs/h264-decoder.md` §1.9).

### B2 — prediction plus residual

`h264_intra` keeps its own pre-deblocking line buffer of bottom rows —
intra prediction reads samples *before* the filter, 8.3.1.2 — sized for
1280-sample pictures, and the right column of the macroblock to its left.
The corner sample of each macroblock is saved when the macroblock to its
left reads the line buffer, because by the time it is wanted that
macroblock has overwritten it.

An Intra_4x4 block takes five cycles: lay its thirteen neighbours in a line
`E = L3 L3 L2 L1 L0 C T0..T7 T7` (with 8.3.1.2's substitution of p[3, −1]
for p[4..7, −1] done first), form every two-tap average `F2[k] = (E[k] +
E[k+1] + 1) >> 1` and three-tap `F3[k] = (E[k−1] + 2E[k] + E[k+1] + 2) >>
2`, and every one of 8-46 to 8-70 is one of E, F2, F3 or the DC at an index
that depends on the mode and (x, y) only. Intra_16x16, chroma, inter and
I_PCM have no dependency inside the macroblock and run a word of four
samples a cycle in raster order; plane prediction is set up once a
macroblock.

### C — deblocking

`h264_deblock` works a band of four rows at a time: the band's vertical
edges edge by edge across its rows, then the horizontal edge on top of it.
That is the order of §8.7 for every sample (a horizontal edge at row 4b
reads rows 4b − 4 .. 4b + 3; no vertical edge of a band below touches them).
The left macroblock's columns are filtered in its own slot, and the
macroblock above's bottom rows (luma 12..15, chroma 6..7) come from a line
buffer sized for 1280-sample pictures.

A sample leaves once nothing later can filter it: the left macroblock's
rows 0..11 when this one is done, the one above's bottom rows when this
one's top edge is, the current macroblock's at the end of a row, and the
whole of the last picture row at once. Every sample is sent exactly once;
the test checks it.

### D — SDRAM

`h264_frame_wr` turns each row record into one burst: a 16-sample luma
segment is 8 words (cmd_len 7), an 8-sample chroma one 4 (cmd_len 3), two
samples a word with the even one in bits 7:0. A segment starts at a multiple
of 16 (or 8) samples and a tile is 32 wide, so no burst leaves its tile's
SDRAM row. `pic_done` pulses when a picture's last burst is accepted.

`h264_frame_addr` is a stand-in for the `frame_addr` module the memory
contract gives to `ip/memory/sdram_burst`: 32x32 tiles in raster order, a
frame every 2^21 words, no bank swizzle. Swapping in the real one is
replacing that file.

## Interfaces

| port | |
|---|---|
| `tok_valid`, `tok_ready`, `tok[31:0]` | the token stream, `docs/h264-decoder.md` §1 |
| `pred_valid`, `pred_ready`, `pred_data[31:0]` | inter prediction from `h264_inter`: per inter macroblock, luma 16 rows of 4 words, Cb 8 rows of 2, Cr 8 rows of 2; four samples a word, the first lowest |
| `frame_idx[3:0]` | the frame buffer the next picture is written to, sampled with its `SLICE_FIRST` 0 |
| `pic_done` | one cycle, when a picture's last burst has been accepted |
| `cmd_*`, `wr_*`, `rd_*` | one `sdram_burst` client port; only writes are made |

Tokens read: `SPS_DIMS`, `SLICE_HDR` (constrained_intra_pred_flag),
`SLICE_FIRST`, `SLICE_QP` (both chroma offsets), `SLICE_DBLK`, `MB_START`,
`MB_TYPE`, `I4_MODE`, `MV`, `MB_CBP` (intra_chroma_pred_mode,
transform_size_8x8_flag), `MB_QP`, `COEF_BLK`, `COEF`, `PCM`, `MB_END`; the
rest are passed over. `MV` is `h264_mvpred`'s, as its section of
`docs/h264-decoder.md` gives it; only list 0 is read.

## Measured

**Cycles a macroblock, 576 in steady state**, from
`recon_keeps_within_the_720p30_budget`, for a dense I picture (every block
coded), a P picture and a P_Skip-only one alike: the deblocker's fixed
cost sets the rate. The budget for 1280x720 at 30 pictures a second at
100 MHz is 925 (3600 macroblocks, 30 times a second). It assumes the
token stream keeps up at a token a cycle; on a dense I picture the parser
is the real limit. With every port stalling at random,
`recon_decodes_pictures_through_the_sdram_port` sees 626.

## Resources

`reticle fpga ip/video/h264_recon/rtl/*.v --top h264_recon --device
xc7a100t-fgg484 --speed-grade 2 --memories legacy --report`, measured:

| | used | of the XC7A100T |
|---|---|---|
| LUT6 | 24 222, 10 levels deep at most | 63 400 |
| flip-flops | 6 602 | 126 800 |
| RAMB18E1 | 23 | 270 |
| RAM64X1D (distributed RAM) | 353 | |
| DSP48E1 | 19 | 240 |
| CARRY4 | 2 082 | |

**`--memories legacy`, because the default does not finish.** With the
default (`vivado`) memory mapping the flow did not finish `h264_intra`
in 40 minutes, and the cause was narrowed, by removing one thing at a
time, to its Intra_4x4 path through the registered two- and three-tap
averages; with `legacy` the same module maps in 6 seconds. Every other
module maps either way. That is a fault of the flow on this input, not
yet found.

**Not measured: the routed delay.** The 10 levels are the mapped
netlist's; a placed and routed figure needs the package's pins assigned
in a design around this block, and has not been made. A longest path of
10 LUT levels with carry chains is plausible at 100 MHz on a -2 part and
is not a claim.

What changed on the way to these figures, measured on the same flow:
`h264_deblock` was 205 796 LUTs as first written — every line of an edge
picked out of any byte of its 144 bytes of band registers, through
part-selects at computed offsets, which this flow builds as shifters
across the whole vector — and is 6 432 now that the registers turn under
the edges (see its header).

## What is and is not checked

| claim | how | checked or quoted |
|---|---|---|
| the edge filter, both bS < 4 and bS = 4, luma and chroma | 20 000 lines against 8-460 .. 8-489 | **checked** against a model transcribed from the equations |
| Tables 8-16 and 8-17 | every index and bS | **checked** against the same tables typed again into the test; both were read from one PDF |
| scaling, transforms, Hadamard, chroma DC, I_PCM | 48 macroblocks of levels drawn inside the bitstream's range limits | **checked** against the model |
| every intra mode, availability, constrained intra prediction | whole pictures, three slices, every mode the neighbours allow, all used | **checked** against the model |
| the deblocking order, slot and line buffer movement, every sample sent once | whole pictures with back-pressure | **checked** against the model |
| bS derivation, token fields, the stages in step, bursts inside a row | eight pictures end to end through an SDRAM model | **checked** against the model |
| **every sample of every picture of real streams** | `recon_matches_the_models_dumps_of_real_streams`: the four committed dumps (`testdata/ip/h264_*_{intra_32x32_qp26,intra_48x32_qp12,p_32x32_dump,craft_pcm}.txt`), 9 pictures; `recon_matches_more_of_the_models_dumps` (ignored; `RETICLE_H264_DUMPS`): 13 more streams of `testdata/ip/h264` dumped locally, 41 pictures — QP 1 and 51, Intra_16x16, constrained intra prediction, three slices, FilterOffsets −3/+3 and +6/−6, deblocking off, P_Skip-heavy, four references, P_8x8 sub-partitions, cropping | **checked** against the model's `post` rows, the model being bit-exact with ffmpeg on these streams (`tests/h264_model.rs`). The tokens are the parser's as the model dumps them; the `MV`s and the inter prediction are taken from the model's inter dump, so this is recon alone |

A test that passes against a wrong implementation is worse than none, so
the tests were checked against deliberate faults: the transform's rounding
constant, a chroma DC sign, an Intra_4x4 index, a deblocking QP taken from
the wrong neighbour, a motion vector difference of exactly 4 read as
small, and constrained intra prediction left off the macroblock above and
to the right — each made its test fail. The last two were not caught by
the first version of the end-to-end test, whose pictures were noise: the
filter's thresholds reject almost every line of noise, so it now draws
smooth pictures at high QPs too.

## Not done

- **Reference pictures are compared by ref_idx** for bS 1, which is the same
  thing within a slice whose list names no picture twice. A list that names
  a picture twice, or two slices of one picture with different lists, needs
  the picture each index names, which the DPB manager has.
- **The QPC of a neighbouring macroblock** is computed with the current
  slice's chroma offsets. Two slices of one picture with different
  `chroma_qp_index_offset`s (different PPSs) would differ.
- High profile: the 8x8 transform, Intra_8x8 and scaling matrices; B
  slices (two motion vectors); SP and SI; MBAFF and fields.
- **Errors**: an `ERROR` token ends the current macroblock as if it were
  complete, so the stages stay in step, but nothing is concealed and a
  picture that loses macroblocks never reaches `pic_done`.
