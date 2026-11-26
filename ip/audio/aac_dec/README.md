# `aac_dec`

An AAC-LC decoder: an ADTS stream, or the raw access units MP4 carries,
in; 16-bit PCM out. MPEG-4 AAC Low Complexity (ISO/IEC 14496-3 subpart 4,
audio object type 2) and MPEG-2 AAC LC (ISO/IEC 13818-7), mono or stereo,
7.35 to 48 kHz, every coding tool of the profile.

**Nothing in this package has been run on a board.** What is proved is
what `tests/ip_aac_dec.rs` checks in simulation against two references —
this decoder's own bit-accurate model and ffmpeg's decoder — and what
the FPGA flow reports; *What is and is not proved* at the end says which
is which.

| module | what it is |
|---|---|
| `aac_dec` | the decoder, and the package's `top`: the stages and the sequencer that runs them |
| `aac_bits` | bytes in, one bit a cycle out |
| `aac_parse` | the syntax: ADTS, `raw_data_block`, the elements, `individual_channel_stream` |
| `aac_huff` | one codeword of any of the twelve codebooks, a bit a cycle, from a tree in block RAM |
| `aac_dequant` | \|q\|^(4/3) and the scalefactor gain |
| `aac_stereo` | perceptual noise substitution, M/S and intensity stereo |
| `aac_tns` | temporal noise shaping |
| `aac_synth` | the filterbank: IMDCT, windows, overlap-add, rounding to PCM |
| `aac_pcm_out` | two frames of PCM and the output stream |
| `aac_ram`, `aac_glen` | a block RAM; the length of a window group |
| `aac_huff_rom`, `aac_huff_root`, `aac_swb`, `aac_pow43_rom`, `aac_win_rom`, `aac_tw_rom`, `aac_gain_mant`, `aac_inv_sqrt`, `aac_tns_refl` | tables, **generated** by `tools/aac_dec_model.py tables` and not to be edited |

## What it decodes, and what it refuses

| | |
|---|---|
| transport | ADTS (`cfg_raw` low), with or without CRC, one to four `raw_data_block`s a frame; or raw access units (`cfg_raw` high) with `in_last` on each one's last byte |
| profile | AAC LC: ADTS `profile_ObjectType` 1. Main, SSR, LTP and the ER types are refused |
| channels | one SCE (mono) or one CPE (stereo) per block, as `channel_configuration` 1 or 2 says. A PCE, DSE or FIL in the block is read and stepped over; a CCE or LFE is refused |
| rates | `sampling_frequency_index` 3 to 12: 48, 44.1, 32, 24, 22.05, 16, 12, 11.025, 8 and 7.35 kHz (7.35 uses the 8 kHz tables, Table 4.82) |
| frame | 1024 samples. The 960-sample frame (`frameLengthFlag`) is not supported: ADTS cannot signal it, and a raw stream that uses it is out of scope |
| tools | section data with escapes, scalefactors, the eleven spectrum codebooks and the escape codebook to \|q\| = 8191, pulse data, M/S (`ms_mask_present` 1 and 2), intensity stereo (codebooks 14 and 15, with the `ms_used` inversion), PNS including correlated noise, TNS with filters up to order 12 (long) and 7 (short), all four window sequences, sine and KBD windows, grouped short windows |
| not decoded | SBR and PS (HE-AAC): an HE-AAC stream's FIL elements are skipped, and what comes out is its AAC-LC core at the core rate — legal, and half the bandwidth |

In raw mode the AudioSpecificConfig's two fields the decoder needs come
in on `cfg_sfi` (`samplingFrequencyIndex`) and `cfg_chan`
(`channelConfiguration`), sampled at the start of each unit.

### The ports

