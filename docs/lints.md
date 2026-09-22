# Verilog lint rules

The Verilog linter (`reticle::verilog::lint`) checks a parsed file without
elaborating it. One traversal collects the facts a rule can need — every
declaration, every read and write with the process it happens in, every
`always` block with its classification, every instance connection — and
each rule then queries that table, so a run costs one pass over the tree
plus a linear scan per rule.

Because nothing is elaborated, the rules only report what the source text
settles by itself. A width that depends on a parameter override, a name
that a package import might supply, a driver in a generate branch that may
not be taken: all of these keep the linter quiet. False negatives are the
deliberate trade; a lint that cries wolf gets switched off.

This document lists every rule with an example. The API reference
(`cargo doc`) documents the types.

## Running the linter

```rust
use reticle::diag::Diagnostics;
use reticle::source::SourceMap;
use reticle::verilog::lint::{LintConfig, run};
use reticle::verilog::{Dialect, NoIncludes, lex_source_full, parse_source};

let mut map = SourceMap::new();
let id = map.add("top.v", std::fs::read_to_string("top.v")?).unwrap();
let mut diags = Diagnostics::new();
let lexed = lex_source_full(&mut map, id, Dialect::Verilog2005, &mut NoIncludes, &mut diags);
let file = parse_source(&mut map, id, Dialect::Verilog2005, &mut NoIncludes, &mut diags);

let config = LintConfig::parse("error:multiple-drivers off:port-connection")?;
run(&config, &map, id, &file, &lexed.comments, Dialect::Verilog2005, &mut diags);
print!("{}", diags.render(&map));
```

## Configuration

`LintConfig::parse` reads a small text form. Items are separated by
whitespace, commas or newlines, and `#` starts a comment:

| Item | Meaning |
|------|---------|
| `off:<lint>` (`allow:`) | the rule does not run |
| `note:<lint>` (`info:`) | report as a note |
| `warn:<lint>` (`warning:`) | report as a warning |
| `error:<lint>` (`deny:`) | report as an error |
| `off:all`, `warn:all`, … | applies to every rule |
| `max-line-length=<n>` | the column `line-length` reports at (default 100) |
| `strict-dialect=<d>` | the dialect `keyword-as-identifier` checks against (`verilog2001`, `verilog2005`, `systemverilog`; default `systemverilog`) |

An unknown rule name or level is an error, so a typo in a project's
settings is reported rather than quietly ignored.

`LintSet` is the registry behind this: `LintSet::all()` builds every rule
at its default level, and `enable`, `disable` and `set_level` change one by
name. A caller with its own rules can start from `LintSet::empty()` and
`register` them, then run the set with `lint::run_with`.

## Turning a rule off in the source

Three forms, all equivalent in effect:

```verilog
(* lint_off = "unused-signal" *) wire scratch;   // this declaration only
(* lint_off *) wire other;                       // every rule, this declaration

// reticle-lint: off unused-signal
wire a, b, c;
// reticle-lint: on unused-signal

wire d;  // reticle-lint: off-line unused-signal
// reticle-lint: off-next-line unused-signal
wire e;
```

An attribute covers the item, port or statement it decorates. A bare `off`
with no names silences every rule; an `off` without a matching `on` runs to
the end of the file. The note at the bottom of every diagnostic repeats the
two shortest forms.

A signal whose name starts with `_`, or which carries `keep`, `dont_touch`,
`unused`, `maybe_unused`, `keep_hierarchy` or `mark_debug`, is exempt from
the unused and undriven rules without any lint directive.

## The rules

