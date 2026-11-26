# The H.264 decoder

A 720p30 H.264 decoder for an XC7A100T, built as a pipeline of IP
packages:

```text
bytes ─► h264_parse ─► tokens ─► h264_mvpred ─► tokens ─► h264_recon ─► SDRAM
                                     │                        ▲
                                     └──► h264_inter ─────────┘ prediction samples
```

Everything here is written from ITU-T Recommendation H.264, edition V16
(06/2026). Clause and table numbers below are that
Recommendation's. No other decoder's source was read; ffmpeg and ffprobe are
run as black boxes to make streams and to check answers.

This file is the **interface** between the packages. The token stream section
is owned by `ip/video/h264_parse` and every other stage conforms to it; the
software model's dump format has a section of its own further down.

## 1. The syntax token stream

`h264_parse` turns an Annex B byte stream into a stream of 32-bit tokens:

```text
tok_valid, tok_ready, tok[31:0]        tok = {tag[7:0], data[23:0]}
```

A token moves on a clock edge where `tok_valid && tok_ready`. Bits of `data`
that a tag does not define are **zero**, always, so a later version can give
them a meaning without breaking a consumer that ignores them. A consumer must
ignore a tag it does not know.

**Signed** fields are two's complement in the width given. Nothing is
saturated silently: a value the field cannot hold is an `ERROR` token instead.

### 1.1 Tag space

| range | what |
|---|---|
| `0x00` | never emitted; a zero tag is a bug |
| `0x01`–`0x0F` | sequence, picture and slice level |
| `0x10`–`0x1F` | macroblock level (`0x16` `MV` is produced by `h264_mvpred`, not the parser) |
| `0x20`–`0x2F` | residual |
| `0x30`–`0x3E` | raw samples |
| `0x3F` | `MB_END` |
| `0x40`–`0x4F` | structure ends |
| `0x50`–`0x7E` | spare, reserved for the parser (CABAC, 8x8, B extras) |
| `0x7F` | `ERROR` |
| `0x80`–`0xFF` | reserved for tokens a **later stage** inserts; the parser never emits them |

### 1.2 Sequence and slice tokens

| tag | name | data |
|---|---|---|
| `0x01` | `SPS_DIMS` | `[23:12]` PicWidthInMbs, `[11:0]` FrameHeightInMbs |
| `0x07` | `SPS_INFO` | `[23:16]` profile_idc, `[15:8]` level_idc, `[7:3]` max_num_ref_frames (0..16), `[2]` frame_mbs_only_flag, `[1]` direct_8x8_inference_flag, `[0]` gaps_in_frame_num_value_allowed_flag |
| `0x08` | `SPS_POC` | `[23:22]` pic_order_cnt_type, `[21:17]` log2_max_frame_num (4..16), `[16:12]` log2_max_pic_order_cnt_lsb (4..16; 0 when pic_order_cnt_type ≠ 0), `[11:10]` chroma_format_idc, `[9:7]` bit_depth_luma_minus8, `[6:4]` bit_depth_chroma_minus8, `[3]` delta_pic_order_always_zero_flag |
| `0x09` | `SPS_CROP` | `[23:18]` frame_crop_left_offset, `[17:12]` right, `[11:6]` top, `[5:0]` bottom, in the crop units of 7.4.2.1.1 (two luma samples each way for 4:2:0 frames); all zero when frame_cropping_flag is 0 |
| `0x02` | `SLICE_HDR` | `[23:20]` slice_type % 5 (0 P, 1 B, 2 I, 3 SP, 4 SI), `[19]` IdrPicFlag, `[18:17]` nal_ref_idc, `[16]` entropy_coding_mode_flag, `[15]` constrained_intra_pred_flag, `[14]` transform_8x8_mode_flag, `[13]` weighted_pred_flag, `[12:11]` weighted_bipred_idc, `[10]` direct_spatial_mv_pred_flag (0 unless B), `[9:8]` cabac_init_idc (0 unless CABAC), `[7:0]` pic_parameter_set_id |
| `0x06` | `SLICE_FIRST` | `[23:0]` first_mb_in_slice |
| `0x0A` | `SLICE_POC` | `[23:22]` which: 0 pic_order_cnt_lsb (unsigned), 1 delta_pic_order_cnt_bottom, 2 delta_pic_order_cnt[0], 3 delta_pic_order_cnt[1]; `[21:0]` value, the deltas signed |
| `0x05` | `SLICE_REF` | `[23:20]` num_ref_idx_l0_active_minus1, `[19:16]` num_ref_idx_l1_active_minus1 (0 unless B), `[15:0]` frame_num |
| `0x0B` | `RPLM` | one ref_pic_list_modification operation: `[23]` list (0 or 1), `[22:21]` modification_of_pic_nums_idc (0, 1 or 2; the closing 3 is not emitted), `[16:0]` abs_diff_pic_num_minus1 or long_term_pic_num |
| `0x0C` | `PWT_DENOM` | `[5:3]` chroma_log2_weight_denom, `[2:0]` luma_log2_weight_denom |
| `0x0D` | `PWT` | one explicit weight: `[23]` list, `[22:18]` ref_idx, `[17:16]` component (0 Y, 1 Cb, 2 Cr), `[15:8]` weight (signed), `[7:0]` offset (signed). Emitted only where the stream's `luma_weight_lX_flag` / `chroma_weight_lX_flag` is 1; an entry not emitted has the default weight 2^denom and offset 0 |
| `0x0E` | `DRPM` | dec_ref_pic_marking, emitted for every slice with nal_ref_idc ≠ 0: `[2]` no_output_of_prior_pics_flag, `[1]` long_term_reference_flag (IDR only), `[0]` adaptive_ref_pic_marking_mode_flag (non-IDR only) |
| `0x0F` | `MMCO` | one memory_management_control_operation (1..6): `[23:21]` operation, `[20:16]` long_term_frame_idx (ops 3 and 6) or max_long_term_frame_idx_plus1 (op 4), `[15:0]` difference_of_pic_nums_minus1 (ops 1 and 3) or long_term_pic_num (op 2). The closing 0 is not emitted |
| `0x03` | `SLICE_QP` | `[23:16]` SliceQPY (signed), `[15:8]` chroma_qp_index_offset (signed), `[7:0]` second_chroma_qp_index_offset (signed; equal to the first when the PPS does not carry it) |
| `0x04` | `SLICE_DBLK` | `[23:22]` disable_deblocking_filter_idc, `[15:8]` FilterOffsetA (signed, = slice_alpha_c0_offset_div2 << 1), `[7:0]` FilterOffsetB (signed) |
| `0x40` | `SLICE_END` | the slice's last macroblock has been sent |
| `0x41` | `PIC_END` | the picture's last macroblock has been sent; follows that slice's `SLICE_END` |

