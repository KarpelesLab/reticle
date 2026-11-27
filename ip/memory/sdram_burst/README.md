# `sdram_burst`

A burst controller for single-data-rate SDRAM, written for the Winbond
W9825G6KH-6 (32 MB, sixteen bits wide, four banks of 8192 rows of 512
columns, 166 MHz at CAS latency 3) on an Artix-7 XC7A100T, for a 720p30
H.264 and AAC decoder that needs some 165 to 210 MB/s of the part's
333 MB/s.

**Nothing in this package has been run on a board.** What is proved is
what `tests/ip_sdram_burst.rs` checks in simulation against a model of the
part written from its datasheet, and what the FPGA flow reports; *What is
checked and what is only quoted* at the end says which is which, and
*Open* says what only a board can settle.

| module | what it is |
|---|---|
| `sdram_burst` | the controller, and the package's `top`: client ports, arbiter, queue, bank scheduling, refresh, read-capture calibration. Portable: no vendor primitive |
| `sdram_phy_xc7` | its 7-series pins: ODDR outputs, a forwarded clock, IDELAYE2 and IDDR on every DQ bit, the IDELAYCTRL |
| `sdram_burst_xc7` | the two wired together, with the part's pins |
| `frame_addr` | where a pixel of a tiled frame lives: the layout below |

`ip/memory/sdram_ctrl` is the simple controller beside it: one sixteen-bit
access at a time, which tops out near 80 MHz and pays the CAS latency on
every word. This one exists because a decoder cannot live on that.

## The port

Every client of the memory has one port; port `p` is bits `[p*W +: W]` of
each packed vector.

```text
cmd_valid, cmd_ready        handshake; cmd_ready is a register
cmd_we                      1 = write
cmd_addr[23:0]              sixteen-bit word address: {row[12:0], bank[1:0], column[8:0]}
cmd_len[3:0]                words minus one: 1 to 16 words
wr_valid, wr_ready,         the write's data, cmd_len + 1 beats, after the
wr_data[15:0], wr_be[1:0]   command is taken; wr_be[0] is bits 7:0
rd_valid, rd_data[15:0],    the read's data, cmd_len + 1 words, in order,
rd_last                     rd_last on the last; there is no ready
```

The rules a client keeps:

* **A burst never crosses a 512-word row**: `column + cmd_len` is at most
  511. The controller does not check it; the model in the tests does, and
  calls it an error.
* **Read data is not flow-controlled.** A client takes a word on every
  cycle `rd_valid` is high.
* **Write data may come at any pace** once the command is taken. The
  controller collects all of a write's words into a 64-word buffer before
  the write joins the queue, so a slow client delays its own writes and
  nobody else's.
* **A port's commands happen in the order it gave them**: its read waits
  until its writes before it have been collected, so a read after a write
  to the same address returns what was written. **Commands of different
  ports are not ordered against each other.**
* Nothing is taken before `init_done`, which rises after the power-up
  sequence and the first calibration.

## The frame layout

`frame_addr` is the only implementation of this, and every client uses it;
this section is its specification, and `frame_addr_is_the_documented_layout`
holds the module to a reference written from this text, at every tile of
23 frames of 1280 x 720.

**A tile is 32 x 32 pixels of one plane and is exactly one SDRAM row**:
512 words, 1024 bytes. Two pixels to a word, little-endian: the pixel at an
even x is bits 7:0, the next one bits 15:8. Inside a tile,

    column = (y % 32) * 16 + (x % 32) / 2

so a line of a tile is sixteen consecutive words and a block inside one tile
is in one open row.

**Planes**: 0 is luma, 1 is Cb, 2 is Cr, each in its own coordinates and its
own tiles: a chroma tile is 32 x 32 chroma pixels, the chroma of 64 x 64
luma pixels in 4:2:0.

**Banks**: tile (tx, ty) = (x / 32, y / 32) is in bank

    bank = (tx + 2 * ty) mod 4

so the four tiles of any 2 x 2 square are in four different banks.
Left, right, up, down and the diagonals are never in a tile's own bank:
a block that straddles a corner of four tiles has all four of its rows open
at once, and the scheduler opens them during the transfers before.

