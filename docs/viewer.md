# The schematic and documentation viewer

A netlist is a wall of text, and a design's interface is spread over a
port list, a parameter list and whatever comments the author wrote. The
`viewer` feature renders both into a small static site: one index, one
schematic per module and one reference page per module, plus a listing of
every source file a location points into.

Every page is self-contained. The style sheet and the script are inlined,
no font is imported, no image is fetched and nothing at all is loaded from
the network, so a page opens from `file://` on a machine with no
connection and can be attached to a bug report as a single file.
`tests/viewer_offline.rs` is what keeps that true: it fails on any
`http:`, `src=`, `<link>`, `@import` or `url(` in any generated page.

```sh
reticle viewer --synth counter.v
xdg-open viewer/index.html
```

## What the pages show

### The index

One row per module: ports, nets, cells, the logic depth estimate (the same
one `reticle synth --report` prints) and the number of instances, linking
to both views. A module whose schematic was skipped for being too large
says so rather than linking to a page that is not there.

### The schematic

The module's cell form, laid out left to right and drawn as inline SVG.

| Drawn as | Means |
|----------|-------|
| a light rounded box | a combinational cell (`and`, `add`, `mux`, `lut4`, ...) |
| a heavier box in another colour, with a clock mark | a flip-flop, latch or memory port — state, which the eye should find first |
| a box with a thicker border | a sub-module instance, a black box, or a process the module still carries |
| a small triangle | a constant feeding one pin |
| a tag on the left or right edge | a module port, pointed in the direction the value travels |
| a thick wire whose label ends in `/4` | a four-bit bus |

The page itself is interactive, with a few dozen lines of hand-written
JavaScript and no library:

- **pan and zoom** — drag, wheel, the toolbar buttons, or `+`, `-` and `0`;
- **hover a wire or a pin** — every wire, pin and box on that net lights up,
  so a net is traced across the drawing without reading a single name;
- **click a cell** — the side panel shows its parameters (reset polarity,
  LUT contents, black-box parameters, attributes), what each pin is wired
  to, and a link to the source line it came from;
- **search** — type a cell or net name and the matches are outlined and the
  first one is centred.

A module still in *process* form — one that has not been synthesised —
is drawn at the same grain, one box per process, with the nets it reads
on the left and the nets it writes on the right. Pass `--synth` to
synthesise first and get the cell-level picture.

### The reference page

Built from the IR and from the comments:

- the run of whole-line comments above the module is its **description**;
- a **port table** with direction, width, type, the net behind the port and
  the comment written after it;
- a **parameter table** with the resolved default, its type and the range
  that type allows;
- the **memories**, with element type, depth and total bits;
- the **inferred storage** — every flip-flop, latch and memory port, with
  its features and the source construct it came from;
- the **instance tree**, with each instance linked to the page of the
  module it names;
- every source location linked to its line in the listing.

## Where the prose comes from

Neither frontend attaches comments to its syntax tree: both lexers push
every comment into a side table keyed by span (`verilog::Lexed::comments`,
`vhdl::Lexed::comments`), which is what the formatters use. The viewer
reads the same table through `viewer::Comments`, which answers two
questions:

- `leading` — the run of comments that occupy whole lines immediately above
  an object, ending at a blank line or at anything else on one of those
  lines. That is a module's description.
- `trailing` — the comment after an object on its own line. That is a
  port's note.

`Comments` holds spans, not text, and recognises `//`, `--` and `/* */`
when it reads the text back out of the `SourceMap`, so the `viewer`
feature builds on its own and the frontends are only needed for the
convenience constructors `add_verilog` and `add_vhdl`.

A design loaded from `.rtl` has no comment table — the IR text format's
parser does not keep one — so its reference pages carry the structure but
no prose.

## How the schematic is laid out

`viewer::graph` turns a module into nodes and wires, and `viewer::layout`
places them. The steps, all integer-only so the output is byte-stable:

1. **Layering.** Each node gets the length of the longest path reaching it,
   by the same memoised walk `synth::report` uses for its depth estimate.
   Edges that close a cycle are found first and left out, so register
   feedback shortens a path instead of hanging the walk. Input ports are
   pinned to the first column and output ports to the last, so the
   interface frames the drawing; a constant is pulled to just before the
   pin it feeds.
2. **Dummies.** An edge spanning more than one rank gets a bend point in
   each column it crosses, which reserves a slot no cell body occupies.
3. **Ordering.** Four sweeps of the barycentre heuristic, down then up.
   A node with no neighbour on that side keeps its place and ties break on
   the current position, so the result is a function of the graph alone.
4. **Placement.** Column width is the widest box in it, rows stack with a
   fixed gap, columns are centred against the tallest, and a second pass
   pulls each bend point towards the straight line between the pins its
   wire joins, clamped so it never overlaps a neighbour.
5. **Routing.** Wires are orthogonal polylines. Every vertical run lives in
   the gutter between two columns, which holds no cell bodies, and each
   gutter is a channel: runs are sorted and given the first track whose
   intervals they do not overlap, which fixes both the gutter's width and
   the wire's `x`. An edge pointing backwards leaves through the gutter
   after its source, runs along a lane below the drawing and comes back up
   the gutter before its target.

`tests/viewer_golden.rs` checks the result on every design in
`testdata/ir` and `testdata/synth`: boxes never overlap, every wire starts
and ends on the pin it joins, no segment is diagonal, and no wire crosses
a cell body.

## The API

```rust
use reticle::source::SourceMap;
use reticle::viewer::{Comments, ViewerOptions, render};

let mut options = ViewerOptions {
    title: "counter".into(),
    comments,            // viewer::Comments, from the lexers' side tables
    ..ViewerOptions::default()
};
options.only = vec!["counter".into()];   // empty means every module

let site = render(&design, &map, &options);
for (path, contents) in &site.files {
    // the library is sans-I/O: the caller writes them
}
```

`Site::files` is a `Vec<(String, String)>` sorted by path, with `/` as the
separator; `source/...` is the one sub-directory. Nothing under `src/`
touches the filesystem, so the CLI does the writing.

`ViewerOptions` also has `schematics`, `docs` and `sources` to leave a view
out, and `max_nodes`, above which a module's schematic is skipped: a
netlist of ten thousand cells makes a page no browser enjoys and no reader
can follow.

## The command

```
Usage: reticle viewer [options] <design.v|design.vhd|design.rtl>...

Options:
  --output-dir <d>   Write the pages here (default: viewer)
  --top <module>     Treat this module as the top
  --module <names>   Render only these modules (comma separated)
  --title <text>     Heading of the index page
  --synth            Synthesise first, so the schematic shows cells
  --max-nodes <n>    Skip a schematic with more boxes than this (default 1500)
  --no-schematic     Skip the schematic pages
  --no-doc           Skip the reference pages
  --no-source        Skip the source listings
  --quiet            Suppress the summary line
```

The command lexes each Verilog and VHDL source a second time to fill the
comment table; that is cheap next to elaboration and keeps every other
command's loading path unchanged.

## Determinism

Geometry is computed, never random; every map in the pipeline is a
`BTreeMap`; iteration follows arena order. The same design renders to the
same bytes, which is what makes the golden files in `testdata/viewer/`
worth reading in a review — they are real pages, so
`xdg-open testdata/viewer/netlist/index.html` shows exactly what is being
approved.