### 1.3 Order at slice level

Every slice is, in this order:

```text
[SPS_DIMS SPS_INFO SPS_POC SPS_CROP]    only when first_mb_in_slice == 0
SLICE_HDR SLICE_FIRST
SLICE_POC*                              pic_order_cnt_lsb, then the deltas, each when present
SLICE_REF
RPLM*                                   list 0's operations, then list 1's
[PWT_DENOM PWT*]                        when weighted prediction applies to the slice
[DRPM MMCO*]                            when nal_ref_idc != 0
SLICE_QP SLICE_DBLK
macroblocks…
SLICE_END [PIC_END]
```

**A picture starts** at a slice whose first_mb_in_slice is 0: arbitrary slice
order is outside this decoder, so that is the first slice. **A picture ends**
when a slice's last macroblock is macroblock PicSizeInMbs − 1, and `PIC_END`
follows that `SLICE_END`. Slices with redundant_pic_cnt > 0 are dropped
whole and emit nothing.

### 1.4 Macroblock tokens

| tag | name | data |
|---|---|---|
| `0x10` | `MB_START` | `[23:12]` mb_x, `[11:0]` mb_y |
| `0x11` | `MB_TYPE` | `[23:19]` kind (table below), `[18:17]` partition 0 prediction (`bit 0` uses list 0, `bit 1` uses list 1; 0 for intra and direct), `[16:15]` partition 1 prediction (0 unless there are two partitions), `[14:13]` Intra16x16PredMode (0 unless I_16x16) |
| `0x15` | `SUB_MB_TYPE` | `[15:12]` sub_mb_type[0], `[11:8]` [1], `[7:4]` [2], `[3:0]` [3], the raw values of Tables 7-17 (P) and 7-18 (B) |
| `0x13` | `I4_MODE` | `[7:4]` luma4x4BlkIdx, `[3:0]` Intra4x4PredMode — the **final** mode |
| `0x17` | `I8_MODE` | `[5:4]` luma8x8BlkIdx, `[3:0]` Intra8x8PredMode — the final mode |
| `0x18` | `REF_IDX` | `[23]` list, `[22:21]` mbPartIdx, `[4:0]` ref_idx |
| `0x14` | `MVD` | `[23]` list, `[22:21]` mbPartIdx, `[20:19]` subMbPartIdx (0 outside P_8x8/B_8x8), `[18]` component (0 horizontal, 1 vertical), `[15:0]` mvd (signed, quarter samples) |
| `0x16` | `MV` | not the parser's: inserted by `h264_mvpred`, see its section |
| `0x19` | `MB_CBP` | `[8]` transform_size_8x8_flag, `[7:6]` intra_chroma_pred_mode (0 for inter), `[5:0]` coded_block_pattern (`[3:0]` luma, one bit per 8x8; `[5:4]` chroma 0..2) |
| `0x12` | `MB_QP` | `[7:0]` QPY (signed) after mb_qp_delta |
| `0x3F` | `MB_END` | — |

