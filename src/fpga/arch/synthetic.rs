//! ============================================================
//! A **SYNTHETIC** iCE40-*like* architecture. NOT THE SILICON.
//! ============================================================
//!
//! Read this before believing anything this module produces.
//!
//! Project IceStorm's chip database — the file that says which wire is
//! where, which pip joins which two of them, and which configuration bit
//! switches it on — is the product of years of fuzzing real parts. It is
//! not in this repository, it is not on the machine this was written on,
//! and it cannot be guessed. Inventing it would produce a file that looks
//! authoritative and programs nothing, which is worse than having none.
//!
//! So this module builds a fabric that is *shaped* like an iCE40 and is
//! *not* an iCE40, so that the placer, the router and the bitstream
//! writer can be built, tested and reviewed end to end against something
//! concrete. What is borrowed from the published material, and what is
//! invented:
//!
//! | Borrowed | Invented here |
//! |---|---|
//! | the 14 x 18 tile grid of the 1k dice | everything inside a tile |
//! | an IO ring on the border, RAM in columns 3 and 10, logic elsewhere | which tile holds which bel |
//! | the tile bitmap sizes (`logic_tile` 16x54, `io_tile` 16x18, `ramb`/`ramt` 16x42) | every bit position in them |
//! | eight global clock networks | which tiles their buffers sit in |
//! | the primitive names, pins and parameters, from [`super::super::device`] | the package pin to site map |
//!
//! The routing is the textbook shape: four groups of local tracks per
//! tile, one per direction, two span-4 and two span-12 lines in each of the four directions, a
//! global network, a carry chain up each column, and a direct
//! LUT-to-flip-flop connection inside a logic cell. Every wire is driven
//! by one multiplexer, and a pip's configuration bits are that
//! multiplexer's code, which is how a real database encodes them too: the
//! pips into one wire share a small field rather than owning a bit each.
//!
//! Four differences from the real part are worth naming, because a design
//! that leans on them will not port:
//!
//! 1. **Two logic cells per tile, so 320 in all**, not the eight and
//!    1280 the 1k datasheet quotes. The device database still says 1280,
//!    because that is what the datasheet says; this fabric is sparser,
//!    for the reason the `LCS_PER_TILE` constant below gives.
//! 2. The LUT, the flip-flop and the carry of one logic cell are
//!    **separate bels on separate sites**, with separate input wires. A
//!    real iCE40 packs them into one `LogicCell` and feeds the carry from
//!    the LUT's own inputs, which is exactly why a real flow needs a
//!    packing step that this one does not have.
//! 3. Each flip-flop has its **own** clock, enable and reset wires. On a
//!    real iCE40 the cells of a tile share all three, which is what
//!    [`FfFeatures::reset_is_shared`] records.
//! 4. A carry cell's `co` reaches the local tracks as well as the next
//!    cell's `ci`, so a badly placed chain still routes. Real carry is
//!    the dedicated path only.
//!
//! Configuration bits are handed out by a counter walking each tile
//! type's bitmap in row-major order, in the order the declarations below
//! are made. They are unique within a tile, which is all the bitstream
//! writer needs, and they correspond to nothing on a die.
//!
//! [`FfFeatures::reset_is_shared`]: super::super::FfFeatures::reset_is_shared

use super::{Arch, BelDecl, ConfigBit, ConfigEntry, PipDecl, TileType, WireDecl, WireRef};

