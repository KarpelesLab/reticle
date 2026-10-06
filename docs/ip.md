# IP integration

Third-party and first-party IP should drop into a design the way a crate
drops into a Rust program. `reticle::ip` (Cargo feature `ip`) is what makes
that true: a manifest format for packages and projects, a dependency
resolver with a lock file, bus interfaces described once and then both
generated and checked, interconnect generators, black boxes for encrypted
vendor cores, a static registry index to find packages in, and an importer
for catalogues that already describe their IP in IP-XACT.

This document is the format reference and one complete worked example. The
API reference (`cargo doc --features ip`) has the per-item details.

## Why not TOML

`ROADMAP.md` calls the project manifest `reticle.toml`. It is spelled
**`reticle.proj`** here, and the reason is a rule rather than a taste:
Reticle ships no third-party code and hand-writes every parser it needs
(see `CONTRIBUTING.md`). Growing a TOML parser to read four small files
would be the largest piece of foreign-shaped machinery in the crate, and
calling a file `.toml` that a TOML parser cannot fully read is worse than
calling it something else.

So all four files — `reticle.ip`, `reticle.proj`, `reticle.lock` and the
bus definitions in `src/ip/buses/*.bus` — are **one** line-oriented format,
the same one the `.rcf` constraints, the `.dev` device databases and the
IR's own `.rtl` use:

- one declaration per line, keyword first;
- words separated by whitespace, quoted with `"` when they contain spaces;
- `#` or `//` starts a comment; blank lines are free;
- every line carries a span, so every diagnostic points at a word.

Line-oriented text also diffs, reviews and merges better than a nested
document: a new dependency is one added line, and two branches adding one
each do not conflict.

## `reticle.ip`: an IP package

```text
# The manifest of an IP package, in its directory.
name        uart_lite
version     1.2.0
license     MIT
description "8N1 UART transmitter with a one-entry FIFO"

top         uart_lite
target      ice40

source      rtl/uart_lite.v
source      rtl/uart_regs.vhd language vhdl
source      rtl/uart_phy.vp language verilog encrypted
model       sim/uart_model.v

param       BAUD_DIV int 434 1..65535
param       PARITY string none

port        clk in
port        rst_n in
port        tx out
interface   s_axi axi4lite subordinate prefix s_axi_

constraints board/ice40.rcf
testbench   tb/uart_tb.v

depends     fifo_sync ^1.0.0
```

| Keyword | Takes | Meaning |
|---------|-------|---------|
| `name` | one word | the package name, unique in a dependency graph |
| `version` | `major.minor.patch[-pre]` | the package version |
| `license` | one word | an SPDX identifier where there is one |
| `description` | one word (quote it) | one line about the package |
| `top` | one word | the top entity or module; defaults to `name` |
| `source` | `<path> [language <lang>] [encrypted]` | an HDL source, in analysis order |
| `model` | the same | a behavioural model, used when the sources cannot be read |
| `param` | `<name> <type> [default] [lo..hi]` | a parameter: `int`, `bool`, `string` or `bits` |
| `port` | `<name> <in\|out\|inout> [width]` | a port that does not belong to a bus |
| `interface` | `<name> <bus> <role> [prefix <p>]` | a whole bus interface the package exposes |
| `target` | one word | a device or family the package supports; none means any |
| `constraints` | a path | a constraints file the package ships |
| `testbench` | a path | a testbench the package ships |
| `depends` | `<name> <requirement>` | IP this package needs |

A `source` line's language comes from the extension (`.v`, `.vh`, `.sv`,
`.svh`, `.vhd`, `.vhdl`, `.rtl`) unless the `language` word overrides it.
A width is a number, a parameter name, or a parameter divided by a
constant (`DATA_WIDTH/8`).

An IP manifest's `depends` line says *what* the package needs and never
where that lives. That is deliberate: a package that hard-codes a path to
its dependency is a package nobody else can use. The project places it.

## `reticle.proj`: the user's design

```text
name        blinky
top         top
device      ice40-hx1k-tq144

source      rtl/top.v
constraints board/ice40.rcf
testbench   tb/top_tb.v

library     ../ip

depends     uart_lite ^1.2.0
depends     fifo_sync >=1.0.0 git https://example.invalid/fifo.git rev v1.0.4
depends     cdc_sync  *       registry
depends     vendor_pll 2.1.0  path vendor/pll
```

| Keyword | Takes | Meaning |
|---------|-------|---------|
| `name` | one word | the project name |
| `top` | one word | the top module of the design |
| `device` | one word | the target device, as `reticle::fpga::target` names them |
| `source` | `<path> [language <lang>] [encrypted]` | the project's own HDL |
| `constraints` | a path | a constraints file |
| `testbench` | a path | a testbench |
| `library` | a path | an IP library root to look a dependency's *name* up in; repeatable |
| `depends` | `<name> <requirement> [where]` | IP to pull in, and where from |