`MB_TYPE` kinds:

| kind | mb_type | kind | mb_type |
|---|---|---|---|
| 0 | I_NxN (I_4x4, or I_8x8 when `MB_CBP[8]`) | 8 | P_8x8ref0 |
| 1 | I_16x16 | 9 | P_Skip |
| 2 | I_PCM | 10 | B_Direct_16x16 |
| 3 | SI | 11 | B 16x16 (prediction in `[18:17]`) |
| 4 | P_L0_16x16 | 12 | B 16x8 |
| 5 | P_L0_L0_16x8 | 13 | B 8x16 |
| 6 | P_L0_L0_8x16 | 14 | B_8x8 |
| 7 | P_8x8 | 15 | B_Skip |

16..31 are spare. An intra macroblock in a P or B slice has the same kind as
in an I slice.

### 1.5 Order within a macroblock

```text
MB_START
MB_TYPE
[SUB_MB_TYPE]                           P_8x8, P_8x8ref0, B_8x8
[I4_MODE x16 | I8_MODE x4]              I_NxN, in block index order
[REF_IDX… MVD…]                         inter, in the order of 7.3.5.1 / 7.3.5.2
MB_CBP
MB_QP
[PCM x128 | (COEF_BLK COEF*)*]
MB_END
```

**`MB_TYPE` comes before the prediction tokens and `MB_CBP` after them**,
because coded_block_pattern and transform_size_8x8_flag follow mb_pred in the
syntax: putting them in `MB_TYPE` would make the parser buffer every `MVD`
of a macroblock, and a consumer of the motion vectors needs the partition
shape before the first `MVD`, not after the last.

**Inter prediction tokens.** In mb_pred order: a `REF_IDX` for each partition
that uses list 0, then for each that uses list 1, then the two `MVD`s
(horizontal, then vertical) of each partition for list 0, then for list 1.
In sub_mb_pred order: `REF_IDX` for each non-direct sub-macroblock that uses
list 0, then list 1; then for list 0 each sub-macroblock's sub-partitions'
`MVD` pairs, then for list 1. **A `REF_IDX` is emitted even where the syntax
infers it** — num_ref_idx_active_minus1 = 0, or P_8x8ref0 — with the
inferred value 0, so that no consumer has to know when a ref_idx is present.
A B_8x8 direct sub-macroblock has neither token.

**Skipped macroblocks** — each macroblock of an mb_skip_run — are
`MB_START, MB_TYPE (P_Skip or B_Skip), MB_CBP (0), MB_QP, MB_END`. Their
motion vectors are `h264_mvpred`'s to derive.

**`MB_QP` is emitted for every macroblock**, skipped and I_PCM included,
with the QPY the spec gives it: QPY,PRED when mb_qp_delta is absent. The
parser does the wraparound of 7.4.5: QPY = ((QPY,PRED + mb_qp_delta + 52) %
52) for 8-bit video. Deblocking's rule that an I_PCM macroblock counts as
qP = 0 (8.7.2.2) is recon's to apply; `MB_QP` carries the syntax's value.

### 1.6 Residual tokens

| tag | name | data |
|---|---|---|
| `0x20` | `COEF_BLK` | `[23:20]` category (table below), `[19:16]` block index, `[4:0]` TotalCoeff |
| `0x21` | `COEF` | `[23:18]` scan position, `[15:0]` level (signed) |
| `0x30` | `PCM` | three samples, the first in `[7:0]`, then `[15:8]`, `[23:16]` |