| port | dir | what |
|---|---|---|
| `clk`, `rst_n` | in | one clock; reset asynchronous, active low |
| `cfg_raw`, `cfg_sfi[3:0]`, `cfg_chan[1:0]` | in | raw mode, and its rate and channels |
| `in_data[7:0]`, `in_valid`, `in_last`, `in_ready` | | bytes; `in_last` only matters in raw mode |
| `out_data[15:0]`, `out_right`, `out_last`, `out_valid`, `out_ready` | | samples, left then right; `out_last` on a frame's last |
| `stat_sfi[3:0]`, `stat_stereo` | out | the last good frame's rate and channel count |
| `frame_ok`, `frame_err`, `err_code[3:0]` | out | a pulse per frame decoded, or refused and why |

`in_ready` and `out_valid` are registers; neither depends on an input.

### Errors

A refused frame produces **no samples** and a `frame_err` pulse; the
decoder skips to the end of the ADTS frame (or of the access unit) and
carries on, so the next good frame overlaps with the last good one.
`refuses_broken_frames_and_resyncs` checks that, for codes 1, 3, 5 and 9.

| code | meaning |
|---|---|
| 1 | an ADTS syncword was expected and something else came (reported once per stretch of junk) |
| 2 | an ADTS header or `cfg_*` this decoder does not take: not LC, a rate above 48 kHz or reserved, a channel configuration other than 1 or 2 |
| 3 | an element that is not allowed: CCE, LFE, a second audio element, the wrong one for the channel count, or no audio element at all |
| 4 | a bit LC forbids: `ics_reserved_bit`, `predictor_data_present`, `gain_control_data_present`, `ms_mask_present` 3 |
| 5 | `max_sfb` past the band table, a section running past `max_sfb`, reserved codebook 12 |
| 6 | a scalefactor outside 0..255 |
| 7 | pulse data in a short window, or a pulse past the spectrum |
| 8 | an escape longer than 8191 allows |
| 9 | the syntax ran past the ADTS frame's `aac_frame_length` |
| 10 | the syntax ran past the access unit's `in_last` |

A refused frame leaves the stream's timing a frame short; a player that
must keep audio and video aligned should count `frame_err` and insert
silence of its own.

## How a frame goes through

```text
bytes -> aac_bits -> aac_parse ----> aac_dequant ----> spectrum L, R  (2 x 1024 x 35 bits)
                      |  aac_huff                          |
                      +-> band, TNS, ics_info ----+        v
                                                  +-> aac_stereo (PNS, M/S, IS)
                                                  +-> aac_tns, per channel
                                                       aac_synth, per channel -> aac_pcm_out -> samples
```

The stages run one after another, sequenced by `aac_dec`; only the output
overlaps, because `aac_pcm_out` holds two frames and plays one while the
next is written. The parser waits for the spectra to be free before each
`raw_data_block` and goes on to the next header meanwhile.

**One bit a cycle.** The parser and the Huffman decoder read the stream a
bit per clock: a frame is a few thousand bits, and the budget below has
room to spare many times over, so a wider reader would cost logic and buy
nothing. Every coefficient comes out of the parser — every one of the
1024 per channel, zeros included — so nothing has to clear the spectrum
before a frame.

## Number formats, and the model that is the specification

Everything after the parser is fixed point, and **`tools/aac_dec_model.py`
(`FixedDecoder`) is the arithmetic, bit for bit**: the RTL must produce
its PCM exactly, and the tests require it.

| quantity | format |
|---|---|
| a spectral value | 35-bit signed, 7 fractional bits, in the units of the standard's `x_rescal` |
| a coefficient: window, twiddle, gain mantissa, TNS reflection coefficient | 25-bit signed, Q1.23 |
| \|q\|^(4/3) | 34-bit unsigned, 20 fractional bits, 1025 entries |

Every product is 35 x 25 bits signed, which is what two DSP48E1 make;
every rounding is *add half and shift right arithmetically*; every
narrowing saturates. The 7 fractional bits keep the rounding of every
stage below a fiftieth of an output LSB (*Against ffmpeg* has what that
buys); the 27 integer bits are four times the largest spectral value any
test stream produced (`loud_s48`, a 100 Hz sine at 0.99 of full scale:
3.4 x 10^7), so a spectrum saturates only for a signal that would clip
anyway or a TNS filter of absurd gain (see below).

