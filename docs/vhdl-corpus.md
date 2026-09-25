# An independent VHDL corpus: CERN's Colibri

Every piece of VHDL this compiler had ever been fed was written inside this
project, which is the weakest possible test of a front end. **Colibri** is
CERN's common VHDL library — production gateware, deliberately vendor
independent, with self-checking testbenches and formal properties of its
own. Pointing Reticle at it is the same exercise that found eight defects on
the Verilog side ([`examples/soc/README.md`](../examples/soc/README.md)).

This page says exactly what was run, what the numbers are, and what is
still wrong. **Most of the library does not work yet**: all of it parses,
three quarters of its files analyse, and a fifth of its entities elaborate.
Two of its designs have been simulated.

## Getting it

The corpus is never vendored and CI never has it. One environment variable
names a checkout:

```sh
git clone https://gitlab.com/colibri-cern/colibri.git /path/to/colibri
export RETICLE_COLIBRI=/path/to/colibri
cargo test --all-features --test vhdl_corpus -- --nocapture
```

Without `RETICLE_COLIBRI` every test in
[`tests/vhdl_corpus.rs`](../tests/vhdl_corpus.rs) prints one line saying
what is missing and passes, so `tools/check.sh` is green without the corpus.
`CORPUS_DUMP=1` additionally prints every diagnostic, which is where a fix
starts.

It is a git checkout rather than a `reticle fetch` database ([see
`docs/fpga-xray.md`](fpga-xray.md)) because `fetch` exists to pin chip
databases to digests compiled into the binary; a source corpus is read, not
shipped, and pinning it would only stop it moving on. The measurements below
are from commit `3fa784121ccea86d9e65b2e0dc08d2a3327f5f2f` of `master`
(2026-07-17). The two submodules (`UVVM`, `vhdl_style_guide`) are not needed
and are not cloned above.

**Licence.** Colibri is REUSE-compliant and multi-licensed per file, by
SPDX identifier: 96 of its 103 design sources and all 134 testbench and
formal sources carry `SPDX-License-Identifier: CERN-OHL-W-2.0`, the images
and charts are `CC-BY-SA-4.0` (`docs/REUSE.toml`), and the readme and CI
metadata are `Apache-2.0`. The seven sources without a header are the
generated register blocks under `src/misc/mmap_fifo/vhdl_if/`, covered by
the repository's default of CERN-OHL-W-2.0. Reading it as test input
implicates none of that; nothing from it is copied into this repository.

## What is in it

Measured, not quoted. The Hackaday write-up says "over 100 components,
functions and procedures"; the count of *design units* is smaller, because
a good part of the library is functions inside packages rather than
entities:

| | Count |
|---|---|
| `.vhdl` files (the extension is `.vhdl`, not `.vhd`) | 231 |
| design sources (`src/`) | 103 |
| testbenches (`sim/`, 87 of them named `*_tb.vhdl`) | 126 |
| formal benches in VHDL (`fv/`) | 2 |
| PSL property files (`fv/**.psl`) | 31 |
| entities | 90 |
| architectures | 90 |
| package declarations (plus 6 bodies) | 12 |

The entity count is Reticle's own, from the analysed library; the file
counts are `find`'s.

## What was run, and what happened

Four measurements, each a test in `tests/vhdl_corpus.rs`, each printing its
numbers and asserting a floor so that a change which makes less of the
corpus work fails.

