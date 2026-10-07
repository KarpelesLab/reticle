# SHA-256 in a fabric, and the padding nobody should have to write twice

`sha256` is FIPS 180-4 SHA-256 over a byte stream. It is the first of the
two blocks in `ip/crypto/`, and the `crypto` category is the first in this
library whose subject makes a claim about an **attacker** rather than about
a protocol. That changes what a page like this is for.

Every other README here answers "does the other end accept this". A
specification says what a descriptor looks like; a host either binds or it
does not; the page records which. A crypto block has that question too —
and it has an easy answer, because a digest either matches a published
vector or it does not — but it also has a question no vector answers:
**what does this block leak, and to whom.** The honest answer is shorter
than one would like, and §4 and §5 are the two halves of it: §4 is the
property that was measured, §5 is everything that was not. They are
separate sections on purpose. A document that mixes them is a document
that implies a defence it does not have.

- §1 says what each kind of claim here rests on.
- §2 is the interface and the one real design choice: whether padding is
  the block's job. It is, and §2 says why, and says what the block that
  does not pad is for.
- §3 is the vectors, by section, and which of them came out of a document
  and which out of a second running implementation.
- §4 is the constant-time property, **measured**.
- §5 is what is not defended against.
- §6 is what the block does not do.
- §7 is area and throughput, with the trade that was declined and its
  numbers.
- §8 is what a board would add, and the cheapest experiment.
- §9 is the reading list.

---

## 1. Confidence, and what it is based on

- **HIGH** — stated in the published standard, with the section named so it
  can be checked without reading any code:
  - *Secure Hash Standard (SHS)*, **FIPS PUB 180-4**, August 2015 —
    "FIPS 180-4" below.
- **MEASURED** — a number this project produced by running something: a
  digest out of `sim::Simulator`, a cycle count, a cell count out of
  `fpga::synthesize_for`. Every one of these is reproducible from a clean
  checkout by the test that is named beside it.
- **CONFIRMED** — agreed with a second, independent implementation:
  `purecrypto`, the user's from-scratch Rust cryptography library, driven
  with the same inputs out of tree. `purecrypto` is **not** a dependency of
  anything in this repository — there are no third-party crates here — and
  what is committed is the numbers it produced.
- **LOW** — inference that explains the rest, with no way to check it here.

There is no **CHECKED** level on this page in the sense
`ip/usb/usb_cdc_acm/README.md` means it. Nothing here has been on a board.
§8 says what that would add and why it is a real gap rather than a
formality.

---

## 2. The interface, and whose job the padding is

### What the ports say

```verilog
sha256 #(
    .LEN_BITS (61)         // bits of the message byte counter
) u_hash (
    .clk          (clk),
    .rst_n        (rst_n),
    .start        (1'b0),  // abandon a message in progress; tie low
    .in_byte      (byte_in),
    .in_valid     (byte_valid),
    .in_last      (byte_last),
    .in_ready     (byte_ready),
    .digest_valid (done),
    .digest       (digest),
    .busy         (busy)
);
```

A byte moves on a rising edge where `in_valid` and `in_ready` are both
high, which is the handshake every other block in this library uses.

**`in_last` is not AXI4-Stream's `tlast`, and the difference matters.**
Here it means: *the message ends at this point in the stream*, and
`in_valid` says whether there is a byte at this point. So the usual case is
`in_valid` and `in_last` together on the final byte — which is what `tlast`
means too — and `in_last` **alone**, with `in_valid` low, ends a message
with no byte here. That second form is the only way to express the empty
message, whose digest is a real value a real caller will one day want.
`in_last` has to be held until `in_ready` is high, like any other input of
a handshake.

**HIGH** (FIPS 180-4 §5.1.1). Padding appends a single `1` bit, then the
fewest zeros that leave room, then the message's length as a 64-bit
big-endian count **of bits**.

**MEASURED.** `digest_valid` is one cycle; `digest` then **holds** until the
next message's digest replaces it, so a consumer does not have to catch a
pulse. That costs 256 flip-flops the block would not otherwise need, and
the alternative was a contract of the form "the digest is valid until the
next message has supplied 64 bytes", which is free and is exactly the kind
of subtlety a caller gets wrong once and never finds. The flip-flops are in
§7's table.

`start` is not a required step. Out of reset the block is already at the
beginning of an empty message, so a design that only ever hashes whole
messages ties `start` low and never thinks about it. It is there to abandon
one, which is what a caller who lost its input stream needs.

### The byte order, stated once

