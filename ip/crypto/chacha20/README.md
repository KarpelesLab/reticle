# ChaCha20 in a fabric, and the sixteen words that say which round is wrong

`chacha20` is RFC 8439 ChaCha20: the quarter round, the block function and
the stream cipher, as three modules. It is the second of the two blocks in
`ip/crypto/`, and like
[`ip/crypto/sha256/README.md`](../sha256/README.md) it has two sections
about side channels that are deliberately not one section — §4 is the
property that was **measured**, §5 is everything that was **not**.

It also has a section the SHA-256 page does not need. SHA-256 is a hash: a
caller who gets it right gets integrity. ChaCha20 is a stream cipher
**without** Poly1305, which means a caller who gets everything on this page
right still has no authentication, and a ciphertext whose bits can be
flipped one for one into the plaintext's. §6 is about that, and it is the
section to read before using this block for anything.

- §1 says what each kind of claim here rests on.
- §2 is the three modules and why a quarter round is one of them.
- §3 is the interface: the key, the nonce, the counter, the byte order, the
  32-bit port, and what the caller has to hold still.
- §4 is the constant-time property, measured.
- §5 is what is not defended against.
- §6 is **not an AEAD**, and the nonce.
- §7 is the vectors, by section, including the intermediate state.
- §8 is area and throughput, with the trade that was declined.
- §9 is the defect a test no document asks for found.
- §10 is what a board would add.
- §11 is the reading list.

---

## 1. Confidence, and what it is based on

- **HIGH** — stated in the published standard, with the section named:
  - *ChaCha20 and Poly1305 for IETF Protocols*, **RFC 8439**, June 2018 —
    "RFC 8439" below. It obsoletes RFC 7539; the algorithm is the same and
    the section numbering used here is 8439's.
- **MEASURED** — a number this project produced by running something, with
  the test that produces it named.
- **CONFIRMED** — agreed with `purecrypto`, the user's from-scratch Rust
  cryptography library, driven with the same inputs out of tree.
  `purecrypto` is **not** a dependency of anything committed here.
- **LOW** — inference that explains the rest, with no way to check it here.

Nothing on this page has been on a board; §10 says what that would add.

---

## 2. Three modules, and why the smallest one exists

| Module | What it is | Why it is separate |
|---|---|---|
| `chacha20_qr` | one quarter round, combinational | **RFC 8439 §2.1.1 is a test vector for it** |
| `chacha20_core` | the block function of §2.3 | a caller who wants keystream blocks, or byte-granular positions, needs no stream machinery |
| `chacha20` | the encryption of §2.4 | `top`: the counter, the exclusive-or, and the counter guard of §6 |

The first row is the one worth arguing for. A quarter round is four
additions, four exclusive-ors and four rotations, and if it is wrong the
keystream is wrong — which tells a reader **nothing** about which of the
twelve operations it was. RFC 8439 publishes two vectors that address a
quarter round directly: §2.1.1 (four numbers chosen to make the arithmetic
visible) and §2.2.1 (QUARTERROUND(2, 7, 8, 13) applied to a sample state,
which is a *diagonal* round's quarter and so reaches the same logic through
different values). A testbench can only reach a module, so the quarter
round is a module, and `chacha20_qr_matches_rfc_8439_2_1_1_and_2_2_1`
drives both vectors straight into its eight ports.

