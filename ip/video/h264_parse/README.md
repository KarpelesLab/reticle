# `h264_parse`

The front end of an H.264 decoder: an Annex B byte stream in, the syntax
token stream of [`docs/h264-decoder.md`](../../../docs/h264-decoder.md) §1
out. Written from ITU-T Recommendation H.264, edition V16 (06/2026); clause
and table numbers here are that edition's. No other decoder's source was
read. ffmpeg is used as a black box, to make streams and to say what is in
them.

| module | what it is |
|---|---|
| `h264_parse` | the package's `top`: parameter sets, slice header, slice data, macroblock layer, the neighbour context and the output slot |
| `h264_nal` | start codes and emulation prevention (7.3.1, B.2): bytes in, each NAL unit's bytes out with its first and last marked |
| `h264_bits` | the bit reader: a registered 32-bit window over an eight-byte ring, any number of bits consumed a cycle, `more_rbsp_data()` |
| `h264_cavlc` | one `residual_block_cavlc()` (7.3.5.3.2, 9.2) into `COEF_BLK` and `COEF` tokens |
| `h264_vlc` | the CAVLC tables and the coded_block_pattern mapping as decoders, **generated** by `tools/gen_vlc.py` from `testdata/ip/h264_parse_vlc_tables.txt` |

## What it decodes, and what it reports instead

Decoded: CAVLC I and P slices of 4:2:0 8-bit frame pictures, which is
Baseline plus the parts of Main and High that CAVLC streams use — the 8x8
transform (`transform_size_8x8_flag`, Intra_8x8 modes), explicit weighted
prediction tables, multiple reference frames with list modification, and
adaptive reference marking. Every sequence and picture parameter set is
stored by id (32 SPS, 256 PPS) and the one a slice names is activated.

Reported with an `ERROR` token and the NAL unit dropped
(docs/h264-decoder.md §1.8): CABAC, B, SP and SI slices, slice groups (FMO),
interlace, chroma other than 4:2:0, bit depth other than 8, lossless
(`qpprime_y_zero_transform_bypass_flag`), scaling matrices, a picture wider
than `MAX_W_MBS` (128 macroblocks, 2048 samples), a `level_prefix` above
16, and every way a stream can break the CAVLC or macroblock syntax.
Redundant slices are dropped silently; SEI, access unit delimiters and every
other NAL unit type are skipped. The VUI is not parsed: nothing after
`vui_parameters_present_flag` is needed, so the SPS is stored at that point.

Not done here, by design (docs/h264-decoder.md §1.7): picture order count,
reference list construction and marking (the syntax is surfaced as `SLICE_POC`,
`RPLM`, `DRPM` and `MMCO` tokens for a DPB manager), motion vector
prediction, and everything after the residual.

## What is checked, and against what

| check | test | corpus |
|---|---|---|
| the table transcription: every VLC prefix-free, Kraft sums as each table's shape implies, cbp columns permutations of 0..47 | `the_vlc_transcription_is_a_set_of_prefix_codes` | `h264_parse_vlc_tables.txt` |
| `h264_vlc` decodes every code of that transcription, under five paddings | `h264_vlc_decodes_every_code_of_the_tables` | the same |
| every macroblock's class and QPY are what ffmpeg's decoder reports, picture for picture, and the token grammar holds | `h264_parse_agrees_with_ffmpeg_on_every_macroblock` | six streams, below |
| the same tokens under random backpressure on both sides | `h264_parse_holds_a_stream_under_backpressure` | the smallest |
| a slice cut short is one `ERROR` code 6 and costs only its own picture; 40 single-bit corruptions of slice data never stop the parser or lose the last picture's end | `h264_parse_reports_truncation_and_survives_corruption` | `tiny` |
| token for token against the software model's dumps | `h264_parse_matches_the_models_token_dumps` | the four committed `h264_tokens_*.txt`: 6 279 tokens |

**The truncation test found two faults** that every valid stream had
missed, both in how a failure is left behind. A NAL unit that ended inside
a residual block left `h264_cavlc` in the middle of that block, so the next
slice's first block was decoded by a machine already half way through
another; and a failure that landed between a state's two phases left the
second phase armed, so the next NAL unit's header was taken from the old
latched window. The first P slice after the cut then parsed one macroblock
past the end of its picture. `h264_cavlc` now has an `abort`, and a
failure clears the phase.