/// Logic cells in one logic tile.
///
/// A real 1k tile has eight, sharing one set of LUT inputs between the
/// LUT and the carry and one clock, enable and reset between all eight.
/// Here the LUT, the flip-flop and the carry are separate bels with
/// separate wires, so a tile of eight would need several times the local
/// routing a real one has. Two keeps the fabric routable without a
/// packing step, at the cost of being sparser than the silicon.
const LCS_PER_TILE: usize = 2;
/// Groups of local tracks per tile, one per span direction.
const LOCAL_GROUPS: usize = 4;
/// Tracks in one local group of a logic or RAM tile, which is the number
/// a real 1k logic tile has (icebox names them
/// `local_g<group>_<index>`).
const LOCALS_PER_GROUP: usize = 12;
/// Tracks in one local group of an IO tile, which holds two buffers and
/// needs far fewer. Its bitmap is a third the size of a logic tile's, so
/// this is also what makes its multiplexer fields fit.
const IO_LOCALS_PER_GROUP: usize = 4;
/// Parallel lines of each length in each direction.
const SPAN_COPIES: usize = 2;
/// Global clock networks, matching `global_buffers 8` in the device file.
const GLOBALS: usize = 8;
/// Address bits of the block RAM's ports, from `SB_RAM40_4K`'s deepest
/// mode (2048 words).
const RAM_ADDR: usize = 11;
/// Data bits of the block RAM's ports, from its widest mode.
const RAM_DATA: usize = 16;

/// The four directions a long line can run; the index is also the local
/// group that collects what arrives from that direction.
const DIRECTIONS: [(&str, i32, i32); 4] = [("e", 1, 0), ("w", -1, 0), ("n", 0, 1), ("s", 0, -1)];

/// One long line of a tile: its name, how far it reaches, and the local
/// group it taps back down into.
struct Span {
    name: String,
    dx: i32,
    dy: i32,
    group: usize,
}

/// Every long line a tile carries: two parallel span-4 lines and two
/// span-12 lines in each of the four directions.
///
/// A line is driven from the local tracks of the group it belongs to and
/// taps back down into that same group; a signal turns a corner by
/// crossing between groups inside a tile, which [`add_track_muxes`]
/// provides for.
fn spans() -> Vec<Span> {
    let mut out = Vec::new();
    for (group, (dir, ux, uy)) in DIRECTIONS.into_iter().enumerate() {
        for length in [4, 12] {
            for copy in 0..SPAN_COPIES {
                out.push(Span {
                    name: format!("sp{length}{dir}{copy}"),
                    dx: ux * length,
                    dy: uy * length,
                    group,
                });
            }
        }
    }
    out
}

/// Where a span line may be tapped back down to a local track: a span-4
/// at every tile it crosses, a span-12 halfway along and at its end.
fn taps(length: i32) -> &'static [i32] {
    if length.abs() == 4 {
        &[1, 2, 3, 4]
    } else {
        &[6, 12]
    }
}

/// The package pins of the tq144 part the device database records, in the
/// order they are mapped onto IO sites.
const TQ144_PINS: [&str; 16] = [
    "8", "9", "21", "78", "79", "80", "81", "87", "88", "89", "90", "95", "96", "97", "98", "99",
];

/// Hands out unique configuration bit positions inside one tile bitmap.
struct BitAllocator {
    cols: u32,
    rows: u32,
    next: u32,
}

impl BitAllocator {
    fn new(rows: u32, cols: u32) -> Self {
        BitAllocator {
            cols,
            rows,
            next: 0,
        }
    }

    /// The next free bit, row-major.
    ///
    /// Running out is a bug in this file, not something a user can cause:
    /// the tile bitmaps below are sized for the declarations below.
    fn take(&mut self) -> ConfigBit {
        let row = self.next / self.cols;
        assert!(
            row < self.rows,
            "the synthetic tile bitmap ({} x {}) is too small for its declarations",
            self.rows,
            self.cols
        );
        let bit = ConfigBit::new(row, self.next % self.cols);
        self.next += 1;
        bit
    }

    /// `n` bits, in order.
    fn take_n(&mut self, n: usize) -> Vec<ConfigBit> {
        (0..n).map(|_| self.take()).collect()
    }
}