In both blocks of `ip/crypto/`, **a byte string has its first byte at the
most significant end.** So `digest[255:248]` is the first byte of the
digest, which is the byte `sha256sum` prints first. That is the order a
standard prints a value in, which means a vector copied out of FIPS 180-4
goes straight into a comparison with no reversal step to get wrong.

### Padding: the choice, and why it went this way

This is the one decision in the block that could reasonably have gone the
other way, and it has a cost either way.

**The case for leaving it out.** A core that takes already-padded 512-bit
blocks needs no byte counter, no pad state machine and no notion of where a
message ends. It is **330 flip-flops and about 164 LUT4 smaller** (§7), and
it is the right thing for a caller that already has whole blocks — a Merkle
tree over fixed-size nodes, a protocol whose record length is known up
front. So that core exists, as `sha256_core`, and a design that wants it
instantiates it and pays for none of this file.

**The case for putting it in.** Padding is where SHA-256 implementations go
wrong, and they go wrong in one specific place. A message whose length is
56 bytes past a block boundary leaves 8 bytes before the end of the block
and the length field needs all 8 — so the `1` bit has nowhere to go, and
the padding spills into a **whole extra block**. FIPS 180-4's own
Appendix B.2 example is exactly 56 bytes long. That is not a coincidence;
it is the standard pointing at the trap.

A library block that leaves that to every caller ships that bug once per
caller. One that does it itself ships it once, and
`sha256_pads_every_length_purecrypto_was_asked_about` drives nineteen
lengths through it — 55 (no zeros at all), 56 (the spill), 57 to 63 (the
spill with the zeros wrapping a block boundary), 0 mod 64 (a whole block of
message and then a whole block of nothing but padding), and three lengths
past two blocks.

So: **both, and the one that pads is `top`.** The decomposition is checked
rather than asserted — `sha256_core_compresses_a_block_somebody_else_padded`
pads `"abc"` by hand in Rust from §5.1.1, feeds the result to the bare core,
and gets B.1's digest. That is the test that would catch a core which only
works behind its own padder.

### Why the port is eight bits wide

**MEASURED, and it is the pleasing part of this block.** The compression
takes 64 cycles, one round per cycle. Sixty-four bytes arrive in 64 cycles
at one byte per cycle. The two are **the same number**, so a byte-wide port
is not the bottleneck and a wider one would not help: a 32-bit port would
cut the load from 64 cycles to 16, taking a block from 129 cycles to 81 —
37 per cent — in exchange for four times the input wiring and a byte-enable
encoding on the final word, which is one more thing to get wrong on exactly
the message lengths §2 says are already the hard ones.

The rule here is the one `ip/crypto/chacha20` applies too, and it lands
somewhere else there: **the port width is the width at which the port stops
being the limit.** ChaCha20's core makes 64 bytes in 22 cycles, so its port
is 32 bits. Same rule, different core, different answer.

### Why one round per cycle

**HIGH** (FIPS 180-4 §6.2.2). T1 is
`h + Σ1(e) + Ch(e,f,g) + K(t) + W(t)` — five 32-bit additions in series.

**MEASURED.** That chain is the critical path of this block: 39 levels of
LUT4 on a flow with no carry cell, 9 levels of `SB_LUT4` on an iCE40 where
`SB_CARRY` is inferred (§7). Two rounds per cycle would make it ten
additions deep to halve the cycles, which trades the clock for the cycle
count at about one for one and buys nothing. One round per cycle.

### The counter that is 61 bits and not 64

**HIGH** (FIPS 180-4 §5.1.1). The appended length is a count of **bits**,
64 bits wide.

This interface is bytes, so the low three bits of that count are
necessarily zero. The block counts **bytes** in a 61-bit register and
concatenates three hard zeros, rather than declaring three flip-flops that
could only ever hold zero. That is this repository's rule — never a
register wider than the values it holds — applied to the one place in this
block where it is not obvious, and `CLAUDE.md` says where the rule came
from and what it cost to learn.

`LEN_BITS` lowers it. At 32 the block hashes messages up to four gigabytes
and the counter is 29 flip-flops smaller; §7 has the measured difference,
which is **70 LUT4 and 29 flip-flops**. Exceeding the range wraps the
counter silently and produces a wrong digest, which §6 lists as the
limitation it is.

---

## 3. The vectors, by section

### Out of the standard

**HIGH.** Every one of these is in FIPS 180-4, cited by its own appendix,
and each is in `tests/ip_library.rs` with the citation beside it.

| Vector | Message | Blocks | What only it reaches |
|---|---|---|---|
| Appendix B.1 | `"abc"`, 3 bytes | 1 | the round, the schedule, the constants, the byte order |
| Appendix B.2 | the 56-byte string | **2** | the **padding edge**: no room for the length field, so the padding spills |
| Appendix B.3 | one million `'a'` | **15 626** | the chaining value over four figures of blocks, and a length field with bits above the low sixteen |

