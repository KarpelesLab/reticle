# The Reticle IR

The IR (`reticle::ir`) is the data structure every stage of Reticle shares.
Both frontends lower into it; simulation, synthesis, formal verification and
the emitters read it. This document describes its shape, its invariants, and
the `.rtl` text format used for golden tests and debugging. The API
reference (`cargo doc`) has the per-item details.

## Design goals

- **Hierarchical.** A `Design` is a set of `Module`s; modules instantiate
  each other by id, or by name when the target is not part of the design (a
  vendor primitive, an encrypted core).
- **Every object has a span.** Nets, ports, expressions, statements, cells,
  instances, memories, processes, parameters and modules all carry a
  `source::Span`, so every later stage can report against source text.
- **Explicitly typed.** Nets and expression nodes carry a `Type`. Operators
  have fixed width rules; a frontend inserts `Resize` nodes where its
  language's context-determined widths demand. There is no implicit
  extension anywhere in the IR.
- **Attributes on everything.** Every object has an ordered `Attrs` map, so
  `(* keep *)`, `ram_style` and pass-to-pass hints travel with the object.
- **Two forms in one module.** The *process form* (structured statements
  with a trigger, plus continuous assigns) and the *cell form* (a netlist of
  generic primitives) coexist. A freshly lowered module has only processes;
  a synthesised one has only cells; passes may leave any mix.
- **Ids, not references.** Objects live in arenas and are addressed by
  `Copy` `u32` newtypes (`NetId`, `ExprId`, `CellId`, ...). Passes mutate
  freely; `ir::walk` has the helpers that keep ids consistent when objects
  are removed.
- **Round-tripping text.** `Design::to_text` and `Design::parse_text` are
  exact inverses on the text side: `parse(to_text(d)).to_text() ==
  to_text(d)`.

## Shape

```
Design
  modules: Arena<ModuleId, Module>
  top: Option<ModuleId>

Module
  name, span, attrs, blackbox: bool, timescale: Option<Timescale>
  ports:     Vec<Port>                  Port { name, dir: In|Out|InOut, net: NetId, span }
  params:    Vec<Param>                 resolved values, metadata only
  nets:      Arena<NetId, Net>          Net { name, ty: Type, kind: Wire|Reg|Variable, attrs, span }
  memories:  Arena<MemoryId, Memory>    Memory { name, elem: Type, size, init: Option<Vec<Const>>, attrs, span }
  exprs:     Arena<ExprId, Expr>        Expr { kind: ExprKind, ty: Type, span }
  instances: Arena<InstanceId, Instance>
  processes: Arena<ProcessId, Process>  process form
  cells:     Arena<CellId, Cell>        cell form
  assigns:   Vec<Assign>                continuous drivers: Assign { target: Lvalue, value: ExprId, delay, attrs, span }
```

### Types and constants

```
Type::Bits { width, signed }      the main path: a packed bit vector
Type::Array { elem, len }         an unpacked array (VHDL arrays, SV unpacked dimensions)
Type::Integer                     64-bit integer, simulation-only values
Type::Real                        64-bit float, simulation-only
Type::String                      simulation-only
```

Memories are not `Array`-typed nets: they are separate objects because
inference and simulation treat them as storage with ports.

`Const` is `logic::Logic`, the crate-wide 4-state bit vector, so constant
folding, the simulator and the frontends share one representation and one
operator set. The text format renders constants in a canonical sized form:
decimal for two-state values up to 64 bits (`8'd255`, `8'sd255`),
hexadecimal above that, binary when any bit is `x` or `z` (`4'b10xz`); it
accepts any Verilog-style literal on input. `Name` wraps a `String` and
will become an interned `Symbol`.

### Expressions

Expression nodes are immutable, arena-allocated and typed. `ExprKind`:

| Kind | Meaning |
|------|---------|
| `Const(c)`, `String(s)`, `Net(n)` | Leaves |
| `Slice { base, hi, lo }` | Constant part-select, inclusive |
| `Index { base, index }` | Variable bit- or element-select |
| `IndexedSlice { base, offset, width, up }` | `base[offset +: width]` / `-:` |
| `Concat(parts)` | First part is most significant |
| `Replicate { count, expr }` | `{count{expr}}` |
| `Unary { op, expr }` | `Not Neg ReduceAnd ReduceOr ReduceXor ReduceNand ReduceNor ReduceXnor LogicNot` |
| `Binary { op, lhs, rhs }` | `And Or Xor Xnor LogicAnd LogicOr Add Sub Mul Div Mod Pow Shl Shr Sshr Eq Ne CaseEq CaseNe WildEq Lt Le Gt Ge` |
| `Ternary { cond, then_, else_ }` | `cond ? then_ : else_` |
| `Resize { expr, width, signed }` | Truncate / zero-extend / sign-extend |
| `MemRead { mem, addr }` | Asynchronous memory read |
| `Call { name, args }` | An unlowered function call; the cached type is authoritative |

Type rules (enforced by `validate`, computed by `ir::expr::infer_type`):
bitwise and arithmetic operators need operands of equal width and yield
that width; comparisons, reductions and logical connectives yield `u1`;
shifts yield the left operand's type; `Ternary` needs a `u1` condition and
equal branch types. A result is signed only when every bit-vector operand is
signed. `Integer` and `Real` operands are accepted when both sides match.

### Processes and statements

```
Process { name: Option<Name>, kind: ProcessKind, body: Block, attrs, span }

ProcessKind::Comb                                   always_comb / always @*
ProcessKind::Sequential { clocks: Vec<Edge>, resets: Vec<Edge> }
ProcessKind::Initial
ProcessKind::Sensitive(Vec<NetId>)                  explicit sensitivity list
ProcessKind::Free                                   controls its own timing with waits

Edge { net, polarity: Pos | Neg | Any }
```

`Stmt { kind, span }` with `StmtKind`:

| Kind | Notes |
|------|-------|
| `Assign { target: Lvalue, value, kind: Blocking\|NonBlocking, delay }` | |
| `If { cond, then_, else_ }` | |
| `Case { subject, kind: Plain\|Z\|X, qualifier: None\|Unique\|Priority, arms, default }` | |
| `For { init, cond, step, body }`, `While`, `Repeat`, `Forever` | every part of `For` is optional |
| `Block { name, body }` | |
| `Wait(Delay(e) \| Event(edges) \| Until(e))` | |
| `SysCall { name, args }` | `$display` and friends, VHDL `report` |
| `MemFile { op, mem, file, start, end, base }` | `$readmemh` `$readmemb` `$writememh` `$writememb` (`op`); names the memory by id, as no expression denotes a whole memory; `start`/`end` are zero-based element addresses, `base` is the address `@hex` lines in the file give element 0 |
| `MemWrite { mem, addr, value, enable }` | |
| `Assert { cond, severity: Note\|Warning\|Error\|Failure, message }` | |
| `Finish`, `Stop`, `Break`, `Continue` | |

`Lvalue`: `Net`, `Slice { net, hi, lo }`, `Index { net, index }`,
`Concat(Vec<Lvalue>)`, `MemElem { mem, addr }`.

Time: `Delay { value: u64, unit: TimeUnit }` with `TimeUnit` from `Fs` to
`S`; a module may carry `Timescale { unit, precision }`.

### Cells

`Cell { name, kind: CellKind, inputs: Vec<(Name, ExprId)>, outputs:
Vec<(Name, NetId)>, params, attrs, span }`. Inputs are expressions so a
slice or constant can feed a cell directly; outputs are nets because the
cell is their driver. Widths follow from the connected types. The primitive
set (documented with its port rules in `ir::cell`):

```
Not And Or Xor Mux Pmux Add Sub Mul Div Mod Shl Shr Sshr
Eq Ne Lt Le Gt Ge ReduceAnd ReduceOr ReduceXor
Dff { clk_pos, has_enable, reset: Option<Reset { asynchronous, active_high, value }> }
Dlatch  MemRdPort { mem, clocked }  MemWrPort { mem, clocked }
Lut { k, init }  Buf  Tristate  Blackbox(name)
```

