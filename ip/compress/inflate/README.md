# Decompression in a fabric, and the five decisions that are the design

`inflate` is RFC 1951 DEFLATE with RFC 1950's zlib framing around it,
decompressing. It is the first block in `ip/compress/`, and the first in
this library whose input is **somebody else's bytes**.

That changes what this page is for. Every other README here answers
"does the other end accept this", and `ip/crypto/`'s two answer "what
does this leak". A decompressor's question is neither: it is *what does
this do when the bytes are wrong*. A compressed stream is a program —
a short one, in a small language, but a program, with loops (`copy 258
bytes from 32768 back`) and memory (the window) and a reader that has to
believe the lengths it is told. The whole of §6 and §7 is about that, and
they are separate sections because one is the list of what is caught and
the other is the argument that nothing is left to wait forever on.

The rest is design. Decompression is small enough that the interesting
part is not the algorithm — RFC 1951 is eleven pages — but five choices
the RFC does not make for you, and §4 is them.

- §1 says what each kind of claim here rests on.
- §2 is the interface, and the wrapper decision.
- §3 is the corpus: 132 streams, where they came from and how they were
  chosen.
- §4 is the five design decisions.
- §5 is the window, and the distance it refuses.
- §6 is every malformed input and how it is reported.
- §7 is why it cannot be made to hang.
- §8 is area, block RAM and **measured** throughput, with the trades
  declined and their numbers.
- §9 is what this block does not do.
- §10 is what a board would add, and the cheapest experiment.
- §11 is the reading list.

---

## 1. Confidence, and what it is based on

- **HIGH** — stated in the published specification, with the section
  named so it can be checked without reading any code:
  - *DEFLATE Compressed Data Format Specification version 1.3*,
    **RFC 1951**, May 1996 — "RFC 1951" below.
  - *ZLIB Compressed Data Format Specification version 3.3*,
    **RFC 1950**, May 1996 — "RFC 1950" below.
- **MEASURED** — a number this project produced by running something: a
  byte count or a cycle count out of `sim::Simulator`, a cell count out
  of `fpga::synthesize_for`. Every one is reproducible from a clean
  checkout by the test named beside it.
