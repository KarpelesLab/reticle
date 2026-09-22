# The C API

Reticle's compiler is a Rust library, but the pipeline it implements —
Verilog and VHDL in, a netlist, a simulation or a diagnostic out — is
useful from anywhere. The `ffi` feature adds a flat C ABI over it, so an
editor plugin, a build tool or a test harness written in C, C++, Python,
Go or anything else with a C FFI can drive the whole thing.

The header is [`src/ffi/reticle.h`](../src/ffi/reticle.h); it ships with
the crate. [`examples/ffi/demo.c`](../examples/ffi/demo.c) is a complete
program that elaborates, simulates, synthesises and emits.

## Building

The crate's `[lib]` is a plain `rlib`, so a shared or static library is a
separate `cargo rustc` invocation rather than a different `cargo build`.
That is deliberate: a Rust consumer should not pay to link a 150 MB
archive it will never use.

```sh
# A shared library: target/release/libreticle.so (.dylib, reticle.dll)
cargo rustc --lib --release --features ffi --crate-type cdylib

# A static library: target/release/libreticle.a (reticle.lib)
cargo rustc --lib --release --features ffi --crate-type staticlib
```

`--features ffi` alone gives you the ABI with no stages behind it, which
is only useful for checking that something links. Ask for the stages you
want:

```sh
cargo rustc --lib --release --no-default-features \
    --features ffi,verilog,vhdl,sim,synth --crate-type staticlib
```

Every symbol in `reticle.h` exists in every build. A call into a stage
that is not there returns `RETICLE_ERR_UNSUPPORTED`, and
`reticle_features` reports what is present, so one header and one set of
bindings work against any build.

### Linking

Against the shared library, nothing special:

```sh
cc -I src/ffi -o demo examples/ffi/demo.c -L target/release -lreticle
LD_LIBRARY_PATH=target/release ./demo
```

Against the static library, Rust's `std` pulls in a few system libraries:

| Platform | Extra flags |
|----------|-------------|
| Linux    | `-lpthread -ldl -lm` |
| macOS    | `-lpthread -ldl -lm -framework CoreFoundation` |
| Windows (MSVC) | `kernel32.lib userenv.lib ws2_32.lib bcrypt.lib ntdll.lib advapi32.lib` |

```sh
cargo rustc --lib --release --features ffi --crate-type staticlib
cc -I src/ffi -o demo examples/ffi/demo.c target/release/libreticle.a -lpthread -ldl -lm
./demo
```

`rustc --print native-static-libs --crate-type staticlib` prints the exact
list for your toolchain if the table above is out of date.

## The shape of the ABI

Six rules hold for every function, which is most of what there is to
learn.

**Opaque handles.** `reticle_design`, `reticle_sim` and
`reticle_diagnostics` are pointers to types whose layout C never sees.
Each comes from a named constructor and goes back through its `_free`,
which ignores `NULL`. No struct crosses the boundary, so a field added on
the Rust side can never break a compiled caller.

Handles are independent. `reticle_sim_create` copies the design into the
simulator, so the two may be freed in either order, and a diagnostics
handle resolves its spans when it is built and then holds no borrow at
all.

**A status and an out-parameter.** Every fallible call returns an `int`
and writes its result through a pointer:

```c
reticle_design *design = NULL;
reticle_diagnostics *diags = NULL;
int status = reticle_elaborate_verilog_source("top.v", source, "top", &design, &diags);
if (status != RETICLE_OK) {
    fprintf(stderr, "%s\n", reticle_status_message(status));
}
```

A non-zero status leaves every out-parameter untouched, so checking the
status is enough; nothing is half-written.

| Code | Meaning |
|------|---------|
| `RETICLE_OK` | the call succeeded |
| `RETICLE_ERR_INVALID` | a `NULL` pointer, a string that is not UTF-8, or an index out of range |
| `RETICLE_ERR_DIAGNOSTICS` | the stage ran and reported errors; the diagnostics handle has them |
| `RETICLE_ERR_NOT_FOUND` | a name (a net path, a module, a format) does not exist |
| `RETICLE_ERR_UNSUPPORTED` | the stage was not compiled into this build |
| `RETICLE_ERR_INTERNAL` | a failure that is a bug rather than bad input |
| `RETICLE_ERR_UNKNOWN_VALUE` | the value has `x` or `z` bits and has no integer form |
| `RETICLE_ERR_PANIC` | a panic was caught at the boundary |

**Owned strings.** A `char **` out-parameter receives a NUL-terminated
UTF-8 string that you free with `reticle_string_free`. The two
`const char *` returns — `reticle_version` and `reticle_status_message` —
point at static storage and must not be freed.