B.1 and B.2 together are the pair that matters, and the reason is that B.1
is one block and B.2 is two: a chaining value that does not carry from one
block to the next passes B.1 and fails B.2.

**MEASURED.** B.3 runs as an `#[ignore]`d test, because 15 626 blocks at
129 cycles is about two million clock edges and that is minutes rather than
the two seconds the rest of this section takes. It was run:

```
FIPS 180-4 B.3: 2015755 cycles for 1 000 000 bytes
cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0
```

A million bytes is 15 625 whole blocks, so the padding needs a
**15 626th** — the case §2 calls 0 mod 64 — and 15 626 x 129 + 1 is exactly
2 015 755. Two million cycles for a million bytes is §7's 0.496 bytes per
cycle arrived at the long way round.

### Out of a running implementation

**CONFIRMED.** Two things on this page rest on `purecrypto` rather than on
FIPS 180-4, and it is worth being exact about which.

The **empty message**. FIPS 180-4 Appendix B does not give it.
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` is
published in a great many places, is derivable from §5.1.1 by hand — one
block, `0x80` then 55 zeros then eight more — and is what `purecrypto`
computes for the empty input on this machine. It is in the test beside the
two FIPS vectors, labelled as what it is.

The **nineteen-length padding table**. No published document gives nineteen
digests of an arbitrary message, and nineteen is what it takes to cover
every branch of the padding. So the expectations came from `purecrypto`
driven over the same `crypto_pattern` rule the test uses — byte *i* is
`(i * 7 + 13) & 0xff`, which is neither zeros nor text and is cheap to
restate on both sides.

**What earns that trust:** `purecrypto` reproduced all three FIPS vectors
and the empty message byte for byte, and all nine of RFC 8439's ChaCha20
vectors for the other block in this category. By the time it is used as an
oracle it has agreed with both authorities everywhere both of them speak.
That is the same method that validated a 6502 monitor against a real ROM
earlier in this project: a second implementation is not a document, but
two independent things agreeing is a measurement where one thing asserting
is not.

---

## 4. Constant time: what was measured

The doctrine comes from `purecrypto`'s own foundation, stated in its `ct`
module: *every operation here runs in time independent of the secret values
it touches, so higher layers can be built without secret-dependent branches
or memory accesses.* In hardware a secret-dependent branch becomes a
secret-dependent **cycle count**, and a secret-dependent memory access
becomes a secret-dependent **address**. Both are checked.

### Fixed latency

**MEASURED**, by `sha256_takes_the_same_cycles_whatever_the_message_says`.
Nine message bodies as different as bodies of one length can be made — all
zeros, all ones, `0xaa`, a counting pattern, its complement, one bit set at
the start, one at the end, two more arithmetic patterns — at each of six
lengths: 0, 1, 55, 56, 64 and 130. Those lengths are chosen to put both
sides of the padding edge in. The assertion is that **every body of a given
length takes the same number of cycles** from `start` to `digest_valid`.

Two things make that assertion worth something rather than vacuous:

- the driver holds `in_valid` high for every byte with no gaps, so the
  cycle count is a property of the block and not of the testbench's
  back-pressure;
- the test also asserts that the nine digests were **all different**
  (except at length 0, where there is only one empty message). Nine equal
  cycle counts from nine identical runs would prove nothing at all.

### What the cycle count does depend on

The **length**. 129 cycles per padded block, and the number of padded
blocks follows from the message length. That is not a leak worth chasing: a
caller streams the bytes in through a handshake, so the byte count is
visible on the interface whatever the block does inside, and no block could
hide it. A protocol that needs to hide a message length pads the message
before it reaches here.

### No secret-dependent addressing

**MEASURED**, by `crypto_blocks_hold_no_memory_to_index`, which synthesises
every variant in the footprint table and asserts the module holds **no
memory array at all**.

The one table in SHA-256 is the sixty-four K constants of FIPS 180-4
§4.2.2. It is written as a `case` over the round counter, which counts 0 to
63 and is a function of nothing but the clock, and synthesis turns it into
logic. So there is nothing in this block a secret byte *could* index,
because there is no indexable storage for it to index.

**And here is what that test does not establish.** It does not establish
that no *multiplexer* is selected by a secret. A one-hot mux over sixteen
words whose select came from key material would pass it and would still be
a timing channel in a netlist, because it is not a memory. Nothing in this
block does that — every select here is `round_q`, `fill_q`, `pos_q` or
`lenix_q`, all counters — but **this test does not prove it and no test in
this repository does.** Proving it would take a taint analysis from the
input ports forward through the IR, which Reticle has not got. That is the
gap, and it is named rather than papered over.

---

## 5. What this does not defend against

Everything in §4 is about **logical** time: how many clock edges pass. None
of it is about what the part does during those edges, and that is where the
attacks on this primitive actually are.

- **Power analysis.** A SHA-256 round is five 32-bit additions whose
  operands are message and chaining-value bits. The current the fabric
  draws in a cycle depends on how many flip-flops toggled in it, which
  depends on those bits. Differential power analysis works on a block with
  a perfectly fixed cycle count — the cycle count is not what it measures.
  **Nothing in this repository has recorded a power trace of this block,
  and nothing in it is masked, randomised, duplicated or otherwise
  hardened against one.**
- **Electromagnetic emission.** The same argument with a probe instead of a
  shunt, and no measurement either.
- **Gate delay.** §4's measurement is a cycle count in a four-state
  zero-delay simulation. It says the *control path* is data-independent. It
  says nothing about whether the data path's propagation delay is — a
  carry that ripples further for one operand than another is real, and
  invisible here. It does not break the cycle count (the clock period is
  the same either way), but it is a physical quantity this page has not
  measured.
- **Fault injection.** No redundancy, no checking, no detection. A glitched
  clock or an undervolted core will produce a wrong digest and say nothing.
- **Key storage.** This block hashes; it holds no key. But a caller using
  it inside HMAC holds one, and nothing here zeroises anything: the message
  schedule `w_q` keeps the last block's bytes until the next block
  overwrites them, and `wk_q` keeps the last round's working variables
  indefinitely. A design that cares pulses `start`, which does **not**
  clear either of those — it clears the chaining value. §6 lists that as
  the gap it is.

**The short version: this block is for integrity, not for secrecy against
an attacker holding the board.** If an adversary has physical access to the
part, nothing on this page helps.

---

## 6. What this block does not do

- **No bit-granular messages.** FIPS 180-4 §5.1.1 pads a bit string; this
  interface is bytes, so a message of 11 bits cannot be expressed. Nothing
  in practice wants one.
- **No SHA-224.** Same compression function, a different H(0), a truncated
  digest. It would be an initial-value parameter and a mux and it would
  change no port, so adding it later costs nothing. Nothing here needs it.
- **No SHA-512.** Different word width, different constants, different
  rotations, a 1024-bit block and a 128-bit length field. Not a parameter;
  a second block.
- **No HMAC.** HMAC needs two passes with derived keys, a key-length
  reduction and a stored inner state, and the right place for it is above
  this block rather than inside it. `sha256_core`'s existence — a
  compression function that can be fed arbitrary blocks and whose chaining
  value is a port — is most of what an HMAC wrapper needs.
- **No length-range error.** Exceeding `2^LEN_BITS - 1` bytes wraps the
  counter and produces a wrong digest **silently**. At the default of 61
  the limit is FIPS 180-4's own, so the only way to reach it is to lower
  the parameter deliberately.
- **No zeroisation.** See §5. `start` resets the chaining value and the
  padding state; it does not scrub the message schedule or the working
  variables.
- **No interleaving.** One message at a time. Two concurrent streams need
  two instances, which is also the honest answer: the state is most of the
  area and there is nothing to share.
- **No back-pressure relief.** `in_ready` is low for the 65 cycles a block
  compresses in, so a producer that cannot wait needs `ip/memory/fifo_sync`
  in front. That is the same answer `ip/bus/uart`'s transmitter gives, for
  the same reason.

---

## 7. Area, throughput, and the trade that was not taken

**MEASURED**, by `footprints_match_the_documentation`; the table in
[`docs/ip-library.md`](../../../docs/ip-library.md#resource-footprints) is
the authority and these are the rows of it that matter here.

| | LUT4 | LUT6 | iCE40 HX1K `SB_LUT4` | flip-flops | LUT depth (LUT4 / iCE40) |
|---|---|---|---|---|---|
| `sha256_core` | 3041 | 2013 | 2402 | 1039 | 39 / **9** |
| `sha256` | 3205 | 2270 | 2557 | 1369 | 39 / **9** |
| `sha256`, `LEN_BITS=32` | 3135 | 2219 | 2501 | 1340 | 39 / **9** |

So the padder costs **164 LUT4 and 330 flip-flops** over the bare core, of
which 256 flip-flops are the held digest register §2 argues for. And
`LEN_BITS=32` buys back **70 LUT4 and 29 flip-flops**, which is what a
29-bit slice of a counter and its carry chain are worth.

**The two depth figures are the same path counted twice.** On the iCE40 the
five-addition T1 chain is 9 levels, because `SB_CARRY` is inferred and a
32-bit add is one carry chain. On the generic LUT4 and LUT6 mappings and on
the **ECP5**, it is 39, because neither of those flows emits a carry cell
and a 32-bit ripple-carry add is twenty-odd levels of logic. That is a
property of `src/fpga/trellis`, not of this block — every arithmetic block
in the library pays it — and it is the single change that would most move
this block's achievable clock on an ECP5. It is noted and not made; `src/`
belonged to another round.

### Throughput

**MEASURED.** 64 bytes per 129 cycles: 64 to load, 64 rounds, one to add
into the chaining value. **0.496 bytes per cycle.** At a 100 MHz clock that
is 49.6 MB/s, 397 Mbit/s. The B.3 run in §3 is 2 015 623 cycles for a
million bytes, which is that figure arrived at over 15 626 blocks.

There is no measured clock behind the 100 MHz: it is a round number to
multiply by, and §8 is about why a real one needs a board.

### The trade declined

A **second 512-bit buffer** would let the next block load while this one
compresses. 64 bytes per 65 cycles instead of 129: **0.985 bytes per
cycle**, within two per cent of twice as fast. It costs 512 flip-flops —
more than half of everything `sha256_core` holds, and more than the whole
rest of this block — and on an iCE40 HX1K a flip-flop comes with a logic
cell whether the logic is used or not.

It was not taken. The reason is that it doubles the smallest useful
instantiation to buy a factor of two in a block that is not the bottleneck
in anything this library builds, and a block that cannot be instantiated on
a small part is a block nobody uses. A design that is actually limited by
hashing rate should want more than a factor of two anyway, and the thing to
build for it is a different core: two rounds per cycle *and* double
buffering, with the clock cost measured rather than assumed.

The equivalent trade in `ip/crypto/chacha20` was declined for the same
reason with the same arithmetic, which is written up there.

---

## 8. What a board would add, and the cheapest experiment

**This block has not been on a board, and it does not need one to be
correct.** It is pure logic: no PLL, no device primitive, no pin timing, no
double-data-rate register. Simulation sees everything the part would about
*function*, which is why `ip/usb/usb_device_ulpi`'s warning about
simulation not simulating an IO register does not apply here.

Two things a board would add that simulation cannot produce:

1. **A real throughput figure.** Everything in §7 is cycles. Bytes per
   second needs a clock that place and route actually closed, and on the
   ECP5 that number is exactly the one the missing carry inference above
   would move most — so measuring it would also price that fix.
2. **A power trace.** It is the only way the side channel §5 disclaims
   could be characterised at all. Without it, §5 is a statement that
   nothing was checked, which is the honest thing to say and is not a
   result.

**The cheapest experiment, for the first:** `sha256` with its byte port fed
from `ip/bus/uart`'s receiver and its digest clocked back out through the
transmitter. No new HDL but a top level — both blocks exist and both have a
byte-wide ready/valid port — one serial cable, and a host that compares
what comes back against `sha256sum`. That gives the closed clock, the real
bytes per second, and an end-to-end check of the whole block on silicon in
one afternoon. It would also be the first time anything in `ip/crypto/`
computed a digest outside a simulator.

The second needs a shunt resistor, an oscilloscope and a trigger, and is
not an afternoon. It is also the only one of the two that would let §5
become a measurement instead of a disclaimer.

---

## 9. Reading list

- *Secure Hash Standard (SHS)*, **FIPS PUB 180-4**, August 2015. §4.1.2 and
  §4.2.2 are the functions and the constants, §5.1.1 the padding, §5.3.3
  the initial value, §6.2.2 the round, and Appendix B the three worked
  examples. Everything in this block is in it.
- [`docs/ip-library.md`](../../../docs/ip-library.md), **The crypto
  category, and what a constant-time claim is worth** — the doctrine both
  blocks are built to, where each fact came from, and the seven things AES
  should inherit from this round.
- [`ip/crypto/chacha20/README.md`](../chacha20/README.md) — the other block,
  and the one whose §6 is about the thing it is **not** (an AEAD).
- `tests/ip_library.rs` — the vectors, the padding table, the fixed-latency
  measurement, and a `What it would and would not catch` paragraph on each.
- `purecrypto`'s `ct` module — the constant-time doctrine in its original
  form, and worth reading for the sentence about `black_box` being
  best-effort, which is the software analogue of this page's §5.