It also has a footprint row of its own in
[`docs/ip-library.md`](../../../docs/ip-library.md#resource-footprints),
which is what makes §8's area argument checkable rather than asserted: one
quarter round is 573 LUT4, and a round instantiates four.

### Why one round per cycle, and not four quarter rounds in four

**HIGH** (RFC 8439 §2.3.1). `inner_block` is eight `Qround` calls: four on
the columns, then four on the diagonals. The four in each group touch
**disjoint** words — QR(0,4,8,12), QR(1,5,9,13), QR(2,6,10,14),
QR(3,7,11,15) for the columns — so all four can be computed in the same
cycle.

That is the whole argument, and it is unusually clean:

- **one quarter round per cycle**, one instance: 80 cycles a block, and the
  critical path is one quarter round — four 32-bit additions in series.
- **one round per cycle**, four instances: 20 cycles a block, and the
  critical path is **still** one quarter round, because the four are side
  by side and not in series.
- **one double round per cycle**, eight instances: 10 cycles a block, and
  the critical path is eight additions. That halves the clock this block
  closes at, which gives back most of what it bought.

So the middle one is four times faster than the cheap one for no extra
depth at all, and the expensive one trades clock for cycles at about one
for one. **MEASURED**: 20 rounds, one cycle to add the initial state back
and one to start is **22 cycles from `start` to `valid`**, asserted by
every one of the ChaCha20 tests that calls `block_of`.

The four instances are wired to do both kinds of round from one set of
multiplexers: the `a` input is word *n* either way, and only `b`, `c` and
`d` rotate by one, two and three positions on a diagonal round.
`chacha20_core`'s header draws the write-back.

---

## 3. The interface

```verilog
chacha20 u_cipher (
    .clk       (clk),
    .rst_n     (rst_n),
    .start     (open_stream),   // one cycle; captures `counter`
    .key       (key),           // 256 bits, first byte at the top
    .nonce     (nonce),         // 96 bits, first byte at the top
    .counter   (counter),       // a NUMBER, not a byte string
    .in_data   (word_in),       // 32 bits, first byte at the top
    .in_valid  (word_valid),
    .in_ready  (word_ready),
    .out_data  (word_out),      // one cycle after each accepted word
    .out_valid (word_out_valid),
    .active    (streaming),
    .exhausted (counter_ran_out)
);
```

### Byte order, and the one port that is not a byte string

In both blocks of `ip/crypto/`, **a byte string has its first byte at the
most significant end**. So `key[255:248]` is key byte 0 and
`nonce[95:88]` is nonce byte 0 — the order RFC 8439 prints them in, so a
vector copied out of the document goes straight in. `in_data[31:24]` is the
first of its four bytes and `out_data[31:24]` is that byte's result.

**HIGH** (RFC 8439 §2.3). The state's words are little-endian loads of
those byte strings. That load happens inside `chacha20_core`, as wiring,
which is where it belongs: a caller holding a key as thirty-two bytes
should not have to byte-swap it to use this block. There is **no byte swap
anywhere on the data path** either, because the keystream comes off the
core already serialised.

`counter` is the exception and says so in the header. §2.3 calls word 12 "a
block counter" and the RFC's own vectors give it as a number — "Block
Counter = 1" — so this port is that number and no swap is applied to it.
Appendix A.1's test vectors #4 (counter 2) and #5 (counter 0 with a nonce
that is not symmetric) are what would catch getting that wrong, and §2.3.2
alone would not: its counter is 1, which is the same byte string either way
round.

### Why the port is 32 bits wide

**MEASURED.** The core needs 22 cycles for a block of 64 bytes. A 32-bit
port drains those 64 bytes in 16 cycles, which fits inside the 22 with
room, so the port is free and the core is the limit. An 8-bit port would
take 64 cycles to drain a block the core made in 22, and a block would cost
87 cycles instead of 39 — well under half the rate, for a saving nobody
wants. A 64-bit port would drain in 8 and buy nothing, because the 22 are
still there.

`ip/crypto/sha256` is 8 bits wide for the same reason read the other way:
its compression takes 64 cycles for 64 bytes, so one byte per cycle is
already as fast as the core. **The rule is the width at which the port
stops being the limit**, and the two blocks land in different places
because their cores do.

### What the caller must hold still, and the 384 flip-flops that buys