`where` is `path <dir>`, `git <url> [rev <r>]` or `registry`, **or nothing
at all**, in which case the name is looked up in the `library` roots — see
[the next section](#the-ip-library-a-dependency-with-no-path). The library
itself does no I/O and no networking: the `PathProvider` it ships resolves
`path` dependencies through a caller-supplied closure, `RegistryProvider`
resolves `registry` ones through an index and another closure (see [the
registry](#the-registry-a-static-index)), and `git` is still declined with a
clear diagnostic. The CLI — or a WebAssembly playground, or a test — is the
only thing that ever opens a file.

## The IP library: a dependency with no path

A package already declares its own identity and already names what it
needs without a path (`depends fifo_sync ^1.0.0`). A project can now do
the same:

```text
library ../../ip

depends uart    ^1.0.0
depends mos6502 ^1.0.0
```

`uart` is found wherever it sits under `../../ip` — including nested in a
category folder — by the **name its own `reticle.ip` declares**, not by
its directory's name. That is the point: `ip/uart/` and `ip/bus/uart/`
are the same package to everything above this line, so moving a block is
a change to `ip/` and to nothing else. The round after this one moved
all twenty-nine, from the first spelling to the second, and no
`depends` line in the repository that had dropped its path needed
editing.

### Where the root comes from

From `library` lines in `reticle.proj`, each relative to the directory
holding that manifest, searched in the order written, plus any
`reticle build --library <dir>` for a build against a library that is not
the project's own.

There is no default and no environment variable. A path in this project
comes from a manifest or from the command line and never from a guess: a
manifest is committed and reviewed, so a reader of the repository can see
which HDL a build used, and `$RETICLE_IP_PATH` would let the same project
build from different sources on two machines with nothing written down
anywhere. A default of `../../ip` would be worse still — a guess that is
right for this repository's examples and wrong everywhere else.

Order decides nothing, because a name found twice is an error rather than
a shadowing. It only decides which root a diagnostic lists first.

### How the index is built, and what it costs

The caller walks each root for `reticle.ip` files and hands the
`(path, text)` pairs to `LibraryIndex::from_manifests`, which reads each
manifest's `name` and `version` — by a small scan, not by
`IpManifest::parse`, so a package nobody depends on cannot push its own
diagnostics into an unrelated build — and sorts the result.

`reticle build` does that walk (`library_manifests` in
`src/bin/reticle/main.rs`): a directory holding a manifest is a package
and is not descended into, so the walk never enters an `rtl/` or a `tb/`;
a symbolic link to a directory is not followed, so a loop cannot hang it;
every level is sorted, because `read_dir` returns filesystem order and
this reaches a lock file; and eight levels is as deep as it looks. The
real `ip/` uses two of those eight — a category folder and then the
package — and the walk needed no change to descend them.

Measured over the real `ip/` — 29 packages, 38910 bytes of manifest, a
release build, `the_library_index_cost` in `tests/ip_library.rs`, which
prints and never asserts:

| Step | Per build |
|------|-----------|
| walk the tree and read every manifest | 100 µs |
| scan, sort and build the index | 59 µs |

Re-measured after the packages were grouped into seven category folders,
which is one more level for the walk to descend: 95 µs became 100 and
57 became 59, over three runs each. Five microseconds is not a finding,
and it is here because the alternative to re-measuring is assuming.

**No cache.** A tenth of a millisecond is four orders of magnitude below
anything a person notices and five below the ECP5 database load that this
project learned to measure rather than assume. A library of ten thousand
packages would be 50 ms, still not worth a cache file that could go
stale; the figure to re-measure is the one above, and the test to
re-measure it with is committed.

### One name, one package

Two manifests under the roots declaring the same name is an error naming
both paths, not a first-wins. A library is a layout — one directory per
package — and the mistake this really catches is a package *copied* where
it should have been *moved*, which is the mistake a reorganisation makes.
The reorganisation came, all twenty-nine packages of it, and this check
caught nothing: the moves were `git mv` and the guard was watching.
A library that genuinely wants two versions of one package side by side
is what a registry is for, or `path`, which says exactly which directory
is meant.

`tests/ip_library.rs` indexes the real `ip/` and asserts that no name is
claimed twice, that every manifest declares a name, and that every
`depends` any block states is a name the library has.

### The three failures

| Code | Reads |
|------|-------|
| `P0801` | `the IP library has no package named `uarte`` — with how many packages were searched under which roots, the nearest name the library does have, and the reminder that a package outside the library is named with `path <dir>` |
| `P0802` | `the IP library has 2 packages named `uart`` — with a note per manifest that declares it |
| `P0102` | `no version of `uart` satisfies every requirement` — the existing conflict, which now says *where* each available version lives: `available: 1.0.0 in the library directory `../../ip/bus/uart`` |

A version mismatch is deliberately **not** a library error. The library
answers "where is this name", and the version machinery already collects
every requirement on a package with the path through the graph that
stated it; giving it a second, library-shaped way to complain about a
version would make a failure depend on which requirement was walked
first. Naming the directory in its note is what that diagnostic was
missing.

### Sans-I/O

Walking a directory is I/O, so the walk belongs to the caller — the CLI,
a test, or a WebAssembly bundle — and the index, the lookup and the
diagnostics are a pure function of the `(path, text)` pairs it hands
over. `src/ip/library.rs` opens nothing, and the paths it is given are in
the same space as every other path the resolver sees: relative to the
project manifest, exactly like a `path` dependency's.

## Version requirements

Four forms, and no more, because a requirement nobody can read is a
requirement nobody can audit:

| Written | Accepts |
|---------|---------|
| `1.2.3` | exactly `1.2.3` |
| `^1.2.3` | `1.2.3` up to the next breaking change |
| `>=1.2.3` | `1.2.3` and anything above |
| `*` | any release |

The caret rule is Cargo's: the leftmost non-zero number may not change, so
`^1.2.3` accepts `1.9.0` but not `2.0.0`, `^0.2.3` accepts `0.2.9` but not
`0.3.0`, and `^0.0.3` accepts only `0.0.3`. A pre-release version only ever
satisfies a requirement that names the same three numbers, so `^1.0.0` does
not quietly pick up `1.1.0-rc1`.

## Resolution

1. The graph is walked depth first from the project's `depends` lines.
2. A transitive dependency's *location* comes from the project's own
   `depends` line for that name when it has one; otherwise the provider
   decides, and `PathProvider` looks the name up in the `library` roots,
   falling back — with no library — to a directory named after the
   package next to the project.
3. Every requirement on a package is collected, and the **highest version
   satisfying all of them** is selected.
4. A cycle, a conflict and a package that cannot be fetched are each a
   diagnostic naming the path through the graph.
5. Packages come back leaves first, ties broken by name, so the build, the
   report and the lock file never depend on iteration order.

## `reticle.lock`

The resolver always produces one, in the same format, so the next build
resolves the same way:

```text
# reticle.lock: the exact IP versions this project resolved to.
# Generated by Reticle. Edit reticle.proj and resolve again.
version 1

package cdc_sync 0.3.1 path ../../packages/cdc_sync
package fifo_sync 1.0.4 path ../../packages/fifo_sync
package uart_lite 1.2.0 path ../../packages/uart_lite

requires blinky cdc_sync ^0.3.0
requires blinky fifo_sync ^1.0.0
requires blinky uart_lite ^1.2.0
requires fifo_sync cdc_sync ^0.3.0
requires uart_lite fifo_sync ^1.0.0
```

`package` lines are sorted by name and `requires` lines by dependent then
dependency, so the file changes only when the resolution does.

The origin is **the answer the resolution got**, not the words the
manifest wrote: `path <dir>` when the manifest named a directory,
`library <dir>` when a `library` search found the name there, `registry`
or `git` for those. So a lock file still encodes a layout — that is its
job. It records what was built from, and a layout that changes is
something a build must be able to notice.

Nothing ever resolves *from* a lock file: resolution always goes through
the manifests and the index, and the lock file is compared against the
result. That is what makes a stale one an error rather than a wrong
build. `LockFile::differences` says in words how a stored lock file and a
fresh resolution disagree, `LockFile::mismatch` turns that into a `P0302`
diagnostic, and `reticle build --locked` fails with it and writes
nothing:

This one is `examples/mos6502_computer` built against the lock file it had
before `ip/` was grouped into category folders, which is the real case
rather than a constructed one:

```text
error[P0302]: the lock file does not describe this resolution
 --> reticle.lock:1:1
  |
1 | version 1
  | ^
  |
  = note: `mos6502` moves from the library directory `../../ip/mos6502`
          to the library directory `../../ip/cpu/mos6502`
  = note: `uart` moves from the library directory `../../ip/uart` to the
          library directory `../../ip/bus/uart`
  = note: resolve again without `--locked` to write the new one
```

Without `--locked`, a build that resolves cleanly rewrites the lock file,
which is the same decision Cargo makes. No lock file is committed in this
repository — each example's is written into a scratch directory by its
test — so the move had nothing to regenerate and a build simply wrote the
new paths. A consumer who commits one gets the two notes above, which is
the whole point of recording the directory and not only the version.

## Bus interfaces

A bus is described **once**, as data, in a `.bus` file:

```text
bus axi4lite
  param ADDR_WIDTH 32
  param DATA_WIDTH 32

  signal awaddr out ADDR_WIDTH
  signal awprot out 3 optional
  signal awvalid out 1
  signal awready in 1
  ...
end
```

Every direction is the one the **manager** sees; a `role` of
`subordinate` mirrors them all and `monitor` makes them all inputs. A
width is a number or a parameter, optionally divided, which is how `wstrb`
is `DATA_WIDTH/8`. AXI4, AXI4-Lite, AXI4-Stream, Wishbone (classic and
pipelined), APB and Avalon-MM ship built in, under `src/ip/buses/`; adding
another bus is adding another file and touching no Rust.

Clocks and resets are not part of a bus definition on purpose: one clock
usually serves several interfaces, so an IP declares it with a `port` line.

From that one description Reticle does three things.

**Checks a module.** `bus::match_ports(module, bus, role, prefix)` finds
the bus on a module's ports by naming convention — the prefix plus the
standard signal name, compared without regard to case — and reports:

| Code | Problem |
|------|---------|
| `P0201` | a required signal with no port |
| `P0202` | a port whose direction is the wrong way round |
| `P0203` | a port whose width contradicts the parameters |

Widths are *inferred* where the module does not declare the parameter: the
first port that pins `DATA_WIDTH` down binds it, and everything after is
checked against that. A 32-bit `wdata` next to a 2-bit `wstrb` is reported
even when the module never says what `DATA_WIDTH` is.

**Wires two instances.** `bus::connect` matches both ends, pairs the
signals up and returns the nets that join them; `bus::wire` applies them.
Nineteen port lines per side become one call.

**Generates interconnect.** `ip::Crossbar` builds a parameterised AXI4-Lite
crossbar — N managers by M subordinates, with an address decode map and
round-robin arbitration — and `ip::WishboneArbiter` builds a Wishbone
shared-bus arbiter. Both take their ports from the same bus definitions, so
a generated crossbar passes `match_ports` by construction rather than by
two pieces of code agreeing.

The crossbar is a *shared-access* crossbar: full N×M connectivity, one
transaction in flight, one manager granted at a time, the next grant
starting at the manager after the last so nothing starves. An address that
decodes to no subordinate is answered by the crossbar itself with `DECERR`.
That is deliberately the simple thing rather than the fast thing; a
generator whose output cannot be read is a generator that cannot be
trusted.

## Encrypted and vendor IP

A vendor core often arrives as ciphertext. Its manifest is still plain
text, and that is enough:

```text
name        vendor_ddr
version     2.1.0
license     proprietary

top         ddr_phy

source      rtl/ddr_phy.vp language verilog encrypted

param       DATA_WIDTH int 32 8..64

port        clk in
port        rst_n in
port        ddr_ck out
port        ddr_dq inout DATA_WIDTH
interface   s_axi axi4lite subordinate prefix s_axi_
```

From that, `ip::blackbox::stub` builds an `ir::Module` with the declared
ports — the one `interface` line expanding into all nineteen AXI4-Lite
ports at the declared widths — `blackbox` set and no contents. The design
around it elaborates, validates, lints and emits; the vendor tool fills the
box in.

When the manifest names a `model`, simulation runs that model instead and
the build report says so, because a simulation that quietly ran a model is
a result nobody should have to guess at.

## A complete worked example

Everything below is in `testdata/ip/` and is built by
`tests/ip_project.rs`.

### The layout

```text
testdata/ip/
  packages/
    cdc_sync/     reticle.ip  rtl/cdc_sync.v
    fifo_sync/    reticle.ip  rtl/fifo_sync.v  tb/fifo_sync_tb.v
    uart_lite/    reticle.ip  rtl/uart_lite.v
  projects/
    two_deps/     reticle.proj  rtl/top.v
```

### The project

`testdata/ip/projects/two_deps/reticle.proj`:

```text
name blinky
top top
device ice40-hx1k-tq144

source rtl/top.v

depends uart_lite ^1.2.0 path ../../packages/uart_lite
depends fifo_sync ^1.0.0 path ../../packages/fifo_sync
depends cdc_sync ^0.3.0 path ../../packages/cdc_sync
```

The only HDL its author writes is `rtl/top.v`, which instantiates
`uart_lite` and wires it to the board's pins.

### The two packages it names

`packages/uart_lite/reticle.ip` declares a dependency of its own:

```text
name uart_lite
version 1.2.0
license MIT
description "8N1 UART transmitter with a one-entry FIFO"

top uart_lite
target ice40

source rtl/uart_lite.v

param BAUD_DIV int 434 1..65535

port clk in
port rst_n in
port tx_data in 8
port tx_valid in
port tx_ready out
port tx out

depends fifo_sync ^1.0.0
```

`packages/fifo_sync/reticle.ip` declares one more:

```text
name fifo_sync
version 1.0.4
license MIT
description "Synchronous FIFO with a synchronised reset"

top fifo_sync
target ice40
target ecp5

source rtl/fifo_sync.v

param WIDTH int 8 1..64

port clk in
port rst_n in
port wdata in WIDTH
port push in
port pop in
port rdata out WIDTH
port full out
port empty out

testbench tb/fifo_sync_tb.v

depends cdc_sync ^0.3.0
```

So the graph is a diamond: the project needs `uart_lite` and `fifo_sync`,
`uart_lite` needs `fifo_sync` too, and `fifo_sync` needs `cdc_sync`, which
the project places even though its own sources never name it.

### Building it

```rust
use reticle::diag::Diagnostics;
use reticle::ip::{self, PathProvider};
use reticle::source::SourceMap;

// The one place the filesystem appears. Everything else is sans-I/O.
let read = |path: &str| std::fs::read_to_string(root.join(path)).ok();

let text = read("reticle.proj").expect("a project manifest");
let mut map = SourceMap::new();
let mut diags = Diagnostics::new();
let project = ip::load_project(&mut map, "reticle.proj", &text, &mut diags).unwrap();

let mut provider = PathProvider::new(".", read);
let mut resolved = ip::resolve(map, &project, &mut provider, &mut diags);
assert!(resolved.is_complete());

let build = ip::elaborate(&project, &mut resolved, &mut diags);
assert!(!diags.has_errors(), "{}", diags.render(resolved.source_map()));
print!("{}", build.report());
std::fs::write("reticle.lock", resolved.lock.to_text()).unwrap();
```

### What comes out

The build report, which is `testdata/ip/two_deps.build`:

```text
sources
  cdc_sync: rtl/cdc_sync.v (verilog)
  fifo_sync: rtl/fifo_sync.v (verilog)
  uart_lite: rtl/uart_lite.v (verilog)
  blinky: rtl/top.v (verilog)
design
  top: top
  modules: cdc_sync, fifo_sync, top, uart_lite
```

Sources come in dependency order — the deepest package first, the
project's own last — so a frontend never sees an instantiation before its
module. The lock file is the one shown under
[`reticle.lock`](#reticlelock) above.

From there the design is an ordinary `ir::Design`: `reticle::sim` runs it,
`reticle::synth` synthesises it, `reticle::fpga` maps it to a device, and
`Design::to_text` writes the `.rtl` that `testdata/ip/two_deps.rtl` pins.

### When it goes wrong

Change the project to ask for a `fifo_sync` that does not exist alongside a
package that asks for another, and resolution says exactly who wanted what
(`testdata/ip/conflict.diag`):

```text
error[P0102]: no version of `fifo_sync` satisfies every requirement
  --> packages/uart_strict/reticle.ip:16:1
   |
16 | depends fifo_sync ^2.0.0
   | ^^^^^^^^^^^^^^^^^^^^^^^^ `conflicted > uart_strict` requires ^2.0.0
  ::: projects/conflict/reticle.proj:9:1
   |
 9 | depends fifo_sync ^1.0.0 path ../../packages/fifo_sync
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ `conflicted` requires ^1.0.0
   |
   = note: available: 1.0.4
```

### A project around encrypted IP

`testdata/ip/projects/encrypted/` depends on two vendor cores: one with no
readable source at all, one with a behavioural model. Both elaborate, and
the report is explicit about which is which
(`testdata/ip/encrypted.build`):

```text
sources
  vendor_pll: sim/vendor_pll_model.v (verilog)
  encrypted_soc: rtl/top.v (verilog)
black boxes
  black box `ddr_phy` from vendor_ddr 2.1.0 (encrypted sources)
    interface s_axi: 19 ports
    23 ports in total
  black box `vendor_pll` from vendor_pll 1.0.0 (the behavioural model `sim/vendor_pll_model.v`)
    4 ports in total
design
  top: top
  modules: ddr_phy, top, vendor_pll
```

### A generated interconnect

`testdata/ip/projects/crossbar/` has no `top` and no `source` at all: its
top level is generated.

```rust
use reticle::ip::bus::{BusEndpoint, connect, wire};
use reticle::ip::{AddressRange, Crossbar};

let crossbar = Crossbar::new(
    "axil_xbar",
    2,                                    // managers
    vec![
        AddressRange::new(0x0000, 0x1000),
        AddressRange::new(0x1000, 0x1000),
    ],
    span,
);
assert!(crossbar.problems().is_empty());   // aligned, non-overlapping
let xbar = design.add_module(crossbar.build());

// ... instantiate the crossbar and two copies of the resolved
// `axi_regs` package in a top module, then:
let links = connect(
    &design,
    top,
    &BusEndpoint::new(u_xbar, "m0_"),
    &BusEndpoint::new(u_regs0, "s_"),
    reticle::ip::bus::builtin("axi4lite").unwrap(),
)?;
wire(&mut builder, u_xbar, u_regs0, &links);
```

`tests/ip_project.rs` then runs it: manager 0 writes to both subordinates,
manager 1 reads both back, and an address no subordinate claims comes back
with `DECERR` while leaving the bus usable. That is what proves the
generator, rather than a golden netlist that could be wrong in exactly the
same way twice.

### A project with no path at all

`testdata/ip/projects/library/` is the same shape as the first example
with every `path` removed. It declares `library ../../library` and one
dependency, `clock_div ^1.0.0`; the fixture library under
`testdata/ip/library/` holds that package at `timing/divider/` and the
package it needs, `pulse_edge`, at `glue/edge/` — two levels down, in
directories deliberately not named after the packages in them, and the
second one not mentioned by the project at all. Its lock file is the
whole of what the search decided:

```text
package clock_div 1.0.0 library ../../library/timing/divider
package pulse_edge 1.0.0 library ../../library/glue/edge

requires clock_div pulse_edge ^1.0.0
requires library_search clock_div ^1.0.0
```

`examples/mos6502_computer` is the same thing over the real library: a
`library ../../ip` line, `depends mos6502 ^1.0.0`, `depends uart ^1.0.0`,
and nothing in the manifest that knows where either of them sits.

## The registry: a static index

A registry is what the crates.io index is: a **git repository of
manifests**, one file per package and one line per released version. There
is no server and no API — cloning or pulling the repository is the whole
protocol, `git log` is the audit trail, and a mirror is a clone.

`registry::index_path` puts each package where its name says, so no
directory grows past the packages sharing four leading characters:

| Name | Path |
|------|------|
| `a` | `1/a` |
| `ab` | `2/ab` |
| `abc` | `3/a/abc` |
| `fifo_sync` | `fi/fo/fifo_sync` |

Each file is the same line-oriented format as everything else here:

```text
package fifo_sync 1.0.0 checksum 6f1e… description "A synchronous FIFO"
package fifo_sync 1.0.4 checksum 91a2… description "A synchronous FIFO" depends cdc_sync ^0.3.0
package fifo_sync 1.1.0 checksum 0c77… yanked
```

After the name and the version the words come in a fixed order: `checksum
<word>`, then `yanked` if it is, then `description "<text>"` if it has one,
then one `depends <name> <requirement>` per dependency. A line is a
*summary* of a manifest — enough to resolve a whole graph without fetching
anything, and nothing more.

What the checksum covers, and with which algorithm, is the publisher's
business: Reticle ships no hash function, so verification belongs next to
the bytes, in the fetcher. The field is carried so a lock file can be
checked against the index next time round.

The API over it:

| Call | What it does |
|------|--------------|
| `Index::parse` / `Index::to_text` | read and write index text |
| `Index::files` | every `(path, contents)` pair, for writing the tree out |
| `Index::search(query)` | substring and prefix over names and descriptions |
| `Index::versions(name)` | every release, oldest first |
| `Index::best(name, &req)` | the highest release satisfying `req` |
| `IndexEntry::from_manifest` | summarise a `reticle.ip` for publication |

`search` ranks by how the package matched — exact name, then name prefix,
then name substring, then description — and breaks ties by name, so the
same query over the same index always returns the same list. Each package
appears once, showing the newest version that is neither yanked nor a
pre-release. `best` skips yanked releases unless the requirement names one
exactly, which is what lets a lock file keep building after a yank.

### Resolving through it

`RegistryProvider` implements the same `SourceProvider` trait `PathProvider`
does, so a registry dependency resolves exactly like a path one:

```rust
let mut provider = RegistryProvider::new(&index, |path: &str| cache.get(path))
    .with_root("."); // where the project's own sources are
let resolved = Resolver::new(map).resolve(&project, &mut provider, &mut diags);
```

The library still performs no I/O. The provider is handed a path like
`fifo_sync-1.0.4/reticle.ip` and the closure answers it from whatever the
caller has: a checkout, a cache directory, or a bundle compiled into a
WebAssembly module.

### `reticle add`

`registry::add(&ProjectFile::new(&project, &text), name, &req, &index)` is
the library half of the command a user types. It picks the best version the
index offers, writes the `depends` line into the manifest **text**, and
returns the rewritten text along with the version it chose and the line it
wrote.

The rewrite keeps every comment, every blank line and the existing
alignment, and puts the new line where it belongs: in name order when the
file is already in name order, after the last `depends` line otherwise, and
after a blank line at the end of the file when there are none. A tool that
reformats a file it was asked to add one line to is a tool nobody lets near
their repository twice.

```text
# The IP this needs.
depends     cdc_sync ^0.3.0 registry
depends     fifo_sync ^1.0.0 registry   <- added here, aligned like its neighbours
depends     uart_lite ^1.2.0 registry
```

`AddError` covers the three ways it can refuse: no such package (with a
"did you mean"), no release satisfying the requirement (listing what there
is, yanked ones marked), and a package the project already depends on
(pointing at the line that has it).

## IP-XACT import

A catalogue that already describes its IP in IP-XACT does not have to be
rewritten. `ipxact::import(xml, &ImportOptions::new(file), &mut diags)`
reads one **component** description and produces a `reticle.ip` plus a
report of what happened to every part of it.

The root element's namespace decides which revision is being read, because
the prefix is only a spelling and real catalogues are a mix:

| Namespace | Read as |
|-----------|---------|
| `…/SPIRIT/1685-2009` | IEEE 1685-2009, usually spelled `spirit:` |
| `…/IPXACT/1685-2014` | IEEE 1685-2014, `ipxact:` |
| `…/IPXACT/1685-2022` | IEEE 1685-2022, `ipxact:` |

An older SPIRIT namespace (1.2 to 1.5) is read as 1685-2009 and said to be.
Anything else is an error: guessing at a schema nobody has named is how an
importer silently loses half a component. Both spellings of everything that
moved between revisions are handled — `wire/vector` against
`wire/vectors/vector`, `modelParameters` against a component
instantiation's `moduleParameters`, port maps on the `busInterface` against
port maps on its `abstractionType`, and `master`/`slave` against
`initiator`/`target`.

### What maps to what

| IP-XACT | `reticle.ip` |
|---------|--------------|
| VLNV `name`, `version` | `name`, `version` |
| `description` | `description`, with the memory maps appended |
| a view's model or module name | `top` |
| `model/ports/port` (wire ports) | `port <name> <dir> [width]` |
| `fileSets/fileSet/file` | `source <path> [language <lang>]` |
| `parameters`, `modelParameters`, `moduleParameters` | `param <name> <type> [default] [lo..hi]` |
| `busInterfaces/busInterface` | `interface <name> <bus> <role> [prefix <p>]` |
| `memoryMaps` | a phrase in `description` |

Vector bounds are expressions over the component's parameters
(`ADDR_WIDTH-1` down to `0`), so they are evaluated against the parameters'
values — integers, `+ - * / %`, parentheses, and names resolved by
parameter name *or* by `parameterId`. A bound that does not evaluate but
has the shape `PARAM-1` or `PARAM/n-1` becomes the manifest's own derived
width, which is exactly what carries a `DATA_WIDTH/8` byte strobe across.
Anything else is reported and the **port is dropped**: a width Reticle had
to invent would be worse than a missing line, because nobody would ever
check it.

Memory maps and address blocks become a phrase in the description —
`memory map REGS: CTRL at 0x40000000 range 0x1000 width 32` — and nothing
else. A register map is not logic, and inventing logic from one is how an
importer produces a design that looks right and is not.

### The bus mapping

A `busType` is a VLNV of its own. Its name is normalised (upper case,
everything but letters and digits removed) and looked up:

| `busType` name | Reticle bus | Note |
|----------------|-------------|------|
| `AXI4LITE`, `AXILITE` | `axi4lite` | |
| `AXI4STREAM`, `AXISTREAM`, `AXIS` | `axi4stream` | |
| `AXI4` | `axi4` | |
| `AXI3`, `AXI` | `axi4` | approximated: AXI4 is the closest built-in |
| `APB`, `APB2`, `APB3`, `APB4` | `apb` | |
| `WISHBONE`, `WISHBONEB4`, `WB` | `wishbone` | |
| `WISHBONEPIPELINED`, `WBPIPELINED` | `wishbone_pipelined` | |
| `AVALON`, `AVALONMM`, `AVALONMEMORYMAPPED` | `avalon_mm` | |

`ImportOptions::with_bus` adds to that table, for a site with its own `.bus`
files. A bus nothing matches is **not** guessed at: the interface is dropped
with a diagnostic naming the whole VLNV, and its ports stay in the manifest
as plain `port` lines, so the component loses an abstraction but never a
signal.

The port-name prefix comes from the port maps — a physical `s_axi_awaddr`
for the logical `AWADDR` leaves `s_axi_` — and from the interface's own name
when there are none. Ports a recognised interface stands for are not written
again as `port` lines, since the `interface` line already declares them.

### Silence is the enemy

`ImportedIp::report` has three lists — `translated`, `approximated`,
`dropped` — and `describe()` renders them:

```text
translated
  standard IEEE 1685-2014 (ipxact)
  component reticle.example:peripherals:axil_gpio:2.1 -> package axil_gpio 2.1.0
  interface S_AXI: amba.com:AMBA4:AXI4-Lite:r0p0_0 -> axi4lite subordinate (prefix `s_axi_`, 5 signals)
approximated
  version `2.1`: the missing numbers are zero, giving 2.1.0
  parameter GPIO_WIDTH: the default `ADDR_WIDTH-4` evaluates to 8
dropped
  port scan_mode: a phantom port is not in the HDL
  interface M_AHB: no bus matches amba.com:AMBA3:AHBLite:r2p0_0 (1 ports kept as plain ports)
  file syn/axil_gpio.sdc: the type `SDC` is not an HDL source
  vendorExtensions: vendor extensions are not read
```

An import that quietly halves a component is the failure that bites months
later, when a synthesised design turns out to be missing an interrupt line
nobody noticed had gone. Everything interesting is a diagnostic as well.

Version strings are the other place a catalogue and a resolver disagree.
IP-XACT leaves the VLNV version as free text, so `1.2` becomes `1.2.0`,
`1.2.3.4` keeps its first three numbers, `2.0.1_beta` becomes
`2.0.1-beta`, and `r1p3` — which names no number at all — becomes
`0.0.0-r1p3`, sorting below every release. Each is reported, and
`ImportOptions::with_version` overrides all of it.

### What is not read

`import` reads a `component`, and says so when handed a design, a catalogue,
a bus definition or an abstraction definition. Within a component,
transactional (TLM) ports, `vendorExtensions`, `choices`, `cpus`,
`channels`, `whiteboxElements`, `indirectInterfaces`, address spaces,
register field details and a view's file-set references are skipped, most of
them with a line in the report. Every file set is imported, in document
order, since which of them a view refers to is not followed.

### The XML reader

IP-XACT is XML and the crate has no parser, so `ip::xml` is one: elements,
attributes, namespaces resolved to URIs, text, CDATA, comments, processing
instructions, self-closing tags, the five predefined entities and numeric
character references. A malformed document produces exactly **one**
diagnostic, with a span, and no tree; nothing is guessed.

There are no DTDs, no external entities and no schema validation, and the
refusal is deliberate. An entity reference that is not one of the five
predefined ones, and a `<!DOCTYPE>` with an internal subset or an external
identifier, are both errors:

```text
error[P0504]: Reticle will not resolve the entity `&xxe;`
  = note: only `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` and numeric references are
          read: Reticle parses no DTD and never fetches an external entity, so expand
          the document before importing it
```

Resolving external entities is how XML parsers become file-disclosure and
request-forgery holes, and a document that needs one can be expanded by
whatever wrote it. The library performs no I/O in any case, so there would
be nowhere for a resolved entity to come from.

## Diagnostic codes

| Range | Where |
|-------|-------|
| `P0001`–`P0008` | manifest syntax: unknown key, shape, version, requirement, duplicate, missing, unknown word, parameter |
| `P0101`–`P0105` | resolution: not found, conflict, cycle, name mismatch, unsupported source |
| `P0201`–`P0205` | buses: missing signal, direction, width, unknown bus, `.bus` syntax |
| `P0301`–`P0302` | lock file: syntax, and a lock file that does not describe this resolution |
| `P0401`–`P0404` | building: no such top, unknown language, (retired), invalid design |
| `P0501`–`P0507` | XML: syntax, mismatched tag, entity, refused entity or DTD, duplicate attribute, unbound prefix, nesting |
| `P0601`–`P0608` | IP-XACT: unknown standard, not a component, missing element, odd version, unresolved expression, unknown bus, unknown file type, unknown direction |
| `P0701`–`P0704` | registry: index syntax, no such package, no such version, already a dependency |
| `P0801`–`P0802` | IP library: no package of that name, two packages of one name |

An unknown key comes with a "did you mean" over the keys that manifest
kind does have.

## What is not here yet

- **A `git` dependency is declined.** Fetching one is network I/O, which
  the library does not do; vendor the package or use a checkout and a
  `path` dependency. A `registry` dependency now resolves through
  `RegistryProvider`, whose fetcher the caller supplies for the same
  reason.
- **No `reticle add` command yet.** `registry::add` is the library half
  and is tested; wiring it to the binary, along with `reticle search`
  and a place to keep the index checkout, is phase 9 work.
- **`` `include `` is not followed** during a project build: a package
  lists its files in its manifest, which is what the provider reads.
- **The IP-XACT import reads components only**, and within one skips what
  the list above says it skips. Nothing writes IP-XACT back out.

The first-party IP library is in `ip/` and has its own document,
[`ip-library.md`](ip-library.md): twenty-nine Verilog-2005 packages,
filed under `bus/`, `cpu/`, `memory/`, `net/`, `usb/`, `util/` and
`video/` — two processor cores, two FIFOs, two clock domain crossings, a
UART, an SPI master, an I²C master, two Ethernet MACs, seven USB blocks,
a video timing generator and a DVI transmitter among them — each with a
co-simulation test and a measured resource footprint. The folders are a
filing system: a block is reached by the name its own manifest declares,
and nothing reads a category.