**Sans-I/O.** The library never opens a file. Where the API says "a list
of files" it means a list of *(name, text)* pairs: you read the bytes, and
the name is what diagnostics print. Verilog `` `include `` is reported
rather than resolved, so an embedder that wants includes passes the
included text itself.

**No panics.** Nothing unwinds across the boundary. Every entry point runs
inside `catch_unwind` and reports an internal panic as
`RETICLE_ERR_PANIC`; `reticle_self_test_panic` raises one on purpose so
you can check that against your own build. (Rust's default panic hook
still prints the message to `stderr`; the status is what matters.)

**Threads.** A handle carries no locking. Different handles may be driven
from different threads; one handle must not be used from two at once.

## A tour

### Elaborating

```c
const char *names[]   = {"top.v", "adder.v"};
const char *sources[] = {top_text, adder_text};

reticle_design *design = NULL;
reticle_diagnostics *diags = NULL;
int status = reticle_elaborate_verilog(names, sources, 2, "top", &design, &diags);
```

All the sources form one compilation, so a module in one may instantiate a
module in another. `top` may be `NULL`, in which case elaboration picks the
module nothing instantiates. `reticle_elaborate_vhdl` is the same call for
VHDL-2008, compiling into `work` against the bundled `std` and `ieee`
libraries, and both have a `_source` shorthand for a single string.

`reticle_design_load_rtl` reads the `.rtl` IR text format instead, and is
always available whatever frontends the build has.

### Reading diagnostics

Two ways, and both come from the same handle. The rendered form is the
rustc-style text with source excerpts:

```c
char *text = NULL;
if (reticle_diagnostics_render(diags, &text) == RETICLE_OK) {
    fputs(text, stderr);
    reticle_string_free(text);
}
```

Item by item, for a tool with its own presentation:

```c
size_t count = 0;
reticle_diagnostics_count(diags, &count);
for (size_t i = 0; i < count; i++) {
    int severity; uint32_t line, column; char *file, *message;
    reticle_diagnostic_severity(diags, i, &severity);
    reticle_diagnostic_line(diags, i, &line);
    reticle_diagnostic_column(diags, i, &column);
    reticle_diagnostic_file(diags, i, &file);
    reticle_diagnostic_message(diags, i, &message);
    /* … and reticle_diagnostic_code / _note_count / _note … */
    reticle_string_free(file);
    reticle_string_free(message);
}
```

The list is sorted by source position, so two runs over the same input
produce the same order.

### Synthesising and emitting

```c
char *report = NULL;
reticle_design_synth(design, 4, &report, &diags);   /* 0 = generic, 2..8 = LUTs */

char *netlist = NULL;
reticle_design_emit(design, "verilog", &netlist, &diags);
```

`reticle_design_emit` takes `verilog`, `vhdl`, `json` (Yosys), `blif` or
`edif`. The netlist formats describe cells, so they generally want a
design that has been through synthesis; BLIF in particular wants the logic
mapped to LUTs.

### Simulating

Nets are addressed by index. The simulator's net list is snapshotted when
the handle is created, so a path is resolved once and every later access is
a bounds-checked lookup — there is nothing per net for C to free and
nothing to dangle.

```c
reticle_sim *sim = NULL;
reticle_sim_create(design, NULL, &sim, &diags);

size_t clk, q;
reticle_sim_find_net(sim, "counter.clk", &clk);
reticle_sim_find_net(sim, "counter.q", &q);

uint64_t per_ns;
reticle_sim_ticks_per_ns(sim, &per_ns);

for (int i = 0; i < 10; i++) {
    reticle_sim_set(sim, clk, "1'b0");
    reticle_sim_run_for(sim, per_ns);
    reticle_sim_set(sim, clk, "1'b1");
    reticle_sim_run_for(sim, per_ns);
}

uint64_t value;
reticle_sim_get_u64(sim, q, &value);
```

`reticle_sim_get` gives the value as a Verilog literal instead, which is
the general accessor: it represents `x` and `z` bits, which an integer
cannot. `reticle_sim_take_output` drains what `$display` wrote,
`reticle_sim_take_messages` returns the simulator's own diagnostics, and
`reticle_sim_enable_vcd` plus `reticle_sim_vcd` produce a waveform.

## Tests

- `src/ffi/tests.rs` calls every entry point from Rust the way C would,
  and each test asserts through an allocation counter that it handed back
  every handle and string it took.
- `tests/ffi_header.rs` compares the symbols the header declares with the
  ones the Rust sources export, and checks that every `RETICLE_*` constant
  has the same value on both sides. Adding a function to one and not the
  other fails, naming the symbol.
- `tests/ffi_c_example.rs` compiles and runs `examples/ffi/demo.c` against
  a real static library, which is the only thing that proves the header
  describes the symbols a C compiler actually finds. It needs a C compiler
  and a built archive, and prints why it is skipping when either is
  missing. `tools/check.sh` builds the archive first so it runs.

## `unsafe`

The crate lints `unsafe_code` at `deny`, not `forbid`. `forbid` cannot be
lifted even locally, and a C ABI cannot be expressed without raw pointers,
so `src/ffi` and `src/wasm` are the two modules that opt back in with a
file-scoped `#![allow(unsafe_code)]`. Every `unsafe` block in them carries
a comment naming the invariant it relies on, and the safety contract a
caller must meet is written on each function. Nothing else in the crate
may opt in.