### Instances

`Instance { name, module: ModuleRef, connections: Vec<(Name, ExprId)>,
params, attrs, span }` where `ModuleRef` is `Resolved(ModuleId)` or
`Unresolved(Name)`. Output and inout ports must be connected to nets,
slices of nets or concatenations of nets. `Design::resolve_instances`
binds unresolved references whose name matches a module.

### Attributes

Every object carries an ordered `Attrs` map of `Name -> AttrValue`
(`Const`, `String` or `Int`). Both front ends put a source annotation on
the IR object the annotated declaration became, and invent nothing: a
Verilog attribute on a `reg`, `wire` or port declaration and a VHDL
`attribute ... of s : signal is ...` both land on the **net**; one on an
unpacked array lands on the **memory**; one on an `always` block or a VHDL
process lands on the **process**; one on a continuous `assign`, an
instance or a module lands on the assign, the instance or the module.
Nothing lands on a **cell** from source, because cells are inferred.

`Attrs::is_set(key)` is the test for a flag. A string value counts as set
unless it spells a falsehood — `"false"`, `"no"`, `"off"`, `"0"` or blank —
since vendors write these flags as strings and `keep = "false"` has to mean
what it says.

#### `keep`

`keep` asks for an object to be in the netlist that synthesis hands on,
for the benefit of an observer the compiler cannot see: a testbench, a
logic analyser, a probe point, a downstream tool, or the silicon itself.
`ir::KEEP_ATTRS` lists the spellings honoured — `keep`, `dont_touch`,
`mark_debug`, `preserve`, `noprune`, `syn_keep`, `syn_noprune`,
`syn_preserve` — compared without case or separators, so `KEEP` and
`dont-touch` count too. What it promises:

- A kept **net** survives dead-code elimination even when nothing reads
  it, and its readers are never rewritten to read its value or another net
  in its place. It keeps its own name and its own driver.
- A kept net driven by a **state** element keeps that state element: a
  flip-flop or latch whose `q` is kept is not folded to a constant, not
  removed for want of a reader and not merged into another register.
- A kept **cell**, **assign** or **memory** is not removed, not folded
  away and not merged with an identical one.

What still applies to a kept object: the expression cone that feeds it is
still folded, narrowed and shared; a kept flip-flop still has a constant
enable dropped and an enable or synchronous reset lifted out of its `d`
mux, since that is the same flip-flop on the same net; a **combinational**
cell driving a kept net may still fold to a constant assignment or be
absorbed into a lookup-table cover, since the net keeps both its name and
its value; and technology mapping still rewrites a kept `dff` into the
device's own flip-flop, which is what lets a bitstream hold it. A backend's
DSP and carry packers may also fuse a kept `add` or `mul` into a primitive
that computes the same value, leaving the net but not the generic cell.
Only state inherits its output's keep, because only state decides *when* its net takes
a value — a flip-flop's value before its first clock edge is a property of
the fabric rather than of the IR, which is the difference a constant-fed
probe exists to measure.

`keep` is not `keep_hierarchy`: it keeps an object, not a module boundary.
Synthesis warns with `S0034` when a keep sits somewhere it cannot be
honoured (a module, an `initial` block, an instance that wanted
`keep_hierarchy`) and when an attribute reads like a keep and is not one,
such as `keep_signal`. `synth::keep` is the module that decides all of
this; no pass tests the attribute by hand.

## Invariants and validation

`ir::validate::validate(&Design) -> Diagnostics` reports every violation
with a stable code (`I0001` .. `I0020`; the table is in the module docs).
Among them: names are unique per kind per module; ports refer to existing
nets; expression types follow the operator rules and cached types match;
instance ports exist in the target with compatible directions and widths;
no net has two whole-net drivers; constant memory addresses are in range;
cell ports match the kind's table. `validate_module` runs the per-module
subset.

## Building from Rust

