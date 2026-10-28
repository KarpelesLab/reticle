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

**And one column the check did not see: the first UltraRAM.** The region
`reticle fpga` places on first reached interconnect column 38, and the
first design large enough to use the far end of it, a binarized network
of 12 800 lookup tables, failed to route after ten minutes with "no path
exists". In the routing graph of that region (measured 2026-10-10), no
wire crosses the UltraRAM column between interconnect columns 36 and 37
except 856 clock wires, and the 3 840 lookup tables of the slices beyond
it cannot send a signal back west. The ZU3EG has no UltraRAM (quoted from
its data sheet, not checked), so its rules have no joins across one.
Why the check above did not report it is not established: it judges
nodes by Vivado's routing, so it can only see a cut where the base
overlay routes across one. The
region therefore stops at column 36, 26 880 lookup tables, and
`every_lookup_table_of_the_widest_region_reaches_the_middle` in
`tests/fpga_uray_routing.rs` asks the question that does see a cut:
whether every table's output reaches one in the middle, and every
table's input can be reached from it.

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

## The processor's clocks, found by counting them

### What the databases do not say

The processor's clocks enter the fabric through `BUFG_PS` buffers. On
the ZU3EG they sit in `RCLK_INTF_LEFT_TERM_ALTO` tiles. On the ZU7EV the
tiles in that place are `RCLK_RCLK_INTF_LEFT_TERM_DA6_FT`, a type the
2020 snapshot does not describe and whose 2026 `segbits` file is empty.
The ZU3EG type's own features are no better: its input-select features
give the same two bits to unrelated pairs of input and buffer (`07_069
08_071` appears for at least four). Read through those features, Vivado's
bits in the ZU7EV tile explain one thing,
`CLK_BUFG_PS_14_CLK_OUT->CLK_HROUTE14`, and leave the rest unexplained.

### The experiment

`three_clock_counters` in `tests/fpga_uray_board.rs` borrows the clock
network instead of building it. Every bit Vivado set in the fabric's
clock rows (the `RCLK_*` types) and in the processor's clock buffer
tiles is copied verbatim, 7 480 bits in 362 tiles. The I/O banks' clock
rows (`RCLK_HDIO`, `RCLK_HPIO`, `RCLK_XIPHY` …) are left out: they share
frames with the I/O tiles, and the first attempt set 115 bits inside
`HDIO_BOT_RIGHT` and `XIPHY_BYTE_L`, which drive the board's pins.

Each of three 32-bit ripple counters is clocked by a leaf clock that
Vivado switched on beside the processor:

- **counter 0**: track 14 of `RCLK_INT_L_X29Y89`, where Vivado's buffer
  sends `PL_CLK0`, judging by the one feature that explains;
- **counter 1**: track 11, the other track in use in every clock region
  there;
- **counter 2**: track 22.

A leaf reaches the interconnect through `GCLK_B` wires, and only in one
half of its clock region's column. Leaf 10 of that tile reaches
`INT_X29` rows 248–279 of the grid. From there the path to a slice's
`CLK1` is `GCLK_B → INT_NODE_GLOBAL → CTRL`, all documented. Each stage
is flip-flop `A` of its own slice, with its data on the bypass from
lookup table `A` (contents `0x5555…`, an inverter of `AQ`). It carries no
clock-enable or set/reset feature. Linux reads each counter as one
32-bit EMIO bank from the GPIO block's read-only data registers
(`0xFF0A006C`, `…70`, `…74`), twice, a measured interval apart.

### What it counted

**On the ZCU104, 2026-10-09**, in three runs of a quarter of a second:

| counter | track | rate |
|---|---|---|
| 0 | 14 | **100.001 MHz** |
| 1 | 11 | **300.00 MHz** |
| 2 | 22 | 0 |

Track 14 is `PL_CLK0`, which the processor configuration sets to
99.999 MHz. Track 11 is another processor clock, at 300 MHz. Track 22 is
fed by a clock manager, which was not replayed, so it does not run.
Nothing else happened: no interrupt, no over-current.

The same run established, on the part:

- a flip-flop with **no** clock-enable or set/reset feature has its
  enable on and its reset off;
- a lookup table's output, routed to the flip-flop's bypass, works;
- a 32-stage ripple counter, every hop through Reticle's own routing,
  runs at 300 MHz;
- the leaf-to-slice clock path above is right.

