# `adpcm_ima`

IMA/DVI ADPCM, the scheme WAV calls format `0x11` and AVI carries
beside MJPEG: four bits a sample, a fixed 4:1, one encoder and one
decoder sharing one step table.

**Nothing in this package has been run on a board.** What is proved is
what `tests/ip_library.rs` checks in simulation against ffmpeg, and
what the FPGA flow reports; see *What is and is not proved* at the end.

## Why it exists

It is the smallest compression worth having for audio and the one a
container pairs with MJPEG. The whole codec is a predictor, a step
index, and a table of 89 step sizes: no transform, no filter bank, no
psychoacoustic model, nothing to tune and nothing that needs a buffer
of future samples. That makes it the one audio codec whose correctness
is a matter of arithmetic rather than of taste, and the only one that
can be checked exactly.

The rate was never the difficulty. Audio arrives at tens of kilohertz
and each half of this answers in one cycle, so on any sensible clock
the block is idle for more than a thousand cycles out of every
thousand-and-one. There is no pipeline and no need for one.

## Five cycles a sample, four a nibble, and why not one

The arithmetic is a chain: the difference, its magnitude, three
compare-and-subtract steps, the reconstruction sum, the predictor and
its clamp. Written as one cycle of combinational logic that is six
ripple carries end to end, and it measured this:

| | one cycle | a sequencer | a sequencer, table registered |
|---|---|---|---|
| encoder, ECP5 depth | 54 | 20 | **14** |
| encoder, four-input depth | 72 | 20 | **15** |
| decoder, ECP5 depth | 14 | 14 | **14** |
| decoder, four-input depth | 28 | 19 | **15** |
| encoder, ECP5 LUT4 | 633 | 682 | 712 |
| encoder, ECP5 flip-flops | 28 | 69 | 84 |

**54 is deeper than the 19 that did not meet 60 MHz** in
`docs/fpga-trellis.md`, which is an account worth reading before
shipping any block without looking at its depth. So each half is a
sequencer with one arithmetic operation per cycle: the encoder takes a
sample in `S_IDLE` and has the nibble four cycles later, and the
decoder takes a nibble and has the sample three cycles later.

Nothing is lost by iterating. Audio arrives at tens of kilohertz, so
five cycles at even 1 MHz is forty times faster than 48 kHz needs — and
a feedback codec **cannot** be pipelined for back-to-back samples
anyway, because the next sample's difference needs this sample's
predictor. Iterating is the only shape that helps.

Three things keep each state to a single carry:

* **a comparison is the subtraction's borrow.** `acc - w` is computed
  once and its borrow says whether `acc` was the smaller, so the
  quantiser's three compares cost nothing beyond its three subtracts;
* **the clamp to sixteen bits is a test on the top bits**, not two
  comparators: a 19-bit two's-complement value fits in `int16` exactly
  when its top three bits are copies of bit 15;
* **the step table is registered.** The index only moves in `S_UPD`, so
  the lookup is done on the value the index is about to take and
  latched beside it. Left combinational, the 89-entry multiplexer sat
  in front of the subtract that uses its result and held the encoder at
  depth 20 — the same depth as the design that failed. Fifteen
  flip-flops buy it back.

### Where the area goes

The 89-entry table is most of it: 15 bits wide and 7 bits of address is
around 480 four-input lookup tables, against roughly 230 for everything
else in the encoder. A design that instantiates both halves gets two
copies, because each needs it in a different cycle and sharing one
would need arbitration between them. A device with block RAM to spare
could hold the table there instead — a registered `reg [14:0] mem
[0:88]` infers one — at the cost of a block and of only working on a
device that has one. This package does not do that, and the choice is
recorded rather than defended.

## The three modules

| module | what it is |
|---|---|
| `adpcm_ima_enc` | the encoder, and the package's `top` |
| `adpcm_ima_dec` | the decoder, its peer and not its dependency |
| `adpcm_step` | the 89 step sizes, in a module because this flow has no `include` |

A design wanting only playback instantiates `adpcm_ima_dec` and pays
for no encoder.

## Against ffmpeg

**The two halves are not verified the same way, because they are not
specified the same way.** What a decoder does with a nibble is fixed —
every decoder must agree or a stream means nothing. Which nibble an
encoder picks for a sample is a *choice*: any of the sixteen is legal
and the difference is quality, not correctness.

So:

* **`adpcm_ima_dec` is checked against ffmpeg's decoder, sample for
  sample.** `testdata/ip/adpcm_corpus.txt` holds nine cases of 512
  samples; each case's nibbles go through this block and every one of
  the 4608 samples must equal what ffmpeg produced from exactly those
  nibbles and exactly that block header.
