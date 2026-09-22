# IP integration

Third-party and first-party IP should drop into a design the way a crate
drops into a Rust program. `reticle::ip` (Cargo feature `ip`) is what makes
that true: a manifest format for packages and projects, a dependency
resolver with a lock file, bus interfaces described once and then both
generated and checked, interconnect generators, and black boxes for
encrypted vendor cores.

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

depends     uart_lite ^1.2.0 path ../ip/uart_lite
depends     fifo_sync >=1.0.0 git https://example.invalid/fifo.git rev v1.0.4
depends     cdc_sync  *       registry
```

| Keyword | Takes | Meaning |
|---------|-------|---------|
| `name` | one word | the project name |
| `top` | one word | the top module of the design |
| `device` | one word | the target device, as `reticle::fpga::target` names them |
| `source` | `<path> [language <lang>] [encrypted]` | the project's own HDL |
| `constraints` | a path | a constraints file |
| `testbench` | a path | a testbench |
| `depends` | `<name> <requirement> [where]` | IP to pull in, and where from |

`where` is `path <dir>`, `git <url> [rev <r>]` or `registry`. The library
itself does no I/O and no networking: the `PathProvider` it ships resolves
`path` dependencies through a caller-supplied closure and declines `git`
and `registry` with a clear diagnostic, so the CLI (or a WebAssembly
playground, or a test) is the only thing that ever opens a file.

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
   decides, and `PathProvider` looks for a directory named after the
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
`LockFile::differences` says in words how a stored lock file and a fresh
resolution disagree, which is what a `--locked` build reports instead of
moving silently.

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

## Diagnostic codes

| Range | Where |
|-------|-------|
| `P0001`–`P0008` | manifest syntax: unknown key, shape, version, requirement, duplicate, missing, unknown word, parameter |
| `P0101`–`P0105` | resolution: not found, conflict, cycle, name mismatch, unsupported source |
| `P0201`–`P0205` | buses: missing signal, direction, width, unknown bus, `.bus` syntax |
| `P0301` | lock file syntax |

An unknown key comes with a "did you mean" over the keys that manifest
kind does have.

## What is not here yet

- **`git` and `registry` dependencies are declined.** Fetching one is
  network I/O, which the library does not do; vendor the package or use a
  checkout and a `path` dependency. The grammar is there so the manifests
  do not have to change when the registry arrives.
- **`` `include `` is not followed** during a project build: a package
  lists its files in its manifest, which is what the provider reads.
- **IP-XACT import**, the Reticle IP library itself and the registry index
  are the rest of phase 8.
