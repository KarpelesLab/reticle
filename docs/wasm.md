# The WebAssembly build

Reticle is written from scratch with no dependencies, never opens a file
and never reads a clock, so it compiles for `wasm32-unknown-unknown`
essentially as it stands. The `wasm` feature adds a small
JavaScript-facing surface over the same operations as the C API, and
`web/` is a playground built on it: a textarea of Verilog, buttons to
check, synthesise and emit, and a pane of diagnostics and output.

There is no bindgen and no generated glue. The module exports plain
`extern "C"` functions over linear memory with an allocator pair, and
[`web/reticle.js`](../web/reticle.js) — about two hundred lines, written
by hand — turns those into a comfortable API.

## Building

```sh
# Type-check and compile the library for wasm (what CI runs).
cargo build --target wasm32-unknown-unknown --release \
    --no-default-features --features wasm,verilog,vhdl,sim,synth

# Produce the loadable module: target/wasm32-unknown-unknown/release/reticle.wasm
cargo rustc --lib --target wasm32-unknown-unknown --release \
    --no-default-features --features wasm,verilog,vhdl,sim,synth \
    --crate-type cdylib
```

The second command is `cargo rustc` rather than `cargo build` because the
crate's `[lib]` is a plain `rlib`: a Rust consumer should not pay to link
a `cdylib` it will never load, so the crate type is asked for per build
rather than declared. Install the target once with
`rustup target add wasm32-unknown-unknown`.

Then copy the module next to the page:

```sh
cp target/wasm32-unknown-unknown/release/reticle.wasm web/
python3 -m http.server --directory web 8000   # then open localhost:8000
```

`web/index.html` also works straight from `file://`: browsers block
`fetch` for local files, so the page falls back to `XMLHttpRequest` and,
if that is blocked too, offers a file picker to load `reticle.wasm` by
hand.

## Size

Measured with rustc 1.98.0 and the release profile in `Cargo.toml`
(`lto = "thin"`, `codegen-units = 1`), unstripped, straight out of
`cargo rustc` with no `wasm-opt` pass:

| Features | `.wasm` | gzipped |
|----------|--------:|--------:|
| `wasm` (no stages) | 723,741 B (707 KiB) | 215,973 B (211 KiB) |
| `wasm,verilog` | 1,566,710 B (1.49 MiB) | 490,746 B (479 KiB) |
| `wasm,verilog,sim` | 1,819,540 B (1.74 MiB) | 568,431 B (555 KiB) |
| `wasm,verilog,vhdl,sim,synth` | **3,852,535 B (3.67 MiB)** | 1,180,212 B (1.13 MiB) |

The full build is the one the playground uses; the last column is what a
browser actually downloads from a server with compression on. VHDL is the
largest single contributor, because the bundled `std` and `ieee` library
sources are compiled into the binary. A playground that only lints Verilog
is a third of the size, which is why every stage is a feature.

`wasm-opt -Oz` (from Binaryen, not a dependency of this project) takes
another 15–20% off if you have it; nothing here needs it.

## Portability audit

The library is sans-I/O by design, so almost nothing had to change. What
was checked, and what came of it:

- **`std::fs` and `std::env`.** Only in `src/bin/` (the CLI, which is not
  part of a wasm build) and in two `#[cfg(test)]` modules,
  `src/synth/selfcheck.rs` and the golden-file tests under `tests/`.
  Nothing under `src/` outside `src/bin/` touches the filesystem.
- **`std::time`.** Only in `src/formal/sat/tests.rs`, which is
  `#[cfg(test)]`. No stage reads a clock, and nothing seeds a generator
  from one: `SimOptions::seed` defaults to a constant, which is also why
  simulation output is reproducible.
- **Threads and processes.** None anywhere in the library.
- **32-bit arithmetic.** One real bug: `sim::fst::zlib::gzip_compress`
  reduced a length with `data.len() % (1 << 32)`, and `1 << 32` does not
  fit a 32-bit `usize`. On a 64-bit host it was correct and invisible; on
  wasm32 it is a compile-time overflow error. The reduction now happens in
  `u64`.

