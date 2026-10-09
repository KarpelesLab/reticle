# Xilinx UltraScale+: the ZCU104, and what is known about its frames

This is the account of the UltraScale+ backend, which so far is one
layer deep: [`src/fpga/uray.rs`](../src/fpga/uray.rs) reads a bitstream's
frames and gives every set bit to the tile that owns it. It places
nothing, routes nothing and writes no bitstream yet. Each claim below says
whether it was **measured** here or only **quoted** from a document or a
database.

## The board

A Xilinx ZCU104 evaluation kit, rev C (`xlnx,zynqmp-zcu104-revC` in its
device tree). It carries an **XCZU7EV-FFVC1156-2-E**, a Zynq UltraScale+
MPSoC:

- **Programmable logic:** 230 400 LUTs, 460 800 flip-flops, 1 728
  `DSP48E2` slices, 312 block RAMs (11 Mbit) and 96 UltraRAMs (27 Mbit).
  These figures are quoted from AMD's product table, not measured. The
  "504K" printed on the box is AMD's *system logic cells*, a marketing
  figure.
- **Processor system:** four Cortex-A53 cores and two Cortex-R5F cores
  under Linux. The board runs AMD's PYNQ 3.1 image, which is Ubuntu 22.04
  with kernel 6.6.10 (Xilinx 2024.1), and loads the logic through the
  kernel's FPGA manager (`Xilinx ZynqMP FPGA Manager`).
- **USB:** an FT4232H (`0403:6011`). Channel A is JTAG and channel B the
  processor's console; this mapping is quoted from the board's
  documentation and not yet checked.

## The database

