# The Reticle language server

`reticle::lsp` is a Language Server Protocol implementation for Verilog /
SystemVerilog and VHDL, behind the `lsp` Cargo feature. It drives the two
front ends directly, so an editor sees the same parse errors as
`reticle check`, the same findings as `verilog::lint` (all 28 rules, see
`docs/lints.md`) and the same layout as `reticle fmt`.

Everything is written from scratch, JSON included: the crate takes no
dependencies, so `reticle::lsp::json` is a small RFC 8259 parser and
serialiser and `reticle::lsp::protocol` is the `Content-Length` framing,
JSON-RPC 2.0 and the handful of LSP types the server actually uses.

## Running it

The library is sans-I/O. The transport is a thirty-line adapter that a host
points at a pair of streams:

```rust
use std::io::{stdin, stdout, BufReader};

use reticle::lsp::stdio::serve;

let mut input = BufReader::new(stdin().lock());
let mut output = stdout().lock();
serve(&mut input, &mut output)?;
```

`serve` reads `Content-Length` framed JSON-RPC messages, hands each to a
`Server`, writes back what it returns, and stops at `exit` or at the end of
input.

## The server as a value

The interesting property of the design is that the server does no I/O at
all:

```rust
use reticle::lsp::json::Json;
use reticle::lsp::protocol::request;
use reticle::lsp::server::Server;

let mut server = Server::new();
let replies: Vec<Json> = server.handle(request(Json::Int(1), "initialize", Json::object()));
```

`Server::handle` takes one decoded message and returns the messages to send
back: a response for a request, notifications for anything it wants to
publish, and nothing at all for a notification it has no answer to. A whole
editing session is therefore a fold over `handle`, which is how
`tests/lsp_session.rs` replays scripted sessions against golden
transcripts, and why there is no thread, no channel and no timer anywhere
in the implementation.

## Supported requests

| Method | Verilog | VHDL | Notes |
|--------|---------|------|-------|
| `initialize`, `initialized`, `shutdown`, `exit` | ✓ | ✓ | The full lifecycle; requests before `initialize` get `ServerNotInitialized` |
| `textDocument/didOpen`, `didChange`, `didClose`, `didSave` | ✓ | ✓ | Incremental sync (`change: 2`); diagnostics are published after each |
| `textDocument/publishDiagnostics` | ✓ | ✓ | Parse errors, the Verilog linter's 28 rules, VHDL semantic errors |
| `textDocument/definition` | ✓ | ✓ | A name to its declaration, in the same document |
| `textDocument/references` | ✓ | ✓ | Honours `context.includeDeclaration` |
| `textDocument/hover` | ✓ | ✓ | Declaration text, resolved type, width, port list |
| `textDocument/documentSymbol` | ✓ | ✓ | Hierarchical `DocumentSymbol`s |
| `textDocument/completion` | ✓ | ✓ | Port names in a connection list, names in scope, keywords by context |
| `textDocument/prepareRename`, `rename` | ✓ | ✓ | Refuses, with a reason, what it cannot see all of |
| `textDocument/formatting`, `rangeFormatting` | ✓ | ✓ | Through `verilog::format` / `vhdl::format`, as minimal line edits |

Anything else is answered with `MethodNotFound`; unknown *notifications*
are dropped, as the specification requires.

A document's language comes from the client's `languageId` (`verilog`,
`systemverilog`, `vhdl`) and, failing that, from the URI's extension
(`.v`, `.vh`, `.sv`, `.svh`, `.vhd`, `.vhdl`). A document that is neither
is not opened at all, so another server can have it.

## Positions

This is the part that is easy to get wrong, so it is worth stating plainly.

- An LSP `Position` is a zero-based line and a zero-based offset **in
  UTF-16 code units** within that line.
- A Reticle `Span` is a pair of **UTF-8 byte offsets** into a file.

They agree only on ASCII. A line holding `é` is one byte longer than it is
code units; one holding `😀` is two bytes longer *and* two code units wide
for one character. Every crossing therefore goes through
`protocol::LineIndex`, which is rebuilt once per revision and converts in
both directions, clamping a position that is past the end of a line to that
line's end and one that lands inside a character down to that character's
start.

