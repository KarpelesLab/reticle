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

Whether the ECC *matters* for writing is a separate question, and the
board answered it.

### The configuration engine does not check the ECC on write

**Measured on the ZCU104 on 2026-10-09.** `base.bit` was rewritten with
all 48 ECC bits of every one of its 51 906 frames cleared, and the CRC
after `FDRI` recomputed. The CRC is UG470's, and the rewriter was first
checked by reproducing both of Vivado's own CRCs and the board's
`base.bin` byte for byte. The rewritten file was loaded through the
kernel's FPGA manager, the same path PYNQ uses.

It configured: the manager reported success and the state `operating`.
The overlay worked: the LED `axi_gpio` at `0x8004_6000` held every value
written to it and read it back, over the processor's AXI port into the
fabric. To show the load really replaced the fabric's contents rather
than leaving the old ones running, the LED register was set to `0xF`
before each load. It read `0x0` afterwards, both for the zero-ECC file
and for the original reloaded after it.

So a bitstream Reticle writes for this part may leave the ECC zero. That
says nothing about what the ECC computes, which is still open above.

### What the kernel loads

PYNQ loads `/lib/firmware/base.bin`. That file is `base.bit` without its
header, starting at the dummy `0xFFFFFFFF` words before the sync word,
with **every 32-bit word byte-swapped**. Measured: the swapped tail of
`base.bit` is identical to the board's `base.bin`, all 19 311 092 bytes.

```sh
# as root on the board; flags 0 is a full (not partial) configuration
cp design.bin /lib/firmware/
echo 0 > /sys/class/fpga_manager/fpga0/flags
echo design.bin > /sys/class/fpga_manager/fpga0/firmware
```

Loading a design replaces the base overlay. Reload `base.bin` the same
way to leave the board as PYNQ expects it.

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

## A routing graph for the ZU7EV, from the ZU3EG's rules