[`mithro/prjuray-db-ultrascaleplus`](https://github.com/mithro/prjuray-db-ultrascaleplus),
CC0, generated on 2026-09-28 by Project U-Ray's die-independent flow from
Vivado 2025.2 WebPACK. Its readme lists the `xazu7ev` die as covering the
`xczu7ev`, and the board's own bitstream confirms the two share silicon:
both write IDCODE `0x04A5A093`, and the `xczu7cg` die directory's
`tilegrid.json` is byte-identical to `xazu7ev`'s.

The older [`f4pga/prjuray-db`](https://github.com/f4pga/prjuray-db) is not
used. It stopped in 2020 with one die, the ZU3EG, and has no DSP or
UltraRAM bits.

```sh
reticle fetch prjuray-db     # 168 files, 47 MB, pinned to 9e7d3e7
```

Only the database's **data** is read. Project U-Ray's tools are under a
different licence from Reticle's MIT, and nothing here was written from
their source.

## The oracle

The base overlay of the PYNQ 3.1 image: `base.bit`, built by Vivado
2024.1 on 2025-09-22 for `xczu7ev-ffvc1156-2-e`. It holds two MicroBlazes,
an HDMI transmitter and a pile of AXI peripherals. The image installs it
under `/usr/local/share/pynq-venv/lib/python3.10/site-packages/pynq/overlays/base/`,
and `tests/fpga_uray.rs` finds it through `RETICLE_ZCU104_REF`. It is
AMD's build output and is not committed.

## What was measured

### The container is the 7-series one

`xc7::read_bit` reads `base.bit` without change, and every CRC matches.
The sync word, the packet headers, the register numbers and the CRC-32C
over `{register[4:0], data}` are the same as UG470's. Command for command, the sequence is the one
Vivado writes for the Basys 3 (`artix7/harness/basys3/swbut/design.bit`
in Project X-Ray's database). There are three differences: an extra
`FAR` write of zero before `RBCRC`, different `COR0`, `COR1`, `MASK` and
`CTL0` values (`COR1` = `0x00400000`, `CTL0` = `0x101`), and a
400-`NOOP` tail. All 4 827 258 frame words go to `FDRI` in one type 2
packet after a `FAR` write of zero. **Measured.**

### A frame is 93 words, and the stream is 51 906 of them

4 827 258 / 93 = 51 906 exactly. **Measured.**

### The layout is the tile grid's, with two rules the grid does not state

The frame address is block type in bits 26:24, row in 23:18, column in
17:8 and minor in 7:0. **Quoted** from UG570 and **checked**: every `base`
in the ZU7EV `tilegrid.json` decodes to the block, row and column its own
entry states (`TileGrid::parse` refuses one that does not).

The grid gives 6 rows of each of the two block types. In rows 0 to 3, no
tile has bits in columns 0 to 84, where the processor system sits. Two
rules make the layout match the bitstream:

1. **A column with no tile in one row is streamed anyway**, at the width
   the same column has in the other rows.
2. **Each (block, row) ends in two pad frames**, as on 7-series.

With both rules the layout is 6 × 7 367 + 6 × 1 280 = 51 882 data frames
plus 12 × 2 pad frames: **51 906, exactly Vivado's count**, and all 24 pad
frames are zero. That frame count is one equation, but the ownership check
below is the real test: a column of the wrong width shifts every later
column, and the ownership check would then fail by hundreds of thousands
of bits. **Measured.**

### The ECC is word 45 and the low half of word 46, and is not understood

On 7-series the ECC is in word 50 of 101. Here, counting how often each
bit of a frame is set across the 51 906 frames:

| bits | frames with it set |
|---|---|
| word 45, all 32 bits | 8 400 – 8 750 each |
| word 46, bits 0–15 | 8 900 – 10 100 each |
| word 46, bits 16–31 | 6 – 443 each |
| word 47 | 9 – 459 each |
| a typical configuration bit | a few hundred at most |

Close to half the frames set each of 48 bits. That is the signature of a
check code over frame contents. A frame with nothing else set has all 48
clear (29 881 such frames). Word 46's high half and word 47 behave like
configuration, and the database gives them to the `RCLK` tiles, whose
window is the 96 bits from bit 1 440 (words 45 to 47).

Taking those 48 bits as ECC is what makes ownership complete. With them
left in, 50 983 set bits belong to no tile, all in word 45. With them left
out, none do. **Measured.**

What the 48 bits *compute* is open, and the path so far is worth keeping:

- **They are a function of the frame's data alone.** Of 21 259 distinct
  frame contents, no two identical ones carry different ECC.
- **They XOR.** Of the 47 frames with exactly two data bits set whose two
  one-bit frames also occur, all 47 have the XOR of those two ECC values.
- **Yet a GF(2) solve over all the frames is inconsistent.** Elimination
  over the 2 928 data bits reaches full rank and 17 918 frames disagree
  with the solution. Row 0 alone is consistent (rank 2 046). Every other
  row is inconsistent with itself.
- **The inconsistency is not the `RCLK` words.** Leaving word 46's high
  half, word 47 or both out of the data makes it slightly *worse*, not
  consistent.
- **The residual takes only 75 values over 17 915 frames**, and none
  follows the frame address.

So far that reads as a linear code with a small nonlinear or
data-dependent part. A guess at the cause is that Vivado computed some
frames' ECC before changing their data afterwards: the overlay's
MicroBlaze memories are typically filled in after implementation. That
would also explain why only some rows are affected. **This guess is not
checked.**

Whether the ECC *matters* for writing is a separate question, and it is
answerable on the board. A 7-series configuration engine ignores frame ECC
on write; if this one does too, a bitstream with zero ECC configures. That
has not been tried.

### Nearly every bit is a named feature

Of the 3 524 987 set bits outside the ECC, **3 514 160 are explained**: a
feature of an owning tile's `segbits` file has all its must-be-set bits
set and all its must-be-clear bits clear, or the bit is in that tile
type's `defaults` file. **10 827 (0.31 %) are not.** The largest groups
are:

| tile type | unexplained |
|---|---|
| `CLEM` (slice M) | 5 587 |
| `DSP` | 3 675 |
| `RCLK_INT_L` | 459 |
| `CMT_L` | 316 |
| `CLEL_R`, `BRAM` | 181, 177 |

A bit two tiles share is counted once, under whichever owner the decoder
meets first. The `DSP` count is such a bit set: in the Python measurement
the same 3 675 bits were counted under `INT_INTF_R`.

Almost all of the `CLEM` remainder is 19 bit positions repeated across
tiles. Slice M is where LUT RAM and shift registers live, and the base
overlay's MicroBlazes use both heavily. The database's own readme reports
about a hundred undocumented bits per held-out design on this die. The
oracle here is much larger than those designs, so the two counts do not
compare directly. **Measured**; `every_bit_of_vivados_zcu104_bitstream_has_an_owner`
pins the count.

## What remains, in order

1. **Load a bitstream through the board's FPGA manager.** First
   `base.bit` itself as a round trip, then the same bitstream with its ECC
   zeroed. This needs root on the board, and it replaces the logic PYNQ's
   base overlay is using.
2. **A routing graph for this die.** The database ships *bits*. Which wire
   of a tile joins which wire of its neighbour is in the 2020 f4pga
   snapshot's `tileconn.json`, but only for the ZU3EG. The ZU7EV is a
   different grid of the same tile types.
3. **Write a bitstream**, starting with one LUT driving one pin, and watch
   it on the board.
4. **The processor interface.** The fabric talks to Linux through the
   `PS8` block's AXI ports, whose bits are in `INT_INTF_LEFT_TERM_PSS` and
   `PSS_ALTO`.