* **`adpcm_ima_enc` is checked by round trip.** Its `predictor` output
  is its claim about the outside world — "a decoder will reconstruct
  this" — and after every sample it must equal ffmpeg's decode of the
  nibble it just emitted. That is a claim about interoperability and it
  is checked against a decoder nobody here wrote.

The encoder's nibbles are *also* compared against this project's
reference algorithm, and that assertion is weaker on purpose: it says
the RTL implements the algorithm this README describes and no more.

### Where ffmpeg's encoder disagrees, and which is better

This block implements the subtractive quantiser of the IMA reference:
compare the magnitude against the step, subtract, halve the step,
repeat three times. ffmpeg's `adpcm_ima_wav` encoder reaches a
different nibble on some samples. Measured over 2000 samples a case,
both started from the same predictor and step index, with ffmpeg 8.1.2:

| signal | nibbles differing | SNR, ffmpeg | SNR, this block |
|---|---|---|---|
| sawtooth over full scale | 307 | 2.53 dB | 2.52 dB |
| `300 * ((i % 16) - 8)` | 91 | 5.58 dB | 5.43 dB |
| uniform noise, full scale | 0 | 15.50 dB | 15.50 dB |
| a sine at about f/9 | 583 | 27.86 dB | 27.85 dB |
| **uniform noise, plus or minus 50** | 594 | 12.36 dB | **15.41 dB** |
| three summed sines | 329 | 23.55 dB | 23.54 dB |

Within 0.15 dB everywhere except a quiet signal, where this block is
**3 dB better**. They agree exactly on full-scale noise, where the
difference between samples is large against the step and every rule
saturates at seven.

`-trellis 1` is a third answer and a better one — 39.3 against 47.4 in
sum of squares on a square wave — because it searches over future
samples. It needs a buffer of them and this block has neither the
buffer nor the search.

## Writing a WAV block

The format transmits the codec's state, so a file is a sequence of
blocks each of which restarts from its own header. Per channel:

```text
    offset  0  predictor, 16-bit signed little-endian: the block's FIRST
               sample, emitted as-is and not encoded
            2  step index, 0 to 88
            3  zero
            4  the nibbles, two to a byte, LOW nibble first
```

and the `fmt ` chunk is 20 bytes: `wFormatTag` 0x11, `wBitsPerSample`
4, `cbSize` 2 and `wSamplesPerBlock` set to
`(nBlockAlign - 4 * nChannels) * 8 / (4 * nChannels) + 1` — the `+ 1`
being the header's own sample.

So a caller emits the first sample into the header, calls `load` with
that sample and the step index it wrote, and encodes the rest. `load`
touches the state and **not** the output slot, so it must be asserted
between blocks with no nibble pending; asserting it while `out_valid`
is high leaves a nibble of the previous block in the slot, which is a
caller error this block cannot detect.

A stereo stream wants two instances: the per-channel state is
independent and WAV interleaves the two channels' nibbles in four-byte
groups.

## The step table

89 entries from 7 to 32767, a geometric progression in 1.1 with each
entry rounded. `adpcm_step_is_a_rounded_geometric_progression` checks
every entry against that rule and pins both ends, which is the only
check on the table that is **not a second copy of it**: a table
compared against a table typed out again catches a typo in one of them
and nothing else.

What that would not catch is a pair of adjacent entries swapped whose
ratio stayed inside the tolerance. Nothing here would, and the corpus
is what covers it — a wrong step moves every sample after it.

## What is and is not proved

Proved, by `tests/ip_library.rs`:

* **the decoder, against ffmpeg's**: 4608 samples over nine cases,
  every one exact, with the handshake stalled every fifth nibble so the
  one-deep output slot is exercised and not assumed;
* **the encoder's predictor, against ffmpeg's decode of its own
  nibbles**: the same 4608, every one exact, stalled every seventh
  sample;
* **the two halves against each other** over 1024 samples the corpus
  does not contain — 256 of full-scale alternation, which pins the
  predictor at both rails and the step index at the top of the table,
  then a slow walk through zero where the step is 7 and the floor of
  step/8 decides everything, then 512 of noise;
* a block header's step index above 88 — a value a file can legally
  carry, since the field is a byte and the table has 89 entries — held
  at 88 rather than read off the end of the table;
* the sample rules this corpus was generated from, by checksum, so a
  rule that drifts is reported as a rule mismatch and not as a codec
  fault;
* both halves synthesise with no diagnostic and no inferred latch, and
  map for the iCE40 and the ECP5 with the footprints
  `docs/ip-library.md` records.

Not proved, and not claimed: that the encoder's *choice* of nibble is
good, which is the table above rather than a test, and which is a
question about signal-to-noise ratio and not about correctness. Nor
anything about stereo, block framing, RIFF parsing or nibble packing,
none of which is in this package. Nor anything on a part: no bitstream
containing either half has been built or loaded, and nobody has
listened to it.