**Rows**: within its bank a tile is row

    row = BASE_ROW + frame * FRAME_ROWS + plane_base + ty * TQ + tx / 4

where `TQ` is the plane's tiles across divided by four, rounded up (the
`tx / 4` of the tiles in one bank of one tile row run 0 to `TQ - 1`), and the
plane bases are 0 for luma, `LUMA_ROWS` for Cb and `LUMA_ROWS + CHROMA_ROWS`
for Cr. With `TW` and `TH` the luma tiles across and down:

| | formula | 1280 x 720 |
|---|---|---|
| luma tiles | `TW` x `TH` | 40 x 23 (720 / 32 is 22.5; the half tile is stored whole) |
| chroma tiles, each plane | `ceil(TW/2)` x `ceil(TH/2)` | 20 x 12 |
| `TQ`, luma / chroma | `ceil(TW/4)` / `ceil(ceil(TW/2)/4)` | 10 / 5 |
| `LUMA_ROWS` | `TH * TQ_luma` | 230 |
| `CHROMA_ROWS` | `ceil(TH/2) * TQ_chroma` | 60 |
| `FRAME_ROWS` | `LUMA_ROWS + 2 * CHROMA_ROWS` | 350 rows in each of the four banks: 1400 KiB a frame |

Frames go up to `(8192 - BASE_ROW) / FRAME_ROWS`, which is **23** at 720p
with `BASE_ROW` 0. A design wanting other buffers (the bitstream, audio)
gives them rows below `BASE_ROW` or above the last frame.

The word address is `{row, bank, column}`, the split `sdram_burst` uses,
and `run` is the words left from this pixel's word to the right edge of its
tile's line (1 to 16): the longest burst that may start there. A 2D fetch is
one burst per line per tile: start at `x / 2`, take `min(run, words left)`,
and continue in the next tile. `frame_addr` is combinational (two constant
products and four additions); a client registers around it.

## How it goes fast

### Bursts that the next command ends

The part runs in **full-page burst mode**. A READ or WRITE starts a burst
that would run along the row forever, and the cycle after its last word
always carries the command that ends it (§7.7, §7.9, §7.11 and the charts of
§11.19): the next burst's READ or WRITE when it is ready, which makes the two
seamless and pays the CAS latency once for the whole train, or BURST STOP
when it is not. That gives every length from one to sixteen with a single
command, and leaves the command bus free during a burst, which is where the
ACTIVE and PRECHARGE of the bursts behind it go. Auto-precharge is illegal
in full page (§7.14); rows stay open until a different row of the bank or a
refresh needs them closed.

The alternative, a burst length of eight interrupted early, ends trains with
a PRECHARGE of their own bank or not at all, and a burst of one with a READ
every cycle leaves no command slot for ACTIVE. Full page costs one BURST STOP
per train.

### A queue whose head is fed and whose window is prepared

Commands go through an **in-order queue of eight**. The oldest four are a
window; the oldest entry in it that uses a bank owns that bank, and an owner
whose row is not open gets its PRECHARGE and ACTIVE while the transfers in
front of it run. The ownership rule is what keeps the preparation from
undoing itself: nobody closes a row an older entry still needs, and nobody
opens a row an older entry will close. Each entry carries a flag saying
whether its row is open, brought up to date the cycle after every ACTIVE and
PRECHARGE, so the scheduler compares no row addresses when it decides.

The decision each cycle is, in order: the command that ends the burst on the
bus (the next transfer, or BURST STOP); a new transfer when the bus is free
and the head is ready; the preparation chosen the cycle before, unless the
last command touched its bank; PRECHARGE ALL and AUTO REFRESH when a refresh
is due. The preparation is chosen a cycle ahead and registered, which is
what brought the scheduler under 6 ns; its timers were met when it was
chosen, and only a command to its own bank, which cancels it, could have
reset them.

### Three arbiter steps