| Code | Name | Default |
|------|------|---------|
| L0001 | [`unused-signal`](#unused-signal) | warn |
| L0002 | [`unused-input`](#unused-input) | warn |
| L0003 | [`undriven-signal`](#undriven-signal) | warn |
| L0004 | [`undriven-output`](#undriven-output) | warn |
| L0005 | [`multiple-drivers`](#multiple-drivers) | warn |
| L0006 | [`implicit-net`](#implicit-net) | warn (error under `` `default_nettype none ``) |
| L0007 | [`blocking-in-sequential`](#blocking-in-sequential) | warn |
| L0008 | [`nonblocking-in-comb`](#nonblocking-in-comb) | warn |
| L0009 | [`mixed-assignment-styles`](#mixed-assignment-styles) | warn |
| L0010 | [`sensitivity-list`](#sensitivity-list) | warn |
| L0011 | [`reset-style`](#reset-style) | warn |
| L0012 | [`incomplete-case`](#incomplete-case) | warn |
| L0013 | [`latch-inferred`](#latch-inferred) | warn |
| L0014 | [`case-x-z`](#case-x-z) | warn |
| L0015 | [`unreachable-statement`](#unreachable-statement) | warn |
| L0016 | [`constant-condition`](#constant-condition) | warn |
| L0017 | [`width-mismatch`](#width-mismatch) | warn |
| L0018 | [`unsized-literal-in-concat`](#unsized-literal-in-concat) | warn |
| L0019 | [`port-connection`](#port-connection) | note |
| L0020 | [`non-ansi-ports`](#non-ansi-ports) | note |
| L0021 | [`missing-default-nettype`](#missing-default-nettype) | off |
| L0022 | [`keyword-as-identifier`](#keyword-as-identifier) | warn |
| L0023 | [`deprecated-construct`](#deprecated-construct) | note |
| L0024 | [`todo-comment`](#todo-comment) | off |
| L0025 | [`naming`](#naming) | off |
| L0026 | [`line-length`](#line-length) | off |
| L0027 | [`trailing-whitespace`](#trailing-whitespace) | off |
| L0028 | [`tabs`](#tabs) | off |

### unused-signal

A net or variable declared at module level that nothing ever reads. Ports
are covered by `unused-input` instead. Modules that use `.*` on an
instance, and `interface` bodies, are skipped: a name may be used from
outside without the linter seeing it.

```verilog
wire [7:0] scratch;   // warning: `scratch` is never read
reg  [3:0] counter;   // warning: written, but still never read
wire       _spare;    // fine: the leading underscore says so
```

### unused-input

An `input` port the module never reads: either dead weight in the port
list or a signal someone forgot to use.

```verilog
module m(input wire clk, input wire enable);  // warning: `enable` is never read
```

### undriven-signal

A net or variable nothing ever assigns, so it reads as `x` (or `z`) for
the whole simulation. `supply0`, `supply1`, `tri0`, `tri1` and `trireg`
drive themselves and are exempt, as are inputs and `inout` ports.

```verilog
wire mid;              // warning: `mid` is never assigned
assign y = a & mid;
```

### undriven-output

An `output` port of the module that no process, assignment or instance
drives.

```verilog
module m(input wire a, output wire y, output wire z);  // warning: `z` is never driven
    assign y = a;
endmodule
```

### multiple-drivers

Two or more processes drive the same bits of one net or variable: two
`assign`s, an `assign` and an `always`, two instance outputs, and so on.
Several writes inside one process are one driver, since only one of them
wins. Exempt are the resolved net types (`tri`, `wand`, `wor`, `triand`,
`trior`, `trireg`), drivers of constant slices that do not overlap, and
drivers in different arms of one generate `if` or `case`.

```verilog
assign y = a;
assign y = b;              // warning: `y` has more than one driver

assign bus[1:0] = {2{a}};  // fine: the slices are constant and disjoint
assign bus[3:2] = {2{b}};
```

### implicit-net

An identifier used in a port connection, a gate terminal or on the left of
a continuous assignment without being declared. Verilog invents a one-bit
wire, which silently truncates a wide signal and hides typos. Reported as
an error when `` `default_nettype none `` is in effect, because then there
is no net at all. Modules with a wildcard package import are skipped.

```verilog
leaf u0 (.i(a), .o(mid));  // warning: `mid` is used without being declared
assign stray = a;          // warning: so is `stray`
```

### blocking-in-sequential

A blocking assignment (`=` or a compound operator) to a module-level
signal inside an edge-triggered block: `always_ff`, or `always` with an
edge in its sensitivity list. The register it writes races with every read
of it elsewhere in the same time step. Variables declared inside the block
are exempt, since they are temporaries.

```verilog
always_ff @(posedge clk) q = a;   // warning: use `<=`
```

### nonblocking-in-comb

The mirror image: `<=` inside `always_comb`, `always @*`, an `always` with
a level-sensitive list, or `always_latch`.

```verilog
always_comb y <= a & b;   // warning: use `=`
```

### mixed-assignment-styles

One variable written with both `=` and `<=` from procedural blocks, which
no synthesiser accepts. An `initial` block that seeds a register is not
counted.

```verilog
always_ff @(posedge clk) z <= a;
always_comb              z  = b;   // warning: `z` is assigned both ways
```

### sensitivity-list

Three problems with one `always` block's `@(...)`:

- a level-sensitive list that leaves out a signal the body reads, which
  makes the block behave differently in simulation and in hardware;
- a list that mixes edges and levels;
- `always @*` whose body reads nothing, so the block never runs again.

Signals the block writes before reading are temporaries and need not be
listed.

```verilog
always @(a) y = a & b;              // warning: leaves out `b`, use `always @*`
always @(posedge clk or b) z <= a;  // warning: mixes edge and level
always @* w = 1'b0;                 // warning: reads nothing
```

### reset-style

For an edge-triggered block whose sensitivity list has an asynchronous
reset (any edge after the first, which is taken to be the clock):

- the polarity of the `if` must match the edge (`negedge rst_n` needs
  `if (!rst_n)`);
- the reset must be the first condition tested;
- the reset must be tested at all;
- a second, synchronous reset in the same block (a condition on a signal
  whose name contains `rst` or `reset`) is reported.

```verilog
always @(posedge clk or negedge rst_n)
    if (rst_n) q <= 1'b0;   // warning: wrong polarity
    else       q <= d;
```

### incomplete-case

A `case`, `casez` or `casex` without a `default` whose arms do not cover
every value its subject can take. The subject's width must be known and at
most eight bits, so the values can be counted; `casez` and `casex` arms
count the values their wildcards match. A `unique`, `unique0` or
`priority` qualifier asserts full coverage and switches the rule off for
that statement.

```verilog
case (sel)              // warning: covers 2 of 4 values and has no `default`
    2'd0: y = 4'b0001;
    2'd1: y = 4'b0010;
endcase
```

### latch-inferred

A combinational block (`always_comb`, `always @*`, `always` with a
level-sensitive list) that does not assign a variable on every path
through it, so the variable keeps its old value and synthesis infers a
latch. The analysis walks the block: an `if` without `else` assigns
nothing for certain, a `case` assigns only what every arm assigns and only
when it has a `default` or covers its subject, and a sequence assigns the
union of its statements. It is deliberately optimistic about bit selects
(`y[0] = ...` counts as assigning `y`) and loops (taken to run at least
once), so a latch it reports is nearly always real.

```verilog
always_comb if (en) hit = 1'b1;   // warning: `hit` keeps its value otherwise

always_comb begin                 // fine: a default value first
    guarded = 4'b0000;
    if (en) guarded = 4'b1111;
end
```

### case-x-z

`casex` treats an `x` in the *subject* as a wildcard, so an undriven
signal matches any arm; `casez` is what almost every use wants. The rule
also reports an `x` or `z` in the arms of a plain `case`, and an `x` in
the arms of a `casez`, since neither can ever match.

```verilog
casex (op) ...           // warning: use `casez`
case (op) 4'b1x0z: ...   // warning: this arm matches nothing
casez (op) 4'b1?x?: ...  // warning: `casez` wildcards are `?` and `z`
```

### unreachable-statement

A statement in a block after one that always leaves it: `return`, `break`,
`continue`, `$finish` or `$fatal`.

```verilog
return x;
clamp = 0;   // warning: unreachable
```

### constant-condition

An `if` whose condition is a literal, a `while` or `do while` that can
never run, or `repeat (0)`. `while (1)` is the idiomatic forever loop and
is not reported.

```verilog
if (1) y = 1'b0;      // warning: always true
while (0) y = 1'b1;   // warning: the body never runs
```

### width-mismatch

Two forms, both syntactic:

- a declaration whose initialiser needs more bits than the declared width;
- an assignment (continuous or procedural) where both sides have an
  exactly known width and they differ. A width is exactly known for a
  sized literal, a declared signal, a bit or part select with constant
  bounds, a concatenation or replication of such, and a cast. Arithmetic
  and bitwise operators yield a lower bound rather than an exact width,
  because the assignment context may extend them, and nothing is reported
  for those.

```verilog
reg [7:0] seed = 9'h1ff;   // warning: the literal needs nine bits
assign wide = {b, c};      // warning: six bits into eight
assign y = a + b;          // fine: the context decides the width
```

### unsized-literal-in-concat

A literal without a width inside `{}`. Concatenation is one of the few
places where Verilog does not size a literal from its context, so it
becomes 32 bits and the result is rarely what was meant.

```verilog
assign narrow = {b, 1};   // warning: write `1'b1`
```

### port-connection

Instance connection style, reported as notes:

- more than three ports connected by position, where one edit to the port
  list silently rewires the design;
- `.*`, which hides what is connected (and stops several other rules from
  analysing the module);
- a named port left open with `.x()`.

```verilog
leaf u0 (w, w, w, w, y);                     // note: five ports by position
leaf u1 (.a(w), .b(w), .c(w), .d(w), .y());  // note: `y` is left unconnected
leaf u2 (.*);                                // note: `.*` used
```

### non-ansi-ports

A Verilog-1995 port list, with the directions declared in the body. The
ANSI form keeps a port's direction, type and width in one place.

```verilog
module adder(a, b, sum);   // note: uses a Verilog-1995 port list
    input [3:0] a, b;
    output [4:0] sum;
```

### missing-default-nettype

The file never writes `` `default_nettype none ``, so a misspelled signal
becomes a one-bit wire instead of an error. Off by default, because the
directive is file-scoped and a project usually decides once.

### keyword-as-identifier

A declared name that a later standard reserves — `bit`, `logic`, `do`,
`final`, `this` and the rest of the SystemVerilog list in a file parsed as
Verilog-2005. The code is valid today and fails to compile the moment the
file is renamed to `.sv`. The dialect to check against is
`strict-dialect` in the configuration.

```verilog
wire [3:0] bit;   // warning: `bit` is a reserved word in SystemVerilog
```

### deprecated-construct

Reported as notes:

- `defparam`, which IEEE 1364-2005 deprecated in favour of `#(...)`
  overrides at the instance;
- `` `include `` of a `.v` or `.sv` file: headers belong in `.vh` / `.svh`
  and design files belong in the build description;
- `wait` in a module that otherwise looks synthesisable (it has `always`
  blocks and no `initial`).

```verilog
defparam u0.W = 4;        // note: override at the instance instead
`include "other.v"        // note: include headers only
wait (mid) y <= mid;      // note: not synthesisable
```

### todo-comment

A comment containing `TODO`, `FIXME`, `XXX` or `HACK`. Off by default;
useful in a release check.

### naming

Conventions the linter can check without regular expressions (the crate
has no dependencies, and a regular expression engine is not worth writing
for this):

- modules, interfaces, programs and signals in `snake_case`;
- `parameter` and `localparam` in `UPPER_CASE`;
- the clock of an edge-triggered block named `clk...` (any name containing
  `clk` or `clock`);
- a signal used on `negedge` suffixed `_n`, `_b` or `_neg`, since it is
  active low.

Off by default: a project either adopts the whole convention or none of
it.

```verilog
module BadName #(parameter width = 4) (input wire CK, input wire rst, ...);
//     ^ not snake_case    ^ not UPPER_CASE       ^ not a clock name
    always @(posedge CK or negedge rst)   // `rst` is active low: `rst_n`
```

### line-length

A line longer than `max-line-length` characters (100 by default). Off
until the formatter of phase 9 lands.

### trailing-whitespace

Spaces or tabs before the line break. Off by default.

### tabs

A tab character anywhere in a line. Off by default.

## Adding a rule

1. Implement `Lint` in the file of its rule group under
   `src/verilog/lint/` (`signals.rs`, `procedural.rs`, `control.rs`,
   `widths.rs`, `style.rs`, `text.rs`), with the next free `L####` code
   and a kebab-case name.
2. Register it at the end of `LintSet::all()` in `mod.rs`.
3. Add a case to `testdata/verilog/lint/`, with a `// lint-config:` header
   selecting the rule, and run `UPDATE_EXPECT=1 cargo test --test
   verilog_lint` to write the expectation. `every_rule_is_covered` fails
   until a case reports the new rule.
4. Add a section here, with an example.

Rules query the facts in `lint::facts` rather than walking the tree
themselves wherever possible, so the cost of a run stays linear. Report
through `LintContext::report`, which applies the configured level and the
source suppressions; never push a diagnostic directly.