The six structural corpora (`testdata/ip/h264_parse_*.txt`, each the stream
in hex and ffmpeg's `-debug mb_type` and `-debug qp` maps):

| corpus | made by | what it exercises |
|---|---|---|
| `tiny` | libx264 Baseline, 64x48, 3 pictures | I_NxN, I_16x16, P_L0_16x16, P_Skip |
| `slices` | libx264 Baseline, 176x144, 5 pictures, 3 slices, 3 references, all partitions | slices, every P partition shape and sub-macroblock type, ref_idx te(v) |
| `lowqp` | libx264 Baseline, QP 1, intra | large levels, the level_prefix escapes (levels to 739) |
| `openh264` | libopenh264, 128x96, 6 pictures | a second encoder; POC lsb, reference list modification |
| `high8x8` | libx264 High with CAVLC, 8x8 transform, weighted prediction, 2 references | `transform_size_8x8_flag`, `PWT_DENOM`/`PWT`, `RPLM` |
| `pcm` | `tools/make_pcm.py`, by hand from the syntax tables | I_PCM in I and P slices (with emulation prevention in the samples), an I_16x16 whose nC comes from an I_PCM neighbour, MMCO, deblocking offsets, pic_order_cnt_type 1, and Exp-Golomb codes of 33 to 37 bits (idr_pic_id 65535, offset_for_non_ref_pic -100000, delta_pic_order_cnt 70000), which take the two-state long path |

**What the ffmpeg comparison would and would not catch.** ffmpeg prints a
class and a QPY per macroblock, so a wrong coefficient, a wrong intra mode or
a wrong motion vector difference is invisible to it — **unless** it changes
how many bits are read, and in CAVLC almost every mistake does: the next
element is then read from the wrong place and the stream leaves the rails
within a macroblock or two, which is a class or QP mismatch or an `ERROR`.
A wrong level *sign*, a wrong scan position or a wrong resolved intra mode
reads the same bits, and only the model's token dumps can catch it.

One ffmpeg number is not compared: the QPY of an I_PCM macroblock. ffmpeg
prints 0, which is the qP deblocking uses for one (8.7.2.2); 7.4.5 infers
mb_qp_delta = 0 for every macroblock without one, so its QPY is its
predecessor's, and that is what `MB_QP` carries. ffmpeg's own map agrees:
the macroblock after an I_PCM one continues from the predecessor's QPY, not
from 0.

**Against the software model, token for token.** `tools/h264_model.py`
was written by another agent from the same Recommendation, without this
parser's source, and is checked byte for byte against ffmpeg's decoded
pictures; its dumps are what catches what ffmpeg's maps cannot. The four
committed dumps match exactly, and so did a wider sweep, run once and not
committed because it is the model's whole corpus: every stream in
`testdata/ip/h264/` that is CAVLC, I and P — 29 streams, 116 154 tokens,
including crops, MMCOs under all three POC types, constrained intra, four
references, slices, Intra_8x8 and High CAVLC — and the six structural
corpora above, dumped by the model, 48 136 tokens. The two streams of the
sweep that did not match are CABAC, and the parser reports them with
`ERROR` code 4 as it should.

One difference was found and settled on the model's side of the format:
`MB_TYPE`'s partition prediction bits for P_8x8 and P_8x8ref0 are 0, since
Table 7-13 gives their MbPartPredMode as *na*; the prediction is each
sub-macroblock's. docs/h264-decoder.md §1.4 now says so.

Before the model landed, a second implementation of the format in Python,
by the same author as the RTL and so not independent, was the token-level
check. It found two bugs, both in the Python: dcPredModePredictedFlag taken
per macroblock instead of per block, and a picture's end missed when its
last macroblocks are a skip run.

**The truncation test found two faults** that every valid stream had
missed, both in how a failure is left behind. A NAL unit that ended inside
a residual block left `h264_cavlc` in the middle of that block, so the next
slice's first block was decoded by a machine already half way through
another; and a failure that landed between a state's two phases left the
second phase armed, so the next NAL unit's header was taken from the old
latched window. The first P slice after the cut then parsed one macroblock
past the end of its picture. `h264_cavlc` now has an `abort`, and a
failure clears the phase.

The six structural corpora (`testdata/ip/h264_parse_*.txt`, each the stream
in hex and ffmpeg's `-debug mb_type` and `-debug qp` maps):

| corpus | made by | what it exercises |
|---|---|---|
| `tiny` | libx264 Baseline, 64x48, 3 pictures | I_NxN, I_16x16, P_L0_16x16, P_Skip |
| `slices` | libx264 Baseline, 176x144, 5 pictures, 3 slices, 3 references, all partitions | slices, every P partition shape and sub-macroblock type, ref_idx te(v) |
| `lowqp` | libx264 Baseline, QP 1, intra | large levels, the level_prefix escapes (levels to 739) |
| `openh264` | libopenh264, 128x96, 6 pictures | a second encoder; POC lsb, reference list modification |
| `high8x8` | libx264 High with CAVLC, 8x8 transform, weighted prediction, 2 references | `transform_size_8x8_flag`, `PWT_DENOM`/`PWT`, `RPLM` |
| `pcm` | `tools/make_pcm.py`, by hand from the syntax tables | I_PCM in I and P slices (with emulation prevention in the samples), an I_16x16 whose nC comes from an I_PCM neighbour, MMCO, deblocking offsets, pic_order_cnt_type 1, and Exp-Golomb codes of 33 to 37 bits (idr_pic_id 65535, offset_for_non_ref_pic -100000, delta_pic_order_cnt 70000), which take the two-state long path |

**What the ffmpeg comparison would and would not catch.** ffmpeg prints a
class and a QPY per macroblock, so a wrong coefficient, a wrong intra mode or
a wrong motion vector difference is invisible to it — **unless** it changes
how many bits are read, and in CAVLC almost every mistake does: the next
element is then read from the wrong place and the stream leaves the rails
within a macroblock or two, which is a class or QP mismatch or an `ERROR`.
A wrong level *sign*, a wrong scan position or a wrong resolved intra mode
reads the same bits, and only the model's token dumps can catch it.

One ffmpeg number is not compared: the QPY of an I_PCM macroblock. ffmpeg
prints 0, which is the qP deblocking uses for one (8.7.2.2); 7.4.5 infers
mb_qp_delta = 0 for every macroblock without one, so its QPY is its
predecessor's, and that is what `MB_QP` carries. ffmpeg's own map agrees:
the macroblock after an I_PCM one continues from the predecessor's QPY, not
from 0.

**Checked a second way, and why that is weaker than it sounds.** During
development a second implementation of the token format was written in
Python from the same syntax tables, and the RTL's output matches it token
for token on all six corpora (48 134 tokens). It found two bugs, both in
the Python and not the RTL: dcPredModePredictedFlag taken per macroblock
instead of per block, and a picture's end missed when its last
macroblocks are a skip run. But it is the same reader of the same
specification, so where both are wrong in the same way it agrees; it is not
committed, and it is not the model. The model's dumps are the independent
check, and until they land the claim is the ffmpeg one above.

## Throughput

Measured by `h264_parse_cycles_per_macroblock` (`#[ignore]`d; it prints and
asserts nothing), with the consumer always ready and a byte offered every
cycle:

| stream | bytes | MBs | cycles / MB | cycles / byte |
|---|---|---|---|---|
| `tiny` | 626 | 36 | 127 | 7.3 |
| `slices` | 8 510 | 495 | 135 | 7.9 |
| `openh264` | 6 272 | 288 | 172 | 7.9 |
| `high8x8` | 6 850 | 240 | 203 | 7.1 |
| `lowqp` (QP 1, intra) | 5 177 | 24 | 670 | 3.1 |
| `pcm` | 1 603 | 8 | 286 | 1.4 |
| 720p, libx264 4 Mbit/s, 3 pictures (not committed) | 76 009 | 10 800 | 69 | 9.8 |
| 720p, one I picture of noise at QP 24 (not committed) | 443 338 | 3 600 | 1 025 | 8.3 |

720p30 is 108 000 macroblocks a second, so at 100 MHz the budget is **925
cycles a macroblock**. The parser spends 7 to 10 cycles a byte, a little
over one a bit, so the budget is about 750 bits a macroblock, or **80
Mbit/s sustained** -- and Level 3.1, which is 720p30's level, caps the
stream at 14 Mbit/s, so the margin is between five and six times. The last
row is the stress case and the one place the budget is missed: a single
3.5 Mbit picture, a quarter of Level 3.1's whole second of bit rate in one
frame, takes 1 025 cycles a macroblock, 11 % over. The pictures around such
a one would have to be correspondingly small for the stream to be legal,
and a frame of buffering between the parser and reconstruction absorbs
it. The two 720p rows were measured on streams too large to commit (76 KB
and 443 KB), by copying them into `testdata/ip/` for one run of the same
test.

Skipped macroblocks cost 5 cycles, one a token. An intra macroblock is
dominated by its residual: about five cycles a coefficient (h264_cavlc.v's
header counts them).

## Timing

**9.10 to 9.37 ns routed at speed grade 2, on all three placement seeds
tried**, on `xc7a100t-fgg484` (the target XC7A100T-2FGG484I's own device
file): `reticle fpga --device xc7a100t-fgg484 --timing-driven --bitstream`,
a 10 ns clock and the 47 ports on banks 34 and 35. Measured by Reticle's
delay model fitted to Vivado's -- not by Vivado, and not on a part:

| `--place-seed` | routed | bitstream decoded back |
|---|---|---|
| 1 (the default) | **9.37 ns** | 486 476 bits, none unexplained |
| 2 | **9.10 ns** | 471 432 bits, none unexplained |
| 3 | **9.12 ns** | 470 688 bits, none unexplained |

and in each, the interconnect the bits select is exactly what the router
chose. The latest path of all three is now the fixed-length read of a
header field: the state register, the field's width, the shift of the
window by it, into `uval`.

It took eight rounds, each found by reading the latest path the router
reported, and the path to the answer is the useful part:

| round | change | routed |
|---|---|---|
| 1 | one cycle a syntax element, the window a combinational rotator | 22.0 ns |
| 2 | the window registered in `h264_bits`; each reading state in two phases, decode latched then acted on | 15.9 ns |
| 3 | leading zeros as a tree, not a 32-step priority chain; a CAVLC level in three steps; bytes zeroed as they are consumed instead of a mask on the window | 12.6 ns |
| 4 | `consume` latched with the decode, so it is a function of registers and the output slot only | 12.9 ns |
| 5 | a read past a NAL unit's end flagged by the bit reader and reported a state later, instead of compared before consuming | 11.3 ns |
| 6 | ue(v) by selecting one of sixteen fixed fields rather than shifting; se(v) from codeNum + 1 at 17 bits; the nC walk latched a cycle before a block starts | 9.53 ns at seed 1, but 9.9 to 10.6 by seed one commit later |
| 7 | `S_RESW` kept as a register of its own (`resw_q`), and the CAVLC block's `go` made of it, the phase and the output slot; a second phase's `go` needs nothing else; the 256 bits of levels written whenever their state is current, so their enables no longer hang on `go` | 9.93 / 10.64 / 10.07 ns |
| 8 | ue(v) and se(v) worked out for every code length in parallel with the leading-zero count, which then only selects; the window rotated from the ring before this cycle's zeroing; the predicted intra mode latched in the first phase; the long Exp-Golomb path's arithmetic in a third state (`S_LONG3`), reading codeNum + 1 whole; the bits after a level's prefix by a shift without an adder; suffixLength updated from the latched levelCode; the residual's neighbour lookup on its own block index; nC latched and its class worked out as the block starts | **9.37 / 9.10 / 9.12 ns** |

Round 4 is not a regression worth worrying about: the path it removed was
replaced by the next one down, which round 5 then cut. Round 6's number
was one seed, and the next commit's three seeds showed how far that is
from a margin: a design within a quarter of a nanosecond of the clock
moves across it with the placement. Rounds 7 and 8 were judged on three
seeds each.

The cycles a symbol went from one to two, and three for a level, at
rounds 2 and 3; rounds 4 to 8 added none, except a third state on the
long Exp-Golomb path (codes over 31 bits, which no libx264 stream here
has): the 720p stream is 745 014 cycles after round 8 against 745 080
before it, and the six committed corpora are the same to the cycle but
for `pcm`, one more. That was cheap, because the one-cycle design was
nine times inside the budget on the streams 720p30 actually sends.

## Resources

On `xc7a100t-fgg484`, from the seed-1 build (`--report`): **7 827 LUTs**
(12.3 % of the part's 63 400), **1 971 flip-flops** (1 926 `FDCE`, 45
`FDPE`), 332 `CARRY4`, 380 `MUXF7` and 154 `MUXF8`, **3 `RAMB36E1` and 1
`RAMB18E1`**, no DSP. Mapped logic depth 10. Round 8 cost about 1 200
lookup tables, nearly all of it the sixteen parallel Exp-Golomb
subtractors and negators and the duplicated neighbour lookup.

The block RAMs are the line buffer (one 57-bit word per macroblock column,
128 columns), the PPS store (256 x 43) and the SPS store (32 x 95). The
flip-flops are mostly the current macroblock's context (16 TotalCoeffs and
16 modes, the chroma counts, the left neighbour's column), the CAVLC
block's 16 levels, the active parameter sets and the 256 PPS valid bits.

## Regenerating

```sh
python3 ip/video/h264_parse/tools/gen_vlc.py > ip/video/h264_parse/rtl/h264_vlc.v
python3 ip/video/h264_parse/tools/make_corpus.py NAME 'FFMPEG ARGS' > testdata/ip/h264_parse_NAME.txt
python3 ip/video/h264_parse/tools/make_pcm.py /tmp/pcm.264
python3 ip/video/h264_parse/tools/make_corpus.py pcm file:/tmp/pcm.264:2 > testdata/ip/h264_parse_pcm.txt
```

`H264_DUMP_DIR=<dir>` makes `h264_parse_agrees_with_ffmpeg_on_every_macroblock`
write each corpus's tokens there, one hex word a line, for comparing with
another decoder.

## Open

- CABAC, B slices (the format has room for both), interlace and MBAFF,
  4:2:2 and 4:4:4, high bit depth, scaling matrices.
- `level_prefix` 17 to 19, which a High profile stream at very low QP may
  use: the window is 32 bits and the code is up to 36.
- Nothing here has run on a part.