What it does not establish is the clock network itself. That is
Vivado's, copied. Building it, rather than borrowing it, is the job of
an experiment that varies one buffer's bits at a time.

## From Verilog to the part

**Run on the ZCU104 on 2026-10-09: a 32-bit counter written in Verilog,
taken through Reticle's whole flow, counted at 100.000 MHz.**

```verilog
module top(input wire clk, output reg [31:0] count);
  always @(posedge clk) count <= count + 32'd1;
endmodule
```

with `clk` bound to `PL_CLK0` and `count[k]` to `EMIO_I<k>`
(`examples/zcu104/counter.v` and `.rcf`). From the command line:

```sh
reticle fetch prjuray-db prjuray-db-2020
reticle fpga examples/zcu104/counter.v --device xczu7ev-ffvc1156 \
    --constraints examples/zcu104/counter.rcf \
    --bitstream counter.bit --borrow-clock base.bit
```

`--borrow-clock` names the PYNQ base overlay's own `base.bit`, whose
clock network carries `PL_CLK0` (below). The command writes `counter.bit`
and `counter.bin`. The `.bin` is what the FPGA manager loads, and this
one is byte-identical to the file that counted on the board.
`verilog_counter` in `tests/fpga_uray_board.rs` runs the same flow
through the library. Elaboration, synthesis,
technology mapping, the generic placer and the generic router are the
ones every other family uses. What the UltraScale+ backend adds is this.

**A device**, `devices/uray.dev`, declaring only what has run: `LUT1`
to `LUT6`, `FDRE` in its four modes, and an input and an output buffer.
Its pins are the processor's: `EMIO_O<k>` (design inputs), `EMIO_I<k>`
(design outputs) and `PL_CLK0`. There is no global buffer, so no `BUFG`
is inserted. No carry chain, DSP, block RAM or other flip-flop
primitive is declared, so a design that needs one is refused at mapping.

**Bels**, from `uray::slice::slice_model`, for every slice tile:

- eight `lut` bels, with `INIT` through the full 64-bit layout;
- sixteen `ff` bels, each taking its data from its bypass pin, so a
  lookup table's output reaches it through the fabric.

A flip-flop's enable and reset are its control group's wire, and a
pass-through pip from the slice's `CKEN`/`SRST` pin carries the group's
"used" bit. The bit is set exactly when the router routes the group's
enable or reset; left unrouted, the flip-flop runs enabled and unreset,
as measured. The database's shared bits fix which flip-flops form a
group, and `slice_model` refuses a database where they disagree.

**The processor's endpoints**: `io` bels in the `PSS_ALTO` tile on the
`FMIO_GPIO_OUT`/`IN` wires, and `PL_CLK0` on a global wire. A pip from
that wire to every leaf of every `RCLK_INT_*` tile carries the leaf's two
documented features for track 14. The router therefore turns on exactly
the leaves the design uses.

**The rest of the clock network is borrowed.** The distribution rows and
the processor's clock buffers are replayed from Vivado's bitstream, 719
bits in 150 tiles. The leaf tiles are not replayed: their leaves are the
design's. The router is kept off every node the replayed bits drive, and
off every processor input the design does not own.

Then the board test's finishing pass ties the processor's other inputs,
4 122 of them. It gives every unused table Vivado's unused contents, and
holds the result to the checks: all 470 937 set bits outside the replayed
tiles decode to a feature. The only pips on besides the chosen ones are
two that a chosen leaf pip is made of.

The placer filled flip-flops of all four control groups and both halves
of the slice. If the pin-to-group reading in `uray::slice::control_pins`
were wrong anywhere it was used, some bits of the count would be stuck
or wrong. They were not. That is evidence for the clock pins. The enable
and reset pins of each group are still unexercised, because the counter
uses neither.

## Routing a large design

The counter routes in seconds. The first large design, a binarized
network for MNIST, did not route at all, and finding out why took three
measurements, each of which overturned the guess before it. All three
were taken on 2026-10-10 to 10-14, with a timed copy of the flow
(`zcu104::implement` split into its steps).