A port's command waits in a one-entry buffer of its own, so `cmd_ready` is
a register. Candidates are worked out and registered: eligible ports, the
priority class (`PRIO`'s ports first; the display's), and the direction
granted last, kept for up to `BATCH` grants because every change of
direction costs a turn-round. The next cycle one is chosen round robin, and
the cycle after that it is carried out. A port is therefore chosen at most
every other cycle. That was the price of 166 MHz: the arbiter in one cycle
measured 10.4 ns. It costs the 9 x 9 fetches below five points of
efficiency (92.0 % to 87.8 %) and nothing measurable elsewhere.

Writes are collected by a two-slot collector: the write granted next starts
taking data the cycle after the last word of the one before, so a client
streaming writes never sees `wr_ready` drop between them.

### Refresh

AUTO REFRESH every `TREFI` cycles at most, the refresh interval of §9.5
(64 ms over 8192 rows, 7.8125 µs) rounded down: it is asked for 16 + tWR +
tRAS + tRP + 8 cycles early, the train on the bus ends at its next burst
boundary, every bank is closed with one PRECHARGE ALL once tRAS and tWR
allow, and AUTO REFRESH follows tRP later. About 20 cycles in 1300.
Refresh also keeps every row's open time under tRAS(max), 100 µs.

## Timings

The datasheet's figures are parameters as §9.5 states them: nanoseconds
(`T_RCD_NS`, `T_RP_NS`, `T_RAS_NS`, `T_RC_NS`, `T_RFC_NS`, `T_REFI_NS`,
`T_INIT_US` in µs) or cycles (`T_WR`, `T_RRD`, `T_MRD`, which is tRSC).
**tRFC is tRC**: §9.5 has no separate refresh cycle time and states tRC as
"Ref/Active to Ref/Active Command Period". The cycle counts `TRCD` to
`TINIT` are derived and must not be overridden.

They are derived from `CLK_PS`, the clock's period in picoseconds, which
defaults to `1000000 / CLK_MHZ`. **A clock that is not a whole number of MHz
must set `CLK_PS`**: minimums are rounded up and the refresh interval down,
so `CLK_MHZ` 166 for a 166.67 MHz clock makes the 200 µs pause 199.2 µs,
and 167 makes the refresh interval 7.824 µs, each outside §9.5. 166.67 MHz is
`CLK_PS` 6000.

| parameter | -6 at CL3 (§9.5) | cycles at 6.0 ns |
|---|---|---|
| tRCD | 15 ns | 3 |
| tRP | 15 ns | 3 |
| tRAS | 42 ns (max 100 µs) | 7 |
| tRC, and tRFC | 60 ns | 10 |
| tRRD | 2 tCK | 2 |
| tWR | 2 tCK | 2 |
| tRSC | 2 tCK | 2 |
| refresh interval | 64 ms / 8192 | 1302 |
| power-up pause | 200 µs | 33 334 |

The power-up follows §7.1: 200 µs of command inhibit with CKE and DQM high,
PRECHARGE ALL, eight AUTO REFRESH, then MODE REGISTER SET (full page,
sequential, `CAS_LATENCY`, burst writes; §10.4's table).

## The bus between a read and a write

A read and a write may not drive DQ at once. The part drives a read word from
its clock edge until tHZ, 5 ns, after the next (§9.5); the FPGA drives a write
cycle from half a cycle before the part's edge (the forwarded clock is
inverted) to a cycle after it, because its three-state enable is a fabric
register (*The flow* below says why). In half cycles of 6 ns that is
`[2k, 2k + 3]` for the part's word `k` and `[2n - 1, 2n + 2]` for the FPGA's
cycle `n`, and the controller raises its output enable one cycle before a
write's first word. So a write's first word comes at least
`RD2WR_GAP` = 3 empty cycles after a read's last; a read may follow a write
at once (§7.10), its data arriving CAS latency later. The model checks the
overlap on every cycle.

## Read capture and calibration