That is the whole list. `std::io` is used by the FST writer and is
available on `wasm32-unknown-unknown`; it writes to a `Vec`, not to a
file.

## The ABI

The module exports these, and nothing else:

```
reticle_wasm_alloc(len: usize) -> *mut u8
reticle_wasm_free(ptr: *mut u8, len: usize)
reticle_wasm_version() -> *mut u8
reticle_wasm_features() -> *mut u8
reticle_wasm_elaborate(language, name, source, top) -> *mut u8
reticle_wasm_synth(rtl, lut_inputs: u32) -> *mut u8
reticle_wasm_emit(rtl, format) -> *mut u8
reticle_wasm_simulate(rtl, top, ticks: u64, vcd: u32) -> *mut u8
```

Each name above stands for a `(pointer, length)` pair of UTF-8 bytes. A
**result** is one pointer to a buffer laid out as a little-endian `u32`
byte length followed by that many bytes of UTF-8; the caller decodes it
and hands the buffer back with `reticle_wasm_free(ptr, 4 + len)`. A null
result means the allocation failed, and is the only case the wrapper
special-cases. A panic inside the module is caught and comes back as an
ordinary failed reply, never as an unwind through the host.

Every call but `version` and `features` answers with JSON of one shape:

```json
{
  "ok": true,
  "errors": 0,
  "warnings": 1,
  "rendered": "warning: …\n --> top.v:3:12\n …",
  "diagnostics": [
    {
      "severity": "warning",
      "code": "V0007",
      "message": "the assignment truncates the value",
      "file": "top.v",
      "line": 3,
      "column": 12,
      "notes": []
    }
  ]
}
```

plus whatever that call produces: `rtl` from `elaborate` and `synth`,
`report` from `synth`, `output` from `emit` and `simulate`, and `time`,
`status` and `vcd` from `simulate`. `rendered` is the same rustc-style
text the CLI prints; `diagnostics` is the same information as objects, for
a caller that wants to place markers in an editor.

### No handles

A design travels between steps as `.rtl` text rather than as a handle, so
JavaScript never owns a pointer it has to release. That costs a re-parse
between steps and removes the whole class of lifetime bugs a handle API
has in a garbage-collected host. The `.rtl` round trip is exact, so
nothing is lost on the way.

## Using the wrapper

```js
import { Reticle } from './reticle.js';

const reticle = await Reticle.load('./reticle.wasm');
reticle.version();            // "0.1.0"
reticle.features();           // ["verilog", "vhdl", "sim", "synth"]

const source = 'module m(input a, output y); assign y = ~a; endmodule';

// Diagnostics only.
const { ok, diagnostics, rendered } = reticle.check('verilog', 'm.v', source);

// Source to netlist in one call: elaborate, synthesise, emit.
const built = reticle.compile('verilog', 'm.v', source, {
  synth: true,
  lutInputs: 4,
  format: 'verilog',
});
console.log(built.output);

// Or step by step, passing the `.rtl` along.
const elaborated = reticle.elaborate('verilog', 'm.v', source, 'm');
const run = reticle.simulate(elaborated.rtl, { top: 'm', ticks: 1000, vcd: true });
console.log(run.time, run.status, run.vcd.length);
```

`compile` stops at the first step that fails and returns that step's reply
with a `step` field saying where, so a caller has one thing to check
rather than three.

## Tests

`src/wasm/tests.rs` drives the protocol exactly as `reticle.js` does —
allocate, call, read the length-prefixed reply, free — and runs on the
host, so `cargo test` covers it without a browser. It includes a round
trip from Verilog source through synthesis, emission and simulation with
nothing but text in between.

What that cannot check is the module actually loading in a JavaScript
host; that is what `web/index.html` is for.