The 2026 database ships *bits* only. Which wire of a tile is the same
metal as which wire of its neighbour is in the 2020 f4pga snapshot: its
`tileconn.json` and `tile_types/*.json`, CC0, for the ZU3EG alone. This
section is the measurement of how far those carry to the ZU7EV. The
measurement was first done with throwaway scripts over the whole die; the
result is now [`src/fpga/uray/fabric.rs`](../src/fpga/uray/fabric.rs),
and [the last subsection](#in-reticle) is what it reproduces.

### What carries over unchanged

- **The grid coordinates.** For the ZU3EG, which both databases describe,
  all 66 385 tiles have the same name and the same position in both.
- **The wire names.** The 2020 *tile types* use the same Vivado names as
  the 2026 `segbits` files: 3 624 of the 3 651 `INT` switches the new
  database documents are switches of the old `INT` type. (The 2020
  *segbits* files used different names and a multi-bit encoding; they are
  not used.)
- **The interior.** `tileconn.json` is rules: two tile types, a grid
  offset, and the wire pairs joined across it. Applied to the ZU7EV grid,
  86 % of its 18 720 `INT` tiles join exactly 1 578 wire ends, which is
  what 81 % of the ZU3EG's do.

### The check

Decode `base.bit` through the 2026 database. Every one-bit switch feature
whose bit is set is an active switch: 1.6 million of them, nearly all in
`INT`. Every `INT` switch in that database is one bit, so the decoder does
not have to guess. Build each switch's source and destination nodes from
the rules, then count three things:

- nodes driven by **two** active switches, which a wrong join produces;
- source nodes **nothing drives**, which are not a site pin or a constant;
- driven nodes **nothing reads**, which are not a site pin.

The 2026 database writes some features as two switches joined by `&`,
because they share a bit the generator could not separate, and a
bidirectional switch as `<->`. Both kinds are left out of the active set,
because their bit does not say which switch is on.

### Three steps, each measured

| | double-driven | undriven | unread |
|---|---|---|---|
| ZU3EG rules as they are | 1 205 | 76 862 | 103 313 |
| + five type aliases | 1 205 | 55 174 | 80 753 |
| + composition through break tiles | 1 203 | 16 106 | 36 999 |

**The aliases.** The ZU7EV has 133 tile types the ZU3EG lacks. Four are
the same tile under a name ending in `_FT` (`AMS_M12BUF_BOT_L_FT` is the
ZU3EG's `AMS_M12BUF_BOT_L`), and `INT_TERM_P` behaves as `INT_TERM_B`.
Each alias was kept only if the oracle said failures fell and
double-drives did not rise. They were tried one at a time against
name-derived candidates, and only these five passed.

**The composition.** On the ZU3EG a slice-M column is never directly east
of a slice-L column: a block RAM, DSP or clock-region break column always
sits between. On the ZU7EV the two are often adjacent
(`INT_X49 | CLEL_R_X49 | CLEM_X50`), and no ZU3EG rule joins them. The
east–west bus wires pass *through* the break tiles (`CFRM_CBRK_L`,
`CFG_M12BUF`) on the ZU3EG, whose wire names carry the row offset within
the tall tile (`EASTBUSIN_FT0_1_3` is row 1, wire 3). Composing the rules
through those tiles gives direct wire pairs. Applying them between the
next non-blank, non-break tiles in a row removed 39 000 undriven nodes
and created no double-drive.

### Where it still fails

Restricted to nodes made only of interconnect, slice and break tiles —
where a first design lives — **4 069 nodes are undriven and 5 718 unread,
out of 1.33 million**. They sit almost entirely in five interconnect
columns:

| `INT` column | failures |
|---|---|
| X39, X40, X41 | 2 630, 3 027, 382 |
| X68, X69 | 286, 3 241 |
| X0, X1, X2 | 112, 34, 13 |
| every other column | 74 between them |

X39–X41 is the column of the configuration block in the middle of the
die; X68–X69 is the right edge, beside the transceivers; X0–X2 is beside
the processor system. The other sixty-odd interconnect columns, across
every row, agree with Vivado's routing of a 3.5-million-bit design. A
first backend can work in a region that leaves those columns out, as the
7-series one loads a region.

The rest of the failures are outside the core and are understood: block
RAM pins the 2020 tile types name differently, interface tiles
(`INT_INTF_*`) whose outputs are fed through `&` features, and the PCIe
and transceiver interfaces.

### In Reticle

`uray::build_arch` builds an `Arch` for a region of the grid from the
2026 tile grid and bits, the 2020 tile types and the ZU3EG's
`tileconn.json` (`reticle fetch prjuray-db prjuray-db-2020`):

- one tile type per ZU7EV type, its wires being its 2020 type's (through
  the aliases);
- a pip for every routing feature of its `segbits` file;
- two bitless pips, one each way, for every wire pair of every rule.

The composed joins depend on the instance, because whether a `CLEM` is the
next real tile east of a `CLEL_R` depends on where the two sit. So they go
on a *variant* of the western type, such as `CLEL_R+CLEM@3`, and only
where the ZU3EG has no direct rule for that pair at that offset. Declared
on the type instead, the same joins would misfire 1 216 times on this die,
for example joining `INT_INTF_R` to a `CLEM` across the `DSP` between
them.

`tests/fpga_uray_routing.rs` runs the check above over reticle's own
graph for the block between `INT_X5Y290` and `INT_X30Y250`. That block
has 3 257 tiles, 3.9 million pips, 20 million join pips and six variants.
A node the region's edge cuts is left out, exactly. Of the region's pips,
**Vivado set 255 241, and every core node they touch is driven exactly
once and read**. Core nodes are those made only of interconnect, slice
and break tiles. The block RAM and interface nodes are counted apart and
pinned (167 driven twice, 652 undriven, 2 560 unread): they reach the
interconnect through `&` features, which name two pips that share one bit
and cannot be split.

One mistake on the way is worth keeping. The composition walks the rules
from each first step through break tiles, with a budget of 200 states. In
the first Rust version, one budget was shared by all of a wire's first
steps. A bus wire has dozens of them, one per break tile row offset, so
the budget ran out before the bus was reached, and the region showed
16 431 undriven nodes. The scripts had given each first step its own
budget. With that restored, the count fell to 652, all of them outside
the core.

### The processor's side

The processor system sits at the bottom left of the die. Its one `PS8`
site, in the one `PSS_ALTO` tile, reaches the fabric through a column of
`INT_INTF_LEFT_TERM_PSS` interface tiles beside interconnect column X27.
The ZU3EG's rules join every `PS8` pin to an interface tile by a long
offset (159 columns, up to 185 rows). On the ZU7EV the processor block is
anchored at a different distance from the bottom of its column, and the
column is taller, so whether those offsets carry over was not obvious.

**They do, unchanged. Checked against Vivado's choice of pins.** Take
every interface output Vivado's routing reads, map it back to a `PS8` pin
through the ZU3EG offset shifted by *s* rows, and see which pins they
are. At *s* = 0 they are the overlay's own ports: `AXI_PL_PORT2` (the LPD
AXI master, `M_AXI_HPM0_LPD`: 128 `WDATA`, 40 `AWADDR`, 40 `ARADDR` …),
five `FMIO_GPIO_OUT` (EMIO GPIO) and the `AXDS2`/`AXDS6` read data of the
video pipeline's memory reads. No other shift comes close. A count that
ignores pin names cannot find this: nearly every (row, wire) of the
column is some `PS8` pin, so any shift explains most of the used wires.

