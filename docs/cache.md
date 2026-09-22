# Incremental compilation

Reticle can keep what each module elaborated to and read it back instead of
elaborating it again. The store is **content addressed**: an artefact is
filed under a digest of everything that produced it — the source text, the
options, the parameter set, the compiler version, the feature set and the
keys of the module's dependencies — so nothing depends on timestamps, file
sizes or paths, and a file edited behind the build's back cannot go
unnoticed.

The feature is `cache`; the library entry point is `reticle::cache::build`
and the command-line one is `reticle cache`.

```sh
reticle cache --top top --output design.rtl leaf.v mid.v top.v
reticle cache --stats --top top leaf.v mid.v top.v   # what hit, what missed
reticle cache --list                                 # the entries
reticle cache --verify                               # check the store
reticle cache --clear
```

**Read the measurements before turning it on.** On some designs the cache
is a large win; on others it is a loss. The last section says which is
which, honestly.

## What goes into a key

This is the whole problem. A key that misses an input serves a stale
artefact for changed sources, and the result looks like a miscompilation
with no clue as to why. The inputs are enumerated in the module docs of
`reticle::cache::key`, and every one of them has a test in that module
asserting that changing it changes the key.

Every key begins with four things:

1. the key-format version (`cache::key::FORMAT`), bumped whenever the
   composition, the hash or the artefact encoding changes, which retires
   every existing entry rather than misreading it;
2. the compiler version (`reticle::VERSION`);
3. the compiled-in feature set — `verilog`, `vhdl`, `synth`, `formal` —
   because a build without a stage cannot produce that stage's artefacts;
4. the kind of unit: `scan`, `elab` or `synth`, so two kinds cannot alias.

A **scan** key (the per-file dependency summary) then adds the language and
standard the file is read under, and the file's name and full text.

An **elaborated module** key adds:

5. the language, the Verilog dialect or VHDL standard, and the working
   library name;
6. whether this module is the build's designated top;
7. the parameter or generic overrides, in the order given — and only for
   the designated top, because that is the only module the frontends apply
   them to;
8. the module's own name;
9. the name and full text of every source file that defines part of it;
10. the name and full text of every *global* source file — one that defines
    no module of its own (a Verilog package or header, a VHDL package,
    context or configuration). These reach every elaboration, so they are
    folded into every key. Editing one therefore rebuilds everything: that
    is conservative on purpose, since attributing a package to the modules
    that import it would need name resolution, which is most of
    elaboration;
11. the key of each module it instantiates, with that module's name, sorted
    by name.

A **synthesised module** key adds the elaborated module's key and the
synthesis options that can change the netlist.

Point 11 is what makes the dependency direction work. A dependency's key
already covers its sources and its own dependencies, so:

| Edit | What misses |
|------|-------------|
| a leaf | the leaf and every module above it |
| a top-level file | that module only |
| an unrelated module | that module, and anything that instantiates it |

`tests/cache_build.rs` asserts each of those three directly, because that
relationship is the part most likely to be quietly wrong.

### What is deliberately *not* in a key

- **Paths, modification times and sizes.** The text is hashed instead.
  Moving or touching a file changes nothing; editing it always does. Only
  the *name* the caller gave a file is folded in, because that name reaches
  diagnostics.
- **`SynthOptions::validate`.** It only decides whether the synthesis
  passes check their own invariants; it cannot change the netlist, and
  folding it in would give a debug build and a release build different keys
  for identical output. `verify_equivalence` *is* folded in, because a hit
  that skipped the proof would be a silent loss of checking.
- **Which modules the build was asked for** (`--only-top`). That selects
  work, not content, so both modes share one store.
- **Comments and whitespace.** They are part of the source text, so a
  comment-only edit *is* a miss. Normalising them away would mean lexing
  the file to build its key — that is, doing the work the key exists to
  avoid — and would make the key depend on the lexer's idea of a comment.
  `tests/cache_build.rs::editing_only_a_comment_still_misses` pins the
  behaviour, and also checks that the resulting design is unchanged: only
  the key moved.

## The hash

`reticle::cache::hash` implements **xxHash64** (checked against the
published test vectors) and runs it as two lanes with different seeds,
concatenated into 128 bits. The crate ships no dependencies, so it is
written in-crate; xxHash64 was chosen over FNV-1a for its avalanche on the
short, near-identical byte strings a build key is made of, and because it
has published vectors to check an implementation against.

**It is not a security boundary.** It is a non-cryptographic hash: anyone
who can choose the input can construct a collision. A key says "these are
the same inputs", not "this artefact is authentic". A store that an
attacker can write to is a compromised store whatever hash guards it.

Every integer is folded in little-endian and every length goes through
`u64::try_from`, so a 32-bit host, a 64-bit host and WebAssembly all
produce the same key. Strings are length-framed, so `("ab", "c")` and
`("a", "bc")` cannot collide.

## The store

`Cache` is the logic and `Storage` is the hole a backend plugs into:

```rust
pub trait Storage {
    fn get(&self, key: CacheKey) -> Option<Vec<u8>>;
    fn put(&mut self, key: CacheKey, value: Vec<u8>);
    fn remove(&mut self, key: CacheKey);
    fn keys(&self) -> Vec<CacheKey>;
    fn size(&self, key: CacheKey) -> Option<u64> { /* provided */ }
}
```

The library ships `MemoryStorage` and nothing else, because nothing under
`src/` except `src/bin/` touches the filesystem. The `reticle` binary
supplies a directory-backed one in `src/bin/reticle/cache_store.rs`: about
forty lines, one file per entry named `<key>.entry`, writing through a
temporary file and a rename so a build that is killed half way leaves no
truncated entry. Override `size` if the backend can answer it without
reading — opening a store asks for every entry's size, and reading a whole
store to add up its sizes would cost more than the build.

An entry is a text header plus the artefact, so a store can be read with
`cat`:

```text
reticle-cache 1
key 93860cafd363751265687aaa808848b8
producer elab:verilog
created 1790077299
used 7
size 110
content 063c951a2ba3e2087d8f529a68d49ad4

top other

module other
  net %a u1 wire
  net %y u1 wire
  port a in %a
  port y out %y
  assign %y = %a
end
```

`created` is a Unix timestamp the *caller* supplies (`0` when it has none):
the library has no clock. `used` is a logical counter, not a time, so the
least-recently-used order does not depend on the host's clock and does not
go backwards when the clock is corrected.

### Eviction

`Cache::with_capacity` caps the total encoded size. Every `put` that
overflows it drops entries in `used` order, oldest first, with ties broken
on the key so the choice is the same on every run. `reticle cache
--max-size <bytes>` sets it.

Only a capped cache reads the store when it opens it (to recover the
counter); an uncapped one lists the keys and asks their sizes, which a
filesystem backend answers with a `stat`.

### Verification

`Cache::verify` re-reads every entry and checks that the header parses,
that the recorded size matches the payload, that the payload hashes to the
recorded digest, and that the key written in the entry is the key it is
filed under. A store that was truncated, half written, copied from another
machine or edited by hand fails one of those. `reticle cache --verify`
prints what is wrong and exits 1.

It cannot re-derive a key from an artefact: a key describes the *inputs*
that produced the artefact, which the artefact does not contain. That is
why the key is written into the entry.

A damaged entry is never served. A lookup that finds one counts a miss and
drops it, so a corrupted store heals itself rather than failing every build
from then on.

## What is cached, and what is not

**Elaborated modules**, serialised through the IR's `.rtl` text format.
That format already round-trips exactly, so there is no new serialiser to
get wrong and an entry is inspectable by hand — the one above is a real
one. A module's artefact is *the design you get by elaborating that module
as the top of its own hierarchy*, pruned to what it reaches. Nothing in an
entry depends on the build that wrote it, so two builds that want the same
module always agree.

**Synthesised modules**, the same way, under a key that adds the synthesis
options. This is the clearest win, because synthesis costs several times
what elaboration does. A synthesis hit answers on its own: the elaborated
entry underneath it is not even read.

**Dependency scans** — the handful of names a file defines and
instantiates. Parsed ASTs are *not* worth caching: parsing is a small
fraction of elaboration (21 ms of a 69 ms build, measured below), an AST is
large, and it has no stable serialised form. The two-line summary extracted
from one is worth caching, because without it a build with nothing to do
would still have to parse every file to find out what depends on what.

## Consequences you should know about

- **A hit reports nothing.** The warnings a frontend emitted when a module
  was first elaborated do not reappear on a later build. That is inherent
  to caching a compilation step. `reticle cache --clear`, or a build
  without the cache, gets them back.
- **Spans in a cached module point into the entry**, not into the original
  source, because the design was parsed from the entry's text. The entry is
  added to `BuildResult::sources` under `<cache:name>` so the span still
  resolves and renders.