`ir::builder::ModuleBuilder` is the intended way to construct modules:

```rust
let mut b = ModuleBuilder::new("counter", span);
let clk = b.input("clk", Type::bit());
let q = b.output_reg("q", Type::bits(8));
let (qv, one) = (b.net(q), b.const_u64(8, 1));
let next = b.add(qv, one);
let mut p = b.process(Some("count"), ProcessKind::posedge(clk));
p.nonblocking(q, next);
b.end_process(p);
let module = b.finish();
```

Expression helpers infer the node type; an ill-typed node is still created
(typed after its first operand) so lowering can continue and `validate`
reports the problem with a span.

## Passes

`ir::walk` provides `Module::for_each_expr`, `for_each_root_expr`,
`for_each_stmt`, `map_nets`, `map_exprs`, `rename_net`,
`remove_unused_nets`, `gc_exprs`, and `Design::instances_of`, `children`,
`topological_order`. `ir::hier` provides the hierarchy passes below.

## Hierarchy

`ir::hier` reshapes the module tree.

### Flattening

`Design::flatten(top, &FlattenOptions) -> Result<FlattenReport,
Diagnostics>` inlines the whole sub-tree of `top` into `top`. Every inlined
instance contributes a copy of the child's nets, memories, expressions,
assigns, processes and cells, named `instance<sep>object`; several
instances of one module therefore produce independent copies. Copies keep
the child's spans, so diagnostics raised later still point into the source
the child was written in.

```
FlattenOptions {
    keep_hierarchy_attr: bool,   // honour `keep_hierarchy` (default true)
    separator: String,           // between path and name (default ".")
    max_depth: Option<u32>,      // Some(0) inlines nothing, Some(1) one level
    annotate_paths: bool,        // record the path in an `origin` attribute
}
```

Port connections become:

| Port    | Connection                | Result                                  |
|---------|---------------------------|-----------------------------------------|
| `in`    | a plain net of equal type | the two nets are merged, no assign       |
| `in`    | anything else             | `assign <child port net> = <expression>` |
| `out`   | net, slice or concat      | `assign <connection> = <child port net>` |
| `inout` | a plain net of equal type | the two nets are merged                  |

Merging keeps the parent's net, so a chain of direct connections collapses
onto the top-level net and costs nothing. Instances stay in place, under
their hierarchical name, when they are unresolved, when their module is a
black box, when the instance or the module carries `keep_hierarchy`, or
when `max_depth` cuts the recursion off. `FlattenReport` counts the
instances inlined per module, the instances kept, the depth reached and the
nets, cells, processes and assigns of the result. Nothing is committed when
a connection cannot be inlined; the design is left untouched and the
diagnostics are returned:

| Code    | Meaning                                                            |
|---------|--------------------------------------------------------------------|
| `I0030` | The module to flatten, or an instance target, is not in the design |
| `I0031` | The hierarchy is recursive and cannot be flattened                 |
| `I0032` | An output port is connected to something that cannot be driven     |
| `I0033` | An inout port is not connected to a plain net of the same type     |

### Black boxes and missing modules

An instance whose `ModuleRef` is `Unresolved` is a hole, and there are two
very different reasons for one.

A **declared** black box is deliberate. Something states the interface and
says the contents come from elsewhere: a `blackbox module` in the text form,
an IP package whose sources are encrypted (`docs/ip.md`), or a primitive
the target technology declares. Either the design holds a module of that
name — `Design::resolve_instances` binds the reference to it — or the flow
knows the primitive. The widths are checked, the emitters write the
instantiation, and the simulator says the box is empty rather than
pretending otherwise.

An **undeclared** one is a mistake, and almost always the same mistake: the
file that defines the module was not given to the build. Reticle has no
library search path, so nothing will find it later; its outputs drive
nothing, and left alone the build reports every *reader* of those outputs
instead — by the hundred, all of them in whichever file was correct.