**The processor's inputs need fixed wiring the 2026 database does not
name.** An interface tile's `IMUX_FT1_*` reaches the `PS8` input, and the
2020 type joins it to the interconnect through `IMUX_FT0_* ->> IMUX_FT1_*`.
That is a buffered pip with no bits, absent from the 2026 `segbits`.
`build_arch` therefore declares a 2020 buffered pip as bitless wiring when
its tile type has no sites and no feature drives its destination. A
compound `&` feature counts as driving. Both conditions were added
because a looser rule broke something:

- Without the "no sites" condition, a block RAM's internal buffered pips
  became fixed wiring and gave one output wire several drivers. Block RAM
  double-drives rose from 167 to 260.
- Without counting `&` features, `INT_INTF_L`'s `LOGIC_OUTS_R*`, which
  are reached only through such features, would get a bitless path that
  skips their gate.

`vivados_routing_is_consistent_beside_the_processor` runs the check over
the `PS8` tile, the interface column and interconnect columns X27–X31,
over the four clock region rows the processor spans. All 163 313 pips
Vivado set there fit, with every core node driven once and read.

## The first design on the part: an EMIO loopback

**Run on the ZCU104 on 2026-10-09, and it worked.**

`emio_loopback` in `tests/fpga_uray_board.rs` builds the
processor-side fabric and searches it for a path from `FMIO_GPIO_OUT0`
(EMIO GPIO output 0) to `FMIO_GPIO_IN1` (input 1). The search may not
enter any other `PS8` pin's node, so no stray signal can reach a
processor input such as an AXI handshake. It writes the result with
`uray::write_bit`.

The writer is Vivado's command sequence word for word. Given Vivado's own
header and frames, it reproduces `base.bit` byte for byte,
and `uray::bin_from_bit` of that reproduces the board's `base.bin`
(`the_writer_reproduces_vivados_bitstream_byte_for_byte`).

The route is three pips:

```text
(160,159) INT_INTF_LEFT_TERM_PSS  LOGIC_OUTS_L18 -> LOGIC_OUTS_R18   bits 01_021 02_027 03_024
(161,159) INT                     LOGIC_OUTS_W18 -> INT_NODE_IMUX_47_INT_OUT1   bit 17_026
(161,159) INT                     INT_NODE_IMUX_47_INT_OUT1 -> IMUX_W31        bit 11_022
```