At 166 MHz half a cycle is shorter than the part's access time plus the
board, so the read data cannot simply be sampled where `sdram_ctrl` samples
it. Each DQ bit goes through an IDELAYE2 into an IDDR, which hands the
controller two samples a cycle, one from each clock edge (`dq_rise`,
`dq_fall`). The controller delays each by 0 to 3 cycles: eight candidates,
chosen by (edge, latency). `PHY_RD_LAT` is how many cycles before the
latency-0 candidate the data arrives, plus the nominal latency; it is 8 for
`sdram_phy_xc7` (the core's own default, 4, is for a PHY that hands DQ over
the cycle the part drives it, which is what the core-only tests do).

Where in the cycle the data is valid depends on the board, so the capture
point is **calibrated**: after the power-up sequence the controller writes
a sixteen-word pattern at the start of `CAL_ROW` in `CAL_BANK` (every word
different, every bit changing), then for each of the 32 taps loads the
delay, reads the pattern back and records which candidates returned all of
it. It picks the candidate with the longest run of good taps, and the tap
in the middle of that run. A run cut short by tap 0 or 31 is centred on
what it has, not on the eye. `cal_ok` says whether anything read back;
if nothing did, the defaults (`DEF_EDGE`, `DEF_LAT`, `DEF_TAP`) stay.
`cal_req` runs it again, for example as the board warms: it waits for the
clients' traffic in the queue to drain, and serves nobody else until it is
done. **The calibration overwrites those sixteen words**; a design keeps
nothing there.

The IDELAYCTRL's reference is 200 MHz (DS181 Table 25; 300 MHz is allowed on
grades -2 and -3), which makes a tap 78 ps and the 32 taps 2.4 ns: less than
the half cycle between the two edges, so the eight candidates do not cover
every instant of a cycle, but the part's data eye, tOH + tCK - tAC = 4 ns
at CL3, is wider than the gap.

## What was measured

Everything below is counted in cycles in simulation, at 166 MHz against the
model (`cargo test --test ip_sdram_burst -- --nocapture` prints it); MB/s
is that fraction of the part's 333 MB/s.

| traffic | bus busy | MB/s |
|---|---|---|
| one client, sequential reads of 16 | 98.5 % | 328 |
| one client, sequential writes of 16 | 98.5 % | 328 |
| two readers and two writers of sequential 16-word bursts | 82.2 % | 274 |
| motion compensation, 300 random 9 x 9 luma blocks through `frame_addr`, one client | 87.8 % | 292 |
| the same, 13 x 13 | 94.9 % | 316 |
| the same, 21 x 21 | 97.1 % | 323 |
| a decoder's mix on four ports: 9 x 9 fetches (sixteen a macroblock), macroblocks written back, display reads, bitstream writes | 82.7 % | 275 |
| random 1 to 16 word reads and writes from four ports, with stalls | about 70 % | |

What is lost on the sequential streams is refresh and the train ends; with
two readers and two writers, the turn-rounds; on the small blocks, a row of
the next block in the bank the current block is still using (one block
transition in four), and the arbiter's every-other-cycle rhythm on the
two- and three-word bursts where a block crosses a tile.

The display port (`the_display_port_is_served_first`): with ports 0 to 2
saturating the controller with 16-word reads and writes and port 3 asking
for 16 words every 64 cycles (twice what 720p30 4:2:0 display needs), the
display's first word came at worst 207 cycles after its request was taken
(mean 159) with priority, against 297 (mean 188) without. Priority orders
the arbiter, not the queue: what is already queued goes first, and eight
16-word transfers are most of those 207 cycles. 207 cycles is 1.25 µs,
in which a 720p display reads some 100 bytes; a line FIFO of a few hundred
bytes rides it out.

## The 7-series pins

`sdram_phy_xc7` puts every output through an ODDR in its pad's OLOGIC with
both edges given the same bit, so command, address, mask and data all leave
from the same kind of register at the same edge, and forwards the clock
through another ODDR with D1 = 0 and D2 = 1: the part's clock is the
controller's inverted, and the part samples everything half a cycle after
it was launched. At 6 ns, with DS181's -2 figures for LVCMOS33 at 12 mA
fast (TOCKQ 0.49 ns, TIOOP 2.68 ns, the same for every output), that is
3 ns of setup and 3 ns of hold against §9.5's 1.5 ns and 0.8 ns, less the
board's skew between the clock and the other traces.