The tables are computed from the standard's formulas by the same script:
the sine and KBD windows of 4.6.11.3.2 (the Bessel function by its
series), x^(4/3), the twiddles, 2^(f/4), 1/sqrt(width) and the TNS
reflection coefficients of 4.6.9.3. None of them was copied from
anywhere.

### |q|^(4/3) above 1024

The table holds 0..1024. Above that the decoder writes q = 8j + r and uses
q^(4/3) = 16 (j + r/8)^(4/3), interpolating linearly between entries j and
j + 1: the factor 16 is four quarter-steps of scalefactor gain, so it
costs nothing, and the interpolation's error at j >= 128 is under one
part in 10^7. Streams with escapes up to the 8191 the standard allows
(`craft_esc_m48`) decode within one LSB of ffmpeg.

### The IMDCT

The standard defines the IMDCT as a sum (4.6.11.3.1),

    x[n] = 2/N  sum spec[k] cos(2 pi / N (n + n0)(k + 1/2)),  n0 = (N/2 + 1)/2,

which is N/2 times a DCT-IV of the M = N/2 coefficients, read at shifted
indices with two sign symmetries. `aac_synth` computes the DCT-IV with an
M/2-point complex FFT:

    u[n] = (X[2n] + i X[M-1-2n]) W[n],   W[n] = exp(-i pi (n + 1/8) / M)
    U    = FFT_{M/2}(u)
    y[k] = U[k] W[k];  C[2k] = Re y[k],  C[M-1-2k] = -Im y[k]

and the IMDCT's 2M outputs are C[n + M/2], -C[3M/2 - 1 - n] or
-C[n - 3M/2] by range. **This is derived here, not quoted**: the split of
the 1/4-sample twiddle between pre and post (1/8 each) and the output
mapping were checked numerically against the direct sum before any
hardware was written, and the float model built on it agrees with ffmpeg.
The 2/N is folded in as a halving in the pre-twiddle and in each of the
FFT's radix-2 stages, which also means no stage can overflow.

The long window is a 512-point FFT (nine stages), a short window a
64-point one (six), all on one complex multiplier and one 512 x 70-bit
work RAM; a butterfly takes two cycles because it has two words to
write.

### TNS as a lattice

4.6.9.3 converts the transmitted reflection coefficients to LPC
coefficients and filters in direct form. The LPC coefficients of an
order-12 filter can grow into the hundreds, which a fixed-point
coefficient would have to be sized for. `aac_tns` instead runs the
all-pole lattice on the reflection coefficients, which is the same
transfer function 1/A(z) — the step-up recursion the standard writes is
the one that defines the lattice — with every coefficient bounded by one,
so Q1.23 holds them all:

    f = x;  for m = p .. 1:  f -= k[m-1] g[m-1],  g[m] = k[m-1] f + g[m-1];  g[0] = f;  y = f

The equality was checked numerically against the direct form before it
was used, and long-window filters of orders up to 12 decode within one
LSB of ffmpeg (`craft_tns_m24`, `craft_tns_s48`).

### PNS without a square root

4.6.13.3 fills a noise band with random values, measures their energy,
and scales by 2^(nrg/4) over the square root of it. The decoder fills it
with **plus or minus one value V = 2^(nrg/4) / sqrt(width)**, the signs
from a PRBS31 generator: a band of plus-or-minus ones has energy `width`
exactly, so the scaling needs a table of 1/sqrt(width) for the
twenty-four widths a band can have and no square root or division. The
standard asks only for *signed random values whose sum of squares is not
zero*, which this is; it is a choice, and a decoder with uniformly
distributed values would sound marginally different. The energy of every
frame with noise in it agrees with ffmpeg's to within 0.1 dB (measured:
0.01 dB or better).

## The Huffman tables