- **CONFIRMED** — agreed with a second, independent implementation:
  [`compcol`](https://github.com/KarpelesLab/compcol), the user's own
  from-scratch Rust compression library, driven out of tree and read
  only. It produced every stream in `testdata/ip/inflate_corpus.txt` and
  accepted every one of them back before any of them was committed.
- **QUOTED** — taken from `zlib`'s reference implementation rather than
  from either RFC, because the RFC is silent. There is exactly one such
  decision on this page and §6 names it.
- **LOW** — inference that explains the rest, with no way to check it
  here.

There is no **CHECKED** level: nothing here has been on a board. §10
says what that would add and why it is a real gap rather than a
formality.

---

## 2. The interface, and the wrapper decision

### What the ports say

```verilog
inflate #(
    .WINDOW_BITS (15),     // 32768 bytes: RFC 1951 §3.2.5's maximum
    .WRAPPER     (1)       // 1 parses RFC 1950, 0 takes raw DEFLATE
) u_inflate (
    .clk        (clk),
    .rst_n      (rst_n),
    .start      (1'b0),    // abandon a stream in progress; tie low
    .in_byte    (comp_byte),
    .in_valid   (comp_valid),
    .in_last    (comp_last),
    .in_ready   (comp_ready),
    .out_byte   (plain_byte),
    .out_valid  (plain_valid),
    .out_ready  (plain_ready),
    .done       (finished),
    .error      (failed),
    .error_code (why),
    .busy       (busy)
);
```

A byte moves on a rising edge where `valid` and `ready` are both high,
on both ports, which is the handshake every other block in this library
uses.

**`in_last` is `sha256`'s and not AXI4-Stream's `tlast`.** It means *the
compressed stream ends at this point*, and `in_valid` says whether there
is a byte at this point. So the usual case is `in_valid` and `in_last`
together on the final byte, and `in_last` **alone** with `in_valid` low
ends a stream with no byte here — which is the only way to say "the
stream is empty", and the empty stream is a case a real caller will one
day hand over. It has to be held until `in_ready` is high, like any
other input of a handshake.

**`in_last` is not optional, and it is not decoration.** It is how a
truncated stream becomes an error instead of a wait. §7 is about that.

`done` rises only after the consumer has taken the last output byte, so
it means "all of it is yours" rather than "I have finished thinking".
`error` latches with a code; both hold until `start`.

`start` is not a required step: out of reset the block is already at the
beginning of a stream. It is there to abandon one. Hold `in_valid` low
in the cycle `start` is high, for the reason `chacha20` gives — `in_ready`
is a function of registers, so it can be high while the restart discards
the byte.

### The wrapper: both, and zlib is the default

This is the first real decision, and it went **both ways on purpose**.

**Why zlib is in.** RFC 1950 is six bytes: a two-byte header with a
compression method, a window size, a dictionary flag and a check, and a
four-byte Adler-32 of the *uncompressed* data. It costs 164 LUT4 and 32
flip-flops (§8). What it buys is the only **end-to-end** check this
block can have. Without it a corrupted stream decodes to
wrong-but-plausible bytes and nothing says so; with it, 361 of the 482
single-byte corruptions in
`inflate_refuses_or_decodes_every_single_byte_corruption` are caught by
the checksum and nothing else. Six bytes for that is not a close call.

**Why raw DEFLATE is in.** Because every container that carries DEFLATE
*except* zlib carries its own, better check: gzip (RFC 1952) and zip
both use a CRC-32, PNG uses a CRC-32 per chunk. A block that insisted on
RFC 1950 framing would be unusable inside any of them, and a block that
guessed between framings would be worse than either — a raw DEFLATE
stream can begin with bytes that pass RFC 1950 §2.2's check, so
guessing is not reliable and a wrong guess is a wrong decode. So
`WRAPPER` is a parameter and not a sniffer.

**Why gzip is not in.** RFC 1952's header is variable length — an
optional file name, an optional comment, an optional extra field, an
optional header CRC — and its trailer is a CRC-32 and a length. None of
that is decompression; it is container parsing, and the right place for
it is a block above this one that strips the frame and hands the payload
to `WRAPPER = 0`. §9 says the same about what a CRC-32 would cost.

### Byte and bit order, stated once

**HIGH** (RFC 1951 §3.1.1). Everything that is not a Huffman code is
packed **least significant bit first**: block headers, stored-block
lengths, extra bits, the code lengths of §3.2.7. A Huffman code is
packed **most significant bit first**. Getting one of those backwards is
the classic way to lose an afternoon, and `BitWriter` in
`tests/ip_library.rs` has both as two separate methods for exactly that
reason.

**HIGH** (RFC 1950 §2.2, §9). The zlib header's two bytes are plain
bytes, and the trailing Adler-32 is four bytes **most significant
first**.

---

## 3. The corpus: 132 streams, and how they were chosen

**CONFIRMED.** `testdata/ip/inflate_corpus.txt` holds 132 DEFLATE and
zlib streams that `compcol` produced, 123 of them in the tier the gate
runs and 9 in the `#[ignore]`d tier. `inflate_decompresses_the_compcol_corpus`
decodes the first 123 under up to four different consumers — 458 runs,
228 272 output bytes, every byte compared.

This is the whole method of the round, and it is the one
`ip/crypto/sha256` used with `purecrypto`: a second independent
implementation is not a specification, but **two independent things
agreeing is a measurement where one thing asserting is not**. The
difference here is that a compression library is a far better oracle
than a hash is, because its test vectors are free: compress anything,
decompress it in hardware, compare. There is no published table of
DEFLATE vectors and there does not need to be.

`compcol` is **not a dependency of this repository** for any of this. It
is already an optional one, behind `apicula`, for reading xz-compressed
Gowin databases, and this round did not touch that: the generator is an
out-of-tree program that takes `compcol` as a path dependency, calls its
public API and writes the corpus. What is committed is what it produced.

### The plaintext is a rule, not a file

Each record names a **rule** and a length rather than carrying the
plaintext, which is what lets one case be 120 000 bytes of text for a
kilobyte of committed hex. The eight rules are in `plaintext()` in
`tests/ip_library.rs` and were in the generator in the same form; each
record also carries the Adler-32 of its plaintext, and
`inflate_plaintext_rules_match_the_corpus` checks all 132 of them before
any simulation happens — so a rule that has drifted is reported as a
rule that has drifted rather than as a broken decompressor.

| Rule | What it is | What only it reaches |
|---|---|---|
| `zeros`, `ones` | one byte repeated | **distance 1**, the window's write bypass, and §3.2.5's longest length code (285, a flat 258) |
| `pattern` | byte *i* is `(i*7+13) & 0xff` | the rule `sha256`'s padding table uses; short lengths, mostly stored or fixed |
| `counter` | byte *i* is `i & 0xff` | every byte value; **all literals and no matches**, which is the slowest path |
| `lcg:n` | a linear congruential generator | **incompressible**, so the encoder stores it: §3.2.4's LEN/NLEN and the byte-aligned fast path |
| `rep:p` | `pattern` modulo a period | every match at **exactly** distance *p*, for *p* of 3, 7, 64 and 255 |
| `text:n` | English-shaped words | **dynamic Huffman**, which is what a real compressor emits for real data |
| `runs:n` | runs of one byte, 1 to 64 long | many short distances and many short matches, mixed with literals |

### The lengths, and why those

| Group | Lengths | What it is for |
|---|---|---|
| short | 0 to 9 | the bit reader holds one byte and eight bits, so this is every state of it, and every end-of-stream bit position |
| powers of two | 15, 16, 17, 31, 32, 33, 63, 64, 65 | either side of each boundary, because an off-by-one in a counter shows up on one side and not the other |
| alphabet-sized | 127, 128, 129, 255, 256, 257 | around the 256-symbol literal alphabet and the 257 boundary where length codes begin |
| run edges | 1, 2, 3, 4, 258, 259, 260, 300, 517, 1024 | 3 is §3.2.5's shortest match, 259 the shortest plaintext that needs its longest, 517 two of them |
| working sizes | 500, 1000, 2000, 4000, 4096 | long enough for a compressor to build a real dynamic code |
| bulk | 40 000, 70 000, 100 000, 120 000 | **the only ones that can cross the 32 KiB window**, and the reason they are `#[ignore]`d |

Each length is compressed both ways `compcol` frames it — RFC 1950 and
raw RFC 1951 — and at levels 1, 6 and 9 where the level changes the
answer, because level 1 is greedy matching and 4 and above are lazy, so
the two produce different streams from the same bytes.

**All three of RFC 1951 §3.2.3's block types are covered**, and the
corpus records the BFINAL and BTYPE of each stream's first block so that
it can say so rather than be assumed to: of the 123 fast cases, 24 begin
with a stored block, 84 with a fixed Huffman block and 15 with a dynamic
one, and `inflate_decompresses_the_compcol_corpus` asserts that all
three are present as the *whole* of some stream. The bulk cases are all dynamic and all
multi-block, because 120 000 bytes is more than a 16 KiB block.

### What the corpus cannot be asked for

Three gaps, named rather than left:

- **`compcol` has no API to force a block type.** It picks the cheapest
  of the three by exact bit cost per block
  (`src/deflate/encoder.rs`'s `compress_and_emit_block`), so the corpus
  reaches a type by *choosing data that makes it cheapest* —
  incompressible data for stored, tiny inputs for fixed, ordinary text
  for dynamic. That is why the rules above are what they are.
- **A non-final stored block in a short stream is not in the corpus.**
  `Flush::Sync` is the API for that, and it turned out to produce a
  stream `compcol`'s own decoder will not finish: after a sync flush,
  `finish()` reports `StreamEnd` having written two bytes, and the
  resulting stream decodes to the right 440 bytes but never reaches
  `StreamEnd` on the way back in — `UnexpectedEnd` from
  `decompress_to_vec`. That looks like a defect in `compcol`'s
  sync-flush path rather than anything to do with this block, and
  nothing here was changed for it; it is written down because the next
  round that wants multi-block vectors will meet it. The multi-block
  coverage this block does have comes from the bulk cases, which are
  several 16 KiB blocks each.
- **No stream from any other compressor.** Everything well formed here
  came out of one encoder, so an encoder-specific habit — a code-length
  encoding choice, a particular tie-break — would not be noticed. What
  covers that is §6's hand-built streams, which come from the RFC and
  from nothing else.

### Four consumers, because the consumer is the only thing that can stall

Each small case is run four times with four different `out_ready`
patterns: always high; high on every other cycle; high on nine cycles
out of a 23-step pattern; and low for three cycles after every byte
taken. The large cases get the first two.

That is not padding. **A stalled copy is the state this block is most
likely to get wrong**, and the second of those patterns found a real
defect on the first run: `inflate_window`'s `rd_valid` was `rd_en`
delayed by a cycle, so a copy that stalled for one single cycle *lost
the byte it had already fetched*, and a four-byte stream came out three
bytes long with a checksum failure. It is a one-deep valid now, cleared
by an explicit `rd_take`. Published vectors would never have found it,
and neither would a greedy consumer: it is a fault in the sequencing
around the function rather than in the function.

---

## 4. The five decisions

### 4.1 The window is not a FIFO

**HIGH** (RFC 1951 §3.2.5). A match is a length and a *distance*, from 1
to 32768, and the distance is whatever the stream says. So the window
needs a **random read at an arbitrary offset back**, which a FIFO cannot
do: a FIFO gives back its oldest element or its newest, and nothing in
between.

It is a circular buffer in block RAM. 32768 x 8 bits is 262 144 bits,
which the ECP5 flow builds as **sixteen `DP16KD`** in 9x2048 mode — that
is 2048 x 8 of 16 384 usable bits per block, so the packing is exact and
sixteen is the minimum rather than a rounding. (An 18 kbit figure would
suggest fourteen; the usable data width is 16 kbit and the parity bits
are not addressable as data, so sixteen is the number.) Two more go to
the Huffman tables, for eighteen of the LFE5U-12F's 56 — **measured**,
and the row for `inflate_window` on its own in §8 is what separates the
sixteen from the two.

A write pointer, a read pointer and one bit that remembers whether the
write pointer has ever wrapped. That last bit is the whole of §5.

### 4.2 The output asymmetry: the copy engine stalls

**HIGH** (RFC 1951 §3.2.5). The longest match is **258 bytes**, and the
shortest encoding of one is about 25 bits. So a handful of input bits
can owe the consumer 258 output bytes, and there are two honest answers:
keep 258 bytes of output headroom, or stop in the middle of the copy.

**This one stops.** `S_COPY` holds `rem_q`, the bytes still to come, and
a window read pointer, and it produces one byte a cycle for as long as
the output slot is free. When the slot fills, the copy does not advance:
no read is issued, no byte is written, `rem_q` does not move. It resumes
on the cycle the slot frees.

The cost is a nine-bit counter and a pointer. The alternative is 258
bytes of buffer — on this part that is two more block RAMs and a second
pair of pointers, and it would still have to handle the case where the
consumer stops for longer than 258 bytes, so it does not even remove the
stall. The choice is not close, and the only reason to write it down is
that the question is a real one and the answer is cheap in one direction
only.

**What the input side does while a copy is stalled.** It stops, and the
producer finds out within one cycle. A stalled copy takes no bits, so
the bit reader's one-byte slack slot stays full, so `in_ready` is low on
the next edge. Nothing is dropped, because a byte only moves on a cycle
where both sides agree. The entire amount of input in flight anywhere in
this block is **one byte and at most eight bits** — so a producer that
cannot be stalled needs `ip/memory/fifo_sync` in front, which is the
same answer `ip/bus/uart`'s transmitter and `ip/crypto/sha256` give.

### 4.3 Back-pressure: registered ready, and where the slack is

`ip/crypto/chacha20` established the rule and paid for it: **`in_ready`
must be a function of registers only**, so that two blocks back to back
cannot build a combinational path from one's `in_valid` to the other's
`in_ready`. A decompressor is where that rule is easiest to break,
because the obvious arrangement is a pipeline — bit reader, Huffman
decoder, length/distance decoder, copy engine, output — with a handshake
between each stage, and if each stage's ready depends on the next one's,
the chain runs the length of the block.

So **this is not a pipeline.** It is one sequential state machine over
one datapath: the five jobs are states, they take turns, and there is no
internal handshake to get wrong. That costs throughput (§8 has the
number) and it buys a block with exactly **two** boundaries where a
combinational readiness chain could form. Both are cut, and here is
where:

| Boundary | The slack | Why it is not a path |
|---|---|---|
| producer → bit reader | `nxt_q`, one byte, with `nxt_full_q` | `in_ready = !nxt_full_q && !last_q && !done_q && !err_q` — four registers and nothing else. No input a producer drives reaches it. |
| window → copy engine | `rd_q`/`fwd_q` inside `inflate_window`, one byte, with `rv_q` | A block RAM has no other shape: the byte is one cycle behind the address that asked for it, by construction. `rd_take` is what clears the valid, so the byte survives any number of stalled cycles. |
| engine → consumer | `out_q`, one byte, with `ov_q` | `out_valid = ov_q` alone. `out_ready` reaches register enables inside this block and stops there; it never reaches `in_ready`. |

The second row is the one that was wrong, and §3 says how it was found.

**LOW**, and worth saying: "registered ready" is a property of the
source this document can point at, and nothing in this repository
*proves* it. There is no test that asserts `in_ready`'s cone of logic
contains no input port. Reticle has the timing graph that could answer
it (`timing::graph::flatten_for_timing`) and the question is a reachable
one; it is not asked anywhere in the library yet, for this block or for
`chacha20`. That is a gap in the test suite and not in the design, and
it is the one thing from this round worth building for the whole
library rather than for one block.

### 4.4 Huffman decoding: sequential, and all three block types

**All three of RFC 1951 §3.2.3's block types are decoded**: stored
(§3.2.4), fixed Huffman (§3.2.6) and dynamic Huffman (§3.2.7). Nothing
is staged and nothing is partial; §3's corpus covers all three and §9's
list of what this block does not do does not include any of them.

The decoder is **sequential, one bit a cycle**, over the canonical-code
walk `zlib`'s own `puff.c` uses: keep the smallest code of the current
length and the number of codes of that length, compare, and go one bit
deeper if it does not match. Fifteen iterations at worst, nine or so
typically.

The alternative is table-driven: a lookup indexed by the next nine bits,
one cycle a symbol. It is **three to four times faster** on literal-heavy
data (§8) and it needs 512 entries of table.

The reason it went the sequential way is not the memory. It is that
**dynamic blocks are the common case**, and for a dynamic block the
expensive half of a table-driven decoder is not the lookup but the
*build*: 512 entries filled in from the code lengths before the first
symbol of every block. A sequential decoder needs the code lengths
sorted by code length — a counting sort — and a table build needs
exactly the same sort as its first step. So the sequential decoder is
*the table build and then nothing else*, and the table-driven one is the
sort plus a 512-entry fill. Taking the lookup table is therefore a
smaller win than its speed-up suggests on short blocks, and a real one
on long ones. §8 prices it.

The sort is four states — clear, count, prefix-sum, place — and runs
three times per dynamic block: once for §3.2.7's code-length code, once
for the literal/length code, once for the distance code. A **fixed**
block writes §3.2.6's code lengths into the same array and runs the same
sort, which costs about a thousand cycles it does not strictly need; a
one-bit `fixed_q` remembers that the tables in hand are the fixed ones,
so a run of fixed blocks pays for it once.

**One thing in it is worth a paragraph** because it is not obvious. The
sixteen-word count file holds the **counts** during the sort and the
**running offsets** afterwards, in the same sixteen words. The sort
needs counts, then a prefix sum of them, then a write pointer per
length; the decoder needs the counts back. Writing the prefix sums over
the counts leaves `cnt[len]` holding the number of symbols of length
`len` or less — from which the decoder recovers the count as
`cnt[len] - cnt[len-1]` and the symbol index as `cnt[len-1]`, and its
walk is over increasing `len` so it already has `cnt[len-1]` in a
register. One file of sixteen words instead of two, and no third pass
over the symbols.

### 4.5 Narrow registers, one bound each

`CLAUDE.md`'s rule is never to declare a register wider than the values
it holds, because an unrouted ECP5 slice input reads as a **one** and a
flip-flop that can only ever hold zero once came up holding one. This
block has a lot of nearly-wide registers, so each is sized against a
stated bound:

| Register | Width | The bound |
|---|---|---|
| `rem_q` | 9 | §3.2.5's longest match is 258 |
| `dist_q` | **16** | §3.2.5's longest distance is 32768 — see below |
| `first_q` | 16 | the smallest code of the current length, doubled per level; a code set that passes §3.2.2's Kraft check keeps it at or under 32768 |
| `left_q` | 16 | the Kraft slack, at most `2**15`, reached only by a code with nothing in it |
| `nlen_q` | 9 | §3.2.7's HLIT + 257, so 257 to 288 |
| `ndist_q` | 6 | HDIST + 1, so 1 to 32 |
| `ncode_q` | 5 | HCLEN + 4, so 4 to 19 |
| `total_q` | 9 | their sum, 258 to 320 |
| `rep_q` | 8 | §3.2.7's code 18 repeats up to 138 times |
| `state_q` | 5 | 29 states, and the `default` arm reports rather than wanders |

`dist_q` is the deliberate exception, and it is the only register here
that is wider than the window needs. At `WINDOW_BITS = 10` a legal
distance for *this instance* fits in 11 bits — but a narrower register
could not tell a distance this instance cannot reach from one no stream
may name, and **reporting the difference is the point**. §5 is that
argument in full.

---

## 5. The window, and the distance it refuses

### A distance behind the start of the stream

A stream whose first match says "copy from 500 bytes back" when ten
bytes have been produced is **malformed**, and a decompressor has only
two things it may do about it: refuse it, or invent bytes. Inventing is
what an uninitialised buffer does, and what it hands back is whatever
the *last* stream left in the block RAM — which is a disclosure and not
merely a wrong answer. A 32 KiB window holds the last 32 KiB of
somebody's data.

So the window counts. `full_q` latches the first time the write pointer
wraps; before that the history is exactly `wptr_q` bytes long and after
it is the whole window. `dist_bad` is high whenever the distance is zero
— which §3.2.5 never encodes, the smallest distance code being 1 — or
larger than that history, and `inflate.v` reports `E_DIST` instead of
opening the copy. No read is issued and no byte is produced.

`inflate_window_refuses_a_distance_it_does_not_hold` drives it directly,
which is most of why `inflate_window` is a module: **no stream can ask
for distance zero**, so that arm of the check is unreachable from
`inflate.v` and a module boundary is the only way to test it.
`inflate_reports_every_malformed_stream` covers the other half, with a
hand-built fixed block that emits one literal and then asks for a byte
100 back.

### A window smaller than the stream was compressed for

**MEASURED**, by `inflate_reports_a_distance_its_window_cannot_reach`.

A design built with `WINDOW_BITS` below 15 cannot decode every legal
stream, because §3.2.5 lets a compressor name a distance up to 32768.
There were two ways to handle that and the choice matters:

- **Refuse up front**, on RFC 1950 §2.2's CINFO field, which declares
  the window the compressor used. Deterministic and early.
- **Let it fail at the distance**, which is what this block does.

The second was chosen because the common case is a *short* file. An
ordinary compressor declares a 32 KiB window in the header whatever the
file is, so CINFO is 7 on a 300-byte file whose longest distance is 40 —
and a 1 KiB window decodes that perfectly. Refusing it up front would
make a small instance useless for exactly the data it is for. So CINFO
is checked against §2.2's own limit of 7 and not against `WINDOW_BITS`,
and the distance check catches the streams that genuinely do not fit.

The paired test is what makes that a measurement rather than an
intention: the same 4000-byte stream decodes at `WINDOW_BITS = 15` and is
refused with `E_DIST` at `WINDOW_BITS = 8`, and a stream `compcol`
compressed with `max_distance = 256` decodes at both.

It is the same comparison doing both jobs, which is why there is no
separate error code: a distance past the window and a distance past the
start of the stream are both "I do not have that byte", and the honest
answer to both is to say so.

---

## 6. Every malformed input, and how it is reported

**MEASURED**, by `inflate_reports_every_malformed_stream`: 21 hand-built
streams, every one of the seven codes, each asserting the code it should
get. The streams are built with a bit writer from the sections named,
because no compressor will produce any of them — which makes them the
one group of vectors here **derived from the specification** rather than
agreed with a second implementation.

| `error_code` | Name | What it means | Section |
|---|---|---|---|
| 1 | `E_HEADER` | CM is not 8; CINFO is over 7; FDICT asks for a preset dictionary; `CMF*256+FLG` is not a multiple of 31 | RFC 1950 §2.2 |
| 2 | `E_BTYPE` | BTYPE is the reserved 11 | RFC 1951 §3.2.3 |
| 3 | `E_NLEN` | a stored block's NLEN is not the one's complement of its LEN | §3.2.4 |
| 4 | `E_CODE` | an over- or under-subscribed code set; fifteen bits that match no code; a code-length run with nothing to repeat or one that runs past the end; a literal/length symbol of 286 or 287; a distance symbol of 30 or 31 | §3.2.2, §3.2.5, §3.2.6, §3.2.7 |
| 5 | `E_DIST` | a distance of zero, or one past what has been produced or what this window holds | §3.2.5 |
| 6 | `E_TRUNC` | `in_last` arrived while more bits were needed | — |
| 7 | `E_ADLER` | the trailing checksum did not match | RFC 1950 §9 |

Four notes on the edges of that table.

**The one thing QUOTED rather than HIGH.** §3.2.2 does not say whether
an *incomplete* code set — one that does not use its whole code space —
is legal. `zlib` refuses one, except that the literal/length and
distance codes may have a single one-bit code, which is what a block
with one distance or no matches at all produces and what real encoders
emit. This block follows `zlib`: incomplete is refused, unless every
used code is one bit long and the code is not §3.2.7's code-length code.
That rule is taken from a reference implementation and not from the RFC,
and it is the only decision on this page that is.

**Symbols that "will never actually occur".** §3.2.5's tables stop at
length symbol 285 and distance symbol 29, and §3.2.6's fixed code
nevertheless has codes for 286, 287, 30 and 31 — this block builds the
fixed distance code with 32 symbols rather than 30, which is the choice
`zlib`'s own `fixedtables()` makes and which makes the code complete.
Those four symbols therefore decode, and are reported as `E_CODE` at the
point of use rather than being left to compute a nonsense base.

**`E_TRUNC` and `E_ADLER` overlap, and the overlap is the useful part.**
A zlib stream truncated before its trailer is `E_TRUNC`; one whose
trailer is wrong is `E_ADLER`; one truncated *inside* the compressed
data is `E_TRUNC` because the decoder runs out of bits first. A raw
stream has no trailer, so `E_TRUNC` is the only thing that can catch a
truncation at all — which is why
`inflate_reports_every_truncation` sweeps a raw stream as well as two
zlib ones. All 235 prefixes of all three report `E_TRUNC`.

**What the corruption sweep found, and what it means.** For every
single-byte corruption of three streams — one of each block type, two
bit masks per position, 482 runs — the run must end with `error`, or
with `done` and the exact plaintext. 479 were reported (12 `E_HEADER`,
9 `E_NLEN`, 56 `E_CODE`, 29 `E_DIST`, 12 `E_TRUNC` and **361
`E_ADLER`**) and 3 decoded correctly, which is not a fault: a flipped
bit in the padding after the last block changes nothing a decoder reads.
None produced the wrong bytes and said `done`. The distribution is the
argument for §2's wrapper decision in one line: three quarters of the
corruptions are caught by nothing but the checksum.

A refused run's *partial* output is deliberately not checked. A
corrupted stream is a different stream — a flipped bit in a code length
changes the whole code — so the bytes produced before the error is
noticed are legitimately different bytes rather than a prefix of the
truth. The first version of that test asserted a prefix and failed on
byte 35, correctly.

---

## 7. Why it cannot be made to hang

A decompressor that reads attacker-controlled bytes and can be made to
wait forever is a defect and not a limitation. The argument that this
one cannot is short enough to check by reading, and it is in three parts.

**Every state either consumes input, produces output, or ends on a
counter.** There are 29 of them, and they fall into four groups.
`S_BITS` and `S_DEC` consume bits, and `S_ZHDR`, `S_SLEN`, `S_SNLEN`,
`S_SDATA` and `S_ADLER` consume bytes. `S_SDATA`, `S_DECDO`, `S_LIT` and
`S_COPY` produce output bytes. `S_BZERO`, `S_BCNT`, `S_BSUM`, `S_BPUT`,
`S_FIXFILL`, `S_DCLZ` and `S_DFILL` are bounded loops over counters
whose limits come from §3.2.7's header fields — `nlen`, `ndist`,
`ncode`, a repeat count — every one of which is a fixed-width field
whose range is checked, and `S_DFILL` refuses a run that would write
past `total_q`. The rest are single-cycle dispatch states, and `S_DONE`
and `S_ERR` are terminal. There is no state whose exit depends on data
it does not consume.

**Running out of input is an error and not a wait.** `in_last` says the
stream ends, and `starved` — no bit available, no byte buffered, and
`in_last` taken — reports `E_TRUNC` from any state that needs input.
That is the whole of the truncation story, and it is why `in_last` is
part of the contract rather than an optimisation.

**Waiting on the consumer is the consumer's choice.** A copy that stalls
because `out_ready` is low is not hung: it resumes when the consumer
does. That is legitimate back-pressure, and it is the one kind of
waiting this block does.

**MEASURED.** Every test here runs with a **loop bound** — a count of
simulated clock edges, not a number of seconds, so it is the same
number on every machine and reaching it is a failure of the design
rather than of the host. 458 corpus runs, 21 malformed streams, 482
corruptions and 235 truncations: 1196 runs, none of which reached its
bound.

**What that does not establish.** The bound is generous (roughly 200
cycles per input byte plus 20 per output byte), so a block that was ten
times slower than it should be on some input would pass. And nothing
here is a proof: a bounded-loop argument read off 29 states is an
argument, and the thing that would turn it into a result is a liveness
property over the state machine, which `reticle verify` could in
principle be asked for and is not asked for anywhere in this library.

---

## 8. Area, block RAM, and measured throughput

**MEASURED**, by `footprints_match_the_documentation`; the table in
[`docs/ip-library.md`](../../../docs/ip-library.md#resource-footprints)
is the authority and these are the rows of it that matter here, on an
ECP5 45F.

| | LUT4 | flip-flops | `DP16KD` | `TRELLIS_DPR16X4` | LUT depth |
|---|---|---|---|---|---|
| `inflate_adler` | 152 | 32 | 0 | 0 | 26 |
| `inflate_window`, 32 KiB | 314 | 45 | **16** | 0 | 12 |
| `inflate`, 32 KiB, zlib | 1894 | 390 | **18** | 9 | **33** |
| `inflate`, 32 KiB, raw | 1730 | 358 | 18 | 9 | 27 |
| `inflate`, 1 KiB, zlib | 1687 | 376 | **3** | 9 | 31 |

So **RFC 1950's framing costs 164 LUT4 and 32 flip-flops**, and six
levels of LUT depth. That is `inflate_adler` (152 LUT4 on its own) plus
the header check and the four-byte trailer compare, and the depth is the
checksum's two chained additions landing on the critical path — see
below. §2 argues that is cheap for what it catches and §6's 361 is the
number behind the argument.

And **the window is sixteen of the eighteen block RAMs**: at
`WINDOW_BITS = 10` the whole block fits in three, and loses 207 LUT4 and
14 flip-flops with them, because the distance comparison narrows along
with the pointers. That is what a design on a small part needs to know
and why the row is in the table.

**It does not fit an iCE40 HX1K, and the table says so rather than
implying otherwise.** 262 144 bits is 64 `SB_RAM40_4K` and an HX1K has
sixteen; the whole block asks for 67 and for 3271 `SB_LUT4` against the
part's 1280. The manifest still declares `target ice40` because
`WINDOW_BITS = 10` is five block RAMs and is a real configuration on a
larger iCE40 — but the 32 KiB window wants an ECP5 or something like it,
and the row is there so nobody has to build it to find out.

The remaining two `DP16KD` are the Huffman tables: 288 x 9 bits for the
literal/length symbols sorted by code length, and 320 x 4 for §3.2.7's
code lengths. The nine `TRELLIS_DPR16X4` are the two sixteen-word count
files, which are read asynchronously on purpose — a block RAM reads on a
clock edge and the decoder needs `cnt[len]` in the same cycle it decides.

### The critical path

**MEASURED.** 33 levels of LUT4 on the ECP5 with the zlib wrapper and
**27** without it. That is not this block's arithmetic being unusual; it
is `src/fpga/trellis` describing `CCU2C` with no port map, so no carry
cell is emitted and every multi-bit addition is a ripple of logic.
`ip/crypto/sha256`'s five-addition chain is 39 levels on the same flow
and **9** on an iCE40 where `SB_CARRY` is inferred, and a round in
parallel is fixing it.

`inflate_adler` on its own is the clearest measurement of that gap in
this package: **26 levels on the ECP5 and 20 on an iCE40**, for six lines
of modular arithmetic. The same six lines.

The adders on the path here, worst first:

1. **`inflate_adler`'s two chained sums.** `a + byte`, reduce,
   `b + a'`, reduce: two seventeen-bit additions and two seventeen-bit
   comparisons in series, 26 levels on its own, and it is **the reason
   the whole block is 33 levels instead of 27** — the six levels §8's
   table attributes to the wrapper are this. A three-input add
   (`b + a + byte`, with two conditional subtractions instead of one)
   would compute both sums in parallel and take most of it back; it was
   not done, because the current form is RFC 1950 §9 transcribed and the
   backend is being fixed rather than worked around.
2. **The decoder's comparison**, `code < first + cnt[len] - cnt[len-1]`:
   two nine-bit subtractions feeding a sixteen-bit add feeding a
   sixteen-bit compare.
3. **`dist_base + extra`** and **`len_base + extra`**, sixteen and nine
   bits, once per match.
4. The pointers: `wptr + 1` and `src + 1` at `WINDOW_BITS` bits, and
   `wptr - dist` at sixteen.

A carry chain would move all four, and this block is a reasonable thing
to measure the fix against: it has a 16-bit compare, a 17-bit add and a
15-bit counter all in one module.

### Throughput, measured and not calculated

**MEASURED**, by `inflate_throughput_by_block_type`, which is
`#[ignore]`d and prints rather than asserts — a cycle count asserted
against a constant is a test that fails the day somebody makes the block
faster.

| Data | First block | Output bytes | Cycles | **Bytes / cycle** |
|---|---|---|---|---|
| incompressible (`lcg`) | stored | 1000 | 2016 | **0.496** |
| a run of zeros | fixed | 1024 | 2160 | 0.474 |
| period-64 repetition | fixed | 4096 | 6052 | **0.677** |
| runs of one byte | fixed | 2000 | 4869 | 0.411 |
| English-shaped text | dynamic | 4000 | 15 120 | **0.265** |
| every byte value in order | fixed | 1000 | 4287 | **0.233** |

And over the whole 123-case corpus under all four consumers: **0.209
bytes per cycle**, 228 272 bytes in 1 089 618 cycles. That average is
pulled down by the stalling consumers and by the short cases, where a
block header is a large fraction of the work; the nine large cases of
`inflate_crosses_the_32_kib_window` are **0.496 bytes per cycle** over
680 000 bytes, which is the figure to quote for real data:

| Large case | Output bytes | Cycles | Bytes / cycle |
|---|---|---|---|
| 40 000 bytes of text | 40 000 | 122 242 | 0.327 |
| 120 000 bytes of text | 120 000 | 359 826 | 0.333 |
| 70 000 bytes, period 64 | 70 000 | 80 530 | 0.869 |
| 100 000 zeros | 100 000 | 111 188 | 0.899 |

The last two are the copy engine at full rate: **0.9 bytes per cycle is
one byte a cycle less the symbol decodes between matches**, which is as
fast as §4.2's design can go and confirms the copy does not stall on
itself. At a 100 MHz clock the text figure is about 33 MB/s and the
match-heavy one about 90 MB/s, but there is no measured clock behind the
100 — see §10.

The spread is the design, read backwards. A **match** is a byte a cycle
once it starts, so match-heavy data is fast. A **literal** costs one
cycle per bit of its Huffman code and one more to look the symbol up, so
nine or ten cycles a byte — which is why `counter`, every byte of which
is a distinct literal, is the slowest thing here at 0.233.

**A stored block is half a byte a cycle and should be one.** That is the
one number on this page that is a limitation rather than a trade, and
the cause is the one-byte input slack of §4.3: `in_ready` is
`!nxt_full_q`, so the producer can hand over a byte only on every other
cycle, and a stored block consumes a byte a cycle. Fixing it needs a
*two*-deep input buffer with the ready driven by a registered
occupancy comparison rather than by a per-slot flag — eight flip-flops
and a two-bit counter — and it would double the incompressible case and
change nothing else, because every compressed path needs at most one
input byte per eight cycles. It is named here and not taken; it is the
first thing to do to this block.

### The trade declined, with its numbers

A **table-driven decoder**: 512 entries indexed by the next nine bits,
one cycle a symbol instead of nine or ten. On the `text` row above that
would take 4000 bytes from 15 120 cycles to roughly 5000 — about
**three times** — and on the `counter` row rather more. It costs one more
block RAM for the table and, more to the point, a 512-entry fill before
the first symbol of every dynamic block, which on a short block costs
more than it saves.

It was not taken for this round because the sequential decoder is *the
counting sort and nothing else*, so it is the smallest thing that
decodes a dynamic block at all, and a first block in a category should
be the one that fits. The right version of the fast decoder is not the
sequential one with a table bolted on: it is a two-level table of the
shape `zlib`'s `inflate_table` builds, with a root of nine bits and
sub-tables for the longer codes, and its cost should be measured rather
than assumed.

---

## 9. What this block does not do

- **No compression.** Decompression is the small half, and deliberately
  first: a compressor needs hash-chain match finding (a 32 KiB window, a
  hash table, a chain per bucket), two passes over each block to decide
  between the three block types by bit cost, and a Huffman code
  *builder* — the package-merge or Moffat-Katajainen length-limited
  algorithm — which is a different and larger piece of work than the
  sort here. It comes later, and it will go better for this round having
  settled what the interface and the window look like.
- **No gzip (RFC 1952) and no zip.** §2 says why: the framing is
  variable length and the trailer is a CRC-32, so it is a container
  parser above this block. A CRC-32 is a 32-bit shift and an exclusive-or
  per bit, or a 256-entry table — about the size of `inflate_adler`
  again.
- **No preset dictionary.** RFC 1950 §2.2's FDICT asks for the window to
  start loaded from an agreed string; a header that sets it is reported
  as `E_HEADER`. Supporting it needs a port to pre-load the window and a
  check on the four-byte DICTID, and nothing in this library wants one.
- **No stream concatenation and no trailing garbage.** When the final
  block's checksum has been checked, the block is done; whatever follows
  is the caller's business. `compcol`'s own decoder tolerates trailing
  bytes and this one simply stops, which is the same behaviour from the
  other side.
- **No bit-granular output and no partial restart.** `start` abandons a
  stream and discards the buffered byte with it, so a caller that lost
  its input must re-send from the beginning. There is no resume.
- **No interleaving.** One stream at a time; two concurrent streams need
  two instances, which is also the honest answer, since the window is
  most of the area and there is nothing to share.
- **No decompression bomb limit.** A 1 KiB stream can legitimately
  produce a gigabyte, and this block will produce it a byte at a time
  for as long as the consumer takes them. There is no output counter and
  no cap, because the natural place for one is the consumer, which knows
  what it is willing to hold. A caller that needs the cap counts
  `out_valid && out_ready` and stops asserting `out_ready`; the block
  stalls, which §4.2 is about, and stays stalled. That is the mechanism
  and it is not a limit this block enforces — it is named here because
  "it decompressed more than I expected" is the other half of the
  attacker's toolkit and §6 does not cover it.

---

## 10. What a board would add, and the cheapest experiment

**This block has not been on a board.** It is pure logic and block RAM:
no PLL, no device primitive, no pin timing, no double-data-rate
register. Simulation sees everything the part would about *function*.

Two things a board would add that simulation cannot produce:

1. **A closed clock, and so a real throughput figure.** Everything in
   §8 is cycles. Bytes per second needs a frequency that place and route
   actually met, and on the ECP5 that number is exactly the one the
   missing carry inference would move most — so measuring it would also
   price that fix, on a block with a 16-bit compare and a 17-bit adder
   in it. The crypto round found both of its blocks giving *wrong
   answers* at 60 MHz for that reason, which is a stronger argument for
   measuring than for assuming.
2. **The eighteen block RAMs, placed.** Sixteen `DP16KD` for the window
   is a real constraint on an LFE5U-12F's 56, and whether sixteen blocks
   of one memory route at a useful frequency is not a question
   simulation asks. `fpga::synthesize_for` emits them; only place and
   route knows.

**The cheapest experiment, and the pieces already exist.** A top level
that takes a zlib stream in on `ip/bus/uart`'s receiver and sends the
plaintext back out on its transmitter: no new HDL but the top level,
one serial cable, and a host script that pipes a file through
`compcol` and compares what comes back. Both blocks have a byte-wide
ready/valid port and `inflate`'s `in_last` is the one signal the top
level has to invent — an idle timeout on the receiver, or a length
prefix. That gives the closed clock, the real bytes per second, and an
end-to-end check of the whole block on silicon, and it is the same
experiment `ip/crypto/sha256` §8 named and the crypto round then
actually ran. **Not this round: the board belongs to the carry-chain
work.**

A second, larger one would be worth it afterwards: the same thing over
`ip/usb/usb_cdc_acm`, which is a USB serial port the host's own driver
binds to, so the host side becomes `cat file.zz > /dev/ttyACM0` and the
throughput figure is not limited by a 115 200-baud link.

---

## 11. Reading list

- *DEFLATE Compressed Data Format Specification version 1.3*,
  **RFC 1951**, May 1996. §3.1.1 is the bit order, §3.2.3 the three
  block types, §3.2.4 the stored block, §3.2.5 the length and distance
  tables, §3.2.6 the fixed code, §3.2.7 how the dynamic code's own code
  lengths are run-length and Huffman coded. Eleven pages, and everything
  in this block is in it.
- *ZLIB Compressed Data Format Specification version 3.3*, **RFC 1950**,
  May 1996. §2.2 is the two-byte header and §9 is the Adler-32,
  algorithm and all.
- `zlib`'s `puff.c` — the small reference inflater, whose `decode()` is
  the walk §4.4 implements and whose `construct()` is the sort. Worth
  reading next to `inflate.v` because it is the same algorithm with a
  stack instead of a state machine.
- [`docs/ip-library.md`](../../../docs/ip-library.md), **The compress
  category, and a block whose input is somebody else's** — where each
  fact came from and what the next block in this category should
  inherit.
- `tests/ip_library.rs` — the corpus driver, the four consumers, the
  hand-built malformed streams, the corruption and truncation sweeps,
  and a *What it would and would not catch* paragraph on each.
- [`ip/crypto/sha256/README.md`](../../crypto/sha256/README.md) — the
  page this one is built to the shape of, and the one that established
  using a second from-scratch library out of tree as an oracle.