/// Collects the pips of a tile type as multiplexers, one per destination
/// wire, and gives each destination a field wide enough to name its
/// sources.
///
/// This is what a real routing database looks like: the sixteen pips into
/// a local track are sixteen codes of one four-bit field, not sixteen
/// bits. Code 0 means the multiplexer is off, so an unused wire has all
/// its bits clear.
#[derive(Default)]
struct Muxes {
    /// Destinations in declaration order, with their sources.
    entries: Vec<(WireRef, Vec<WireRef>)>,
}

impl Muxes {
    /// Records that `to` can be driven by `from`.
    fn add(&mut self, from: WireRef, to: WireRef) {
        let key = to.to_text();
        match self.entries.iter_mut().find(|(d, _)| d.to_text() == key) {
            Some((_, sources)) => sources.push(from),
            None => self.entries.push((to, vec![from])),
        }
    }

    /// Records that `to` can be driven by any of `sources`.
    fn add_all(&mut self, sources: &[WireRef], to: &WireRef) {
        for source in sources {
            self.add(source.clone(), to.clone());
        }
    }

    /// Turns the multiplexers into pips, allocating each one its field.
    fn finish(self, bits: &mut BitAllocator) -> Vec<PipDecl> {
        let mut out = Vec::new();
        for (to, sources) in self.entries {
            let width = code_width(sources.len());
            let field = bits.take_n(width);
            for (index, from) in sources.into_iter().enumerate() {
                let code = index + 1;
                let set = field
                    .iter()
                    .enumerate()
                    .filter(|(b, _)| (code >> b) & 1 == 1)
                    .map(|(_, bit)| *bit)
                    .collect();
                out.push(PipDecl {
                    from,
                    to: to.clone(),
                    bits: set,
                });
            }
        }
        out
    }
}

/// How many bits it takes to name `sources` sources plus the off code.
fn code_width(sources: usize) -> usize {
    let mut width = 0;
    while (1usize << width) <= sources {
        width += 1;
    }
    width
}

/// The name of local track `k` of group `g`.
fn local(g: usize, k: usize) -> String {
    format!("local_g{g}_{k}")
}

/// The local tracks of one group, as wire references.
fn group(g: usize, per_group: usize) -> Vec<WireRef> {
    (0..per_group)
        .map(|k| WireRef::local(local(g, k)))
        .collect()
}

/// Adds the local tracks and the span lines to a tile type.
fn add_tracks(tile: &mut TileType, per_group: usize) {
    for g in 0..LOCAL_GROUPS {
        for k in 0..per_group {
            tile.wires.push(WireDecl {
                name: local(g, k),
                dx: 0,
                dy: 0,
            });
        }
    }
    for span in spans() {
        tile.wires.push(WireDecl {
            name: span.name,
            dx: span.dx,
            dy: span.dy,
        });
    }
}

/// Adds the multiplexers that drive the local tracks and the span lines.
///
/// `outputs` are the wires the tile's own bels drive. Output `t` reaches
/// the tracks of group `t % 4`; a span line reaching this tile taps into
/// the group that collects its direction; and each track is joined to the
/// track of the same index in the other three groups, which is how a
/// signal turns a corner without passing through a bel.
fn add_track_muxes(muxes: &mut Muxes, outputs: &[WireRef], per_group: usize) {
    let spans = spans();
    for g in 0..LOCAL_GROUPS {
        for k in 0..per_group {
            let track = WireRef::local(local(g, k));
            for (t, output) in outputs.iter().enumerate() {
                if t % LOCAL_GROUPS == g {
                    muxes.add(output.clone(), track.clone());
                }
            }
            for span in spans.iter().filter(|s| s.group == g) {
                let length = if span.dx == 0 { span.dy } else { span.dx };
                for step in taps(length) {
                    muxes.add(
                        WireRef::at(
                            &span.name,
                            -span.dx.signum() * step,
                            -span.dy.signum() * step,
                        ),
                        track.clone(),
                    );
                }
            }
            for h in 0..LOCAL_GROUPS {
                if h != g {
                    muxes.add(WireRef::local(local(h, k)), track.clone());
                }
            }
        }
    }
    for span in &spans {
        muxes.add_all(&group(span.group, per_group), &WireRef::local(&span.name));
    }
}