| category | block | block index | positions |
|---|---|---|---|
| 0 | Intra16x16DCLevel | 0 | 0..15 in the 4x4 zig-zag of the DC matrix |
| 1 | Intra16x16ACLevel | luma4x4BlkIdx | 1..15 |
| 2 | LumaLevel4x4 | luma4x4BlkIdx | 0..15 |
| 3 | ChromaDCLevel, Cb | 0 | 0..3, the index into chromaDCLevel (raster of the 2x2 c) |
| 4 | ChromaDCLevel, Cr | 0 | 0..3 |
| 5 | ChromaACLevel, Cb | chroma4x4BlkIdx (0..3) | 1..15 |
| 6 | ChromaACLevel, Cr | chroma4x4BlkIdx | 1..15 |
| 7 | LumaLevel8x8 | luma8x8BlkIdx | 0..63 (CABAC only; spare until then) |
| 8..15 | spare (4:4:4's Cb and Cr luma-like categories) | | |

**A scan position is the coefficient's index in the block's full zig-zag
scan** (frame scan, 8.5.6), so an AC-only block's first coefficient is at 1,
not 0 as in the `i16x16AClevel` array: recon need not know which blocks are
AC-only to place a coefficient. Positions are in coded order, not
dequantised or inverse-scanned; that is recon's.

**Which blocks get a `COEF_BLK`.** Exactly the blocks whose residual_block()
the syntax of 7.3.5.3 parses: the Intra16x16 DC block always for I_16x16;
the 4x4 luma blocks of each 8x8 whose coded_block_pattern bit is set; both
chroma DC blocks when coded_block_pattern's chroma part is nonzero; all eight
chroma AC blocks when it is 2. A block that is parsed gets its `COEF_BLK`
even when its TotalCoeff is 0. A block that is not parsed has TotalCoeff 0
and no token. Order is parse order: luma DC, luma blocks by luma4x4BlkIdx,
Cb DC, Cr DC, Cb AC 0..3, Cr AC 0..3.

**`COEF` tokens follow their `COEF_BLK`, one per nonzero level, TotalCoeff of
them, in descending scan position** — the order CAVLC codes them in.
A consumer must not depend on that order: CABAC codes coefficients upward,
and its tokens will come in ascending order.

**8x8 transform with CAVLC.** CAVLC codes an 8x8 block as four interleaved
4x4 residual_block()s (7.3.5.3.2). The parser emits them as what they are in
the syntax: category 2, block indices 4·luma8x8BlkIdx + i, positions 0..15.
The 8x8 position is `4 * position + (block index & 3)`; recon applies it.
TotalCoeff is the 4x4 parse's.

**I_PCM** is 128 `PCM` tokens: 256 luma samples in raster order within the
macroblock, then 64 Cb, then 64 Cr, three to a token. 8-bit only.

### 1.7 What the parser resolves, and what it leaves

Resolved by the parser:

- start codes, emulation prevention bytes, NAL unit types, SPS and PPS
  storage by id and activation at each slice;
- every syntax element of the slice header, with the PPS defaults applied
  (num_ref_idx_active, SliceQPY = 26 + pic_init_qp_minus26 + slice_qp_delta,
  FilterOffsetA/B doubled from the `_div2` fields, deblocking idc 0 when
  absent);
- mb_skip_run expanded into skipped macroblocks with their positions;
- mb_type to kind, prediction flags and Intra16x16PredMode, and the cbp an
  I_16x16 mb_type implies;
- coded_block_pattern's me(v) mapping (Table 9-4);
- QPY after mb_qp_delta, with the wraparound;
- **Intra4x4PredMode and Intra8x8PredMode**, from prev_intra_pred_mode_flag,
  rem_intra_pred_mode and the neighbours' modes (8.3.1.1, 8.3.2.1),
  including the unavailable, non-I_NxN and constrained_intra_pred rules;
- **nC for every CAVLC coeff_token** (9.2.1) from the TotalCoeff of the
  neighbouring blocks, kept across the picture in a block RAM line buffer
  (I_PCM neighbours count 16, skipped ones 0);
- the whole of CAVLC: coeff_token, trailing ones, levels with suffixLength
  adaptation, total_zeros, run_before, to (position, level) pairs.

Left to later stages:

- motion vector prediction, P_Skip and direct vectors (`h264_mvpred`);
- picture order count, frame_num gaps, reference list construction and
  modification, and reference marking — the parser **surfaces** the syntax
  (`SLICE_POC`, `SLICE_REF`, `RPLM`, `DRPM`, `MMCO`) for the DPB manager,
  which is outside the parser;
- chroma QP (Table 8-15 with chroma_qp_index_offset), dequantisation,
  inverse scan and transforms, intra prediction, deblocking (`h264_recon`).

### 1.8 Errors

| tag | name | data |
|---|---|---|
| `0x7F` | `ERROR` | `[23:16]` code, `[15:0]` detail |

| code | meaning |
|---|---|
| 1 | unsupported SPS: chroma_format_idc ≠ 1, bit depth ≠ 8, frame_mbs_only_flag = 0, or a picture wider than the line buffer |
| 2 | unsupported PPS: num_slice_groups_minus1 > 0 (FMO) |
| 3 | a slice names a PPS or SPS that was never received |
| 4 | unsupported slice: SP or SI, CABAC (until it is written), or scaling matrices |
| 5 | invalid syntax in slice data: no coeff_token matches, mb_type out of range, TotalCoeff + total_zeros too large, a run longer than the zeros left, a level out of range, a macroblock past the end of the picture |
| 6 | the slice NAL ended inside a macroblock |

`detail` is the macroblock address for codes 5 and 6 and zero otherwise. An
`ERROR` ends the slice: nothing more is emitted for it — no `MB_END`,
`SLICE_END` or `PIC_END` — and the parser resumes at the next NAL unit. A
consumer resynchronises at the next `SLICE_HDR`. Unsupported SPS and PPS are
reported when a slice activates them, not when they arrive, so a stream that
carries one it never uses decodes cleanly.

### 1.9 Room left

- **CABAC** emits the same tokens. Its coefficient order is ascending, which
  is why consumers may not rely on order; category 7 (LumaLevel8x8) and
  positions up to 63 are already in the format.
- **8x8 transform**: `I8_MODE`, `MB_CBP[8]` and category 7 exist now.
- **B slices**: kinds 10..15, the list-1 bit of every list field, the
  partition prediction bits of `MB_TYPE`, `SUB_MB_TYPE`'s four bits per
  sub-macroblock (B's go to 12), and `SLICE_HDR[10]` are already in the format.
- **4:2:2 and 4:4:4**, high bit depth: `SPS_POC` carries chroma_format_idc
  and the bit depths, QP fields are signed for QpBdOffset, categories 8..15
  are spare.
- Tags `0x50`–`0x7E` are spare for the parser; `0x80`–`0xFF` for later
  stages.

## Inter prediction: `h264_mvpred` and `h264_inter`

Both are in `ip/video/h264_inter`; its `README.md` has the design, what is
checked against what, and the measured SDRAM traffic.

### The `MV` token (`h264_mvpred`)

`h264_mvpred` passes every token through unchanged and in order **except
`MVD`, which it consumes**. Each P macroblock (kinds 4..9, P_Skip
included) gets its final vectors instead: for each 4x4 block in
luma4x4BlkIdx order, two `MV` tokens, horizontal then vertical, inserted
**immediately before the macroblock's `MB_CBP`**. Intra macroblocks get
none, and a B macroblock gets none until list 1 is written.

| tag | name | data |
|---|---|---|
| `0x16` | `MV` horizontal | `[23]` list, `[22:19]` luma4x4BlkIdx, `[18]` 0, `[15:0]` mvLX[0] (signed, quarter samples) |
| `0x16` | `MV` vertical | `[23]` list, `[22:19]` luma4x4BlkIdx, `[18]` 1, `[17:13]` refIdxLX, `[12:0]` mvLX[1] (signed) |

The vertical component is 13 bits because Table A-1 bounds it to
[-512, +511.75] samples at every level, which is 12; the horizontal one
reaches [-2048, +2047.75] and needs 14 of its 16. A non-conforming
vertical vector beyond 13 bits arrives truncated. `REF_IDX` and
everything else still pass, so a consumer that wants the parser's view
has it.

### `h264_inter`'s ports

- `tok`, `tok_valid`, `tok_ready`: the stream behind `h264_mvpred`, as a
  tap. It takes every token and changes nothing, so the integration forks
  `h264_mvpred`'s output to `h264_recon` and here, a token moving when both
  are ready. `tok_ready` falls only at a `SLICE_HDR` (below) and when both of
  its two vector banks are full, which is when the prediction for an
  earlier macroblock is still waiting for `h264_recon` to take it.
  The fork cannot deadlock as long as `h264_recon` takes a macroblock's
  tokens up to its `MB_CBP` without waiting for that macroblock's
  prediction: the vectors are in front of `MB_CBP`, and the prediction is
  only needed to add the residual.
- `map_frames[63:0]`, `map_valid`, `map_ready`: refIdxL0 0..15 to a 4-bit
  frame index, refIdxL0 0 in bits 3:0. After taking each `SLICE_HDR`,
  `h264_inter` raises `map_ready` and takes no token until `map_valid`; the
  DPB logic puts that slice's table on the wires and raises `map_valid`.
- `cmd_*`, `wr_*`, `rd_*`: one `sdram_burst` client port, reads only, bursts
  of 1..16 words inside one 32x32 tile row of the contract's layout.
- `pred_data`, `pred_valid`, `pred_ready`: per inter macroblock, luma 16 rows
  of 4 words, Cb 8 rows of 2, Cr 8 rows of 2, four samples a word, the
  leftmost in bits 7:0.

## The software model

`tools/h264_model.py` is a decoder written in Python (standard library
only) from Rec. ITU-T H.264 (V16, 06/2026), downloaded from itu.int. No
other decoder's source was read: not ffmpeg, x264, openh264 or JM. Clause
numbers in its comments are that edition's. The tables it needs are in
`tools/h264_tables.py`, each with its table number; the CAVLC code tables
were converted from the PDF's text by script and are checked to be
prefix-free with a Kraft sum of at most 1 (`h264_model.py selftest`).

It is the golden reference of the hardware: the corpora in
`testdata/ip/h264_*.txt` are its dumps. That is worth exactly as much as the
model is right, so the model is held to ffmpeg, frame by frame:

* **Checked:** every frame of every stream in `testdata/ip/h264/` decodes to
  the same bytes as ffmpeg 6.1.1's decoder. The `.md5` beside each stream is
  the md5 of each cropped I420 frame ffmpeg wrote; `tests/h264_model.rs`
  runs the model on every stream and fails on any frame that differs, on a
  missing frame, or on an extra one. A deliberately corrupted md5 makes it
  fail (tried). ffmpeg and libx264 were run only as black boxes, to make
  streams and reference pictures.
* **Checked, not committed:** a 1280x720 stream of three frames (an IDR and
  two P pictures of a rotating test card, two references, every partition
  shape), made by `tools/h264_streams.sh --hd DIR`, decodes bit-exact too.
  It is 44 KB and takes the model about 8 s, so it is generated on demand.

### What it decodes

Progressive 8-bit 4:2:0, one slice group, frame pictures. In detail:

| part | status |
|---|---|
| NAL units, emulation prevention, SPS, PPS, slice header (7.3) | done; VUI skipped |
| CAVLC (9.2): coeff_token, levels with every escape, total_zeros, run_before | done, checked |
| I slices: Intra_4x4 (9 modes), Intra_16x16 (4), chroma (4) | done, checked |
| I_PCM | implemented, **not checked**: libx264 never wrote one |
| P slices: every partition, P_8x8 sub-partitions, P_8x8ref0, P_Skip | done, checked |
| MV prediction (8.4.1.3), quarter-sample luma, eighth-sample chroma | done, checked |
| multiple reference frames, sliding window marking | done, checked |
| mb_qp_delta, chroma QP | done, checked |
| constrained_intra_pred, intra macroblocks in P slices | done, checked |
| deblocking (8.7): bS, alpha/beta/tC0, offsets, disabled | done, checked |
| several slices per picture | done, checked |
| cropping | done, checked |
| IDR in the middle of a stream, POC types 0 and 2 | done, checked (type 2) |
| MMCO, reference list modification, long-term references | implemented, **not checked**: libx264 never wrote one for these streams |
| explicit weighted prediction | implemented, not checked |
| CABAC, B slices, 8x8 transform | second phase; see below |
| interlace, MBAFF, FMO/ASO, 4:2:2, 4:4:4, high bit depth, SP/SI | rejected with an error |

### The streams

`tools/h264_streams.sh` holds the exact ffmpeg commands; every stream is
Baseline profile, one thread, with x264's SEI stripped. What each covers
(the full tally is `python3 tools/h264_model.py stats STREAM`):

| stream | covers |
|---|---|
| `intra_32x32_qp26` | all-intra, Intra_4x4 |
| `intra_48x32_qp1` | QP 1: levels above 255 (level_prefix 15, the longest escape Baseline allows, occurs in 14 streams) |
| `intra_48x32_qp12` | Intra_4x4 and Intra_16x16 DC, large levels |
| `intra_64x48_qp38` | all nine Intra_4x4 modes, chroma DC/horizontal/vertical |
| `intra_64x48_qp51` | QP 51, Intra_16x16 vertical/horizontal/DC |
| `intra_80x48_i16` | Intra_4x4 at 5x3 macroblocks |
| `intra_64x48_aq` | mb_qp_delta, Intra_16x16 plane, chroma plane |
| `p_64x48_parts` | 16x8, 8x16 and every P_8x8 sub-partition, all 16 fractional positions, vectors off the picture |
| `p_48x32_parts` | sub-partitions with exhaustive motion search, all 16 fractional positions |
| `p_80x48_skip` | mostly P_Skip, low motion |
| `p_80x48_pan` | a steady pan: P_Skip whose predicted vector is not zero, P_L0_16x16 |
| `p_64x48_ref4` | four references, ref_idx 0..3, P_8x8 with a different reference per 8x8, every partition |
| `p_64x48_aq` | mb_qp_delta in P slices, Intra_16x16 plane in P slices |
| `p_48x32_nodeblock` | disable_deblocking_filter_idc 1 |
| `p_48x32_dbk_m3p3` | FilterOffsetA -6, FilterOffsetB +6 |
| `p_48x32_dbk_p6m6` | FilterOffsetA +12, FilterOffsetB -12 |
| `p_64x48_slices` | three slices per picture: availability and deblocking across slice edges |
| `p_64x48_cip` | periodic intra refresh: intra macroblocks in P slices, constrained_intra_pred |
| `crop_30x18`, `crop_50x38` | frame cropping, picture sizes that are not multiples of 16 |
| `idr_32x32_gop3` | an IDR every three pictures, frame_num and POC restarting |
| `p_32x32_dump` | the source of the P dump corpus |

Across them every boundary strength 0..4 occurs, every mv fraction, and
every macroblock type of a P slice except I_PCM.

## Model dumps

`python3 tools/h264_model.py dump STREAM.264 DIR --frames N` writes up to
three text files per stream into `DIR`; `tools/h264_dumps.sh` regenerates
the committed ones, and `tests/h264_model.rs` fails if a committed dump is
not exactly what the current model writes. The committed corpora:

| stream | pictures | files |
|---|---|---|
| `intra_32x32_qp26` | 1 IDR, 2x2 MBs, Intra_4x4 | tokens, recon |
| `intra_48x32_qp12` | 1 IDR, 3x2 MBs, Intra_4x4 and Intra_16x16 | tokens, recon |
| `p_32x32_dump` | IDR + 2 P, 2x2 MBs, P_8x8 sub-partitions, two references | tokens, recon, inter |

Every file is line-oriented: a line starts with a keyword, `#` starts a
comment, blank lines separate pictures. All numbers are decimal unless they
are said to be hex. Samples are hex, two digits per sample, a row left to
right; a block is its rows separated by single spaces, top row first.
Macroblocks appear in decoding order, which for these streams is raster
order.

Lines common to all three files:

    stream NAME WIDTH_MBS HEIGHT_MBS
    pic INDEX TYPE poc POC frame_num N ref 0|1 idr 0|1
    mb ADDR MBX MBY MB_TYPE_NAME [qp QPY]

`INDEX` counts pictures in decoding order from 0; `TYPE` is `I`, `P` or
`B` (of the first slice). `MB_TYPE_NAME` is the name Table 7-11, 7-13 or
7-14 gives (`I_NxN`, `I_16x16_<pred>_<cbp chroma>_<cbp luma 0/1>`, `I_PCM`,
`P_L0_16x16`, `P_8x8ref0`, `P_Skip`, ...).

### `h264_tokens_NAME.txt`: the parser's input and output

    nal HEX        one NAL unit, header byte included, emulation prevention
                   bytes still in: the parser's input, in stream order
    slice FIRST_MB type T qp SLICEQP dbk IDC A B nref N
    tok WORD [comment]

`nal` lines come first (every NAL unit of the dumped pictures), then the
token stream picture by picture. Each `tok` is one 32-bit token, 8 hex
digits, `{tag[7:0], data[23:0]}`; the comment after it is for people. The
tags are the contract's v0 set; until the parse agent's layout lands in the
token section of this file, the data fields are packed as follows (and the
dumps will follow that layout when it does):

| tag | name | data |
|---|---|---|
| 01 | SPS_DIMS | `width_mbs[23:12] height_mbs[11:0]` |
| 02 | SLICE_HDR | `nal_ref_idc[6:5] IdrPicFlag[4] slice_type[3:0]` (slice_type % 5) |
| 03 | SLICE_QP | `second_chroma_qp_index_offset[20:16] chroma_qp_index_offset[12:8] SliceQPY[5:0]`, offsets two's complement |
| 04 | SLICE_DBLK | `FilterOffsetB[20:16] FilterOffsetA[12:8] disable_deblocking_filter_idc[1:0]`, offsets two's complement |
| 05 | SLICE_REF | `frame_num[23:8] num_ref_idx_l0_active_minus1[4:0]` |
| 06 | SLICE_FIRST | `first_mb_in_slice` |
| 10 | MB_START | `mb_x[23:12] mb_y[11:0]` |
| 11 | MB_TYPE | `transform_size_8x8_flag[16] coded_block_pattern[15:10] intra_chroma_pred_mode[9:8] Intra16x16PredMode[7:6] kind[5:0]` |
| 12 | MB_QP | `QPY` after mb_qp_delta |
| 13 | I4_MODE | `luma4x4BlkIdx[7:4] Intra4x4PredMode[3:0]`, the final mode, 16 per I_NxN macroblock in block order |
| 14 | MVD | `mbPartIdx[23:22] subMbPartIdx[21:20] ref_idx[19:16] component[15] mvd[14:0]` (component 0 = x; mvd two's complement) |
| 20 | COEF_BLK | `category[19:16] block index[15:8] TotalCoeff[4:0]` |
| 21 | COEF | `scan position[21:16] level[15:0]`, nonzero levels only, level two's complement |
| 30 | PCM | three I_PCM samples, first in `[7:0]`; 128 per macroblock (256 luma, 64 Cb, 64 Cr) |
| 3F | MB_END | 0 |
| 40 | SLICE_END | 0 |
| 41 | PIC_END | 0 |

`kind`: 0 I_NxN (Intra_4x4), 1 Intra_8x8, 2 Intra_16x16, 3 I_PCM,
4 P_L0_16x16, 5 P_L0_L0_16x8, 6 P_L0_L0_8x16, 7 P_8x8, 8 P_8x8ref0,
9 P_Skip. COEF_BLK `category`: 0 Luma4x4, 1 Intra16x16DCLevel,
2 Intra16x16ACLevel, 3 ChromaDC Cb, 4 ChromaDC Cr, 5 ChromaAC Cb,
6 ChromaAC Cr. Its block index is luma4x4BlkIdx for luma and
chroma4x4BlkIdx for chroma AC (0 for the DC blocks). The scan position is
the index in the coded (zig-zag) order of the block's own list: for
Intra16x16ACLevel and ChromaAC it counts from 1, position 0 being the DC
that the DC block carries.

Per macroblock the order is: MB_START, MB_TYPE, MB_QP, then PCM words, or
the 16 I4_MODE tokens, or the MVD tokens (all list 0 partitions in order,
x then y for each, sub-partitions in order inside each sub-macroblock),
then the coefficient blocks in syntax order (Intra16x16 DC, the 16 luma
blocks in luma4x4BlkIdx order skipping uncoded 8x8 quadrants, chroma DC
Cb then Cr, chroma AC Cb blocks 0..3 then Cr 0..3, again only those
coded), then MB_END. A COEF_BLK is written for every block that is parsed,
even when TotalCoeff is 0. A P_Skip macroblock is MB_START, MB_TYPE, MB_QP,
MB_END. SLICE_END follows the last macroblock of a slice and PIC_END the
last slice of a picture.

### `h264_recon_NAME.txt`: reconstruction

Per macroblock, after the `mb` line (whose `qp` is QPY, 0 for I_PCM):

    pred y|u|v ROWS     the prediction: intra (as the intra process formed
                        it, block by block) or inter (after weighting)
    res y|u|v VALUES    the residual after the inverse transform, decimal,
                        raster order, 256 or 64 values (r_ij of 8.5.12,
                        before it is added to the prediction)
    pre y|u|v ROWS      the macroblock's samples before deblocking
    post y|u|v ROWS     after deblocking the whole picture
    bs v|h E0 E1 E2 E3  boundary strengths

I_PCM macroblocks have no `pred` or `res`. `pre` is `Clip1(pred + res)`.
`post` is the final picture, so it includes the filtering of this
macroblock's right and bottom edges by its neighbours.
For `bs`, `v` is the vertical edges (left to right, edge 0 the macroblock's
left edge) and `h` the horizontal ones (top to bottom); each edge is four
digits, the bS of its four 4-sample segments top to bottom (vertical) or
left to right (horizontal); `-` is an edge that is not filtered (picture
edge, slice edge with disable_deblocking_filter_idc 2, deblocking off, or
edges 1 and 3 with the 8x8 transform). Chroma edges use the bS of the luma
edge at twice their position, as 8.7.2 says.

### `h264_inter_NAME.txt`: motion vectors and prediction

Written when the stream has inter macroblocks. Per inter macroblock:

    mv l0 E x 16        per 4x4 block in raster order: REF:MVX:MVY (quarter
                        samples), or - when the block does not use the list
    refpic l0 I=P ...   ref_idx I refers to the picture dumped as INDEX P
    pred W x 96         the prediction as h264_inter sends it to h264_recon:
                        luma 16 rows of 4 words, Cb 8 rows of 2, Cr 8 rows
                        of 2; each word 8 hex digits, four samples,
                        little-endian (the leftmost sample in bits 7:0)

and after the macroblocks of each picture, the whole decoded (deblocked)
picture, which later pictures predict from:

    frame y|u|v ROWS

P_Skip macroblocks are included: their vector is the predicted one (8.4.1.1).