`Design::check_instance_targets(top, supplied, supplier, diags)` is the
diagnostic for it. It looks only at what `top` reaches, skips a name the
design declares and a name in `supplied`, and reports the rest as `I0034`
at the instantiation, naming the module and the instance.
`Design::undefined_instances` is the same query without the diagnostic.

| Caller | What it passes as `supplied` |
|--------|------------------------------|
| `fpga::synthesize_for` | `Device::primitive_names` — `EHXPLLL`, `TRELLIS_IO`, `SB_RAM40_4K` |
| `asic::synthesize_asic` | the liberty library's cell names |
| `sim::Simulator` | nothing: a simulator has no place-and-route tool to fill a box in |

Each calls it before it maps or runs anything, and refuses the design when
it reports, so the consequences are never reported in place of the cause.
The Verilog front end cannot make this decision on its own — a module it
did not see may still be a VHDL entity, an `.rtl` module or an IP stub
merged into the design afterwards — so there it stays a warning (`V0023`)
and the flow that knows the whole design and its target settles it.

### Unique-ification

`Design::uniquify() -> UniquifyReport` gives every instantiation of a
multiply instantiated module its own module, named `<name>$1`, `<name>$2`,
... with `attrs["uniquified_from"]` holding the original name, so placement
constraints, per-instance attributes and formal properties can be attached
to one instantiation without affecting the others. The first copy reuses
the original module, so existing `ModuleId`s stay valid; the design's top
is never renamed. Modules are processed from the top down, so splitting a
module also splits everything below it.

`Design::dedup() -> DedupReport` is the inverse: modules whose text
rendering is identical once their name and `uniquified_from` are ignored
are merged, instances are repointed at the survivor, and a survivor that is
the last of its group takes its original name back. `uniquify` followed by
`dedup` reproduces the design it started from.
`Design::remove_unused_modules(top)` drops what `top` no longer reaches.

### Queries

- `Design::hier_paths(top) -> Vec<(String, ModuleId)>`: every instance path
  under `top`, depth first in declaration order.
- `Design::resolve_path(top, "u0.u1.state") -> Option<(Vec<InstanceId>,
  NetId)>`: the instances walked through and the net named at the end. At
  every step the rest of the path is first tried as a net name, so the
  flattened net `u0.state` resolves just as well as the hierarchical one.
- `Design::instance_count(top)`: instances in the whole sub-tree.

Examples live in `testdata/ir/hier/*.rtl` with their `.flat.rtl`,
`.uniq.rtl` and `.diag` expectations, driven by `tests/ir_hier.rs`.

## The `.rtl` text format

Line-oriented, two-space indented, `//` comments. One object per line;
`module`, `process`, `if`, `case`, `when`, `default`, `for`, `while`,
`repeat`, `forever` and `block` open a section closed by `end`.

### Names and references

Nets are `%name`, memories `@name`, everything else bare. A name is bare
when it matches `[A-Za-z_$][A-Za-z0-9_$]*` and is not a process kind word
(`comb seq initial sensitive free`); otherwise it is a quoted string with
`\" \\ \n \r \t \u{..}` escapes: `%"a b"`, `process "seq" comb`.

Nets and memories must be declared before use; the writer emits them first.
Ports are resolved at the end of the module so their order is free.

### Types and literals

```
u8  s16          unsigned / signed bit vectors
[4]u8            unpacked array
int real string
8'd255  8'sd255  4'b10xz  16'hbeef  8'o17  8'dx     Verilog sized literals
```

The writer's canonical literal form is decimal for two-state values up to
64 bits, hexadecimal above, binary when any bit is `x` or `z`.

### File layout

```
top <name>                       optional, anywhere at top level

attr <key> = <value>             attributes precede the object they annotate
module <name> [blackbox]
  timescale 1 ns / 1 ps
  param <name> = <value>
  net %<name> <type> wire|reg|var
  port <name> in|out|inout %<net>
  memory @<name> <size> x <type>
    init <const> <const> ...     zero or more lines; a bare `init` means "initialised, empty"
  instance <name> of <module> [#(<p>=<v>, ...)] (<port>=<expr>, ...)
  assign <lvalue> = <expr> [after <n> <unit>]
  process [<name>] comb|initial|free
  process [<name>] seq [posedge|negedge %clk, ...] [async posedge|negedge %rst, ...]
  process [<name>] sensitive %a, %b
    <statements>
  end
  cell <name> <kind...> [#(<p>=<v>, ...)] (<port>=<expr>, ...) -> (<port>=%<net>, ...)
end
```