/// The logic tile: four logic cells, each a LUT, a flip-flop and a carry
/// element on their own sites.
fn logic_tile(ff_primitives: &[String]) -> TileType {
    let mut tile = TileType::new("logic", "logic_tile", 16, 54);
    let mut bits = BitAllocator::new(16, 54);
    add_tracks(&mut tile, LOCALS_PER_GROUP);
    for n in 0..LCS_PER_TILE {
        for i in 0..4 {
            tile.wires.push(WireDecl {
                name: format!("lc{n}_i{i}"),
                dx: 0,
                dy: 0,
            });
        }
        for role in ["out", "d", "q", "clk", "en", "rst", "ci", "ci0", "ci1"] {
            tile.wires.push(WireDecl {
                name: format!("lc{n}_{role}"),
                dx: 0,
                dy: 0,
            });
        }
        // The last cell's carry out reaches the tile above, which is what
        // carries a chain up a column.
        let last = n + 1 == LCS_PER_TILE;
        tile.wires.push(WireDecl {
            name: format!("lc{n}_co"),
            dx: 0,
            dy: i32::from(last),
        });
    }

    for n in 0..LCS_PER_TILE {
        let mut lut = BelDecl::new(format!("lc{n}_lut"), "lut");
        for i in 0..4 {
            lut.pins
                .push((format!("i{i}"), WireRef::local(format!("lc{n}_i{i}"))));
        }
        lut.pins
            .push(("o".to_owned(), WireRef::local(format!("lc{n}_out"))));
        for index in 0..16 {
            lut.config.push(ConfigEntry::Param {
                name: "LUT_INIT".to_owned(),
                index,
                at: bits.take(),
            });
        }
        tile.bels.push(lut);

        let mut ff = BelDecl::new(format!("lc{n}_ff"), "ff");
        for role in ["clk", "d", "q", "en", "rst"] {
            ff.pins
                .push((role.to_owned(), WireRef::local(format!("lc{n}_{role}"))));
        }
        // A mode field wide enough for every `SB_DFF*` the device file
        // declares; the code is the primitive's position in that file,
        // plus one, so that no variant is all zeroes.
        let mode = bits.take_n(code_width(ff_primitives.len()));
        for (index, primitive) in ff_primitives.iter().enumerate() {
            let code = index + 1;
            let set: Vec<ConfigBit> = mode
                .iter()
                .enumerate()
                .filter(|(b, _)| (code >> b) & 1 == 1)
                .map(|(_, bit)| *bit)
                .collect();
            ff.config.push(ConfigEntry::Cell {
                primitive: primitive.clone(),
                bits: set,
            });
        }
        tile.bels.push(ff);

        let mut carry = BelDecl::new(format!("lc{n}_carry"), "carry");
        for (role, wire) in [("ci", "ci"), ("i0", "ci0"), ("i1", "ci1"), ("co", "co")] {
            carry
                .pins
                .push((role.to_owned(), WireRef::local(format!("lc{n}_{wire}"))));
        }
        carry.config.push(ConfigEntry::Cell {
            primitive: "SB_CARRY".to_owned(),
            bits: vec![bits.take()],
        });
        tile.bels.push(carry);
    }

    let mut muxes = Muxes::default();
    let mut outputs = Vec::new();
    for n in 0..LCS_PER_TILE {
        // Input `k` of a LUT comes from local group `k`, the way an
        // iCE40 LUT's inputs come from `local_g0` to `local_g3`.
        for i in 0..4 {
            muxes.add_all(
                &group(i, LOCALS_PER_GROUP),
                &WireRef::local(format!("lc{n}_i{i}")),
            );
        }
        // The direct path inside a logic cell comes first, so the router
        // finds it before the general one.
        let d = WireRef::local(format!("lc{n}_d"));
        muxes.add(WireRef::local(format!("lc{n}_out")), d.clone());
        muxes.add_all(&group(0, LOCALS_PER_GROUP), &d);
        for g in 0..GLOBALS {
            muxes.add(
                WireRef::global(format!("glb{g}")),
                WireRef::local(format!("lc{n}_clk")),
            );
        }
        muxes.add_all(
            &group(1, LOCALS_PER_GROUP),
            &WireRef::local(format!("lc{n}_en")),
        );
        muxes.add_all(
            &group(2, LOCALS_PER_GROUP),
            &WireRef::local(format!("lc{n}_rst")),
        );
        let previous = if n == 0 {
            WireRef::at(format!("lc{}_co", LCS_PER_TILE - 1), 0, -1)
        } else {
            WireRef::local(format!("lc{}_co", n - 1))
        };
        let ci = WireRef::local(format!("lc{n}_ci"));
        muxes.add(previous, ci.clone());
        muxes.add_all(&group(3, LOCALS_PER_GROUP), &ci);
        muxes.add_all(
            &group(0, LOCALS_PER_GROUP),
            &WireRef::local(format!("lc{n}_ci0")),
        );
        muxes.add_all(
            &group(1, LOCALS_PER_GROUP),
            &WireRef::local(format!("lc{n}_ci1")),
        );
        for role in ["out", "q", "co"] {
            outputs.push(WireRef::local(format!("lc{n}_{role}")));
        }
    }
    add_track_muxes(&mut muxes, &outputs, LOCALS_PER_GROUP);
    tile.pips = muxes.finish(&mut bits);
    tile
}