| Stage | Result | Test |
|---|---|---|
| Lex and parse, every file including the testbenches | **231 of 231** | `every_source_file_parses` |
| Analyse the library (`src/`, one `colibri` library) | **77 of 103 files with no error**; 672 errors in 26 | `the_library_analyses` |
| Elaborate each entity to the IR, alone, from its own defaults | **20 of 90**; 31 need generic values first; 39 fail | `the_entities_elaborate` |
| Simulate | the 8b/10b encoder and decoder, 256 bytes round-tripped | `the_8b10b_codec_round_trips_in_the_simulator` |
| Run Colibri's own testbenches | **0 of 126** | `every_testbench_needs_a_framework_that_is_not_bundled` |
| Check its PSL properties | **0 of 31** | — (see [PSL](#psl-and-the-formal-benches)) |

Of the 672 analysis errors, **601 are in seven generated files** (the
PeakRDL register blocks under `src/misc/mmap_fifo/vhdl_if/`), which need
`ieee.fixed_pkg` and the standard context `ieee.ieee_std_context`, neither
bundled. The remaining 71 errors are spread over 19 hand-written files.

### Simulation is the strong result, and it is small

Colibri's 126 testbenches are self-checking, which is what makes them worth
running — and **not one of them can run here**. Every single file under
`sim/` names VUnit (`vunit_lib`, 109 files), UVVM (`uvvm_util` /
`uvvm_vvc_framework` / `bitvis_vip_*`, 63) or OSVVM (59); the only file that
names none of them is a helper package, `sim/memory/data_struct_pkg.vhdl`.
Those three verification frameworks are large VHDL libraries in their own
right, built on protected types, shared variables, file I/O and generic
packages, and Reticle bundles none of them. So the strongest claim the
corpus's own benches could give is not available yet, and saying otherwise
would be dishonest.

What *is* available is a design of theirs, checked by a property of its own
kind, with stimulus written in Rust. `encode_8b10b` and `decode_8b10b` are
a matched pair of table-driven codecs: the encoder holds a 1024 x 11-bit ROM
as a package constant (`c_ENCODE_ROM_8B10B`), selects a code word by data
byte, control flag and running disparity, and flips the disparity when the
table says to. The test encodes all 256 bytes back to back and checks:

- every code word carries four, five or six one bits — the bounded
  disparity 8b/10b exists for, which only a correct ROM lookup *and* a
  correct running-disparity flip can satisfy;
- feeding those words through the decoder returns the 256 bytes that
  produced them, in order, with the K-code flag low throughout.

Both designs elaborate, the 1024-entry constant becomes a memory, and the
run is clean of simulator messages. That is two entities of ninety.

## Known gaps: the defects this found in Reticle

Eighteen, all fixed, each with a regression test that needs no corpus. The
tests are in `testdata/vhdl/` (golden files) and next to the code (unit
tests); the pattern is `examples/soc/README.md`'s.

| Test | The defect, now fixed |
|---|---|
| `testdata/vhdl/parse/range_and_subtype_attributes.vhd` | `data_i'subtype` was a syntax error: `subtype` is a reserved word and was not accepted as an attribute designator, though LRM 16.2.4 spells the attribute with it. This alone broke 38 of the 103 design files |
| `testdata/vhdl/parse/range_and_subtype_attributes.vhd` | `(v'range => '0')` was rejected: a range attribute as an aggregate choice was parsed as an expression, which `'range` can never be |
| `testdata/vhdl/parse/range_and_subtype_attributes.vhd` | `v(other'range)` was rejected for the same reason, instead of being the slice it is |
| `src/vhdl/sema/library.rs::work_orders_units_of_a_library_of_another_name` | `work` was only followed when the working library was literally named `work`, so in a library named `colibri` every `entity work.sub` was analysed before `sub` existed and its port map was checked against an entity with no ports: 147 spurious errors |
| `testdata/vhdl/elab/static_function.vhd` | `maximum` and `minimum` of a scalar type (LRM 5.2.6) were missing from the bundled `std.standard`, so `maximum(WIDTH, 4)` found only `numeric_std`'s vector profiles |
| `testdata/vhdl/elab/static_function.vhd` | a width computed by the design's own function (`log2ceil(g_MODULO)`, the canonical portable idiom) could not be elaborated at all: calls were inlined into the IR and never evaluated. Elaboration now interprets a pure function body — variables, `if`, `case`, `for`, `while`, `exit`, `next`, `return`, and assignment to a whole object, an element, a slice or a field ([`src/vhdl/elab/interp.rs`](../src/vhdl/elab/interp.rs)) |
| `testdata/vhdl/elab/static_function.vhd` | the analyser's cached value for an expression over a variable with a static initialiser (`step * 2` where `step : positive := 1`) was reused after the variable had been assigned, so an interpreted loop never terminated |
| `testdata/vhdl/elab/subtype_and_range_attributes.vhd` | `'left`, `'right`, `'high`, `'low`, `'length` and `'ascending` of an object whose subtype came from a generic were not evaluated after elaboration, though that is exactly where they are written |
| `testdata/vhdl/elab/default_generic_not_static.vhd` | an aggregate (`(others => '0')`) as a generic's default or a signal's initial value was not evaluated after elaboration |
| `testdata/vhdl/elab/default_generic_not_static.vhd` | a generic or port whose default was not *locally* static was treated as having no default at all, and leaving it out of a map (which LRM 6.5.2 allows) was reported as a missing association |
| `testdata/vhdl/elab/default_generic_not_static.rtl` | a `std_logic_vector` generic's value reached the IR as a string of control characters, because the element positions of `std_ulogic` were read as code points |
| `testdata/vhdl/elab/clocked_conditional_assign.vhd` | `q <= d when rising_edge(clk);` — a register as one concurrent statement, and the whole of Colibri's two-process style — was refused with "`rising_edge` is only recognised as the clock condition of a process" |
| `testdata/vhdl/elab/error_conditional_no_else.vhd` | a conditional assignment with no `else` was lowered to a multiplexer against **zero**. The signal keeps its previous value, so that was a different design, built in silence; it is now a diagnostic that names the latch and suggests the clock-edge form |
| `testdata/vhdl/elab/slice_target_generic_bounds.vhd` | a slice *assigned to* whose bounds came from a generic was dropped in silence, leaving the port undriven with nothing said |
| `src/ir/expr.rs::a_reversed_slice_is_an_error_and_not_a_panic` | the IR's type rule for a slice computed its length before checking the bounds, so `hi < lo` panicked with an integer underflow instead of being reported |
| `testdata/vhdl/sema/file_parameter.vhd` | a `file` parameter has no mode (LRM 4.2.2.1); it was given `in` by default, so passing one on to another subprogram — how every routine in `src/fileio/binaryio.vhdl` is written — was "cannot assign to `in` parameter" |
| `testdata/vhdl/sema/file_parameter.vhd` | an `out` parameter passed straight on as the actual for another `out` formal was counted as a *read* of it |
| `testdata/vhdl/sema/file_parameter.vhd` | reading an `out` parameter was refused outright, though VHDL-2008 allows it (LRM 4.2.2.3); only `out` *ports* had been given the 2008 rule |
| `testdata/vhdl/sema/record_element_names.vhd` | resolving `r.f` declared `f` as an ordinary name in the enclosing region, so a signal or parameter of the same name as a record element resolved to the element, or was reported as a redeclaration |

One more change is not a defect but a gap in the library interface:
`ElabOptions::only_top` elaborates the selected entity alone. Without it,
elaborating one entity of a *library* also elaborated every other entity
nothing instantiates, and reported every one of their missing generics
alongside the design asked for.

## VHDL we do not support yet

These are not defects — the diagnostics are clear and point at the right
line — but they are why 26 files do not analyse and 39 entities do not
elaborate. In rough order of how much of the corpus each one blocks:

| What | Where it appears | Count |
|---|---|---|
| `ieee.fixed_pkg` and the standard context `ieee.ieee_std_context` (LRM 16.10) | the seven generated register blocks under `src/misc/mmap_fifo/vhdl_if/` | 601 errors, 7 files |
| A shared variable, and a protected type | `fifo`, `avst_fifo`, `packet_delay`, the Altera/Xilinx dual-port RAMs | 5 entities |
| A port of an array of multi-bit elements (`slv_array_t`), which becomes a memory and cannot cross a module boundary in the IR | all of `src/proto/aurora_64b66b/` | 5 entities |
| An aggregate whose type comes from a port map's formal (`snk_empty_i => (others => '0')`) or from the other operand of a comparison | `rle_decode`, `be_add_lead`, `synchro_pulse`, `counter`, … | 11 entities |
| A record with unconstrained array elements, constrained per object (`avst_master_t(data(N-1 downto 0), …)`, LRM 5.3.3) | `colibri.types`, and everything that uses its stream records | 8 entities |
| A nested assignment target (`v.buf(i)(j)`, a record inside a record) | `i2c_controller`, `avst_ram_write_unaligned`, `simple_dpram_altera` | 3 entities |
| A slice whose discrete range is a *subtype name* (`v_vec(s_crc_range)`, LRM 8.5) | `crc`, `stream_to_wbm`, `aurora_st_decoder` | 13 errors, and most of the 22 type mismatches and 9 operator errors that cascade from them |
| An index on the *formal* of a port map (`data_i(0) => y`), or a conversion on one (`unsigned(q_o) => x`) | 9 files, from `synchro_handshake` to `packet_cc_ram_fifo` | 17 errors |
| `a & b` where both operands are elements (`std_logic & std_logic`), nested inside another concatenation: the array type has to come from the context and is not pushed down through the outer `&` | `encode_8b10b`, `avst_fifo`, `crc`, `packet_fifo`, … | 12 errors |
| An aggregate with named choices as an operand of `&`, whose bounds are its own choices' (LRM 9.3.3.3) rather than the other operand's subtype | `aurora_const_pkg`, `aurora_st_encoder` | 4 errors |
| `hread` / `hwrite` of a `std_logic_vector`: `ieee.std_logic_textio` is bundled with the `bit_vector` profiles only | `mem_pkg` | 2 errors |
| A case choice that is a static concatenation (`4x"0" & c_CMD_ADDR`), which LRM 9.4.2 makes locally static | `stream_to_wbm` | 3 errors |
| A string literal of a logic array type in an expression (`"0" & x`) | `uart_tx` | 2 entities |

Everything in that table is reported with a span and a reason; nothing in it
is mis-parsed or silently mis-elaborated, which is the distinction that
matters. Where a diagnostic *was* confusing — 147 errors about missing
formals, or an assignment vanishing — it is listed as a defect above and
fixed.

## What is the corpus's own problem

One thing, with evidence. `src/misc/mmap_fifo/mmap_fifo_tx.vhdl` declares

```vhdl
type fsm_t is (S_IDLE, S_SOP, S_PACKET, S_FLUSH, S_ERROR);
```

and its `case v_int.state is` at line 350 has alternatives for `S_IDLE`,
`S_SOP`, `S_PACKET` and `S_FLUSH`, no `when others`, and no other mention of
`S_ERROR` anywhere in the file. LRM 10.9 requires the choices of a case
statement to cover every value of the expression's subtype, so

```text
error[V0408]: this case does not cover every value of `fsm_t`
   --> src/misc/mmap_fifo/mmap_fifo_tx.vhdl:350:5
    |
350 |     case v_int.state is
    |     ^^^^^^^^^^^^^^^^^^^ missing: S_ERROR
```

is correct and the source is wrong. Nothing here works around it.

The seven generated register blocks are not a defect in the corpus either;
they are simply VHDL that needs two standard libraries Reticle has not
bundled.

## PSL and the formal benches

`fv/` holds 31 `.psl` files with SymbiYosys scripts beside them, and
Reticle supports **none of it**. The question is what a front end would
take, given that `src/sim/assertion/` already implements an SVA and PSL
subset.

The *property* machinery transfers almost entirely. `src/sim/assertion`
already parses and checks `always`, `never`, `|->`, `|=>`, `not`, `and`,
`or`, the SERE braces `{a; b}` and `{a : b}`, delays and repetitions, a
clocking event, `disable iff`, and the `assert` / `assume` / `cover`
directives, compiling sequences into NFAs over boolean predicates
(`automaton.rs`, 859 lines). Colibri's properties are written in exactly
that vocabulary — `assert always {a;b} |-> c`, `assume always x |=> y`,
plus `abort`, which is `disable iff` in another spelling. The 1653 lines of
`property.rs` and the automaton would be reused as they are.

What is missing is everything around the property:

- **The container.** `vunit name (entity(arch)) { … }` binds a verification
  unit to a design unit, and `default clock is rising_edge(clk_i);` sets its
  clock. Small, but it needs the VHDL library model to find the entity and
  architecture it names.
- **VHDL expressions.** The predicates are VHDL — `data_i = g_RESET_VAL`,
  `snk_valid_i = '0'`, `reg_if.data`, references to the entity's generics
  and to internal signals of the architecture. Today's parser is the
  SystemVerilog flavour (`==`, `!=`, sized literals, `[7:4]`). A PSL front
  end for VHDL therefore wants the *VHDL* expression parser and the
  analyser's name resolution, pointed at the bound architecture's region —
  which is a good fit for `src/vhdl/sema`, not for `src/sim/assertion`.
- **Auxiliary VHDL inside the unit.** `fv/common/edge_detect.psl` declares a
  signal *and a process* inside the `vunit` to remember whether a previous
  value exists. A verification unit is an extension of the architecture, so
  elaborating one means elaborating its declarations and statements into the
  bound design.
- **`prev(x)`**, the PSL builtin these properties lean on, is `$past` in SVA
  terms, and is one of the things `src/sim/assertion` does not support
  (`$rose`, `$fell` and `$stable` are).
- **The engine.** These are *proofs*, driven by SymbiYosys: the `.psl`
  files carry `assume`s and no stimulus, so simulating them would check
  nothing. Reticle's BMC and k-induction engines
  ([`src/formal`](../src/formal)) take properties as 1-bit nets carrying
  `formal_assert` / `formal_assume` / `formal_cover`, so a PSL front end for
  formal use has to emit each property as a *monitor circuit* in the IR —
  the automaton as a state machine — rather than as a runtime checker. That
  is well-understood work and it shares the sequence compiler with the
  simulator, but it is a second back end for the same AST, not a reuse of
  the existing one.

**Assessment:** the two notations share enough that a PSL front end should
be built on `src/sim/assertion`'s property AST and automaton rather than
beside them, with a VHDL-flavoured expression layer and a `vunit` binder in
front, and a monitor-circuit emitter behind it for the formal engines. The
work is in the binding and the second back end, not in the temporal
operators. It is not started, and nothing in this pass touches `src/formal`.

## Reproducing the numbers

```sh
export RETICLE_COLIBRI=/path/to/colibri
cargo test --all-features --test vhdl_corpus -- --nocapture   # the numbers
CORPUS_DUMP=1 cargo test --all-features --test vhdl_corpus \
  the_library_analyses -- --nocapture                         # every diagnostic
```

The gate itself (`tools/check.sh`) does not need the corpus and does not
read `RETICLE_COLIBRI`.
