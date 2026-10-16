# NewAE's ChipWhisperer-Husky gateware, built with Reticle

The ChipWhisperer-Husky is NewAE Technology's side-channel capture and
fault-injection instrument. Its FPGA is a Xilinx Artix-7 **XC7A35T in
the FTG256 package** (`xc7a35tftg256-1`), and its gateware is open:
[`newaetech/chipwhisperer-husky-fpga`](https://github.com/newaetech/chipwhisperer-husky-fpga).
This directory is what Reticle needs beyond that source to build it:
stand-ins for the Vivado IP cores the gateware instantiates, its pin
constraints in Reticle's form, and the command line.

**Nothing built here has run on a Husky.** What is checked, and what is
only quoted, is at the end of this page; read it before loading anything
into an instrument somebody depends on.

## What is here

```text
examples/husky/
  build.sh            the command line: NewAE's file list, defines and top
  husky.rcf           every pin of `cwhusky_top`, from NewAE's cwhusky.xdc
  fifo_generator.v    the six FIFO Generator cores, as one behavioural FIFO
  xadc_wiz_0.v        the XADC Wizard core: the XADC with the wizard's registers
  usr_accesse2.v      USR_ACCESSE2, which reads the build time: here, zero
```

## Building

```sh
git clone https://github.com/newaetech/chipwhisperer-husky-fpga husky
git -C husky checkout 2dfe1bc
git -C husky submodule update --init fpga/fpga-common
git -C husky/fpga clone https://github.com/newaetech/tracewhisperer
git -C husky/fpga/tracewhisperer checkout 542b1cc
git -C husky/fpga/tracewhisperer submodule update --init
examples/husky/build.sh husky /tmp/husky
```

**Today this stops at placement**: the design does not yet fit, and the
section at the end says by how much and why.

The superproject pins `fpga/tracewhisperer` at a commit that is not
public; `542b1cc` on its `pro` branch is the newest public one, and is
the one this was built against.

`build.sh` names the `sources_1` fileset of `fpga/vivado/cwhusky.xpr`,
file for file and in its order, with its three defines
(`INTERVAL_MATCHING=1`, `ESAD`, `BB_LIMIT`). Two corrections: the xpr
reaches `fpga-common/hdl/async_resetFFstyle2.v` through a path outside
the checkout (`../../plus_optimization/...`), and the copy inside is
used; and `hdl/trigger_cdc.v`, which `reg_chipwhisperer.v` instantiates,
is not in the list and is added.

An earlier reading of the xpr took two more defines, `TINYFIFO` and
`SAD_X2`, from it; both are on IP filesets (`xadc_wiz_0`, `clk_wiz_0`),
not on the design, and with them the build asked for FIFO cores the
project does not have.

## The stand-ins

### FIFO Generator

`fifo_generator.v` has one FIFO, `husky_fifo`, and a wrapper per core,
each with the core's ports and the parameters of its `.xci`
(`fpga/vivado/cwhusky.srcs/sources_1/ip/<core>/<core>.xci`): width,
depth, standard or first-word fall-through, common or independent
clocks, and the programmable thresholds.

| Core | Width x depth | Clocks | Read | Flags |
|---|---|---|---|---|
| `adc_fast_fifo` | 12 x 2048 | common | standard | overflow, underflow |
| `fast_slower_fifo` | 72 x 4096 | independent | fall-through | prog_empty 6, overflow, underflow |
| `fifo_generator_0` | 18 x 8192 | independent | fall-through | prog_full 8188, prog_empty 128, underflow |
| `trigger_fifo` | 32 x 1024 | common | standard | underflow |
| `usb_slow_fifo1` | 72 x 2048 | common | fall-through | prog_full 2047, prog_empty 4, overflow, underflow |
| `usb_slow_fifo2` | 72 x 16384 | common | fall-through | prog_full 16384, overflow, underflow |

Vivado builds these from the block RAM's own FIFO logic; here each is a
memory with its pointers and flags in the fabric. What differs, said in
the file's header too: the hardware FIFO's capacity in fall-through mode
is a word or two more than its depth, and its flags move on different
cycles after a write on the other clock. NewAE's logic around these
cores counts samples itself and reads the flags as levels, which is the
reading of their source that makes a stand-in reasonable — a reading,
not a measurement.

Together the six hold 1.83 Mbit, on a part with 1.84 Mbit of block RAM.
They fit only because the block RAM's parity bits carry data (18- and
9-bit modes): 100 `RAMB18E1`, every one the part has, which is also how
many NewAE's own image uses.

### XADC Wizard