The bitstream sets 460 805 bits: those 5, and the 16 `defaults` bits of
every slice tile. The defaults are how Vivado leaves an unused slice:
most of `base.bit`'s slices carry all 16, and the rest are slices in
use. The `HDIO_TOP_RIGHT` defaults are left clear, as Vivado leaves them
on unused I/O tiles. **Every set bit decodes to a feature.** The decoded
pips are the route's plus 29 others: the interface tile's three enable
bits are shared by all 30 of its `LOGIC_OUTS` passes, so turning one on
turns on all of them. The other 29 carry processor outputs onto
interconnect wires that nothing reads.

On the board, through the FPGA manager, with Linux driving `gpio594`
(EMIO 0, as an output) and reading `gpio595` (EMIO 1, as an input):

| output | input, base overlay | input, loopback |
|---|---|---|
| 0, 1, 0, 1, 1, 0 | 0, 0, 0, 0, 0, 0 | 0, 1, 0, 1, 1, 0 |

The output and the input are different lines on purpose. Linux reads an
output line's own output register back, so a loopback onto the same line
reads the same whether it works or not. That was measured first, and it
is why the route ends at input 1.

### What it did besides, and what that says

- **IRQ 54 fired once.** That is the fabric's first interrupt line, GIC
  SPI 121, which PYNQ's `uio_pdrv_genirq` owns. Undriven, it read as
  asserted. The UIO handler masks the line, so it fired once and stopped.
- **The kernel logged `usb usb1-port1: over-current condition`** and the
  same for `usb2-port1`, at the moment of the load. Nothing was plugged
  into either port, and nothing was logged after the base overlay was
  reloaded. A processor input that Vivado's bitstream holds at a safe
  level was left undriven, and read as asserted.

Both say the same thing: **a bitstream for this part must tie the
processor's unused inputs**, as Vivado's evidently does.

## Tying the processor's inputs

### What Vivado does

Every `PS8` input's node was traced back through the pips Vivado turned
on in `base.bit`. 2 958 inputs end at a lookup table of contents zero in
a slice beside the interface column. That includes both USB controllers'
`EMIO_HUB_PORT_OVERCRNT_*`, every fabric interrupt line not in use, and
every unused AXI port's `AWVALID`, `ARVALID`, `WVALID` …. About 1 200 more
are driven by the overlay's own logic. **585 are driven by nothing.** No
input is tied to one.

The 585 are test and scan pins (`BSCAN_*`, `TEST_*`, `FMIO_TEST_*`,
`IDCODE*`), the PHY's analogue test interface (`I_AFE_*`),
characterisation inputs (`FMIO_CHAR_*`, `IO_CHAR_*`), and a handful of
active-low functional inputs: `NIRQ0_LPD_RPU`, `NFIQ*`,
`FMIO_SPI*_SS_IN_B`, `FMIO_SD*_SDIF_WP`. For those, reading 1 — which is
what an undriven input reads — *is* idle. Vivado relies on that, and the
interrupt and over-current effects above are the same reading on inputs
where 1 means "asserted".

The list is `src/fpga/uray/ps8_undriven.txt`.
`the_undriven_processor_inputs_are_the_ones_vivado_leaves` re-derives it
from Vivado's routing on Reticle's own fabric: an input is undriven when
no pip whose bits are set drives its node. The result is exactly the 585.

### A lookup table's two missing bits

Tying to zero needs a lookup table of contents zero, so it needs all 64
of a table's `INIT` bits. The database names 62 of them. `INIT[31]` and
`INIT[63]` are missing from every table, and the 16 `defaults` bits of a
slice are two per table.

**They are the same bits, stored like the others.** The 62 named bits
sit on a grid, `INIT[i]` at frame `f0 + 3 − i mod 4`, bit
`c0 + 15 − ⌊i/4⌋`. Continued, that grid puts `INIT[63]` and `INIT[31]`
exactly on two of the defaults, for every table. Read through the full
layout, the slice where Vivado ties two inputs low (`CLEL_R_X27Y180`)
holds two tables reading 0, the two whose outputs reach the interface
column, and six reading `0x8000_0000_8000_0000`. That is what Vivado
leaves in a table nothing uses, and why the generator filed those two
bits as defaults. Had the bits been stored inverted, the two zero tables
would read `0x8000…` and the others zero.
`vivados_constant_zero_tables_read_zero_through_the_full_layout` pins
this, and `uray::slice::lut_init_bits` fills the two bits in from the
grid, refusing a database whose named bits are off it.