`key`, `nonce` and `counter` must not change between `start` and `valid`.
Nothing in `chacha20_core` latches them: §2.3's final `state +=
initial_state` reads the initial state back **off the ports**.

That is 384 flip-flops not spent — and more to the point, it means the key
exists in this design in one place and not two. The working state does of
course contain the key words during the twenty rounds, so this is not a
claim that the key is nowhere; it is a claim that no second copy was made
for the convenience of the adder.

`chacha20` honours the contract on its caller's behalf for `counter`: it
captures it into `ctr_q` at `start` and drives the core from that register,
so the counter is stable across a block by construction. `key` and `nonce`
it passes straight through, so the requirement reaches the caller
unchanged — and §6 is about why that is the right place for it.

### `advance`, and the multiplexer it is instead of

`chacha20_core`'s `advance` rotates `block` one 32-bit word towards the
most significant end; sixteen pulses bring it back where it started. A
consumer that wants all 512 bits at once ties it low and never notices.

It is there because the alternatives were a 32-bit sixteen-way multiplexer
in `chacha20` — thirty-two 16-input selects, which is not free — or a
512-bit shadow register, which is the trade §8 declines. Rotating the
register that already exists costs **nothing**: the rotation is the same
wires the load uses. That is why §8 can say the whole wrapper costs 144
LUT4 and 73 flip-flops over the core.

---

## 4. Constant time: what was measured

The doctrine is `purecrypto`'s, stated in its `ct` module: *every operation
here runs in time independent of the secret values it touches, so higher
layers can be built without secret-dependent branches or memory accesses.*
A secret-dependent branch becomes a secret-dependent **cycle count** in
hardware; a secret-dependent memory access becomes a secret-dependent
**address**.

### Fixed latency

**MEASURED**, by `chacha20_takes_the_same_cycles_whatever_the_key_is`. Nine
keys as different as thirty-two bytes can be made — all zeros, all ones,
`0xaa`, `0x55`, a counting pattern, its complement, one byte set at each
end, and an arithmetic spread — against three nonces (all zeros, all ones,
and §2.3.2's) and three counters (0, 1 and 0xFFFFFFFF). **Eighty-one
combinations, 22 cycles every time.**

And the test asserts that the eighty-one blocks were **all different**,
because eighty-one equal cycle counts from eighty-one identical runs would
prove nothing. The stream is measured the same way: nine keys over 256
bytes, one cycle count (157), nine different ciphertexts.

ChaCha20's cycle count has no honest reason to depend on the key, which is
exactly why measuring it is worth doing: an implementation where it *does*
has something in it that short-circuits — and that is the same class of
thing a table-driven AES S-box has, and part of why AES is not in this
round. [`docs/ip-library.md`](../../../docs/ip-library.md#what-aes-should-inherit)
has the rest of that argument.

### No secret-dependent addressing

**MEASURED**, by `crypto_blocks_hold_no_memory_to_index`, which synthesises
every variant and asserts the module contains **no memory array at all**.
ChaCha20 has no table of any kind to begin with — no S-box, no constant
table beyond four words wired in — so there is nothing here a secret could
index even in principle.

**What that does not establish**, and it is the same gap
[`ip/crypto/sha256/README.md`](../sha256/README.md) §4 names: a
*multiplexer* selected by a secret is not a memory and would pass that
test. Nothing in this block does it — the only selects here are `round_q`,
`widx_q` and `state_q`, all counters — but no test in this repository
proves it. Proving it would take a taint analysis from `key` and `nonce`
forward through the IR, and Reticle has none.

---

## 5. What this does not defend against

Everything in §4 is about **logical** time. None of it is about what the
part does during those clock edges, and that is where the attacks on this
primitive actually are.

- **Power analysis.** A ChaCha20 round is sixteen 32-bit additions whose
  operands are key, nonce, counter and state bits. The current the fabric
  draws in a cycle depends on how many flip-flops toggled, which depends on
  those bits. Differential power analysis does not measure the cycle count,
  so a perfectly fixed cycle count does not touch it. **Nothing in this
  repository has recorded a power trace of this block, and nothing in it is
  masked, randomised or duplicated against one.** ChaCha20's first round
  operates directly on key words, which is the structure that makes a
  first-round attack the obvious one.
- **Electromagnetic emission.** Same argument, a probe instead of a shunt,
  and no measurement either.
- **Gate delay.** §4's figure is a cycle count in a four-state zero-delay
  simulation. It says the *control path* is key-independent. It says
  nothing about whether a carry ripples further for one operand than
  another, which is real and invisible here.
- **Fault injection.** No redundancy and no checking. A glitch that skips a
  round produces a keystream the attacker may well be able to work with,
  and nothing here would notice.
- **Key residue.** The key is on the ports and its words are in `st_q` for
  the twenty rounds. **Nothing is zeroised.** When a block finishes, `st_q`
  holds the keystream block, not the key — the feed-forward addition
  overwrites it — but during the rounds it holds key-derived state, and
  after a `start` that is abandoned it holds whatever the last round left.
  A design that cares has to hold the key off the port and reset the
  instance, and this block gives it no help.

**The short version: this block is for confidentiality against an attacker
on the wire, not against one holding the board.**

---

## 6. Not an AEAD — and the nonce

This is the section that matters most, because the mistake it is about is
not a mistake in this block.

### There is no Poly1305 here

**HIGH** (RFC 8439 §2.8 is the AEAD construction; §4 is the security
considerations). ChaCha20 on its own is a keystream cipher. Exclusive-or is
its own inverse, so:

- **a ciphertext from this block is malleable.** Flip bit *i* of the
  ciphertext and bit *i* of the plaintext flips, with no other change. An
  attacker who knows the plaintext's format can change it to anything of
  the same length without the key.
- **there is no integrity and no authenticity.** This block cannot tell a
  ciphertext it produced from one an attacker wrote, and neither can its
  caller.

Anything that needs those needs ChaCha20-Poly1305, which is §2.8: the
Poly1305 one-time key is the first 32 bytes of the keystream at counter 0,
the message is encrypted from counter 1, and the tag covers the additional
data, the ciphertext and both lengths with their padding. **That is a
second block and it is not here.** What is here that it would need: this
block already starts at an arbitrary counter, which is the half of §2.8
that touches the cipher.

### The nonce is the caller's problem, and it is the sharp one

**HIGH** (RFC 8439 §4, first paragraph of the nonce discussion). Using the
same (key, nonce) pair for two messages destroys the confidentiality of
**both**: the exclusive-or of the two ciphertexts is the exclusive-or of
the two plaintexts, and the keystream drops out. This block cannot detect
it, cannot generate a nonce, and does not try.

What it **can** see is the one adjacent failure that is local:

**MEASURED.** RFC 8439 §2.3's block counter is 32 bits, so a (key, nonce)
pair is good for 2^32 blocks — 256 gibibytes. Wrapping it hands out the
same keystream twice, which is the same disaster by a different route. So
`chacha20` latches `exhausted`, **stops accepting data**, and requires a
`start` with a new nonce. `chacha20_stops_rather_than_repeat_its_keystream`
opens a stream at counter 0xFFFFFFFE, checks that the two blocks it is
entitled to come out, that the next word is **not** accepted, that
`exhausted` is up, and that the two blocks differ. That test is also the
only one here whose counter carries across a byte boundary, so it is what
would catch a counter incremented a byte at a time.

A guard like that is the kind of thing hardware can do and a software
library mostly leaves to its caller, and it was worth one flip-flop and a
32-input AND.

### What else is not here

- **No XChaCha20**, so no 192-bit nonce and no random-nonce safety margin.
  It needs HChaCha20, which is this core with the feed-forward addition
  omitted and a different output selection — close, and still a second
  module.
- **No original 64-bit-nonce ChaCha20** (the DJB variant), which splits
  word 12 and 13 differently.
- **No ChaCha8 or ChaCha12.** The round count is not a parameter. It could
  be; nothing needs it, and a reduced-round cipher behind the same module
  name is a thing a caller can select by accident.
- **No resuming mid-word.** The keystream position advances four bytes at a
  time. A caller that needs byte granularity uses `chacha20_core` and does
  the exclusive-or itself. A message whose length is not a multiple of four
  is the easy case: present a final word padded however you like and use
  the bytes you need — the keystream depends on the key, the nonce and the
  position and on nothing in `in_data`, so the discarded bytes change
  nothing.

---

## 7. The vectors, by section

**HIGH.** Every vector below is in RFC 8439 and is in `tests/ip_library.rs`
cited by its section.

| Section | What it pins down |
|---|---|
| §2.1.1 | the quarter round: the rotation amounts, the operand order, that the rotation follows the exclusive-or |
| §2.2.1 | the same logic reached through a *diagonal* quarter's values |
| §2.3.2 | the state setup, the round alternation, the round count, **the sixteen-word state after twenty rounds**, the feed-forward addition, the serialisation |
| Appendix A.1 #1–#5 | counters 0, 1 and 2; a zero key, a key with its last byte set, a key with `0xff` in its *second* byte; a nonce with a 2 in its last byte |
| §2.4.2 | the stream across a block boundary: 114 bytes from counter 1, with the RFC's keystream as well as its ciphertext |
| Appendix A.2 #2 | 375 bytes, six blocks, counter 1 |
| Appendix A.2 #3 | 127 bytes from counter **42**, with a key that is nothing but entropy |

A.2 #1 is 64 zero bytes under a zero key, which is exactly A.1 #1 by
another name — encrypting zeros *is* the keystream — so it is not a
separate test.

### The intermediate state, and why one test reads a register

RFC 8439 §2.3.2 prints **three** sixteen-word tables for one block: the
state as set up, the state after twenty rounds, and the state after the
original is added back. The middle one is the valuable one, and
`chacha20_core_reaches_the_state_rfc_8439_2_3_2_prints` reads it off the
working register rather than any port.

The reason is diagnostic and it is the whole argument for this block's
decomposition:

- keystream wrong, after-twenty-rounds state **right** → the fault is in
  the feed-forward addition or the serialisation;
- after-twenty-rounds state **wrong** → the fault is in a quarter round, or
  in which words a column or diagonal round touches, or in the round
  count — and `chacha20_qr`'s own two vectors say which of those.

Without it, a wrong keystream is one bit of information. With it, a wrong
keystream is localised to one of two small places before anyone opens a
waveform.

### Out of a running implementation

**CONFIRMED.** All nine of the vectors above were also produced by
`purecrypto` on this machine, byte for byte, from the same inputs. That
matters less here than it does for `ip/crypto/sha256`, where a
nineteen-length padding table has no published source — here the document
covers everything, and `purecrypto` agreeing is what earns it the right to
be the oracle over there.

The ciphertexts and keystreams in the test came out of RFC 8439's own
hexdumps, extracted by columns rather than by tokenising, so a gutter that
happens to read as hexadecimal could not get into an expectation.

---

## 8. Area and throughput

**MEASURED**, by `footprints_match_the_documentation`; the table in
[`docs/ip-library.md`](../../../docs/ip-library.md#resource-footprints) is
the authority.

| | LUT4 | LUT6 | iCE40 `SB_LUT4` + `SB_CARRY` | flip-flops | depth (LUT4 / iCE40) |
|---|---|---|---|---|---|
| `chacha20_qr` | 573 | 441 | 447 + 124 | 0 | 87 / **5** |
| `chacha20_core` | 5834 | 4189 | 4641 + 996 | 520 | 87 / **8** |
| `chacha20` | 5978 | **3810** | 4764 + 1030 | 593 | 87 / **8** |

`chacha20_core` is the **largest single module in this library** at LUT4,
and the reason is arithmetic: sixteen 32-bit additions in the four quarter
rounds and sixteen more in the feed-forward addition. `chacha20_qr`'s row
is what makes that checkable — 573 LUT4 for one quarter round, four of
them in a round, and the rest is the column/diagonal muxing and the final
add.

**The two depth figures are the same path counted twice.** Four 32-bit
additions in series is 5 levels on the iCE40, because `SB_CARRY` is
inferred and a 32-bit add is one carry chain; it is 87 on the generic LUT4
and LUT6 mappings and on the **ECP5**, because neither of those flows emits
a carry cell and a 32-bit ripple-carry add is twenty-odd levels. That is
`src/fpga/trellis` having no CCU2 inference — every arithmetic block in the
library pays it — and it is the single change that would most move this
block's achievable clock on an ECP5. It is noted and not made.

**One number in that table is unexplained and is left visible.** The whole
of `chacha20` maps to **fewer** LUT6 than `chacha20_core` alone (3810
against 4189) while using *more* LUT4 (5978 against 5834). The wrapper
drives the core's `counter` from a register and leaves its `busy`
unconnected, so the cut enumeration sees a different graph — and
`docs/ip-library.md`'s own account of the technology mapper says that with
eight cuts kept per node a different eight survive and some nodes get a
better one. That is a plausible explanation and it is not a measurement, so
it is marked **LOW** and both numbers stand as they came out.

### Throughput

**MEASURED.** 16 cycles of data, 22 computing the next block and one to
notice it arrived is **39 cycles for 64 bytes, 1.64 bytes per cycle** —
164 MB/s, 1.31 Gbit/s, at 100 MHz.
`chacha20_takes_the_same_cycles_whatever_the_key_is` prints 157 cycles for
256 bytes, which is four whole blocks and the one cycle `start` takes, so
the 39 is arrived at and not assumed.

There is no measured clock behind the 100 MHz; it is a round number to
multiply by, and §10 is why a real one needs a board.

### The trade declined

A **second 512-bit register** would hold the block while the core computes
the next, hiding the 16 cycles of draining inside the 22 of computing: 64
bytes per 22 cycles, **2.91 bytes per cycle, 2.33 Gbit/s — 1.77 times**
this. It costs 512 flip-flops, which is more than `chacha20` and
`chacha20_core` hold together (593).

It was not taken, for the reason
[`ip/crypto/sha256/README.md`](../sha256/README.md) §7 gives about its own
second buffer: it doubles the smallest useful instantiation to buy a factor
under two, and a block that will not fit a small part is a block nobody
uses. A design that is genuinely limited by cipher rate wants more than
1.77x and should have a different core — two rounds per cycle *and* double
buffering, with the clock cost measured rather than assumed.

---

## 9. What a round trip nobody asked for found

**MEASURED**, and it is the most useful thing in this document.

Encryption and decryption are the same operation for a keystream cipher, so
putting a ciphertext back through must give the plaintext. RFC 8439 does
not state that as a vector and does not have to — it is what a keystream
cipher *is*. `chacha20_encrypts_the_text_of_rfc_8439_2_4_2` asserts it
anyway, as a third call after the ciphertext and the keystream.

It failed, on the first 64 bytes only, and the defect was this.
`chacha20` asks the core for the next keystream block as soon as the
current one is drained, so a stream whose last word fell on a block
boundary leaves a request **in the air**. A new `start` arriving then set
up the new counter and raised a fresh request — but `chacha20_core` took
`start` only from `S_IDLE`, and it was in `S_RUN`. So it ignored the
request, finished the block it was already computing, raised `valid`, and
the wrapper took that block as the new stream's first: a keystream from the
**old** counter.

**Every one of the nine published vectors passed with that bug in place**,
because every one of them is the first thing the block does after reset.
Nine vectors from the authority, and the thing that found it was three
lines of "and then do it backwards".

The fix is two lines in two files. `chacha20_core` takes `start` from any
state now, which is also the semantics its header always claimed and is
what `sha256_core`'s `start` already did. And `chacha20` ignores a `valid`
that arrives while its own request is still up, because the core takes a
request a cycle after it is made and a `valid` in that cycle belongs to the
block the request is replacing.

**What a test would and would not catch**, which is this repository's
habit: the round trip catches any state the wrapper and the core can
disagree about across a message boundary. It would **not** have caught the
same bug inside a single message, because within a message the counter the
stale block was computed for is the right one anyway — the stale block only
becomes wrong when `start` changes the counter under it. A test that
interleaved two streams on one instance would reach further still, and is
not here.

---

## 10. What a board would add

**This block has not been on a board, and it needs none to be correct.** It
is pure logic: no PLL, no device primitive, no pin timing. Simulation sees
everything a part would about *function*.

What a board would add is the same two things
[`ip/crypto/sha256/README.md`](../sha256/README.md) §8 names, and for the
same reasons — a **real throughput figure**, which means a clock that place
and route actually closed (and on the ECP5 that is the number the missing
carry inference above would move most), and a **power trace**, which is the
only way §5 could stop being a disclaimer and become a result.

**The cheapest experiment here** is not this block: it is `sha256` fed from
`ip/bus/uart`'s receiver with its digest sent back out of the transmitter,
because a digest is something a host can check with `sha256sum` and no key
has to cross the wire. ChaCha20's equivalent needs the key and nonce on the
device — a constant in the bitstream for a test, which is fine for a
throughput measurement and is exactly the wrong habit for anything else —
and the host checks the ciphertext against `purecrypto`, which is the same
oracle §7 already uses. Doing `sha256` first is the right order: it closes a
clock on the same flow with one fewer thing to get wrong.

---

## 11. Reading list

- *ChaCha20 and Poly1305 for IETF Protocols*, **RFC 8439**, June 2018. §2.1
  and §2.2 are the quarter round, §2.3 the block function, §2.4 the
  encryption, §2.8 the AEAD this block is **not**, §4 the security
  considerations — which are three pages and worth all of them — and
  Appendix A the vectors.
- [`docs/ip-library.md`](../../../docs/ip-library.md), **The crypto
  category, and what a constant-time claim is worth** — the doctrine both
  blocks are built to, where each fact came from, and what AES should
  inherit.
- [`ip/crypto/sha256/README.md`](../sha256/README.md) — the other block,
  and the one whose §2 is about a real choice (padding) rather than a
  decomposition.
- `tests/ip_library.rs` — the vectors, the fixed-latency measurement, the
  counter guard, and a `What it would and would not catch` paragraph on
  each.
- `purecrypto`'s `ct` module — the constant-time doctrine in its original
  form. Its note that `black_box` is best-effort and that real constant-time
  behaviour "should be validated with timing-analysis tooling" is the
  software analogue of §5 on this page, and it is the same honesty.