`xadc_wiz_0.v` is the `XADC` primitive with the wizard's initial
registers, from `xadc_wiz_0.xci`, and its DRP passed straight through.
Every register was compared with the XADC tile of NewAE's image as
decoded here, and one disagrees with the `.xci`: Vivado wrote `INIT_5C`
as `0x5111` where the `.xci` says `0x9999`. This uses Vivado's value.
`tests/fpga_xray_xadc.rs` is that comparison.

### USR_ACCESSE2

On the part, `USR_ACCESSE2` reads a word the bitstream sets, which
Vivado fills with the build time. Reticle neither places that site nor
writes that register, so the stand-in returns zero — what NewAE's own
source gives it under Icarus. Software that prints the build time will
print zero.

## What the build needed from Reticle

Building this design is what found each of these, and each has a test
of its own:

- `--define`, `--include-dir`, and macros carried from file to file.
- Every primitive the gateware names: `BUFG`, `BUFGCE`, `BUFGMUX`,
  `IBUFG`, `BUFMRCE`, `BUFR`, `IBUFDS`/`OBUFDS` (on bits of vector ports,
  as the ADC's six lanes are), `IDDR`, `ODDR`, `PLLE2_ADV`,
  `MMCME2_ADV`, `PULLUP` and `XADC` — each compared with what Vivado
  wrote for the same cell on the same site of NewAE's image.
- The `xc7a35t-ftg256` package itself.
- Block RAM parity bits as data.
- Bits of one register driven by several clocked processes.
- An array addressed only by constants made registers, not a memory: the
  `esad` trigger's 256 counters did not finish mapping in 40 minutes and
  now take 12 seconds.
- LUT mapping that spends LUTs only on bits something reads, products
  with a power of two as wiring, and an aligned part-select write as one
  comparison per group: a byte written into the clock-glitch module's
  1024-bit register cost over twenty thousand LUTs, and costs about a
  thousand.
- Two walks over shared expressions that had no memory of where they had
  been, one of which asked for 64 GiB.

## Checked, and quoted

| Claim | Status |
|---|---|
| each primitive's configuration bits equal Vivado's for the same cell on the same site | **checked**, per primitive, against NewAE's `cwhusky_top.bit` decoded through prjxray-db |
| the pins, standards, drive and pulls are NewAE's | **transcribed** from `cwhusky.xdc` by script |
| the FIFO stand-ins behave as the FIFO Generator does where NewAE's logic looks | **a reading** of PG057 and of NewAE's source |
| the whole design fits the part and routes | **no**: see below |
| it works on a Husky | **not checked**: nothing has been loaded into one |

## Status

**It does not fit yet.** The whole design elaborates, synthesises and
maps — in about 25 seconds, where the first attempt did not finish in
83 minutes — and the result is, measured on 2026-10-16:

| | Reticle | NewAE's image (Vivado 2022.1) |
|---|---|---|
| LUTs | 35,176 `LUT6` cells | 23,274 lookup tables with contents |
| flip-flops | 27,618 | about 26,000 with an initial value of zero |
| `CARRY4` | 1,429 | — |
| `RAMB18E1` | 100 | 100 |

The database describes this part with the XC7A50T's tile grid, 8,150
slices or 32,600 lookup tables, and decoding NewAE's image finds
configured slices in about 93 % of those tiles — more than the 20,800
lookup tables the XC7A35T is specified with. Why Vivado uses them has not
been looked into; what is measured is that it does, so 32,600 is the
ceiling that matters, and Reticle's netlist is over it.

What is known about the gap:

- **No dual-output lookup tables.** The 7-series backend places one
  function per `LUT6`; Vivado packs two of up to five shared inputs into
  one (`LUT6_2`). In a 64-sample `esad`, 1,014 of 2,095 lookup tables use
  three inputs or fewer.
- **Four flip-flops per slice, of eight.** The backend offers each
  slice's `AFF`..`DFF` and not the `A5FF`..`D5FF` beside them, which it
  cannot yet wire, so the slices hold half the flip-flops Vivado can put
  in them. `build.sh` stops at placement on exactly this: *the design needs
  27618 `ff` site(s) and the part has 26800* (the default region, around
  the constrained pins).
- **No wide multiplexers.** Vivado builds 7- and 8-input selections from
  the slice's `MUXF7`/`MUXF8`; Reticle from lookup tables.
- **`esad`**, NewAE's extended SAD trigger, is half the design: 256
  counters at about 62 lookup tables and 76 flip-flops each.

Past fitting, nothing has placed and routed a design this full on this
die, and the ADC side runs at up to 200 MHz while the 7-series placer is
not timing-driven. Nothing has been loaded into a Husky; none is attached
to the machine this was built on.
