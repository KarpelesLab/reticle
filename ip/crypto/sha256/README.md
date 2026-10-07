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
- §8 is **what the board said**, which is **no** — the design runs on an
  ECP5 and both cores give wrong answers on it, and §8 is the measurement
  that says why and the one figure it confirmed.
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
- **CHECKED** — observed on a real part, in the sense
  `ip/usb/usb_cdc_acm/README.md` means it. There is exactly one such claim on
  this page and §8 is it, and it is **negative**: this block has run on an
  ECP5 at 60 MHz and the digests it produced there were wrong, and wrong
  differently between identical runs. §8 says what that measures — a data
  path with no carry cell under it — what it does not (§3's vectors still
  pass and §5's list is untouched by it), and the one figure the part
  *confirmed*.
- **LOW** — inference that explains the rest, with no way to check it here.

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
belonged to another round, and it still does.

**That sentence has now been tested rather than argued.** §8's design runs
this block at 60 MHz on an ECP5 with all thirty-nine levels in the clock
period, and **it does not**: §8's design runs this block at 60 MHz on an
ECP5 — the clock that board's USB transceiver forces and the only one
available — and the digests that come out are wrong, and wrong differently
between identical runs. Thirty-nine levels of LUT4 do not settle in 16.67 ns
on this part. That sentence has now been tested rather than argued, and it
turned out to be an understatement: inferring the carry cell is not a change
that would move this block's achievable clock, it is the change that stands
between this block and working on this family at all.

### Throughput

**MEASURED.** 64 bytes per 129 cycles: 64 to load, 64 rounds, one to add
into the chaining value. **0.496 bytes per cycle.** At a 100 MHz clock that
is 49.6 MB/s, 397 Mbit/s. The B.3 run in §3 is 2 015 623 cycles for a
million bytes, which is that figure arrived at over 15 626 blocks.

There is no measured clock behind the 100 MHz: it is a round number to
multiply by, and it stays because every cycle figure on this page has always
been multiplied by it.

**The 0.496 is measured on a part now, and it came out 0.494.** §8 has the
run: sixteen mebibytes and 262 144 blocks of it, on an ECP5 at 60 MHz,
**29.63 MB/s**, which is four tenths of a per cent off what a four-state
zero-delay simulator computed. That is this page's one claim the board
*confirmed* — and it is a claim about the **control** path, which is the half
of this block that closes 60 MHz. The other half does not, and §8 is about
that.

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

## 8. What the board said

**CHECKED**, and it is the most important section on this page, because what
the board said is **no**.

The design is `testdata/fpga/cynthion/usb_crypto_console.v`: this block and
`ip/crypto/chacha20` behind `ip/usb/usb_cdc_acm`, so a `/dev/ttyACM*` the
kernel's own `cdc_acm` driver binds answers typed lines.
`testdata/fpga/cynthion/crypto_console.v` is the protocol and
`tests/usb_crypto_console.rs` is the test. The part is a Great Scott Gadgets
Cynthion r1.4, an ECP5 `LFE5U-12F-8CABGA256`, serial
`35L6H2CMGJJVCIBAEA3GCLAN74`, at **60 MHz** — the board's own oscillator, which
is not a choice: `ip/usb/usb_device_ulpi` needs exactly that, so there is no
slower clock to retreat to. Linux 6.18.41-gentoo, `xhci_hcd`, the device on a
full-speed downstream port of a hub. Everything below is **quoted**.

This replaces the section that used to be here, which proposed `ip/bus/uart` and
a cable as "the cheapest experiment" and said it would give "the closed clock,
the real bytes per second, and an end-to-end check of the whole block on silicon
in one afternoon". Two of those three happened. The third is the headline and it
went the other way.

### Three bitstreams, because the one with both cores does not route

The design with both cores is 11 955 lookup tables, and `reticle fpga
--bitstream` **does not converge** on it: after 40 ripup iterations and an hour
and fifty minutes,

```text
error: routing did not converge: 886 node(s) are still oversubscribed after 40
iteration(s), worst at X26Y1/H06E0303 (2 signals), ... X45Y2/V02S0701 (2 signals), ...
  X45Y2/V02S0701 is a control wire of X45Y2: 17 of that tile's bels have a pin
  that can only be reached through it, and 2 signals were routed onto it —
  `u_console.u_hash.u_core.h_q$ff$mux[191]` and
  `u_console.u_hash.u_core.w_q$pmux[23]`.
```

The router's own message says the placer rejects that arrangement before it is
made (`SiteRules` in `src/fpga/place.rs`), so a design that reaches it has found
a control wire the architecture does not describe yet. **That is a `src/fpga`
fact, reported here rather than fixed**, and the named wire is the thing to act
on.

So `crypto_console` takes `WITH_HASH` and `WITH_CIPHER`, and the two single-core
builds route in minutes:

| build | LUT4 | flip-flops | depth | routing |
|---|---|---|---|---|
| both cores | 11 955 | 2 999 | 87 | **gives up after 40 iterations, 1 h 50 m** |
| `WITH_CIPHER=0` | 5 891 | 2 378 | **39** | 8 353 of 8 353 signals, 11 m 29 s |
| `WITH_HASH=0` | 8 375 | 1 652 | **87** | 10 111 of 10 111 signals, 4 m 50 s |

Both bitstreams have every set bit decoding back through the database with
nothing unexplained — 231 610 bits into 77 578 arcs for the first, 279 362 into
88 667 for the second — and the arcs they select are exactly the arcs the router
chose. **The bitstreams are not in question.**

### What works on the part, and it is most of it

```text
$ stty -F /dev/ttyACM1 115200 raw -echo clocal min 0 time 10
$ ?  -> H t|h x|E x|e n|Z n|X n|K x|N x|C x|k|n|c|?
$ k  -> 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
$ n  -> 000000000000004a00000000
$ c  -> 00000001
$ C 00000007 -> OK
$ c  -> 00000007
$ K 00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff -> OK
$ K 00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff0 -> ERR
$ k  -> 0112233445566778899aabbccddeeff00112233445566778899aabbccddeeff0
$ Q  -> ERR
$ h 6 -> ERR
```

The kernel bound `cdc_acm` and gave the device `/dev/ttyACM1`; the help line is
exact; the 256-bit key, the 96-bit nonce and the 32-bit counter come out of reset
holding RFC 8439 §2.4.2's values and read back byte for byte; a typed counter
reads back; a 65-digit `K` answers `ERR` and leaves the key shifted up by one
nibble, which is **exactly** what `crypto_console.v`'s header says it will do and
is the first time that paragraph has been checked anywhere; and five kinds of
malformed line answer `ERR` with the console still working after them.

So: the USB stack, the class layer, the bulk endpoints, the parser, the padding
state machine's *control*, the string table, the three wide registers and their
rotating hexadecimal printer, and every error path — all of it closes 60 MHz and
all of it is right.

### And the digest is wrong, and **not the same wrong twice**

```text
$ h  -> 9182a9ff537747493eac092f706c8757600082bda13a5417e8cd6be261a66ca1
$ h  -> 0db02563334a2035247386933558969ae09e6e8c9e9c12777520e6b554ea000b
$ H abc -> 2bd8ce53a22d6b45c22e2189592fcd7857370a886191c1eae1a490024cc24f52
$ H abc -> e9880856b005b3ed25948cf610081632b3171c6f7cb8e4d74218c758b3b89f7e
$ Z 40 -> 5f31796850a3eb89abdf34b31d55e427a4164906883b64b0a85aa39f3a7cdbf1
```

`ba7816bf…` is what `H abc` should answer, `e3b0c442…` is what `h` should and
`f5a5fd42…` is what `Z 40` should; none of them is what came back. **`h` is the
empty message**: no byte of it crosses USB, none of it is typed, it is one block
of pure padding the device generates itself, and it is therefore the most
deterministic thing this console can be asked. It gave a different answer every
time.

That non-repeatability is the whole finding, and `tests/usb_crypto_console.rs` is
built around measuring it rather than around the digest, because **which**
commands stop being functions of their own line is what separates the two reasons
an answer can be wrong. A wrong *constant* is a mis-mapped lookup table or an
unrouted wire. A wrong *varying* answer is a path that does not settle inside the
clock period. Sixteen runs of each, with the two shallow commands as the control
group:

```text
--- one line, one answer: 16 runs each ---
  ?                  -> one answer in 16 runs: H t|h x|E x|e n|Z n|X n|K x|N x|C x|k|n|c|?
  k                  -> one answer in 16 runs: 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
  n                  -> one answer in 16 runs: 000000000000004a00000000
  c                  -> one answer in 16 runs: 00000001

thread '…' panicked at tests/usb_crypto_console.rs:414:5:
`h` is a function of its own line and gave 7 distinct answers in 16 runs, so a
path in it does not settle inside 16.67 ns:
  4 x 1526135a94747b51ba3d9b2c6eb0ef9af72837d7848343c80dc6ec98263fef54
  4 x cc39bdad648c1eeaaf755dd5e42fe240c33b3f5988ff7c453798a00788965556
  3 x ef71af907e80288efcc1d4defc5360cfef4e79028557694213cd5ce56b6f4b4e
  2 x 24d5863d88e1b87e4fc2ac2a8a0334fa40e08627faa3c4439c982758f5c40397
  1 x 588817dbcf8511378bbec1a55fe08d27c2e990db9f83332f0517e18e67b3e255
  1 x 7e3a402f690f2a7f07f89e5659af15479302218f96fb72758012521c873740b7
  1 x f05c3730399da5998d9f4f7e6cdfb0ccdacf3daa243541a39d4998aff3c25201
```

Taken at forty runs rather than sixteen, across both bitstreams:

| line | the deepest thing in it | depth, LUT4 | forty runs |
|---|---|---|---|
| `?` | a 64-entry string table | shallow | **40 of 40 right** |
| `k` | a 256-bit rotate and a nibble encoder | shallow | **40 of 40 right** |
| `e 10` | ChaCha20's quarter round | **87** | 2 distinct, 39 : 1, both wrong |
| `E 00000000` | the same, plus an exclusive-or | **87** | 2 distinct, 36 : 4, both wrong |
| `h` | SHA-256's T1 chain | **39** | 7 distinct in 16, all wrong |

**The two paths with no adder in them never fail and the two with chained 32-bit
additions always do.** That is the conclusion, and it is an argument from two
populations rather than a delay anybody measured: there is no vendor timing model
in this repository — `reticle timing` says in its own help that its device
numbers are placeholders — so "the clock did not close" cannot be a slack figure
here. It can be, and is, a wrong answer that changes between identical runs.

Two details worth keeping because they are the kind that localise a fault. The
*tail* of a wrong digest is often right: `h` once gave
`…85c7d9b0ddeb65bbfef098a3b886335c` and `Z 0` — the same empty message by another
route — gave `…51c7d9b0ddeb65bbfef098a3b886335c`, agreeing in the last thirteen
bytes and differing above them, which is what a chain that settles at its cheap
end and not at its expensive end looks like. And the **deeper** block is the
*more* repeatable one: ChaCha20 at 87 levels lands on the same wrong value 39
times in 40, where SHA-256 at 39 levels scatters. A path that misses by a little
captures whatever has arrived, which varies; a path that misses by a lot captures
a partial result that is itself stable.

**The cause is §7's last paragraph, which has been corrected because it was too
optimistic.** `src/fpga/trellis` describes `CCU2C` without a (ci, i0, i1, co)
port map, the flow says so — `50 adder(s) stay generic` for the hash build, `85`
for both — and a 32-bit addition becomes a ripple of LUT4 about twenty-one levels
deep. Five of those chained is 39 levels; four of ChaCha20's is 87. On an iCE40,
where `SB_CARRY` is inferred, the same two paths are **9** and **5**. Inferring
the carry cell on the ECP5 is no longer a nicety that would improve a clock:
**it is what stands between these two blocks and working on this part at all.**
It is a `src/` change and it is named rather than made.

A **pipeline stage** inside the round would be the other way round, and it is a
change to `ip/crypto/` rather than to `src/`: it would break the
129-cycles-a-block figure every section of this page rests on, and §4's
fixed-latency measurement would have to be retaken. Fixing the backend fixes
every arithmetic block in the library at once; pipelining fixes one, slower.

### 1. A real throughput figure, and the bottleneck

**MEASURED**, and these are good even though the answers are not: a rate is a
cycle count, the cycle count is the control path, and the control path is the
half of this design that works. Taken on the `WITH_CIPHER=0` bitstream unless
marked.

| what | what it measures | measured |
|---|---|---|
| `Z 1000000` — 16 MiB made on the device | **this block**, nothing on USB | **29.63 MB/s**, **0.494 bytes/clock** |
| `Z 400000` — 4 MiB | the same | 29.44 MB/s, 0.491 bytes/clock |
| `Z 100000` — 1 MiB | the same, with the round trip visible | 27.92 MB/s, 0.465 bytes/clock |
| `h` + 256 KiB as hex | the **link, outbound** | 132.0 kB/s hashed, **264.0 kB/s on the wire** |
| `h` + 64 KiB as hex | the same | 131.8 kB/s hashed, 263.6 kB/s |
| `e 4000` — 16 KiB of keystream | the **link, inbound** (`WITH_HASH=0`) | 502.7 kB/s, 1 005 kB/s on the wire |

**§7's 0.496 bytes per cycle is now measured on silicon: 0.494.** That is the one
number on this page the board *confirmed* rather than contradicted, and it is
worth saying what it means — the cycle count a four-state zero-delay simulator
computed is the cycle count a part takes, to four tenths of a per cent, over
sixteen mebibytes and 262 144 blocks. The *control* path of this block is exactly
as fast as §4 says and §7 multiplies.

**The bottleneck is the link and it is not close.** 29.63 MB/s of core against
0.132 MB/s of message over USB is a factor of **225**. Half of that factor is
this protocol's own: a message byte crosses as two hexadecimal digits, so
264.0 kB/s on the wire carries 132.0 kB/s of message, and hex framing costs
exactly two. The other half is somebody else's measurement agreeing —
`ip/usb/usb_cdc_acm/README.md` §5 has `tests/usb_loopback.rs` timing the *bulk
loopback* of `testdata/fpga/cynthion/usb_ulpi_device.v` at **255 500 bytes/s each
way** at a 64-byte packet size, and 264 kB/s is that ceiling. So the OUT
direction of this console runs at the endpoint's own rate and nothing in
`crypto_console` is in the way.

The two directions are **not** the same, and that is new: 264 kB/s out against
1 005 kB/s in, a factor of 3.8. An IN endpoint is filled when the host asks and a
full-speed frame has room for nineteen 64-byte bulk transactions; an OUT endpoint
that NAKs while the console is busy costs the host a transaction per NAK. So the
OUT figure is the one that matches a *round trip* and the IN figure the one that
matches a stream.

### 2. A closed clock — what was asked for, and what there is instead

The question was whether this block meets timing on the part at the clock it
uses. The answer is **no, at 60 MHz, which is the only clock this board's USB
transceiver allows**, and the three things that make that a measurement rather
than a guess are the two shallow control commands being right forty times out of
forty, the two deep ones being wrong every time, and the wrongness varying.

What a slack number would need, and nobody has: a delay model for this device in
`src/fpga`. What a *faster* answer would need: the carry cell. What a **slower
clock** would need, and is the obvious next experiment: a second clock domain,
because 60 MHz is the ULPI interface rate and not negotiable — the console would
stay at 60 and the cores would run at 60/N behind a handshake, which is a real
design change and a clock-domain crossing where there is currently one domain
and a test asserting it.

### 3. What this still does not establish

A wrong answer off a real part is a result about **function**, and a right one
would have been too. Neither is a result about anything in §5, and the
distinction is the whole reason §4 and §5 are two sections:

- **Power analysis.** Nothing has recorded a power trace of this block. Running
  on silicon makes one *possible* — there is now a part drawing current while
  hashing a message somebody chose, with LED 5 high for exactly the window it
  takes, so ball C11 is a hardware trigger. It does not make one *taken*.
  Nothing here is masked, randomised or duplicated.
- **Electromagnetic emission.** The same, with a probe instead of a shunt.
- **Gate delay.** §4's fixed-latency claim is a cycle count, and the board
  confirmed the cycle count to 0.4 per cent. It is still not a propagation delay.
  A carry that ripples further for one operand than another does not change how
  many clock edges pass and is exactly as invisible to a hardware cycle count as
  to a simulated one. What this round did add is that the data path's delay is a
  *physical quantity large enough to matter*, which §5 called "real and invisible
  here" and which is now real and visible as a wrong digest.
- **Fault injection.** No redundancy and no checking, on silicon as in
  simulation — and a round that produced wrong answers out of correct logic is a
  reminder of what §5 says about one: a glitched clock "will produce a wrong
  digest and say nothing", and so did this.

**And the one thing a reader must not take from this section.** None of it says
the block is wrong. Every vector in §3 still passes, the decomposition is still
checked, and the fault is in the path from this block's logic to this part's
flip-flops. What it says is that **until the ECP5 backend infers a carry cell,
this block is a simulation result on that family** — which is exactly the
sentence §1's `CHECKED` level exists so that this page can write.

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