/// An IO tile: two IO buffers, and on eight of them a global clock
/// buffer.
fn io_tile(name: &str, global: Option<usize>) -> TileType {
    let mut tile = TileType::new(name, "io_tile", 16, 18);
    let mut bits = BitAllocator::new(16, 18);
    add_tracks(&mut tile, IO_LOCALS_PER_GROUP);
    for n in 0..2 {
        for role in ["din", "dout", "oe"] {
            tile.wires.push(WireDecl {
                name: format!("io{n}_{role}"),
                dx: 0,
                dy: 0,
            });
        }
    }
    if global.is_some() {
        tile.wires.push(WireDecl {
            name: "gb_in".to_owned(),
            dx: 0,
            dy: 0,
        });
    }

    for n in 0..2 {
        let mut io = BelDecl::new(format!("io{n}"), "io");
        // `pad` is off-fabric: it reaches the package, not a wire, which
        // is why it has no pin here and is never routed.
        for role in ["din", "dout", "oe"] {
            io.pins
                .push((role.to_owned(), WireRef::local(format!("io{n}_{role}"))));
        }
        for index in 0..6 {
            io.config.push(ConfigEntry::Param {
                name: "PIN_TYPE".to_owned(),
                index,
                at: bits.take(),
            });
        }
        io.config.push(ConfigEntry::Param {
            name: "PULLUP".to_owned(),
            index: 0,
            at: bits.take(),
        });
        tile.bels.push(io);
    }
    if let Some(g) = global {
        let mut gb = BelDecl::new("gb", "gb");
        gb.pins.push(("i".to_owned(), WireRef::local("gb_in")));
        gb.pins
            .push(("o".to_owned(), WireRef::global(format!("glb{g}"))));
        gb.config.push(ConfigEntry::Cell {
            primitive: "SB_GB".to_owned(),
            bits: vec![bits.take()],
        });
        tile.bels.push(gb);
    }

    let mut muxes = Muxes::default();
    let mut outputs = Vec::new();
    for n in 0..2 {
        muxes.add_all(
            &group(0, IO_LOCALS_PER_GROUP),
            &WireRef::local(format!("io{n}_dout")),
        );
        muxes.add_all(
            &group(1, IO_LOCALS_PER_GROUP),
            &WireRef::local(format!("io{n}_oe")),
        );
        outputs.push(WireRef::local(format!("io{n}_din")));
    }
    if global.is_some() {
        muxes.add_all(&group(2, IO_LOCALS_PER_GROUP), &WireRef::local("gb_in"));
    }
    add_track_muxes(&mut muxes, &outputs, IO_LOCALS_PER_GROUP);
    tile.pips = muxes.finish(&mut bits);
    tile
}