- **`` `include `` is not followed**, exactly as `ip::elaborate` does not
  follow it: a build lists its files. An included file that is also listed
  is treated as a global, so editing it rebuilds everything; one that is
  not listed is not seen at all, and its content is then missing from every
  key. Do not point the cache at sources that include files outside the
  list.
- **Mixed-language builds** elaborate each half with its own frontend and
  merge, keeping the first definition of a repeated name — the rule
  `ip::elaborate` uses. Two roots that parameterise a shared module
  differently therefore keep the first root's copy.
- **A recursive hierarchy** (which neither language allows) has no
  well-founded key, so the modules in the cycle, and those that reach one,
  fall back to a key over the whole source set. Conservative: any edit
  rebuilds them.
- **The store grows with the square of the hierarchy's depth** in the
  default mode, because each module's artefact holds its whole cone. The
  631-module design below produces a 6.3 MB store from 652 KB of IR. Cap it
  with `--max-size`, or use `--only-top`.

## Measurements

AMD Ryzen Threadripper 9970X, Linux, rustc 1.98, `--release`, warm page
cache, median of seven runs, wall clock including process start-up. The
designs:

| Design | Files | Lines | Shape |
|--------|-------|-------|-------|
| `wide` | 631 | 25 539 | 600 leaves under 30 mids under one top (generated) |
| `deep` | 121 | 601 | a chain of 120 stages (generated) |
| `uart` | 3 | 242 | the bundled `ip/uart` core (real) |

The repository has no large design of its own — the biggest real one is the
bundled IP library, and its largest core is three files — so the two big
designs are generated. They are shaped to be the two extremes a hierarchy
can take.

"Uncached" is `reticle emit --format verilog --output /dev/null <files>`,
which elaborates the same sources and writes the result out; it is the
closest thing the CLI has to "elaborate this and throw it away", and it
includes the emission, so it slightly overstates the baseline.

### `wide`: 631 files, 25 539 lines

| Run | Time | vs uncached |
|-----|------|-------------|
| uncached | 69 ms | — |
| cold, every module | 160 ms | **2.3× slower** |
| cold, `--only-top` | 100 ms | 1.4× slower |
| warm, every module | 55 ms | 1.3× faster |
| **warm, `--only-top`** | **27 ms** | **2.6× faster** |
| after editing one leaf | 57 ms | 1.2× faster |
| after editing the top | 57 ms | 1.2× faster |

Store: 6.3 MB in 1262 entries.

### `deep`: a chain of 120

| Run | Time | vs uncached |
|-----|------|-------------|
| uncached | 4 ms | — |
| cold, every module | 63 ms | **16× slower** |
| cold, `--only-top` | 7 ms | 1.8× slower |
| warm, every module | 37 ms | 9× slower |
| warm, `--only-top` | 3 ms | 1.3× faster |

Store: 2.3 MB in 242 entries, for 30 KB of source. This is the shape the
default mode is worst at: every one of the 120 stages is elaborated as its
own top, and each one's cone is the whole chain below it, so the cold build
does quadratic work and the store is quadratic in size. `--only-top` avoids
all of it.

### Synthesis, on `wide`

| Run | Time | vs uncached |
|-----|------|-------------|
| uncached `reticle synth` | 122 ms | — |
| cold `cache --synth --only-top` | 165 ms | 1.35× slower |
| **warm `cache --synth --only-top`** | **40 ms** | **3.1× faster** |

### VHDL

| Run | Time |
|-----|------|
| uncached | 6 ms |
| cold | 6 ms |
| **warm** | **1 ms** |

A single 30-line entity, and still a 6× speed-up, because a VHDL build
analyses the bundled `std` and `ieee` libraries before it can elaborate
anything. The cache builds that analysis only when something actually
misses, so a build with nothing to do skips it entirely. This is a fixed
cost, so the ratio falls as the design grows — but it is also the cost that
makes a small VHDL edit feel slow.

### Where the time goes, and why the numbers look like this

On `wide`:

| Step | Time |
|------|------|
| parse 631 `.v` files | 21 ms |
| elaborate them (uncached run) | 69 ms |
| parse the 652 KB `design.rtl` back | 18 ms |

Reading a design back from `.rtl` is about a quarter of the cost of
elaborating it from Verilog. That ratio is the whole story: the cache can
only ever save the difference, so

- **it helps** when the cached stage is much more expensive than reading
  the IR back — synthesis (3.1×), VHDL with its library analysis (6×), and
  a large Verilog design built with `--only-top` (2.6×);
- **it does not help**, and costs, on a cold build: 1.4× slower with
  `--only-top` and 2.3× slower in the default mode, because every module is
  elaborated separately and every artefact is written out;
- **it is actively bad** for a deep chain in the default mode, where
  elaborating each module as its own top is quadratic (16× slower cold);
- **it is noise** on a small design, where everything is a millisecond and
  process start-up dominates.

The rule of thumb: use it when you rebuild the same sources repeatedly and
the expensive stage is synthesis or VHDL, and pass `--only-top` unless you
actually want per-module entries. Do not use it for a one-shot build.

## Correctness before speed

The property that matters more than any of the above is that a cached build
and a cold build produce the same design, byte for byte. Four tests check
it:

- `a_cached_build_and_a_cold_build_agree_byte_for_byte` — a warm build, a
  cold build and an independent cold build in a fresh store all render the
  same `.rtl`, which is also checked in against
  `testdata/cache/design.rtl`.
- `a_partly_cached_build_still_agrees_with_a_cold_one` — the interesting
  case, where some modules come from the store and some are fresh.
- `only_top_builds_one_module_and_shares_the_store` — an artefact does not
  depend on which modules were asked for.
- `cli::a_second_run_hits_and_writes_the_same_design` — the same thing
  through the binary and a real directory.

## Left out

- **No sharing between machines beyond copying the directory.** The key has
  nothing host-specific in it, so a store *is* portable; there is just no
  protocol for fetching one.
- **No cached diagnostics.** See "Consequences" above.
- **No compression.** Entries are plain text, which is what makes them
  inspectable; a 6 MB store for a 25 000-line design is the price.
- **Nothing between elaboration and synthesis is cached** — no mapped
  netlists, no timing results, no simulation state.