### The loopback, tied, and with nothing besides

`uray::processor::route_constant` grows one tree from the lookup tables
of the slices beside the interface column to every input it must drive.
Reusing the tree costs nothing, and the tree never enters a node that
already has a driver or an input that must stay undriven. For the
loopback it ties **4 153 inputs** with 7 201 pips from 313 tables, with
none unreached. Every other table on the die gets Vivado's unused
contents. The bitstream's 467 380 set bits all decode. Every table reads
back what it was given, and the only pips on besides the chosen ones are
the interface tile's 29 siblings.

**On the board, 2026-10-09**: the loopback worked as before (EMIO 1
follows EMIO 0 through `0, 1, 0, 1, 1, 0`), and this time **nothing else
happened**. IRQ 54 stayed at its count of 1, both ports' over-current
counters stayed at 2, and the kernel logged nothing but the load. The
base overlay was reloaded afterwards and answered on AXI.

## A flip-flop, clocked by hand

**Run on the ZCU104 on 2026-10-09, and it behaved as an `FDRE` in every
step.**

The database names a slice's flip-flop features after Vivado's
attributes (`CE_ACTIVE=TRUE`, `SR_ACTIVE=TRUE`, `SYNC_ATTR=SYNC`,
`FFINIT=INIT0`, `FFSR=SRLOW`). It also files some single bits under
several of those names at once: `AFF.CE_ACTIVE=TRUE` and
`AFF.CE_GND=TRUE` are both bit `14_007`, which `BFF` names too. That bit
is shared by a group of flip-flops, not owned by one. Which bits an
`FDRE` needs is a question the names alone do not settle. Vivado's
base overlay offers a hypothesis: 350 of its `AFF`s whose data comes from
the bypass carry exactly the bypass select (`FFMUXA1.SP.BYP.OUT1`) and
those five features.

`one_flip_flop` in `tests/fpga_uray_board.rs` tests it on the part.
`AFF` of `CLEL_R_X27Y206` takes its data from `AX`. Its `CLK1`, `CKEN1`
and `SRST1` are each routed from an EMIO output through the interconnect:
the slice's clock pin is fed by an `INT_NODE_GLOBAL` node, which ordinary
routing can drive. Linux can therefore clock it by hand, and needs no
clock buffer for the experiment. `AQ` goes to EMIO input 1.

| step | expected | Q |
|---|---|---|
| after the load, before any edge | 0 (`INIT0`) | 0 |
| D=1, CE=1, no edge | 0 | 0 |
| edge, D=1, CE=1 | 1 | 1 |
| edge, D=0, CE=1 | 0 | 0 |
| edge, D=1, CE=0 | 0 (hold) | 0 |
| edge, D=1, CE=1 | 1 | 1 |
| SR=1, no edge | 1 (synchronous) | 1 |
| edge, SR=1, D=1 | 0 (`SRLOW`) | 0 |
| edge, SR=0, D=1 | 1 | 1 |

Nothing else happened: no interrupt, no over-current. That is one
flip-flop of sixteen, with its data from the bypass. The second
flip-flops (`AFF2`), the other letters, the lookup table path
(`FFMUXA1.SP.D6.OUT1`) and which flip-flops share a clock enable are
still readings of the database, not measurements.

## What remains, in order

1. **Bels.** The slice's lookup tables, flip-flops and carry chain as
   bels the placer can use, from the 2026 `segbits` names and the 2020
   site types. The lookup tables' contents are complete and one flip-flop
   has run (above). Which flip-flops share each clock enable and reset
   is the next thing to measure.
2. **A clock.** The processor's `PL_CLK0`, through its `BUFG_PS`, onto
   the global network and down to the slices.
3. **The processor interface.** The fabric talks to Linux through the
   `PS8` block's AXI ports, whose bits are in `INT_INTF_LEFT_TERM_PSS` and
   `PSS_ALTO`.
