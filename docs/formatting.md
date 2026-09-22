# Formatting Verilog and VHDL

Reticle ships a source formatter for both languages
(`reticle::verilog::format` and `reticle::vhdl::format`). Each one parses a
file with the ordinary frontend and lays the tree out again from scratch:
the output depends on the *structure* of the design and not on how it was
typed. Whitespace, line breaks and the few purely stylistic choices the tree
does not record are taken from one `FormatOptions`; everything the tree does
record comes back unchanged.

## API

```rust
use reticle::verilog::Dialect;
use reticle::verilog::format::{FormatOptions, format_check, format_source};

let opts = FormatOptions::default();
let formatted: String = format_source(src, Dialect::SystemVerilog, &opts)?;

// Check mode: did it change, and what would the change look like?
let check = format_check(src, Dialect::SystemVerilog, &opts)?;
if check.changed {
    print!("{}", check.diff); // a unified diff
}
```

VHDL is the same with `reticle::vhdl::format` and a
`reticle::vhdl::Standard` in place of the dialect. Both are sans-I/O: the
text goes in as a `&str` and comes back as a `String`.

Both return `Err(Diagnostics)` rather than a result when the file cannot be
formatted safely. See [Refusing to format](#refusing-to-format).

## Options

`FormatOptions` lives in `reticle::fmt_doc` and is shared by both
languages.

| Option                  | Default        | Effect                                                              |
|-------------------------|----------------|---------------------------------------------------------------------|
| `indent`                | `Spaces(2)`    | One indentation level: `Indent::Spaces(n)` or `Indent::Tabs`         |
| `line_width`            | `100`          | The column the layout tries to stay inside                           |
| `align_port_lists`      | `true`         | Line up the columns of a port, generic or parameter list             |
| `align_assignments`     | `true`         | Line up the operators of a run of consecutive assignments            |
| `case_items_on_one_line`| `true`         | Keep a short `case` / `when` arm on the line of its choices          |
| `keyword_case`          | `Lower`        | How VHDL reserved words are spelled (Verilog keywords are lowercase) |
| `complete_end_labels`   | `true`         | Repeat a construct's label after its `end` when the source left it out |
| `newline`               | `Lf`           | `Lf` or `CrLf` line endings                                          |

A tab counts as four columns when deciding whether a line fits. Column
alignment is always written with spaces, so an aligned list stays aligned
whatever the indentation unit is.

## House style

### Verilog

- ANSI headers: `module m #(...) (...);`, with one parameter and one port
  per line when the header does not fit on one; both lists break together,
  so a stacked port list never sits under a one-line parameter list.
- Port lists align in columns: direction, type, packed dimensions, name.
- Two-space indentation, `begin` on the line of the `if`, `else`, `always`
  or `for` that owns it, and `end else begin` on one line.
- A body that is not a block goes on the same line when it fits
  (`if (tick) state <= DATA;`) and on the next line, indented, when it does
  not.
- `case` arms are aligned on their `:`; an arm whose body is one short
  statement stays on that line.
- Instance connections are one per line when the list does not fit, with
  the nets lined up: `.port (net)`. A list that fits stays as `.port(net)`.
- Operators are spaced (`a + b`, `[WIDTH - 1:0]`), event controls
  normalised (`always @(posedge clk or negedge rst_n)`), and a long
  expression breaks after the lowest-precedence operator with a
  continuation indent.
- Attributes (`(* keep *)`) are kept, on their own line before the item
  they annotate.

### VHDL

- `entity` / `architecture` / `process` bodies are indented one level, with
  `begin` and `end` at the level of their header.
- `port (` and `generic (` lists align on the `:`, then on the mode and the
  subtype.
- `=>` is aligned in port maps, generic maps and aggregates.
- Reserved words are lowercase by default; identifiers and literals keep
  their source spelling exactly (`16#FF#`, `x"F_F"`, `\Extended Name\`).
- `when` / `else` chains in conditional and selected assignments break one
  arm per line when they do not fit.
- `end process;` regains its label, `end entity;` its name, when
  `complete_end_labels` is set.

## What is guaranteed

Three properties are tested over every file of both parser corpora and of
the formatter's own corpus:

- **Idempotence.** `format(format(x)) == format(x)`.
- **Semantic preservation.** The AST of the formatted text equals the AST of
  the original, ignoring positions. Nothing is added, dropped or reordered.
- **Comment preservation.** The multiset of comment texts is unchanged. A
  comment may move to the end of the line before it or the start of the line
  after it, but it never disappears and its text is never rewritten — a
  block comment spanning lines is copied byte for byte, inner indentation
  included.

Output is deterministic: the same text and options always give the same
result.

### What is *not* preserved

The tree records precedence, not parentheses, so redundant parentheses are
dropped: `(a > b) && (c < d)` comes back as `a > b && c < d`. Parentheses
that the precedence rules need are of course re-inserted. Likewise, a
construct the parser keeps verbatim as a run of tokens (a Verilog
`specify` block, a `property` body, a UDP table row, a clocking block) is
written back as those tokens separated by single spaces, not with its
original spacing.

## Refusing to format

A formatter that mangles a file is worse than no formatter, so both
formatters return the diagnostics and no result when:

- the lexer or parser reported an **error** — the tree is then a partial
  recovery and writing it back would lose whatever was skipped; or
- (VHDL) the parser reported that it **skipped** source it does not model —
  PSL directives are the only such case today.

A caller that gets an `Err` keeps the original text.

## Comments, blank lines and directives

Comments are not tokens: the lexer puts them in a side table, and the
formatter re-attaches them to the tree by position. A comment before an item
becomes a *leading* comment of it, one on the same line after it a
*trailing* comment, and one between the last item of a block and its closing
keyword a *dangling* comment; anything left at the end of the file is
flushed there. A comment written inside an expression surfaces at the end of
the statement that contains it.

Blank lines are structure too: wherever the source had one or more blank
lines between two items, the output has exactly one.

### Verilog macros

The formatter deliberately does **not** run the preprocessor — expanding
`` `define `` would rewrite the source into text the author never wrote.
Instead, every line whose first non-blank character is a backtick is treated
as opaque: it is blanked out of the copy handed to the parser (blanked, not
removed, so every span still points at the original text) and re-emitted
verbatim in place.

That keeps `` `define ``, `` `ifdef `` / `` `else `` / `` `endif `` and macro
uses byte for byte and in order, so re-preprocessing the formatted text
yields the same design. Two consequences:

- The contents of *both* arms of an `` `ifdef `` are formatted, since both
  are syntactically present once the directives are blanked.
- A macro used where a token is expected (`reg [`WIDTH-1:0] q;`), or a
  directive that splits a construct in half, leaves the file unparseable
  without the preprocessor. The formatter reports the parse errors and the
  file is left alone.

Macro-heavy files therefore format conservatively, and some of them not at
all.

## Inside: `fmt_doc`

Both formatters build a `reticle::fmt_doc::Doc`, a Wadler / Prettier-style
document that says what the output contains and where it *may* break; the
layout algorithm then decides, for each `group`, whether it fits on the rest
of the line. `text`, `line`, `softline`, `hardline`, `concat`, `group` and
`nest` are the whole vocabulary, plus `verbatim` for text (comments) that
must be copied exactly.

Nesting counts *levels*, not columns, so one document renders correctly
with two spaces, four spaces or tabs. Column alignment is done by the rules,
which measure a piece with `Doc::flat_width` and emit explicit padding.

`fmt_doc` also carries the line diff (`unified_diff`, on a Myers
shortest-edit-script search) that `format_check` reports with, so a future
`reticle fmt --check` needs nothing outside the crate.

## Test corpora

- `testdata/verilog/format/<name>.v|.sv` → `<name>.fmt.v|.fmt.sv`
- `testdata/vhdl/format/<name>.vhd` → `<name>.fmt.vhd`

Each corpus holds a copy of the parser corpus' construct groups plus
deliberately badly formatted input: everything on one line, ragged
indentation, comments in awkward places, and CRLF endings. Run the golden
tests with `UPDATE_EXPECT=1` to accept an intended change:

```sh
UPDATE_EXPECT=1 cargo test --test verilog_format --test vhdl_format
```