**One routing pass took 21 minutes.** The network was 12 800 lookup
tables and 9 416 signals, and the router's 40 passes would have taken
fourteen hours. Its worst nodes were clock row buffers
(`CLK_BUFCE_ROW_FSR_*`) carrying ten signals each: nothing closed the
clock network's own wires to data, only the nodes Vivado's replayed bits
drive. `Board::closed_nodes` now closes every wire named `CLK_…` but a
leaf's. That was right, and it changed nothing measurable about the time
(1 424 seconds a pass afterwards): the counter's bitstream came out
byte-identical, and the cost was elsewhere.

**Every piece of a wire cost the router one.** The fabric declares each
tile's piece of a wire as its own node and joins neighbours with a
bit-less pip each way (`fabric::build_arch`). Two thirds of the region's
six million nodes are such pieces, and a twelve-column wire is a median
of 22 of them. At one apiece a long wire looked twenty times as dear as
a short one, and the search walked along every piece.
`Board::node_bases` makes a node only bit-less pips enter free, so a
wire costs one where its driver's pip reaches it. One pass then took 125
seconds. Welding the pieces into one node, as the 7-series backend does,
is the thorough repair; this is the cheap one, and it leaves the graph
as it was checked.

**And the design was too dense for the router to finish.** With both
fixes, 3 924 tables routed in 12 passes, and 6 989 (a network of 16
neurons) in 34 passes and 44 minutes. (That was before the router judged
contention by physical node; built through `reticle fpga` afterwards,
with the processor tied first, the same network took 24 minutes 48
seconds end to end.) The 12 800 did not converge in 31
hours. Its placement filled 1 509 of the 1 757 slices it used with all
eight tables, in one block against the processor, where every input
and output is; each interconnect tile then has every one of its tables'
inputs to feed, and the router cannot swap a table's inputs to ease
that. Spreading the same design to six or four tables a slice made it
worse while a wire still cost one a piece, which was what pointed at
the costs; with free pieces it is being measured again.

**It routes now, in ten passes and ten minutes.** Built through
`reticle fpga --timing` on 2026-10-20, the same 12 800-table network (32
neurons, below) converged at the tenth pass: 33 590 nodes shared after
the first, 995 after the sixth, none after the tenth, 623 seconds into
the run; 1 012 938 pips, 1 271 637 set bits, every one decoded. The
build of 2026-10-15, from a binary of that day, had run five days
without reporting anything and was stopped.

What changed between them is the placement, measured with the same
probe at both commits: the placer of 2026-10-15 filled **1 509** of its
used slices with all eight tables in rows 171 to 328, the figure above,
and the placer of 2026-10-20 (which since spreads a kind that fills a
third of its sites before legalising it) fills **406**, over rows 129 to
370. The router changed too in between — a net of more than 64 sinks
now starts each search from the part of its tree near the sink — so the
spread placement is the difference that was measured, and the reason
given above for the failure is borne out, but which of the two changes
did the work was not isolated.

The settings the flow uses are the ones that converged: free pieces,
`astar_weight` 2.0 (`zcu104::ASTAR_WEIGHT`), and rerouting only the
signals that share a node after the first pass
(`RouteOptions::reroute_congested_only`). A weight of 2.0 makes the
search greedy: a first pass of the 12 800 took 90 seconds at 2.0 and 125
at the default 0.3. The 3 924-table design converged with these settings
in 12 passes and 12 minutes using 477 000 pips (bit-less ones counted),
and with the router's defaults — no free pieces, weight 0.3, every
signal rerouted — in 14 passes and 47 minutes using 314 000. So the
settings buy time with longer routes; they are a speed setting, not a
measured optimum.

## A binarized network on the part