/// The pin roles of the block RAM bel, inputs first, matching the port
/// order the device database gives `SB_RAM40_4K`: port 0 reads, port 1
/// writes.
fn ram_pins() -> (Vec<String>, Vec<String>) {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for port in 0..2 {
        for role in ["clk", "en", "ce"] {
            inputs.push(format!("p{port}_{role}"));
        }
        for bit in 0..RAM_ADDR {
            inputs.push(format!("p{port}_addr{bit}"));
        }
    }
    for bit in 0..RAM_DATA {
        outputs.push(format!("p0_dout{bit}"));
        inputs.push(format!("p1_din{bit}"));
    }
    (inputs, outputs)
}

/// The lower half of a block RAM, which holds the whole bel here.
fn ramb_tile() -> TileType {
    let mut tile = TileType::new("ramb", "ramb_tile", 16, 42);
    let mut bits = BitAllocator::new(16, 42);
    add_tracks(&mut tile, LOCALS_PER_GROUP);
    let (inputs, outputs) = ram_pins();
    for name in inputs.iter().chain(outputs.iter()) {
        tile.wires.push(WireDecl {
            name: name.clone(),
            dx: 0,
            dy: 0,
        });
    }

    let mut bel = BelDecl::new("ram", "bram");
    for name in inputs.iter().chain(outputs.iter()) {
        bel.pins.push((name.clone(), WireRef::local(name)));
    }
    for (param, width) in [("READ_MODE", 2), ("WRITE_MODE", 2)] {
        for index in 0..width {
            bel.config.push(ConfigEntry::Param {
                name: param.to_owned(),
                index,
                at: bits.take(),
            });
        }
    }
    tile.bels.push(bel);

    let mut muxes = Muxes::default();
    for (index, name) in inputs.iter().enumerate() {
        muxes.add_all(
            &group(index % LOCAL_GROUPS, LOCALS_PER_GROUP),
            &WireRef::local(name),
        );
    }
    let driven: Vec<WireRef> = outputs.iter().map(WireRef::local).collect();
    add_track_muxes(&mut muxes, &driven, LOCALS_PER_GROUP);
    tile.pips = muxes.finish(&mut bits);
    tile
}

/// The upper half of a block RAM: routing only, but it has a bitmap of
/// its own, which is why it is a tile type and not a hole.
fn ramt_tile() -> TileType {
    let mut tile = TileType::new("ramt", "ramt_tile", 16, 42);
    let mut bits = BitAllocator::new(16, 42);
    add_tracks(&mut tile, LOCALS_PER_GROUP);
    let mut muxes = Muxes::default();
    add_track_muxes(&mut muxes, &[], LOCALS_PER_GROUP);
    tile.pips = muxes.finish(&mut bits);
    tile
}