`testdata/lsp/unicode.session` pins this down end to end: on a line whose
`data` argument sits at UTF-16 column 34 and byte offset 39, hovering at
column 34 describes the port and hovering at column 39 — which is inside
the string literal — describes nothing.

The server advertises `"positionEncoding": "utf-16"` so a client does not
negotiate UTF-8 offsets and get back ones it cannot use.

## Diagnostics

Every change reparses the whole file. That is a deliberate choice, not a
placeholder: the Verilog preprocessor and the VHDL analysis both work from
the start of the text, an HDL source file is rarely more than a few
thousand lines, and a full parse and analysis of one is a few milliseconds.
The *sync* is incremental — the client sends only the range it changed —
but the parse is not.

Debouncing is the client's job. The server never sleeps, never spawns
anything and answers every notification before returning.

Severities map as `Error` → 1, `Warning` → 2, `Note` → 3 (Information),
and `Help` diagnostics, which only ever accompany another, are dropped.

The `code` of a diagnostic is the **lint rule name** for a lint finding
(`unused-signal`, not `L0007`), because that is what `docs/lints.md` lists
and what a `// reticle-lint: off` comment takes. Front-end diagnostics keep
their own stable code (`V0200` and friends). Secondary labels become
`relatedInformation`, and the notes the command-line renderer would print
after the excerpt are appended to the message — except the linter's own
"disable with ..." note, which the code already says.

The Verilog linter runs only once the file parses cleanly, so a file in the
middle of an edit reports the one syntax error and not the forty
consequences the recovery left behind.

## What each feature knows

The index the features read is *shallow on purpose*. It records what the
source says, not what elaboration would make of it, because a language
server runs on half-written text and has neither a top module nor the
parameter overrides that would settle the rest.

**Verilog** has no name resolution outside elaboration, so `lsp::verilog`
does its own: one walk of the AST, opening a scope for every declarative
region (module, subroutine, named block, generate block), recording every
declaration and every simple name used, resolving the uses against the
scope chain at the end — which is what makes a module instantiated above
its own definition resolve anyway. Widths come from literal packed
dimensions (`[7:0]` is 8 bits) and from the type keywords (`int` is 32),
and from nowhere else: `wire [W-1:0] q` reports no width, because `W` may
be overridden at every instantiation and a hover that guesses is worse than
one that says nothing.

**VHDL** needs none of that. `vhdl::sema::Analysis` has already resolved
every name and typed every expression into span-keyed side tables, so
`lsp::vhdl` walks the tree only for *structure* — what encloses what, how
far each declaration extends, where a port map is — and gets uses by
scanning the identifier tokens through `Analysis::decl_of`. A call is the
one name that table does not hold, because the analysis records what
`f(x)` resolved to under the span of the whole call; that span is
reconstructed from the token stream (an identifier, a parenthesis, up to
the matching one) and looked up in `Analysis::call_of`, so a call site is
a reference to its subprogram like any other name. A reconstruction that
was not a call simply misses in the table, which is why this is not a
guess. Types and widths are the real, resolved ones: a hover says
`std_logic_vector(7 downto 0)` and `8 bits` because the front end worked
them out.

Both sides then meet in one language-neutral `lsp::index::Index`, and every
feature is written once over it.

### Hover

The declaration as it was written, then its kind, its resolved type and its
width, then the port list for a module, entity or component:

````markdown
```vhdl
signal count : std_logic_vector(7 downto 0);
```
signal `count` — `std_logic_vector(7 downto 0)`, 8 bits
````

A VHDL name the document does not declare but the analysis resolved —
`std_logic`, `rising_edge`, anything from the bundled `std` and `ieee`
sources — gets a hover too, saying what it is, what its type is and which
bundled file declares it. Go-to-definition stays silent there, because
there is no document to send the client to.

When the current revision does not parse, hover falls back to the last one
that did — and then returns **no range**, since the spans of that revision
belong to a text the client has already replaced.

### Completion