The twelve codebooks were taken from the text of ISO/IEC 14496-3,
Annex 4.A, Tables 4.A.1 to 4.A.12, in two editions (2001 and 2009),
whose extractions agree line for line. On extraction each book was
checked to have the entries Table 4.151 implies (121, 81 x 6, 64 x 2,
169 x 2, 289), contiguous indices, no codeword a prefix of another, and a
Kraft sum of exactly one — a complete code, so the decoder has no invalid
codeword to handle. They are in `tools/aac_dec_codebooks.txt`.

`aac_huff_rom` holds each book as a binary tree, two entries a node; a
leaf of a spectrum book holds the n-tuple already split into its fields
(4.6.3.3's divisions done by the generator), so the hardware never
divides. `every_codeword_decodes_through_the_tree_rom` walks all 1362
codewords through it.

## Against ffmpeg

**AAC decoders are not required to agree to the bit**, and this one does
not agree with ffmpeg's to the bit: ffmpeg decodes in floating point and
rounds once, this decoder rounds at every stage. The tolerance is **one
LSB on every sample** of every frame that can be compared, and it is
tight on purpose:

* two correct decoders computing nearly the same real value will round
  it to different integers whenever it sits near a half, so one LSB is
  the smallest tolerance two independent decoders can be held to;
* the double-precision model in the same script, which shares nothing
  with the fixed-point one but the syntax, is within one LSB of ffmpeg
  too — so a frame two LSBs out would be the fixed point's fault, not the
  reference's.

Measured with ffmpeg 6.1.1, over every comparable frame of the 29 test
streams (324 frames; 19 of the streams are in the corpus):

| | fixed point (= the RTL) | double precision |
|---|---|---|
| largest difference | **1 LSB** | 1 LSB |
| samples that differ, per stream | 0.2 to 0.8 % | 0 to 0.23 % |
| SNR against ffmpeg, per stream | 87 to 112 dB | 103 to 118 dB, or exact |

The fixed point's extra disagreements are its rounding: an error of a few
hundredths of an LSB moves a sample across a half about as often as that
fraction says.

Frames that **cannot** be compared sample for sample, and why:

* **noise**: a frame with PNS in it, or right after one (the overlap
  carries the noise forward). The noise is random by definition; these
  frames are held to their energy, within 0.1 dB.
* **the first frame**: 4.6.11.3.2 says the first frame's left half takes
  the frame's own window shape, and ffmpeg starts from a sine left half
  instead, so where the first frame is KBD the two differ (by 93 LSB in
  `clicks_m44`). This decoder follows the standard. The first frame is the
  encoder's priming in any case; the corpus records it and the test does
  not hold it to the tolerance.
* **faac's first frame**, which ffmpeg drops (the test lines the frames up
  by the offset that fits).

Two things found while measuring, recorded rather than fixed:

* **ffmpeg 6.1.1 refuses an ADTS frame with more than one
  `raw_data_block`** ("not implemented"). The multi-block streams
  (`craft_multi_*`) are therefore referenced to ffmpeg's decode of the same
  blocks one to a frame.
* **A TNS filter with near-unit reflection coefficients at order 12 has a
  gain far beyond sixteen bits**, and there the decoders part: ffmpeg
  filters in floating point and clips at the end, this decoder saturates
  its 35-bit spectrum. The crafted TNS streams use reflection coefficients
  up to about 0.4, which an encoder shaping real audio produces; random
  indices across the whole table produced differences of thousands of
  LSB. No encoder at hand writes such filters.

## Which coding tools the tests exercise

`tools/aac_dec_streams.py` made the streams: ffmpeg's `aac` encoder and
faac 1.30 run as black boxes on sines, sweeps, clicks, noise and square
waves at every rate class, and re-serialisations of their output, from
the model's parse, with changes neither encoder makes (the `craft_`
streams). Each crafted stream is still decoded by ffmpeg for its
reference. The generator's own parse counts what each stream uses, and
`the_decode_tests_cover_the_corpus` requires every item below to appear
in the corpus.