/// Builds the synthetic architecture.
///
/// The flip-flop primitive names come from the device database, so the
/// mode field covers exactly the variants the family declares and nothing
/// has to be repeated here.
///
/// ```
/// let arch = reticle::fpga::arch::synthetic::ice40_like();
/// assert_eq!(arch.name, "ice40-synthetic");
/// assert_eq!((arch.width, arch.height), (14, 18));
/// // Two logic cells in each of 160 logic tiles; see the module docs
/// // for why that is not the 1280 of the real part.
/// let graph = arch.build_graph();
/// assert_eq!(
///     graph.site_counts().iter().find(|(k, _)| k == "lut").unwrap().1,
///     320
/// );
/// ```
pub fn ice40_like() -> Arch {
    let device =
        super::super::target("ice40-hx1k-tq144").expect("the built-in iCE40 device database");
    let ff_primitives: Vec<String> = device
        .ff_variants()
        .map(|(bel, _)| bel.name.clone())
        .collect();

    let mut arch = Arch::new("ice40-synthetic", "ice40", 14, 18);
    arch.asc_device = "1k".to_owned();
    arch.parts = vec!["ice40-hx1k-tq144".to_owned(), "ice40-lp1k-tq144".to_owned()];
    for g in 0..GLOBALS {
        arch.globals.push(format!("glb{g}"));
    }

    arch.tile_types.push(logic_tile(&ff_primitives));
    arch.tile_types.push(io_tile("io", None));
    arch.tile_types.push(ramb_tile());
    arch.tile_types.push(ramt_tile());
    for g in 0..GLOBALS {
        arch.tile_types.push(io_tile(&format!("iogb{g}"), Some(g)));
    }
    let logic = 0;
    let io = 1;
    let ramb = 2;
    let ramt = 3;
    let iogb = |g: usize| 4 + g;

    // The IO ring, with the corners left empty as the 1k dice have them.
    for x in 1..13 {
        arch.set_tile(x, 0, io);
        arch.set_tile(x, 17, io);
    }
    for y in 1..17 {
        arch.set_tile(0, y, io);
        arch.set_tile(13, y, io);
    }
    // Eight global buffers, spread around the ring.
    for (g, (x, y)) in [
        (1, 0),
        (12, 0),
        (1, 17),
        (12, 17),
        (0, 1),
        (0, 16),
        (13, 1),
        (13, 16),
    ]
    .into_iter()
    .enumerate()
    {
        arch.set_tile(x, y, iogb(g));
    }
    // Two RAM columns, each block two tiles tall.
    for x in [3, 10] {
        for y in 1..17 {
            arch.set_tile(x, y, if y % 2 == 1 { ramb } else { ramt });
        }
    }
    // Logic everywhere else inside the ring.
    for x in 1..13 {
        if x == 3 || x == 10 {
            continue;
        }
        for y in 1..17 {
            arch.set_tile(x, y, logic);
        }
    }

    // The package pins the device database records, onto the IO sites of
    // the bottom edge and up the left one, two per tile. Which pin is
    // where is invented; that a pin reaches exactly one IO bel is not.
    let mut sites: Vec<String> = Vec::new();
    for x in 1..13 {
        for n in 0..2 {
            sites.push(format!("X{x}Y0/io{n}"));
        }
    }
    for y in 1..17 {
        for n in 0..2 {
            sites.push(format!("X0Y{y}/io{n}"));
        }
    }
    for (pin, site) in TQ144_PINS.iter().zip(sites) {
        arch.pinmap.push(((*pin).to_owned(), site));
    }
    arch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mux_codes_are_wide_enough_for_their_sources() {
        assert_eq!(code_width(0), 0);
        assert_eq!(code_width(1), 1);
        assert_eq!(code_width(2), 2);
        assert_eq!(code_width(3), 2);
        assert_eq!(code_width(12), 4);
        assert_eq!(code_width(16), 5);
    }

    #[test]
    fn the_grid_matches_the_1k_geometry() {
        let arch = ice40_like();
        assert_eq!(arch.tile_at(0, 0), None, "the corners are empty");
        assert_eq!(arch.tile_at(13, 17), None);
        assert_eq!(arch.tile_at(1, 1).unwrap().name, "logic");
        assert_eq!(arch.tile_at(3, 1).unwrap().name, "ramb");
        assert_eq!(arch.tile_at(3, 2).unwrap().name, "ramt");
        assert_eq!(arch.tile_at(10, 15).unwrap().name, "ramb");
        assert_eq!(arch.tile_at(0, 5).unwrap().name, "io");
        assert_eq!(arch.tile_at(1, 0).unwrap().name, "iogb0");
        let logic = arch.tiles.iter().flatten().filter(|t| **t == 0).count();
        assert_eq!(logic, 160);
    }

    #[test]
    fn every_tile_type_fits_its_bitmap() {
        let arch = ice40_like();
        for tile in &arch.tile_types {
            let mut used: Vec<ConfigBit> = Vec::new();
            for pip in &tile.pips {
                used.extend(pip.bits.iter().copied());
            }
            for bel in &tile.bels {
                for entry in &bel.config {
                    match entry {
                        ConfigEntry::Cell { bits, .. } => used.extend(bits.iter().copied()),
                        ConfigEntry::Param { at, .. } => used.push(*at),
                    }
                }
            }
            for bit in &used {
                assert!(
                    bit.row < tile.bit_rows && bit.col < tile.bit_cols,
                    "{}: {bit:?} is outside {} x {}",
                    tile.name,
                    tile.bit_rows,
                    tile.bit_cols
                );
            }
        }
    }

    /// Every wire is driven by exactly one multiplexer, and every pip of
    /// that multiplexer has a distinct code. A repeated code would make
    /// two pips indistinguishable in the bitstream.
    #[test]
    fn every_multiplexer_has_distinct_codes() {
        let arch = ice40_like();
        for tile in &arch.tile_types {
            let mut by_destination: std::collections::BTreeMap<String, Vec<Vec<ConfigBit>>> =
                std::collections::BTreeMap::new();
            for pip in &tile.pips {
                let mut bits = pip.bits.clone();
                bits.sort();
                by_destination
                    .entry(pip.to.to_text())
                    .or_default()
                    .push(bits);
            }
            for (destination, codes) in by_destination {
                let mut seen = codes.clone();
                seen.sort();
                seen.dedup();
                assert_eq!(
                    seen.len(),
                    codes.len(),
                    "{}: two pips into {destination} share a code",
                    tile.name
                );
            }
        }
    }

    /// A flip-flop's mode field distinguishes every variant the device
    /// declares, which is what makes the bitstream reversible.
    #[test]
    fn flip_flop_modes_are_distinct() {
        let arch = ice40_like();
        let logic = arch.tile_at(1, 1).unwrap();
        let ff = logic.bel("lc0_ff").unwrap();
        assert_eq!(ff.config.len(), 20);
        let mut seen: Vec<Vec<ConfigBit>> = Vec::new();
        for entry in &ff.config {
            let ConfigEntry::Cell { bits, .. } = entry else {
                panic!("a flip-flop mode is a cell entry");
            };
            let mut bits = bits.clone();
            bits.sort();
            assert!(!seen.contains(&bits), "two variants share a code");
            seen.push(bits);
        }
    }

    #[test]
    fn the_carry_chain_reaches_the_tile_above() {
        let arch = ice40_like();
        let logic = arch.tile_at(1, 1).unwrap();
        let top = logic
            .wires
            .iter()
            .find(|w| w.name == format!("lc{}_co", LCS_PER_TILE - 1))
            .unwrap();
        assert_eq!((top.dx, top.dy), (0, 1));
        assert!(
            logic
                .pips
                .iter()
                .any(|p| p.from.name == top.name && p.from.dy == -1)
        );
    }

    #[test]
    fn every_package_pin_maps_to_a_site_that_exists() {
        let arch = ice40_like();
        let graph = arch.build_graph();
        assert_eq!(arch.pinmap.len(), TQ144_PINS.len());
        for (pin, site) in &arch.pinmap {
            let found = graph
                .site(site)
                .unwrap_or_else(|| panic!("pin {pin} names the missing site {site}"));
            assert_eq!(found.kind, "io");
        }
    }
}