`the_xc7_pins_capture_reads_after_calibration` simulates `sdram_burst_xc7`
with the flow's models of ODDR, IDDR, IDELAYE2 and IDELAYCTRL
(`src/fpga/devices/xc7_sim.v`) and a board in picoseconds: the outputs
3.17 ns after the edge, a trace each way, 1.34 ns into the input (TIOPI)
and 1.5 ns of clock insertion delay the data path does not have, and the
part driving each word from tAC (5 ns) after the edge before until tOH (3 ns)
after its own, unknown in between. With 300 ps of trace calibration chose
the falling edge, latency 2, tap 15; with 900 ps the rising edge, latency 1,
tap 19; and 384 words came back right in each. Those delays are DS181's and
the datasheet's, not a board's: the test proves the PHY's wiring, its
latency and that the calibration finds a window when there is one; it says
nothing about where the window is on the real board.

## The FPGA build

`testdata/ip/sdram_burst_fpga.v` is a whole design around
`sdram_burst_xc7`: a PLLE2 making 166.67 MHz and 200 MHz from 50 MHz, and
four pseudo-random clients (LFSRs) that keep every port busy so that
nothing is optimised away. Built for the decoder's part, the XC7A100T in
FGG484, at speed grade 2. **Its pins are placeholders**: the board's pinout
is not known, so every SDRAM pin is in HR bank 35 (one IDELAYCTRL) and the
rest in bank 34, as `testdata/ip/sdram_burst_fpga.xdc` says.

```sh
reticle fpga --device xc7a100t-fgg484 --speed-grade 2 --timing-driven \
    --top sdram_burst_fpga --constraints testdata/ip/sdram_burst_fpga.xdc \
    --bitstream /tmp/sdram.bit testdata/ip/sdram_burst_fpga.v \
    ip/memory/sdram_burst/rtl/sdram_burst.v ip/memory/sdram_burst/rtl/sdram_phy_xc7.v \
    ip/memory/sdram_burst/rtl/sdram_burst_xc7.v
```

| | |
|---|---|
| lookup tables | 2028 LUT6 (logic depth 5), 40 MUXF7, 1 MUXF8, 83 CARRY4 |
| distributed RAM | 6 RAM64M: the 64-word write buffer |
| flip-flops | 1475 (the four test clients are 138 of them) |
| block RAM, DSP | none |
| IO | 39 ODDR, 16 IDDR, 16 IDELAYE2, 1 IDELAYCTRL, 1 PLLE2 |
| longest path, routed estimate | **5.00 ns** against 6.0 ns (166 MHz) and 7.5 ns (133 MHz) |

The estimate is the flow's model of Vivado's timer, not a measurement, and
it moves with the placement: the same design on the FTG256 package (the
same die) measured 5.15 ns, and builds of nearly the same logic there 5.28
and 5.60 ns. Before the arbiter was split into steps and the bank
preparation registered, the same model put the longest path at 10.44 ns.
The FGG484 bitstream's 137 124 set bits all decode back (0 unexplained), and the
interconnect arcs they select are exactly the ones the router chose.

### The flow

Four things the flow could not build, each worked around and each said here
so that it can be fixed rather than rediscovered:

1. **An instantiated `IOBUF` on one bit of an inout vector** is refused at
   flattening: `I0033`, "connects inout port `IO` of module `IOBUF` to
   something that is not a net of the port's type". Each DQ bit is an
   inferred three-state buffer instead (`assign dq[i] = t ? 1'bz : o`),
   which places, routes and keeps its IDELAYE2 on the pad. *Fixed in the
   flow since (2026-11-27, `docs/ir.md`, "Flattening"); the inferred
   buffers are kept.*
2. **An ODDR into a buffer's T** (the OLOGIC's three-state register): "no
   path exists from the driver of `g[0].t` to `dq$io0.T[0]`: the
   architecture has no wire joining them". The enable is a fabric
   register, one per pin, and the controller raises it a cycle early.
   *Still not built (2026-11-27), but refused while mapping now, with the
   port and the ball: `docs/fpga-xray.md`, "An `ODDR` into a buffer's
   enable", says what the fabric has.*