| tool | where | how it got there |
|---|---|---|
| SCE, CPE, common window and not | every stream; faac's CPEs have no common window | encoders |
| ONLY_LONG, LONG_START, EIGHT_SHORT, LONG_STOP; sine and KBD; grouped short windows | most streams | encoders |
| codebooks 1 to 11, escapes | every stream; `loud_s48` | encoders |
| escapes above 1023, up to 8191 | `craft_esc_m48` | crafted: values placed in an escape band |
| M/S, `ms_mask_present` 1 and 2 | `corr_s48`, `tones_s44`, `loud_s48` | ffmpeg |
| intensity, codebooks 15 and 14 | `tones_s44`, `sweep_s32`, `loud_s48` | ffmpeg |
| PNS | `noise_s48_is`, `faac_s44`, `faac_clk48` | ffmpeg, faac |
| correlated PNS (`ms_used` on a band noisy in both channels) | `craft_corr_noise_s48` | crafted |
| TNS, short windows, compressed coefficients | `corr_s48` | ffmpeg |
| TNS, long windows, orders to 12, 3-bit coefficients, downward | `craft_tns_m24`, `craft_tns3_s48` | crafted: ffmpeg's long-window filters sit above `max_sfb`, where they do nothing |
| pulse data | `craft_pulse_s44` | crafted: part of a coefficient moved into a pulse, which must decode to the same spectrum, and does in ffmpeg too |
| DSE (aligned and not), PCE, FIL | `craft_extras_s44` | crafted |
| ADTS CRC, several blocks a frame | `craft_multi_crc_m22` | crafted |
| a CPE whose channels switch windows apart: one long while the other is short, sine against KBD | `craft_split_s48` | crafted: the channels of two mono streams in one CPE without a common window; faac's own such CPEs keep the two alike |
| raw access units | `corr_s48`, `sine_m48` without their headers | the test |
| rates: 48, 44.1, 32, 24, 22.05, 16, 11.025, 8 kHz | the stream names | encoders |