Three groups, ordered by `sortText` so the client keeps them apart:

1. **Port names of the unit being instantiated**, inside an instantiation's
   connection list or a VHDL `port map`. This is the one completion that
   saves real typing, because those names live in another declaration
   entirely. Ports already connected are not offered again — except the one
   the cursor is inside, which the parser has already seen. After a `.`,
   nothing else is offered at all.
2. **Names in scope** at the position, with their types.
3. **Reserved words** for the context: what can start a design unit at file
   level, a declaration or concurrent statement inside a module or
   architecture, a statement inside a process or subprogram. Verilog
   keywords are filtered by the document's dialect, so `always_ff` is not
   offered in a `.v` file.

Both lists are sorted, so the same document always yields the same
completion in the same order.

### Rename

A rename rewrites every occurrence in the document — or refuses, and says
why:

- A module, interface, entity, architecture, package or component name is
  visible to every other file of the design, and the server only sees what
  the client has opened. Refused.
- A local name that another *open* document also declares globally.
  Refused, naming the other document.
- A new name that is not an identifier in both languages (a letter or
  underscore, then letters, digits and underscores). Refused.

The refusal comes back as a JSON-RPC error with code `-32803`
(`RequestFailed`) and the reason as its message, which is what the editor
shows. `textDocument/prepareRename` says the same thing in advance by
returning `null` for a name it will not rename.

### Formatting

Both `formatting` and `rangeFormatting` run the file through the existing
formatter (`docs/formatting.md`) and return the **minimal line edits** that
get there, computed with the same line diff the `--check` mode uses, so the
client's cursor and folds survive. Range formatting formats the whole file
and then keeps only the hunks that touch the requested range: a port list's
alignment depends on every port in it, so formatting a fragment in
isolation would not agree with formatting the file.

`tabSize` and `insertSpaces` from the client's `FormattingOptions` choose
the indentation unit; everything else is the house style, which is the
point of having one. A file that does not parse is not formatted, and the
request comes back as an error whose message is the first parse error.

## Testing

- Unit tests next to each module: the JSON parser and serialiser (round
  trips, escapes, surrogate pairs, numbers, nesting, malformed input), the
  UTF-16 conversion against two-, three- and four-byte characters in both
  directions, the two index builders, and each feature on small documents.
- `tests/lsp_session.rs` replays `testdata/lsp/<name>.session` scripts —
  one per scenario: a Verilog session, a VHDL session, an edit that breaks
  a document and one that repairs it, renames that are allowed and renames
  that are refused, and the non-ASCII positions — and compares the whole
  transcript with `<name>.expected`. `UPDATE_EXPECT=1` rewrites them.

The script format is line-oriented, like the crate's other text formats,
and is documented at the top of `tests/lsp_session.rs`:

```text
open    file:///counter.v verilog counter.v
edit    file:///counter.v 2 9 16 9 17 "("
ask     3 textDocument/hover file:///counter.v 7 13
doc     7 textDocument/documentSymbol file:///counter.v
request 1 initialize {"clientInfo":{"name":"golden"}}
notify  initialized {}
```

## Limits

Worth knowing before pointing an editor at it:

- **One document at a time.** There is no workspace index: the server
  answers from the document a request names, plus, for a rename, a check
  against the other open ones. Go-to-definition across files, workspace
  symbols and a module hierarchy view need a project model
  (`reticle.proj`, phase 8) wired in, which is the natural next step.
- **No elaboration.** Widths that depend on a parameter, a generic or a
  generate loop are not reported. Hooking the language server up to
  elaboration would give exact widths for a chosen top, at the cost of
  needing one.
- **Verilog includes are not resolved.** `` `include `` is preprocessed
  against an empty resolver, which reports `cannot find include file` and
  leaves the file's macros unexpanded; the linter then waits for a clean
  parse that will not come. Resolving them needs a search path from the
  client, which the server does not read yet.
- **No code actions, inlay hints, semantic tokens, signature help or
  call hierarchy.** The protocol has a long tail; these are the parts of it
  that a hardware designer asks for first, and the rest can follow.
