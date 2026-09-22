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
`topological_order`. Flattening and unique-ification are not implemented
yet.

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
memwrite @<mem>[<addr>] = <expr> [enable <expr>]
assert note|warning|error|failure <expr> [report <args>]
finish | stop | break | continue
```

Lvalues: `%n`, `%n[7:0]`, `%n[<expr>]`, `{<lvalue>, ...}`, `@m[<expr>]`.

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