The first design with a purpose: a binarized two-layer network for
14×14 MNIST, 196 binary pixels, 16 hidden neurons and 10 classes,
trained off the board and folded to integers (a hidden neuron fires when
its pixels agree with its weights at least a threshold many times; a
class's score is its hidden agreements plus a bias). The model scores
71.07 % on the test set. Each popcount is written as a tree of six-input
sums marked `(* keep *)`, so that every bit of every sum is one lookup
table: 6 989 of them, through the open flow from Verilog, `--east 36`.

Linux drives it over EMIO with `/dev/mem`: four 64-bit writes of the
image, each latched on a toggle of `wr`, then a toggle of `go`; the
fabric answers with ten 8-bit scores and the class on EMIO inputs, and
`ack` follows `go` six clocks later. The pipeline is four registers
deep (hidden layer, scores, the argmax's two halves, the outputs).

**Measured, 2026-10-14, at `PL_CLK0` = 100 MHz:** all 10 000 test
images gave scores and a class identical, bit for bit, to the model's;
accuracy 71.07 %, as trained. A round trip through EMIO took 107.1 µs
per image, and the reply was always there at the first read (no
polls): the time is Python and the bus, and the fabric's few tens of
nanoseconds do not show in it.

**The clock, swept.** `PL0_REF_CTRL` (`0xFF5E00C0`, source IOPLL at
1 500 MHz, divisors in bits 13:8 and 21:16) was rewritten between
runs of 2 000 images, and put back to 100 MHz after:

| Clock | Bit-exact |
|---|---|
| 100, 150, 187.5, 250 MHz | 2 000 / 2 000 |
| 300 MHz | 490 / 2 000 |
| 375 MHz | 2 000 / 2 000 |
| 500 MHz | 1 695 / 2 000 |

This is not an fmax and must not be read as one. The image is still for
microseconds before `go`, so every stage of logic that reads it has
settled whatever the clock; what the sweep exercises is the
synchronisers, the six-clock `ack` and the last stages. At 300 MHz every
failing image had scores off by one, as if one hidden bit read wrong;
that 375 passes where 300 fails is not explained (the odd divisor, 5,
is one guess, its duty cycle another — neither was measured).

What the harness cannot measure is the network's own latency, so a
second build measures it. It loads the image into a register on `go`,
captures the scores and the class one clock later, and has nothing but
lookup tables between (38 deep): the highest clock at which the answer
still agrees with the model bounds the time from one register to the
other. Built 2026-10-14 (6 992 tables, 1 010 701 set bits, every one
decoded), it needed more than the router's 40 passes — one node was still
shared after 40, at 128 minutes — and converged within 100
(`zcu104::MAX_ITERATIONS`), in 154 minutes. Measured on the part, 2 000
images a step:

| Clock | Period | Bit-exact |
|---|---|---|
| 100 MHz | 10 ns | 0 |
| 50 MHz | 20 ns | 563 |
| 33.3 MHz | 30 ns | 901 |
| 31.25 MHz | 32 ns | 1 572 |
| 30 MHz | 33.3 ns | 1 800 |
| 28.85 MHz | 34.7 ns | 1 931 |
| 27.78 MHz | 36 ns | 1 993 |
| 26.79 MHz | 37.3 ns | 2 000, and 10 000 / 10 000 |
| 25.86 MHz down to 1 MHz | | 2 000 |

**So the network answers within 37.3 ns of the image being registered**,
and most images within 32. The failures thin out with the period rather
than stopping at one, as they should: which path is slowest depends on
the image and on the one before it. That makes the figure a
measurement of these 10 000 transitions, not a bound on every image,
and it includes the register's clock-to-output, its setup and the skew
between the two registers. Unpipelined, that is some 27 million images
a second if they could be fed that fast; through EMIO from Python they
arrive one per 107 µs, four thousand times slower than the fabric.

Below 1 MHz it fails again (12 / 2 000 at 0.38 MHz), and that one is the
harness: it toggles `wr` and changes the word without waiting, faster
than a clock that slow samples them.

Two things the build showed in passing. `--place-effort` 12 gave a
bitstream byte-identical to the default 10: the placer clamps its moves
per temperature at a million (`moves_for`), and at 7 689 instances any
effort above 7 reaches it. And neither build was given a timing goal;
the router knew nothing of delay, so 37 ns is what the placement
happened to give, not what the part can do.

**Thirty-two neurons.** The same network with 32 hidden neurons scores
73.84 % on the test set: 13 457 instances, built as above. **Measured,
2026-10-20, at 100 MHz over EMIO:** all 10 000 test images gave the
model's scores and class bit for bit, accuracy 73.84 %, at 107.3 µs an
image — the time is the bus, as before.

## Timing-driven routing

`reticle fpga --timing-driven` gives the router a delay to weigh against
congestion (`route::Timing`). After each pass a static timing analysis
over the routed netlist gives every connection a criticality, one on the
longest path and falling with its slack. The next pass charges a
connection `criticality × delay + (1 − criticality) × congestion` for
each node it enters, routes a signal's most critical sinks first, and
reroutes any signal with a connection at 0.9 or more even when nothing
contends with it. That is VPR's timing-driven router on PathFinder,
written from the published description.

**The delays are a model, not a measurement.** Reticle reads no vendor
timing for this die, so `Board::node_delays` charges 10 for each buffered
wire (a node a configurable pip enters), 1 for each further tile-piece of
it, and 5 for a lookup table: the one assumption is that a path is slow
by its hops more than by its length. Whether that is good enough was
settled on the part rather than argued.

**Measured, 2026-10-15.** The same combinational network as above, from
the same placement (the placer is deterministic and was not changed), so
any difference is the routing. Both bitstreams were loaded the same
afternoon and swept with the same harness; the untimed one reproduced
its earlier figures to within 2 %.

| `PL_CLK0` | Period | Untimed routing | Timing-driven |
|---|---|---|---|
| 41.67 MHz | 24 ns | | 822 / 2 000 |
| 37.5 MHz | 26.7 ns | | 1 426 / 2 000 |
| 33.3 MHz | 30 ns | 879 / 2 000 | 1 960 / 2 000 |
| 31.25 MHz | 32 ns | 1 555 / 2 000 | 1 995 / 2 000 |
| 30 MHz | 33.3 ns | 1 803 / 2 000 | 2 000, and 10 000 / 10 000 |
| 27.78 MHz | 36 ns | 1 995 / 2 000 | 2 000 / 2 000 |
| 26.79 MHz | 37.3 ns | 2 000 (earlier) | 2 000 / 2 000 |

**The network answers within 33.3 ns rather than 37.3, about 11 %
faster, and most images within 30 rather than 32.** The routing took
1 129 485 pips against 1 068 663 (6 % more wire) and 56 passes against
the untimed build's convergence, 6 hours 53 minutes against 2 hours 34;
every set bit decoded.

What the model reported is a warning about the model. Its longest path
was 9 329 units after the first pass and **10 079** after the last, so
by its own account the routing got worse while the part got faster. The
first pass is not the untimed build (it is routed under the same timing
options with every criticality still zero), so the two numbers do not
compare the two builds either; but they do say the model's arithmetic
is not the part's, and most likely that charging every tile-piece of a
long wire over-counts the wires that are fastest per tile. The
criticalities it hands out evidently point at the right connections
more often than not. Calibrating the three numbers against the part —
a ring oscillator, or this sweep on a design built to isolate one wire
class — is what would turn the model into a measurement.

## Reaching the fabric over AXI

EMIO is a bit-wide path through Linux's GPIO registers, and at 107 µs an
image the network's tens of nanoseconds do not show. The processor's
AXI master ports are the wide path. Of the three, the base overlay uses
`M_AXI_HPM0_LPD`, which Linux reaches at `0x8000_0000`: the overlay's
LEDs are there, and the PYNQ image enables and configures the port.
Reticle drives it from the fabric side only, and changes nothing on the
processor's.

**Which pins.** The port is the `PS8`'s `AXI_PL_PORT2_*`, 514 pins, which
the processor-side check above had already matched to the overlay's own
AXI traffic. Its clock is not named for it. Of the three
`PL_GP0..2_CLOCKIN` inputs, Vivado's bitstream drives only
`PL_GP2_CLOCKIN` from a clock leaf (`RCLK_INT_L_X27Y209`, leaf 12, into
`INT_X27Y198`); the other two are tied to a lookup table like any unused
input. So `PL_GP2_CLOCKIN` is this port's clock: **read from the
bitstream, then confirmed by the port working.** `fabric::hpm0_lpd_pins`
names them for a design, `HPM0_LPD_AWADDR0` … `HPM0_LPD_RREADY` and
`HPM0_LPD_ACLK`, and `uray.dev` declares them. A design binds them as it
binds EMIO, and drives the clock pin with `assign aclk = clk`: the router
takes `PL_CLK0` through a leaf to the pin, as Vivado did.

The port's data width is set on the processor's side. `LPD_SLCR`'s
`AFI_FS` (`0xFF419000`) read `0x200`, which by the reference manual's
encoding is 128 bits; that is a reading, not a measurement. The slave
was written not to depend on it: it takes a write's word from the lane
its strobes name, and returns a read's word on all four lanes.

**Measured, 2026-10-15.** The 16-neuron network behind a small AXI slave
(registers: seven image words, a start, three score words, and the
class with a count of answers), 8 013 instances, routed in 4 hours 38
minutes; 1 033 430 set bits, every one decoded. Linux maps the window
through `/dev/mem` and a C program does seven 32-bit stores, a start,
polls the count and reads the scores:

| `PL_CLK0` | One read | One store (posted) | Per image | Bit-exact |
|---|---|---|---|---|
| 100 MHz | 182 ns | 20 ns | 1.11 µs | 12 / 2 000 |
| 50 MHz | 241 ns | 41 ns | 1.58 µs | 2 000 / 2 000 |
| 25 MHz | 361 ns | 80 ns | 2.53 µs | 2 000 / 2 000 |
| 10 MHz | 804 ns | 200 ns | 5.51 µs | 2 000 / 2 000 |

At 50 MHz the network answers every image as the model does, **68 times
faster than over EMIO**, and the time is the bus: eleven transactions at
a few hundred nanoseconds. Python is not in the loop.

At 100 MHz it does not. The answers are wrong in ways that depend on
the image and on the image before it, and the same image gives the same
wrong answer. Two experiments narrowed it. Starting the same image a
second time, 200 ns later, gave the same wrong answer, so the network was
not still settling. Writing every word twice fixed some images but not
all. Neither stuck pixels nor misplaced words explain the 30 answers
recorded. The likely cause is the write path, which took a pin, two
lookup tables and a register across the die in one clock; the EMIO
design held its data still for several clocks before latching it, and
never needed that. The router knew nothing of delay then (see
"Timing-driven routing").

**That was it.** A version that registers every pin coming from the
processor before any logic reads it — write address, data, strobes,
the valid and ready bits, the read address — and otherwise the same
network and slave (8 148 instances, routed in 5 hours 4 minutes;
1 037 537 set bits, every one decoded) answers every image at 100 MHz:

| `PL_CLK0` | One read | One store (posted) | Per image | Bit-exact |
|---|---|---|---|---|
| 100 MHz | 203 ns | 30 ns | 1.26 µs | 10 000 / 10 000, accuracy 71.07 % |
| 125 MHz | | | 1.14 µs | 1 255 / 2 000 |
| 150 MHz | | | 1.05 µs | 640 / 2 000 |
| 187.5 MHz | | | 0.97 µs | 1 243 / 2 000 |

The extra register costs a cycle each way, which is the 20 ns more a
read takes and why 100 MHz is slower per image than the unregistered
design at 100 MHz claimed to be — that one was fast and wrong. Above
100 MHz it fails again. Nothing there was expected to work: the slave
takes the network's answer a fixed four clocks after the start, and the
network needs 37 ns, so at 125 MHz the answer is taken before it is
ready. That is a property of this slave, not a limit of the port.

So the measured figure for this network on this part is **1.26 µs an
image from a C program on the processor, 85 times faster than over
EMIO**, and of that the network is 37 ns; the rest is eleven bus
transactions.

### Where a processor pin is

**A slave that hung the board, 2026-10-22.** The 32-neuron network behind
a slave taking whole 128-bit beats — an image in two stores and a start,
the answer in one read, against eleven transactions above — matched the
model in simulation and built with every set bit decoded (13 952
instances, 14 minutes). Loaded at 100 MHz, the first read never came
back: the processor stopped, the board answered neither SSH nor ping,
and only a power cycle recovered it. A read on this port that the
fabric never answers stalls the core.

**The cause was the placer, not the slave.** Every `PS8` pin is a bel of
the one `PSS_ALTO` tile, so to the placer all 514 of the port's pins and
every EMIO pin sat at that tile's position, row 309 of the grid, and it
put the logic they connect to there. The wires do not leave from there:
`HPM0_LPD_ARVALID` reaches the interconnect at `(160, 167)`. The slave's
ready bits are combinational, so `ARVALID` to `ARREADY` crossed some
142 rows and back within a 10 ns clock. The 16-neuron slave had the same
placement and happened to make it; this one did not. That is the
diagnosis from the placement, **not a measurement of the path**: no
delay on this die has been measured.

**The fix.** `Board::processor_pin_tiles` follows each processor pin's
wire up to four pips, either way, to the first tile outside the
processor, and `implement` gives the placer that tile for the pin. The
handshake logic moved from row 309 to rows 153–154. Every later step
sees the pin's site where it really is.

**The router had the same fault, and it cost most of every build.** The
router's distance estimate aims at a sink wire's own tile, and a pin's
wire is in `PSS_ALTO` too. With the logic moved and the estimate not,
the build took 71 minutes and its first pass visited 7.5 billion nodes,
against 0.95 billion before. Giving the pin wires their entry tile for
routing too, and putting it back before the routing is verified and the
bitstream written:

| Same design, same flags | First pass visits | Passes | Build |
|---|---|---|---|
| Pins at `PSS_ALTO` (before) | 953 538 829 | 9 | 14 min |
| Placer moved, router not | 7 516 089 597 | 10 | 71 min |
| Both moved | 20 506 999 | 12 | 2 min 32 s |

Every set bit decodes in all three. The 4- and 5-hour AXI builds above
were made with the pins at `PSS_ALTO` and were probably slow for the
same reason; that is likely, **not re-measured**. The rebuilt slave has
**not yet been run on the board**: that waits for a power cycle, and it
will be brought up at 25 MHz before 100.

**Corrected, 2026-10-28: the fix was not one, and it is off by default
now** (`ImplementOptions::pins_at_entry`, `reticle fpga
--pins-at-entry`). The reading above — that the hang was a long path
missing a 10 ns clock — did not survive the board:

- The rebuilt 32-neuron slave hung the board **at 25 MHz**, a 40 ns
  clock, during its first few reads. So speed was not the cause, or not
  the whole of it.
- A second slave written to be right on any port width and any burst
  length, and checked in simulation (nine images by three different
  masters, no protocol errors; six planted faults, each caught),
  built the same way, answered every 32-bit read at 25 MHz — **with the
  previous read's data**. Word *w* read back what word *w* − 1 holds,
  consistently over three sweeps of all sixteen words; the values
  themselves were right (a network answer, the diagnostic words).
- **The deciding experiment.** The 16-neuron registered slave of
  "Reaching the fabric over AXI", unchanged. Its original bitstream,
  placed the old way, loaded the same day at 25 MHz: 500 / 500 images
  bit-exact. The same Verilog rebuilt with the pins at their entry
  tiles (2 min 14 s instead of 5 h 4 min, 499 486 pips instead of
  1 145 442, every set bit decoded), loaded at 25 MHz: the board hung on
  its first reads after the first one.

Same source, same clock, same day; only the placement and routing
differ, and the short one fails at a speed where no setup path can be
long enough to matter. **The likely cause is hold time**, and it is an
inference, not a measurement: the processor's port clock reaches it
through a route into `INT_X27Y198` and the processor's own clock tree,
so it can sample after the fabric's flip-flops have already changed.
With the logic beside the pins, a response's `RVALID` reaches the
processor in time and its data does not — exactly the off-by-one seen —
and a ready bit that changes too early can break a handshake outright,
which stalls the core. The long routes the old placement made by
accident were delay enough. Nothing in Reticle models hold, so nothing
warned.

What this leaves: the old placement works and is the default again;
`Board::processor_pin_tiles` and its test stay, because where the wires
enter is a fact and a hold-aware flow will need it. Before the option
can be on, the flow needs the skew between `PL_CLK0` at the fabric's
flip-flops and at `PL_GP2_CLOCKIN`, measured, and a minimum delay on
every path to a processor input.

## What remains, in order

1. **More of the slice**: the carry chain, for adders that are not
   lookup-table ripples; the lookup table's direct path to its
   flip-flop; and the enable and reset pins of every group, measured.
2. **A clock of Reticle's own.** `PL_CLK0` now reaches slices through
   Vivado's clock network, replayed (above). Building that network rather
   than borrowing it needs the processor's clock buffers on this die,
   which no database describes.
3. **The rest of the processor interface.** `M_AXI_HPM0_LPD` works
   (above). The other masters, the slave ports a design would use to
   reach DDR, and interrupts are not yet exposed; each needs the
   processor configured for it, which the PYNQ image may not have done.
4. **Instantiated primitives.** A `LUT6` written as an instance in Verilog
   stays an instance: the frontend does not know the device's primitives,
   and the flow does not turn such an instance into a cell with the
   device's port directions. Until 2026-10-10 the place-and-route netlist
   read only cells and dropped every instance without a word, in every
   family; a network of 1 355 instantiated tables was routed and written
   without them. It now refuses the first instance by name. Meanwhile a
   design that wants a particular lookup-table structure can mark the
   nets between the tables `(* keep *)`, which synthesis treats as mapping
   boundaries.