3. **An ODDR on the pad of a single-pad IO tile** (`IO_0` and `IO_25`
   balls; measured on E6, bank 35's `IO_0` in the FTG256): "no path exists from the
   driver of `sclk` to `sclk$io0.I[0]`", for a plain output as much as
   for a three-state one. The constraints leave those balls out. **A board
   for this controller should not put an SDRAM signal on an `IO_0` or
   `IO_25` ball** until the flow can. *Refused while mapping since
   (2026-11-27), naming the ball: the clock pips such a register needs are
   not described for those tiles (`docs/fpga-xray.md`, "An `ODDR` on a
   `_SING` pad"). The advice stands.*
4. **Logic clocked from a PLLE2's CLKOUT0 is in no domain of the timing
   estimate**: the controller on CLKOUT0 was reported as "clk_in 0.00 ns
   of 20 ns", and moved to CLKOUT1 it is timed. The design uses CLKOUT1
   and CLKOUT2. *Fixed in the flow since (2026-11-27): a PLL's `CLKOUT0`
   was a role the timing read as no output (`docs/fpga-xray.md`, "Clocks
   the managers make").*

## What is checked and what is only quoted

**Checked in simulation**, against the model in `tests/ip_sdram_burst.rs`,
written from the W9825G6KH datasheet (revision A04, 20 March 2017) and
held to it by the `the_model_*` tests, which break each rule once and
expect the model to say so:

* every command's timing: tRCD, tRP, tRAS and tRAS(max), tRC (and tRFC),
  tRRD, tWR, tRSC, the refresh interval, and the 200 µs power-up pause
  with CKE and DQM high (the full pause is simulated once, in
  `the_power_up_pause_is_the_datasheets`);
* every command's legality: ACTIVE to an idle bank, READ and WRITE to an
  open one without auto-precharge, AUTO REFRESH and MODE REGISTER SET with
  every bank idle, the mode word, the init order, CKE never low;
* data: every word written is stored with its byte mask, every read word
  is compared with what the client wrote or what the part held, given to
  the right port in order with `rd_last` right;
* the controller's own rules: no burst over sixteen words or across a row,
  a write cycle always driven, no bus contention as drawn above;
* the frame layout, at every tile of 23 frames, against this text;
* calibration on an emulated window (`calibration_*`) and on the 7-series
  primitives' own models with a board in picoseconds;
* 166 MHz at CAS latency 3, 133 MHz at 3 and 2, 100 MHz at 2.

Mutated, the bench fails: tRCD a cycle short gives "Write to bank 3 before
tRCD" from the model, and read data taken from the wrong candidate gives
2445 wrong reads.

**Quoted and not measured**: every timing number of §9.5, the meaning of
the truth table (§8) and of the charts in §11 (burst stop and precharge
termination, read-to-write and write-to-read), DQM's two-cycle read
latency, and that tHZ bounds when the part lets go of the bus. DS181's I/O
figures (TOCKQ, TIOOP, TIOPI, the tap of 78 ps at 200 MHz, the
IDELAYCTRL's reference range at grade -2) are quoted the same way. The
model's reading of the datasheet and the controller's are the same reading:
a misreading both share would not show.

## Open

* **The capture tap, edge and latency on the board.** Calibration finds
  them, but only a board shows where the window is, how wide, and whether
  it moves with temperature (re-run `cal_req` if it does).
* **The board's skew**: clock against command and data, both ways.
  Half a cycle of setup and hold leaves 1.5 ns either side for it at
  166 MHz.
* **Signal integrity** at 166 MHz on LVCMOS33: drive strength and slew are
  the board's choice (`DRIVE 12 SLEW FAST` in the constraints is a guess).
* Whether the forwarded clock should be shifted rather than inverted (an
  HR bank has no output delay; a PLL phase would do it).
* The display's latency is bounded by the queue, not by priority: a
  controller that let the display overtake queued transfers would bound it
  tighter.