Not covered by the corpus: 12 and 7.35 kHz (their band tables are 16 and
8 kHz's, which are covered). A frame of eight ungrouped short windows
occurs once in the development streams, outside the corpus.

## Cycles

Measured by the tests, the cycles from one `frame_ok` to the next with
the input never starved:

| stream | slowest frame |
|---|---|
| mono, 48 kHz (`sine_m48`) | 14 600 |
| stereo with M/S and intensity (`tones_s44`) | 25 400 |
| stereo with short-window TNS (`corr_s48`) | 30 500 |
| mono with two long TNS filters of order up to 12 (`craft_tns_m24`) | 37 500 |

**The budget**: at 48 kHz a frame is 1024 / 48 000 s, which is 1 066 666
cycles at 50 MHz. A stereo frame uses about 3 % of it. The worst case
this design can meet is bounded, not measured: TNS over all 1024
coefficients of both channels at order 12 is about 2 x 32 000 cycles, the
parser at the largest frame the standard allows (6144 bits a channel) about
15 000, and the rest about 20 000 — about 100 000 cycles, under a tenth of
the budget at 50 MHz, and under a fifth at 25 MHz.

**The input must arrive faster than real time.** The parser waits for
the spectra to be free before it reads a block, so a stream trickled in
at exactly its own bit rate would leave the decoder parsing for most of
the frame and only then synthesising. Fed from a buffer — the way a
demultiplexer reading memory feeds it — it is idle 95 % of the time.

## Resources

`reticle fpga ip/audio/aac_dec/rtl/*.v --top aac_dec --device
xc7a100t-ftg256 --speed-grade 2` (the XC7A100T die of the target
`fgg484`):

| | |
|---|---|
| LUT6 | 9 199 |
| flip-flops | 3 586 |
| DSP48E1 | 14 (seven 35 x 25 multipliers) |
| block RAM | 16 RAMB36E1 + 10 RAMB18E1 |
| logic depth | 11 lookup tables |

Of the XC7A100T that is 15 % of the lookup tables, 6 % of the DSP slices
and 15 % of the block RAM. By unit, mapped alone: `aac_parse` 2 195 LUT,
`aac_synth` 2 245 (and 8 DSP), `aac_stereo` 1 864, `aac_tns` 1 350,
`aac_dequant` 796, the rest a few hundred.

### Timing

Placed and routed by the same flow with `--timing-driven --bitstream`,
the 51 pins in banks 34 and 35 and the clock on a clock-capable pin, a
10 ns period asked:

| | |
|---|---|
| routed timing estimate, speed grade 2 | **15.4 ns, about 65 MHz** |
| latest path | `aac_stereo`'s intensity scaling: the product, the rounding add, a 70-bit shift, saturation and a negation, into the write register |
| the other units' latest | `aac_parse` 13.7 ns, `aac_dequant` 13.6, `aac_synth` 10.9, `aac_tns` 10.6 |
| the bitstream | every set bit decodes back through the database, none unexplained, and the arcs they select are the ones the router chose |

So 50 MHz has 4.6 ns to spare and 100 MHz does not close yet. This is
the flow's delay model, fitted to Vivado's timer, and not a measurement
on a part.

The scalefactor band tables were a case statement in each of six
instances, 400 lookup tables each; as a block RAM read on the clock in
each of the three units that use them, they cost three RAMB18E1 and
2 400 lookup tables less.

## What is and is not proved

Proved, by `tests/ip_aac_dec.rs` in simulation:

* **the RTL against its model, bit for bit**: 19 streams, 55 frames,
  every sample of every frame (by hash) equal to `FixedDecoder`'s, with
  bytes offered on six cycles of seven and samples taken on four of
  five; and, outside the test, all 29 development streams, every frame —
  324 frames — compared sample for sample the same way;
* **the RTL against ffmpeg, directly**: 5120 of its samples (three mono
  frames and a stereo one) within one LSB of ffmpeg's own;
* **the model against ffmpeg**: every comparable frame of the corpus
  within one LSB, at most 1 % of samples differing, noise frames within
  0.1 dB — the corpus records it and the test holds it;
* raw access units decode to the same samples as their ADTS frames;
* syncword loss, a CCE, a reserved codebook and a short ADTS frame are
  each reported with their code, produce nothing, and the frames after
  them are what the model makes of the stream without them;
* every codeword of the twelve codebooks decodes through the tree ROM to
  its own index, and the band tables have the standard's shape;
* the whole package maps for the XC7A100T with the figures above, and
  places, routes and makes a bitstream every bit of which decodes back.

That last is the flow's own check and was run by hand; no test repeats
it, since a place and route of this size takes minutes.

Quoted, not measured: the standard's text is read, not tested — where
this decoder, its model and ffmpeg all misread it the same way, nothing
here would notice. The extraction of the codebooks is checked for
completeness and agreement between two editions, not for every value
against a third source.

Not proved: anything on a part. No bitstream has been loaded; the timing
is the flow's estimate; nobody has listened to it.

## Regenerating

```sh
python3 tools/aac_dec_model.py tables ip/audio/aac_dec/rtl     # the ROMs
python3 tools/aac_dec_streams.py streams /tmp/aac             # needs ffmpeg and faac
python3 tools/aac_dec_streams.py check /tmp/aac               # model vs ffmpeg, coverage
python3 tools/aac_dec_streams.py corpus /tmp/aac testdata/ip/aac_dec_corpus.txt
```

`cargo test --release --features verilog,sim --test ip_aac_dec
dump_internals -- --ignored` with `AAC_DUMP_IN`, `AAC_DUMP_OUT` and
`AAC_DUMP_FRAMES` writes the decoder's spectra, IMDCT work RAM and PCM for
a stream, for comparing a failing frame against the model stage by stage.