Attribute and parameter values are a sized literal, a quoted string or an
integer (`keep = 1`, `ram_style = "block"`, `INIT = 16'h8000`).

Cell kinds: the bare keyword (`and`, `add`, `mux`, ...), or

```
dff pos|neg [en] [srst|arst pos|neg <const>]
memrd @<mem> [clocked]
memwr @<mem> [clocked]
lut <k> <const>
blackbox <name>
```

### Statements

```
%x = <expr> [after 2 ns]                 blocking
%x <= <expr> [after 2 ns]                non-blocking
if <expr>  ...  [else ...]  end
case|casez|casex <expr> [unique|priority]
  when <expr>, <expr>  ...  end
  default  ...  end
end
for [<lvalue> = <expr>]; [<expr>]; [<lvalue> = <expr>]  ...  end
while <expr>  ...  end
repeat <expr>  ...  end
forever  ...  end
block [<name>]  ...  end
wait for <expr> | wait on posedge %clk, %a | wait until <expr>
sys <name>(<args>)
readmemh|readmemb|writememh|writememb(<file>, @<mem>[, <start>[, <end>]]) [base <n>]
memwrite @<mem>[<addr>] = <expr> [enable <expr>]
assert note|warning|error|failure <expr> [report <args>]
finish | stop | break | continue
```

Lvalues: `%n`, `%n[7:0]`, `%n[<expr>]`, `{<lvalue>, ...}`, `@m[<expr>]`.

`base` is omitted when it is 0. The file format the memory file
statements read and write, and their address rules, are in
`ir::memfile`, which the simulator and synthesis share with the
`FileProvider` trait they read files through.

### Expressions

```
8'd3  "text"  %net  @mem[<addr>]
<expr>[7:0]  <expr>[<index>]  <expr>[<offset> +: 4]  <expr>[<offset> -: 4]
{a, b, c}  {3{a}}
not(e) neg(e) rand(e) ror(e) rxor(e) rnand(e) rnor(e) rxnor(e) lnot(e)
and(a, b) or xor xnor land lor add sub mul div mod pow shl shr sshr
eq ne ceq cne weq lt le gt ge
mux(c, t, f)
resize(e, u16)  resize(e, s16)
call <name>(<args>) as <type>
(<expr>)
<expr> as <type>
```

Inside `[...]`, a plain integer means a constant slice bound; an index is
always an expression, so a constant index is written `%a[1'd0]`.

`as <type>` sets the node's cached type. The writer emits it only when the
cached type differs from what the operator rules give, which is always the
case for `call`; so a well-typed design shows no ascriptions and an
ill-typed one still loads back unchanged for `validate` to report.

### Example

```
top counter

module counter
  timescale 1 ns / 1 ps
  param WIDTH = 8
  net %clk u1 wire
  net %rst u1 wire
  net %en u1 wire
  attr keep = 1
  net %q u8 reg
  port clk in %clk
  port rst in %rst
  port en in %en
  port q out %q
  process count seq posedge %clk
    if %rst
      %q <= 8'd0
    else
      if %en
        %q <= add(%q, 8'd1)
      end
    end
  end
end
```

More examples, one per feature area, live in `testdata/ir/*.rtl`; the runner
in `tests/ir_text.rs` checks that each parses, validates and prints back
identically (`UPDATE_EXPECT=1` rewrites them after a format change).

### Diagnostics

Parse errors use codes `I0100` (syntax), `I0101` (unknown name) and
`I0102` (malformed literal), and carry spans into the `.rtl` text, so they
render with excerpts through `Diagnostics::render`.
