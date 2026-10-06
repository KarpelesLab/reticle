# `ppu2c02`

The picture processing unit of an NES-compatible console: a 256 x 240
raster drawn out of a tiled background and up to 64 sprites, the eight
registers a processor reaches it through, and the fetch pipeline whose
timing is what makes scrolling work.

Most of the library's blocks are documented entirely in their source and
in [`docs/ip-library.md`](../../docs/ip-library.md), and this one is too:
[`rtl/ppu2c02.v`](rtl/ppu2c02.v) opens with the raster, the register
map, the `v`/`t`/`x`/`w` layout, the background and sprite pipelines, the
bus contract, and a list of the behaviours it does **not** reproduce.
This page exists for the one thing that is not an engineering question.

## What this is, and what it is not

The 2C02 is a machine. What it does to a nametable byte, where it takes
an attribute from, which sprite wins a pixel and on which dot the sprite
zero flag goes up are **publicly documented, reverse-engineered facts
about a piece of hardware** — the same kind of fact as a UART's frame
format or an SDRAM's command truth table, and implemented here the same
way: from the description, into Verilog, and checked against the
description again.

What is *not* here, and will not be:

- **no game data of any kind.** No cartridge image, no character data,
  no music, no text, no tile that came from anyone's cartridge. The only
  graphics in this repository are the eleven tiles in
  [`examples/nes/sw/chr.s`](../../examples/nes/sw/chr.s), drawn for that
  example and drawn twice — as art in a comment and as bytes — so a
  reader can check them against each other.
- **no lockout silicon.** There is no CIC and nothing that talks to one.
  This is a picture unit for a design that runs its own software on an
  FPGA; it is not a cartridge for a real console and it would not work
  as one.
- **no colour palette taken from a measurement of anyone's part.**
  `ppu_palette` is a table written for this package, laid out the way the
  part's own index is laid out — four levels of sixteen hues — and
  documented as one choice among many, because the 2C02 generates a
  composite signal and has no colours in it at all.

## The block

```text
ip/ppu2c02/
  reticle.ip            the manifest: two sources, one top, 21 ports
  rtl/ppu2c02.v         the part
  rtl/ppu_palette.v     six-bit index in, 24-bit RGB out
```

`ppu2c02` takes a clock and a **dot enable**, exactly as `video_timing`
in [`ip/dvi_tx`](../dvi_tx) does, so a design running faster than the dot
rate gives it an enable rather than a second clock. Its picture comes out
as one palette index per dot on `vid_*`; its memory accesses go out on a
14-bit bus that expects the byte back on the *next* enabled edge, which
is what a synchronous memory does and what makes each of the part's
two-dot fetches one address and one latch.

## Footprint

`ecp5-45f-CABGA381`, through `reticle fpga`:

| Resource | Used |
|----------|------|
| `LUT4` | 2612 |
| `TRELLIS_FF` | 869 |
| `DP16KD` | 1 |

The one block RAM is OAM — 256 bytes, one write port and one registered
read port. Everything else is registers and logic: the palette is 32
entries of six bits read four ways at once, and the eight sprite slots
are counters and shift registers, which is the arrangement the silicon
has and is a great deal cheaper than eight comparators against the pixel
number.

## Testing

[`tests/nes.rs`](../../tests/nes.rs) drives it, on its own and inside a
console. The tests that are about this block rather than about the
example are:

| Test | What it proves |
|------|----------------|
| `the_write_latch_is_shared_by_2005_and_2006` | one `w` toggle between the two registers, the interleaving that allows, and a read of $2002 putting it back — all read off the address pins, because with rendering off the block puts `v` on them every dot |
| `sprite_zero_hits_on_the_dot_the_pixels_meet` | the flag goes up on exactly the dot where sprite zero's own opaque pixel meets an opaque background pixel, and stays down for a whole line when the background is turned off |
| `nine_sprites_on_a_line_set_the_overflow_flag` | eight sprites on one line and nine on the next, and which line the flag goes up on |
| `the_data_port_reads_one_access_behind_and_steps_by_what_2000_says` | the buffered `$2007` read, the palette read that is not buffered, `$3F10` being another way of writing `$3F00`, and `$2000`'s increment and nametable bits |
| `the_frame_comes_out_of_the_video_port` | a whole frame, pixel by pixel, against a frame buffer computed from the nametable, the pattern table and the palette by the documented rules and by nothing this block does |

The last one is the important one, and
[`docs/writing-a-cpu.md`](../../docs/writing-a-cpu.md) section 3 is the
argument for why it is built the way it is: the checker and the thing
checked must not share a parent.
