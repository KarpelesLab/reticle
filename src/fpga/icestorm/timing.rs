//! Timing for an iCE40 on Project IceStorm's database: the delays of
//! `timings_up5k.txt`, and a static timing analysis of a configuration
//! image.
//!
//! # The figures
//!
//! IceStorm's timing files are in icetime's format: per `CELL`, one
//! `IOPATH <from> <to> <rise> <fall>` line per arc and `SETUP`/`HOLD` lines
//! per input, each figure `min:typ:max` in picoseconds. [`TimingDb`] keeps
//! one number per arc, the larger of the two `max` figures, as nextpnr's
//! `chipdb.py` (`read_timings`) does, and of an arc stated twice the last
//! line, as that dictionary does.
//!
//! # A routing connection's delay
//!
//! A database multiplexer has no cell name in the timing file. Which cell
//! it is follows from the wires it joins, and [`pip_delay`] is nextpnr's
//! rule for that (`chipdb.py`, `pipdelay`) read line for line: a span-4
//! destination is a `Span4Mux_h4` or `_v4` (`Sp12to4` from a span-12,
//! `IoSpan4Mux` from an IO span), a span-12 one a `Span12Mux_h12` or `_v12`,
//! a local track a `LocalMux`, a logic cell's input an `InMux`, a clock,
//! enable or set/reset a `ClkMux`, `CEMux` or `SRMux`, the carry-in
//! multiplexer `ICE_CARRY_IN_MUX`. icetime, IceStorm's own timer, chooses
//! its cells differently (a span's multiplexer by how far along the span a
//! connection leaves it, an `Odrv4` or `Odrv12` at a logic cell's output, a
//! `CascadeMux` on a third input); `docs/fpga-icestorm.md` says how far
//! apart the two come out.
//!
//! # The analysis
//!
//! [`analyse`] reads a configuration image back through the database —
//! the arcs its bits select, each logic cell's table, carry and flip-flop,
//! the block RAMs in use — and finds, per clock, the longest path from a
//! register's output to a register's input: a flip-flop's clock-to-output,
//! then routing and logic, then its setup, which for a logic cell's data
//! input is the table's delay from that input plus an offset per family —
//! less 50 ps on the UltraPlus and the iCE40 Ultra, 30 ps more on an LP
//! part, 20 more on an HX (nextpnr's `getPortClockingInfo`, kept as
//! [`TimingDb::lut_setup`]) — and 100 ps for any other register input. A path from one clock to another is not timed,
//! and nor is one from or to a pin. The clock's own network is skew, which
//! this does not model; between two registers on one global network it is
//! one `ClkMux` each way and cancels.
//!
//! Nothing here reads a file: the timing file's text and the image come in
//! as values.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::db::TileKind;
use super::image::Image;
use super::{IceFabric, LC_CARRY_ENABLE, LC_DFF_ENABLE, LUT_PERM, PllConfig};
use crate::fpga::Constraints;
use crate::fpga::arch::RoutingGraph;
use crate::fpga::place::{Netlist, PathDomain, PinDelays, PinTiming, PlaceTiming};
use crate::ir::{Design, ModuleId};

/// The delays of an IceStorm timing file, in picoseconds.
#[derive(Clone, Debug, Default)]
pub struct TimingDb {
    /// `(cell, from, to)` to the arc's delay.
    paths: HashMap<(String, String, String), f64>,
    /// What a logic cell's flip-flop needs after its table settles, added
    /// to the table's delay from a data input to give that input's setup:
    /// a family's figure, not the file's ([`TimingDb::lut_setup_for`]).
    /// [`TimingDb::parse`] starts it at the UltraPlus's, -50 ps.
    pub lut_setup: f64,
}

/// Why a timing file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingError(pub String);

impl std::fmt::Display for TimingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TimingError {}

impl TimingDb {
    /// Reads the text of an IceStorm timing file (`timings_up5k.txt`).
    ///
    /// # Errors
    ///
    /// [`TimingError`] naming the line, for an arc outside a `CELL` or a
    /// figure that is not a number.
    pub fn parse(text: &str) -> Result<TimingDb, TimingError> {
        let mut db = TimingDb {
            lut_setup: -50.0,
            ..TimingDb::default()
        };
        let mut cell: Option<String> = None;
        for (n, line) in text.lines().enumerate() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields.first() {
                Some(&"CELL") => cell = fields.get(1).map(|c| (*c).to_owned()),
                Some(&"IOPATH") => {
                    let (Some(cell), [_, from, to, rise, fall, ..]) = (&cell, fields.as_slice())
                    else {
                        return Err(TimingError(format!(
                            "line {}: an IOPATH outside a CELL, or short",
                            n + 1
                        )));
                    };
                    let max = |figure: &str| -> Result<f64, TimingError> {
                        let last = figure.rsplit(':').next().unwrap_or("0");
                        if last == "*" {
                            return Ok(0.0);
                        }
                        last.parse::<f64>().map_err(|_| {
                            TimingError(format!("line {}: `{figure}` is not a figure", n + 1))
                        })
                    };
                    let delay = max(rise)?.max(max(fall)?);
                    db.paths
                        .insert((cell.clone(), (*from).to_owned(), (*to).to_owned()), delay);
                }
                _ => {}
            }
        }
        if db.paths.is_empty() {
            return Err(TimingError("no IOPATH in the timing file".into()));
        }
        Ok(db)
    }

    /// The offset nextpnr adds to a lookup table's delay for its flip-flop's
    /// setup (`getPortClockingInfo`), by the speed word of a device file:
    /// `up` and `u` (UltraPlus, iCE40 Ultra) -50 ps, `lp` +30, `hx` +20.
    /// Another word gets the UltraPlus's.
    #[must_use]
    pub fn lut_setup_for(speed: &str) -> f64 {
        match speed {
            "lp" => 30.0,
            "hx" => 20.0,
            _ => -50.0,
        }
    }

    /// The delay of `cell`'s arc from `from` to `to`, in picoseconds.
    #[must_use]
    pub fn delay(&self, cell: &str, from: &str, to: &str) -> Option<f64> {
        self.paths
            .get(&(cell.to_owned(), from.to_owned(), to.to_owned()))
            .copied()
    }

    /// The same, zero when the file does not state it.
    fn or_zero(&self, cell: &str, from: &str, to: &str) -> f64 {
        self.delay(cell, from, to).unwrap_or(0.0)
    }

    /// How many arcs the file states.
    #[must_use]
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// Whether it states none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// A logic cell's table input, `lutff_<k>/in_<j>`: `(k, j)`.
fn lut_input(name: &str) -> Option<(u32, u32)> {
    let rest = name.strip_prefix("lutff_")?;
    let (k, pin) = rest.split_once('/')?;
    let j = pin.strip_prefix("in_")?;
    Some((k.parse().ok()?, j.parse().ok()?))
}

/// The delay of the multiplexer joining wire `src` to wire `dst` of one
/// tile, in picoseconds: nextpnr's `pipdelay`, see the module
/// documentation. A pair that rule does not name is zero.
#[must_use]
pub fn pip_delay(db: &TimingDb, src: &str, dst: &str) -> f64 {
    let io = |s: &str| s.starts_with("span4_");
    let sp12 = |s: &str| s.starts_with("sp12_") || s.starts_with("span12_");
    if dst.starts_with("sp4_") || dst.starts_with("span4_") {
        if sp12(src) {
            return db.or_zero("Sp12to4", "I", "O");
        }
        if io(src) {
            return db.or_zero("IoSpan4Mux", "I", "O");
        }
        return if dst.starts_with("sp4_h_") {
            db.or_zero("Span4Mux_h4", "I", "O")
        } else {
            db.or_zero("Span4Mux_v4", "I", "O")
        };
    }
    if sp12(dst) {
        return if dst.starts_with("sp12_h_") {
            db.or_zero("Span12Mux_h12", "I", "O")
        } else {
            db.or_zero("Span12Mux_v12", "I", "O")
        };
    }
    if dst == "fabout" || dst == "clk" {
        return 0.0;
    }
    if src.starts_with("glb_netwk_") && dst.starts_with("glb2local_") {
        return 0.0;
    }
    if dst == "carry_in_mux" {
        return db.or_zero("ICE_CARRY_IN_MUX", "carryinitin", "carryinitout");
    }
    match dst {
        "lutff_global/clk" | "io_global/inclk" | "io_global/outclk" | "ram/RCLK" | "ram/WCLK" => {
            return db.or_zero("ClkMux", "I", "O");
        }
        "lutff_global/s_r" | "io_global/latch" | "ram/RE" | "ram/WE" => {
            return db.or_zero("SRMux", "I", "O");
        }
        "lutff_global/cen" | "io_global/cen" | "ram/RCLKE" | "ram/WCLKE" => {
            return db.or_zero("CEMux", "I", "O");
        }
        _ => {}
    }
    if dst.starts_with("local_") {
        return db.or_zero("LocalMux", "I", "O");
    }
    if src.starts_with("local_")
        && matches!(
            dst,
            "io_0/D_OUT_0"
                | "io_0/D_OUT_1"
                | "io_0/OUT_ENB"
                | "io_1/D_OUT_0"
                | "io_1/D_OUT_1"
                | "io_1/OUT_ENB"
        )
    {
        return db.or_zero("IoInMux", "I", "O");
    }
    if lut_input(dst).is_some() {
        return db.or_zero("InMux", "I", "O");
    }
    if let Some(rest) = dst.strip_prefix("ram/")
        && ["MASK_", "RADDR_", "WADDR_", "WDATA_"]
            .iter()
            .any(|p| rest.starts_with(p))
    {
        return db.or_zero("InMux", "I", "O");
    }
    0.0
}

/// One path the analysis found: where it starts and ends, how long it is,
/// and each net on it with the time it settles there.
#[derive(Clone, Debug, PartialEq)]
pub struct CriticalPath {
    /// The clock both of its registers are on: the wire at the root of its
    /// network (`glb_netwk_7`), `!` before it for the falling edge.
    pub clock: String,
    /// From launch to capture, the setup included, in picoseconds.
    pub delay_ps: f64,
    /// The frequency the path allows, in MHz.
    pub fmax_mhz: f64,
    /// The nets along it, as `X<x>Y<y>/<wire>` where the net is driven, and
    /// the time each settles, in picoseconds after the clock.
    pub nets: Vec<(String, f64)>,
}

/// What [`analyse`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimingReport {
    /// Per clock, its longest register-to-register path, slowest first.
    pub clocks: Vec<CriticalPath>,
    /// Registers timed, register inputs timed, and arcs read.
    pub counts: (usize, usize, usize),
}

impl TimingReport {
    /// The slowest clock's path, if any path was timed.
    #[must_use]
    pub fn worst(&self) -> Option<&CriticalPath> {
        self.clocks.first()
    }
}

/// A node of the analysis: a database net.
type Net = u32;

/// The graph [`analyse`] times.
struct Graph {
    /// Per net, the arcs leaving it: `(to, delay)`.
    out: HashMap<Net, Vec<(Net, f64)>>,
    /// Per net, the one arc driving it, `(from, tile)`, for the clock's
    /// root.
    driver: HashMap<Net, Net>,
    /// Registers: `(output net, clock, launch)`.
    starts: Vec<(Net, (Net, bool), f64)>,
    /// Register inputs: `(input net, clock, setup)`.
    ends: Vec<(Net, (Net, bool), f64)>,
}

/// The static timing analysis of `image` on `fabric` with the figures of
/// `db`: per clock, the longest register-to-register path. See the module
/// documentation for what is and is not timed.
#[must_use]
pub fn analyse(fabric: &IceFabric, image: &Image, db: &TimingDb) -> TimingReport {
    let decoded = fabric.decode(image);
    let net = |x: u32, y: u32, name: &str| fabric.net_of(x, y, name);
    let name_in = |n: Net, x: u32, y: u32| -> Option<&str> {
        fabric
            .db
            .nets
            .get(n as usize)?
            .iter()
            .find(|(sx, sy, _)| (*sx, *sy) == (x, y))
            .map(|(_, _, s)| s.as_str())
    };
    let mut g = Graph {
        out: HashMap::new(),
        driver: HashMap::new(),
        starts: Vec::new(),
        ends: Vec::new(),
    };
    for &((x, y), dst, src) in &decoded.arcs {
        let (Some(s), Some(d)) = (name_in(src, x, y), name_in(dst, x, y)) else {
            continue;
        };
        let delay = pip_delay(db, s, d);
        g.out.entry(src).or_default().push((dst, delay));
        g.driver.insert(dst, src);
    }
    // The logic cells.
    let lc_bits: Vec<Vec<super::ConfigBit>> = (0..8)
        .map(|k| {
            fabric
                .db
                .function_bits(TileKind::Logic, &format!("LC_{k}"))
                .map(<[_]>::to_vec)
                .unwrap_or_default()
        })
        .collect();
    let neg_bit = fabric
        .db
        .function_bits(TileKind::Logic, "NegClk")
        .and_then(|b| b.first().copied());
    let clk_to_q = db.or_zero("LogicCell40", "posedge:clk", "lcout");
    let pins = ["in0", "in1", "in2", "in3"];
    for y in 0..fabric.db.height {
        for x in 0..fabric.db.width {
            if fabric.db.tile(x, y) != TileKind::Logic || !image.tiles.contains_key(&(x, y)) {
                continue;
            }
            let clock = net(x, y, "lutff_global/clk").map(|c| {
                let neg = neg_bit.is_some_and(|b| image.get(x, y, b.row, b.col));
                (c, neg)
            });
            let mut tile_dffs = 0usize;
            for (k, bits) in lc_bits.iter().enumerate() {
                if bits.len() != 20 {
                    continue;
                }
                let on = |i: usize| image.get(x, y, bits[i].row, bits[i].col);
                let init: u16 = LUT_PERM
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| on(**p))
                    .fold(0, |a, (i, _)| a | 1 << i);
                let carry = on(LC_CARRY_ENABLE);
                let dff = on(LC_DFF_ENABLE);
                let input = |j: usize| net(x, y, &format!("lutff_{k}/in_{j}"));
                let Some(out) = net(x, y, &format!("lutff_{k}/out")) else {
                    continue;
                };
                let lout = net(x, y, &format!("lutff_{k}/lout"));
                // Which inputs the table reads.
                let reads =
                    |j: usize| (0..16u16).any(|i| (init >> i & 1) != (init >> (i ^ (1 << j)) & 1));
                for (j, pin) in pins.iter().enumerate() {
                    let Some(i) = input(j) else { continue };
                    if !reads(j) {
                        continue;
                    }
                    let lut = db.or_zero("LogicCell40", pin, "lcout");
                    if dff {
                        if let Some(clock) = clock {
                            g.ends.push((i, clock, lut + db.lut_setup));
                        }
                    } else {
                        g.out.entry(i).or_default().push((out, lut));
                    }
                    if let Some(lout) = lout {
                        let to_lt = db.or_zero("LogicCell40", pin, "ltout");
                        g.out.entry(i).or_default().push((lout, to_lt));
                    }
                }
                if dff && let Some(clock) = clock {
                    g.starts.push((out, clock, clk_to_q));
                    tile_dffs += 1;
                }
                if carry && let Some(cout) = net(x, y, &format!("lutff_{k}/cout")) {
                    let cin = if k == 0 {
                        net(x, y, "carry_in_mux")
                    } else {
                        net(x, y, &format!("lutff_{}/cout", k - 1))
                    };
                    for (j, pin) in [(1, "in1"), (2, "in2")] {
                        if let Some(i) = input(j) {
                            let d = db.or_zero("LogicCell40", pin, "carryout");
                            g.out.entry(i).or_default().push((cout, d));
                        }
                    }
                    if let Some(cin) = cin {
                        let d = db.or_zero("LogicCell40", "carryin", "carryout");
                        g.out.entry(cin).or_default().push((cout, d));
                    }
                }
            }
            // A register's enable and set/reset are inputs too.
            if let Some(clock) = clock {
                for wire in ["lutff_global/cen", "lutff_global/s_r"] {
                    if let Some(n) = net(x, y, wire)
                        && g.driver.contains_key(&n)
                        && tile_dffs > 0
                    {
                        g.ends.push((n, clock, 100.0));
                    }
                }
            }
        }
    }
    // The block RAMs in use: their read data registers, every other input
    // a register input on its port's clock.
    for &(x, y) in decoded.rams.keys() {
        let wire = |name: &str| {
            let w = format!("ram/{name}");
            net(x, y, &w).or_else(|| net(x, y + 1, &w))
        };
        let rclk = wire("RCLK").map(|c| (c, false));
        let wclk = wire("WCLK").map(|c| (c, false));
        let launch = db.or_zero("SB_RAM40_4K", "posedge:RCLK", "RDATA[0]");
        for i in 0..16 {
            if let (Some(n), Some(c)) = (wire(&format!("RDATA_{i}")), rclk)
                && g.out.contains_key(&n)
            {
                g.starts.push((n, c, launch));
            }
        }
        for (prefix, count, clock) in [
            ("RADDR_", 11, rclk),
            ("WADDR_", 11, wclk),
            ("WDATA_", 16, wclk),
            ("MASK_", 16, wclk),
        ] {
            for i in 0..count {
                if let (Some(n), Some(c)) = (wire(&format!("{prefix}{i}")), clock)
                    && g.driver.contains_key(&n)
                {
                    g.ends.push((n, c, 100.0));
                }
            }
        }
    }
    // Each clock pin's net, followed back to the root of its network.
    let root = |mut n: Net| {
        let mut seen = BTreeSet::new();
        while let Some(&up) = g.driver.get(&n) {
            if !seen.insert(n) {
                break;
            }
            n = up;
        }
        n
    };
    let clock_of = |c: (Net, bool)| (root(c.0), c.1);
    let starts: Vec<(Net, (Net, bool), f64)> = g
        .starts
        .iter()
        .map(|(n, c, l)| (*n, clock_of(*c), *l))
        .collect();
    let ends: Vec<(Net, (Net, bool), f64)> = g
        .ends
        .iter()
        .map(|(n, c, s)| (*n, clock_of(*c), *s))
        .collect();
    let clocks: BTreeSet<(Net, bool)> = starts.iter().map(|s| s.1).collect();
    // A topological order of every net a register reaches.
    let order = topological(&g.out, starts.iter().map(|s| s.0));
    let mut report = TimingReport {
        counts: (starts.len(), ends.len(), decoded.arcs.len()),
        ..TimingReport::default()
    };
    // A net by the name it has where it is driven: a logic cell's output is
    // `lutff_<k>/out` in its own tile and `neigh_op_*` in eight others.
    let wire_name = |n: Net| -> String {
        let segs = fabric.db.nets.get(n as usize);
        segs.and_then(|segs| {
            segs.iter()
                .find(|(_, _, s)| !s.starts_with("neigh_op_") && !s.starts_with("logic_op_"))
                .or_else(|| segs.first())
        })
        .map_or_else(|| format!("net{n}"), |(x, y, s)| format!("X{x}Y{y}/{s}"))
    };
    for clock in clocks {
        let mut arrival: HashMap<Net, (f64, Option<Net>)> = HashMap::new();
        for (n, c, launch) in &starts {
            if *c == clock {
                let e = arrival.entry(*n).or_insert((f64::NEG_INFINITY, None));
                if *launch > e.0 {
                    *e = (*launch, None);
                }
            }
        }
        for &n in &order {
            let Some(&(t, _)) = arrival.get(&n) else {
                continue;
            };
            for &(to, d) in g.out.get(&n).map_or(&[][..], Vec::as_slice) {
                let e = arrival.entry(to).or_insert((f64::NEG_INFINITY, None));
                if t + d > e.0 {
                    *e = (t + d, Some(n));
                }
            }
        }
        let worst = ends
            .iter()
            .filter(|(_, c, _)| *c == clock)
            .filter_map(|(n, _, setup)| arrival.get(n).map(|(t, _)| (*n, t + setup)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let Some((end, delay)) = worst else { continue };
        let mut nets = Vec::new();
        let mut at = Some(end);
        while let Some(n) = at {
            let (t, from) = arrival.get(&n).copied().unwrap_or((0.0, None));
            nets.push((wire_name(n), t));
            at = from;
            if nets.len() > 10_000 {
                break;
            }
        }
        nets.reverse();
        report.clocks.push(CriticalPath {
            clock: format!("{}{}", if clock.1 { "!" } else { "" }, wire_name(clock.0)),
            delay_ps: delay,
            fmax_mhz: if delay > 0.0 {
                1e6 / delay
            } else {
                f64::INFINITY
            },
            nets,
        });
    }
    report
        .clocks
        .sort_by(|a, b| b.delay_ps.total_cmp(&a.delay_ps));
    report
}

/// The nets reachable from `from` over `out`, each before every net it
/// drives; a net on a loop is left out, with everything only it reaches.
fn topological(out: &HashMap<Net, Vec<(Net, f64)>>, from: impl Iterator<Item = Net>) -> Vec<Net> {
    // The nets reached, and how many arcs from reached nets enter each.
    let mut reached: BTreeSet<Net> = BTreeSet::new();
    let mut stack: Vec<Net> = from.collect();
    while let Some(n) = stack.pop() {
        if reached.insert(n) {
            for (to, _) in out.get(&n).map_or(&[][..], Vec::as_slice) {
                if !reached.contains(to) {
                    stack.push(*to);
                }
            }
        }
    }
    let mut indegree: BTreeMap<Net, usize> = reached.iter().map(|n| (*n, 0)).collect();
    for n in &reached {
        for (to, _) in out.get(n).map_or(&[][..], Vec::as_slice) {
            if let Some(d) = indegree.get_mut(to) {
                *d += 1;
            }
        }
    }
    let mut ready: Vec<Net> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| *n)
        .collect();
    let mut order = Vec::with_capacity(reached.len());
    while let Some(n) = ready.pop() {
        order.push(n);
        for (to, _) in out.get(&n).map_or(&[][..], Vec::as_slice) {
            if let Some(d) = indegree.get_mut(to) {
                *d -= 1;
                if *d == 0 {
                    ready.push(*to);
                }
            }
        }
    }
    order
}

// ---------------------------------------------------------------------------
// What placement and routing are driven by
// ---------------------------------------------------------------------------

/// A connection's delay before it is routed, `a + b * tiles` (and `c` more a
/// tile on a signal of many sinks), in nanoseconds: what a connection costs
/// under [`pip_delay`]'s figures. One to a neighbouring tile's cell is a
/// `LocalMux` and an `InMux`, 1.76 ns; a longer one adds a span, a span-4's
/// multiplexer every four tiles (0.54 ns) or a span-12's every twelve (1.23
/// ns), a little over 0.1 ns a tile. Checked against routed designs by
/// `the_timing_driven_flow_is_faster` (`tests/fpga_icestorm.rs`), which
/// reports the estimate beside the routed figure.
pub const WIRE: (f64, f64, f64) = (1.76, 0.12, 0.05);

/// The period of the clock a pin's `signal` is, and the clock's name: a
/// `create_clock` on the signal at the root of its global buffers, or what
/// a PLL makes of its reference's, or an oscillator's nominal frequency.
fn clock_of(
    design: &Design,
    top: ModuleId,
    netlist: &Netlist,
    constraints: &Constraints,
    signal: usize,
) -> Option<(String, f64)> {
    let mut signal = signal;
    for _ in 0..8 {
        let Some(driver) = netlist.signals[signal].driver else {
            break;
        };
        let inst = &netlist.instances[netlist.pins[driver].instance];
        if inst.kind != "gb" {
            break;
        }
        match inst
            .pins
            .iter()
            .find(|p| !netlist.pins[**p].output && netlist.pins[**p].role == "i")
            .and_then(|p| netlist.pins[*p].signal)
        {
            Some(next) => signal = next,
            None => break,
        }
    }
    let name = &netlist.signals[signal].name;
    if let Some(clock) = constraints.clocks.iter().find(|c| c.net == *name) {
        return Some((clock.name.clone(), clock.period_ns));
    }
    let driver = netlist.signals[signal].driver?;
    let pin = &netlist.pins[driver];
    let inst = &netlist.instances[pin.instance];
    let cell = design.modules.get(top)?.cells.get(inst.cell)?;
    match inst.kind.as_str() {
        "pll" => {
            let config = PllConfig::of_cell(cell, &inst.primitive).ok()?;
            let reference = inst
                .pins
                .iter()
                .find(|p| netlist.pins[**p].role == "ref")
                .and_then(|p| netlist.pins[*p].signal)?;
            let (_, ref_period) = clock_of(design, top, netlist, constraints, reference)?;
            let get = |k: &str| config.params.get(k).copied().unwrap_or(0) as f64;
            let select = if pin.role.ends_with('b') {
                get("PLLOUT_SELECT_B")
            } else {
                get("PLLOUT_SELECT_A")
            };
            let half = if select == 1.0 { 2.0 } else { 1.0 };
            let period =
                ref_period * (get("DIVR") + 1.0) * 2f64.powf(get("DIVQ")) / (get("DIVF") + 1.0);
            Some((name.clone(), period * half))
        }
        "hfosc" => {
            let div = match cell.params.get("CLKHF_DIV") {
                Some(crate::ir::AttrValue::String(s)) => {
                    u32::from_str_radix(s.trim_start_matches("0b"), 2).unwrap_or(0)
                }
                _ => 0,
            };
            Some((
                name.clone(),
                1000.0 / (48.0 / f64::from(1u32 << div.min(3))),
            ))
        }
        "lfosc" => Some((name.clone(), 100_000.0)),
        _ => None,
    }
}

/// The timing a placement of `netlist` on an iCE40UP5K is driven by, from
/// `db`'s figures, and the name of each of its domains, in
/// [`PlaceTiming::periods`] order. `design`'s module `top` is what the
/// netlist was built from, for each logic cell's table, carry and
/// flip-flop.
///
/// A logic cell with its flip-flop on starts paths at its output, a
/// clock-to-output later, and ends them at each table input it reads, with
/// that input's setup — the table's delay less 50 ps, as [`analyse`] has
/// it; its carry inputs pass paths on to its carry out as well. A cell
/// without one passes each input it reads to its output after the table's
/// delay from that input, and its carry operands and carry in to its carry
/// out. A block RAM's read data, a single-port RAM's and a DSP block's
/// outputs start paths and their inputs end them, on their own clocks; a
/// pin, a buffer and the clock generators are not timed.
#[must_use]
pub fn place_timing(
    design: &Design,
    top: ModuleId,
    netlist: &Netlist,
    constraints: &Constraints,
    db: &TimingDb,
) -> (PlaceTiming, Vec<String>) {
    let n = netlist.instances.len();
    let np = netlist.pins.len();
    let mut pins = vec![PinTiming::Ignored; np];
    let mut launch = vec![0.0; n];
    let mut capture = vec![0.0; n];
    let mut domain = vec![PathDomain::Untimed; n];
    let mut pin_domain: Vec<Option<PathDomain>> = vec![None; np];
    let mut reaches = vec![u8::MAX; np];
    let mut through: Vec<Option<[f64; 8]>> = vec![None; np];
    let mut own_capture: Vec<Option<f64>> = vec![None; np];
    let mut names: Vec<String> = Vec::new();
    let mut periods: Vec<f64> = Vec::new();
    let mut domain_of = |clock: Option<(String, f64)>| -> PathDomain {
        let Some((name, period)) = clock else {
            return PathDomain::Untimed;
        };
        if let Some(i) = names.iter().position(|x| *x == name) {
            return PathDomain::Clock(i);
        }
        names.push(name);
        periods.push(period);
        PathDomain::Clock(names.len() - 1)
    };
    let ns = |ps: f64| ps / 1000.0;
    let lut = |j: usize| db.or_zero("LogicCell40", ["in0", "in1", "in2", "in3"][j], "lcout");
    let module = design.modules.get(top);
    for (index, inst) in netlist.instances.iter().enumerate() {
        let pin_of = |role: &str| {
            inst.pins
                .iter()
                .copied()
                .find(|p| netlist.pins[*p].role == role)
        };
        let signal_of = |role: &str| pin_of(role).and_then(|p| netlist.pins[p].signal);
        let clock = |role: &str| {
            signal_of(role).and_then(|s| clock_of(design, top, netlist, constraints, s))
        };
        match inst.kind.as_str() {
            "lc" | "lcc" => {
                let Some(cell) = module.and_then(|m| m.cells.get(inst.cell)) else {
                    continue;
                };
                let param = |key: &str| -> u64 {
                    match cell.params.get(key) {
                        Some(crate::ir::AttrValue::Const(c)) => c.to_u64().unwrap_or(0),
                        Some(other) => other
                            .as_int()
                            .and_then(|v| u64::try_from(v).ok())
                            .unwrap_or(0),
                        None => 0,
                    }
                };
                let init = param("LUT_INIT");
                let dff = param("DFF_ENABLE") != 0;
                let carry = param("CARRY_ENABLE") != 0;
                let reads =
                    |j: usize| (0..16u64).any(|i| (init >> i & 1) != (init >> (i ^ (1 << j)) & 1));
                // The outputs in pin order, for `reaches` and `through`.
                let outputs: Vec<&str> = inst
                    .pins
                    .iter()
                    .filter(|p| netlist.pins[**p].output)
                    .map(|p| netlist.pins[*p].role.as_str())
                    .collect();
                let bit = |role: &str| outputs.iter().position(|r| *r == role);
                if dff {
                    domain[index] = domain_of(clock("clk"));
                    launch[index] = ns(db.or_zero("LogicCell40", "posedge:clk", "lcout"));
                    capture[index] = 0.1;
                }
                for p in inst.pins.iter().copied() {
                    let role = netlist.pins[p].role.as_str();
                    let input = role
                        .strip_prefix('i')
                        .and_then(|j| j.parse::<usize>().ok())
                        .filter(|j| *j < 4);
                    let mut delays = [0.0f64; 8];
                    let mut mask = 0u8;
                    match (role, input) {
                        ("o", _) => {
                            pins[p] = if dff {
                                PinTiming::Start
                            } else {
                                PinTiming::Through
                            };
                        }
                        ("cout", _) => pins[p] = PinTiming::Through,
                        ("cin", _) => {
                            if carry && let Some(b) = bit("cout") {
                                mask |= 1 << b;
                                delays[b] = ns(db.or_zero("LogicCell40", "carryin", "carryout"));
                                pins[p] = PinTiming::Through;
                            }
                        }
                        (_, Some(j)) => {
                            if !dff
                                && reads(j)
                                && let Some(b) = bit("o")
                            {
                                mask |= 1 << b;
                                delays[b] = ns(lut(j));
                            }
                            if carry
                                && (j == 1 || j == 2)
                                && let Some(b) = bit("cout")
                            {
                                mask |= 1 << b;
                                let from = if j == 1 { "in1" } else { "in2" };
                                delays[b] = ns(db.or_zero("LogicCell40", from, "carryout"));
                            }
                            if dff && reads(j) {
                                own_capture[p] = Some(ns(lut(j) + db.lut_setup));
                                pins[p] = if mask != 0 {
                                    PinTiming::Through
                                } else {
                                    PinTiming::End
                                };
                            } else if mask != 0 {
                                pins[p] = PinTiming::Through;
                            }
                        }
                        ("en" | "rst", _) if dff => pins[p] = PinTiming::End,
                        _ => {}
                    }
                    if mask != 0 {
                        reaches[p] = mask;
                        through[p] = Some(delays);
                    }
                }
            }
            "bram" => {
                let read = domain_of(clock("p0_clk"));
                let write = domain_of(clock("p1_clk"));
                domain[index] = read;
                launch[index] = ns(db.or_zero("SB_RAM40_4K", "posedge:RCLK", "RDATA[0]"));
                capture[index] = 0.1;
                for p in inst.pins.iter().copied() {
                    let pin = &netlist.pins[p];
                    if pin.role.ends_with("clk") || pin.signal.is_none() {
                        continue;
                    }
                    pins[p] = if pin.output {
                        PinTiming::Start
                    } else {
                        PinTiming::End
                    };
                    if !pin.role.starts_with("p0_") {
                        pin_domain[p] = Some(write);
                    }
                }
            }
            "spram" | "mac16" => {
                let (clk, from, to) = if inst.kind == "spram" {
                    ("CLOCK0", "posedge:CLOCK", "DATAOUT[0]")
                } else {
                    ("CLK0", "", "")
                };
                domain[index] = domain_of(clock(clk));
                launch[index] = db.delay("SB_SPRAM256KA", from, to).map_or(0.1, ns);
                capture[index] = 0.1;
                for p in inst.pins.iter().copied() {
                    let pin = &netlist.pins[p];
                    if pin.role == clk || pin.signal.is_none() {
                        continue;
                    }
                    pins[p] = if pin.output {
                        PinTiming::Start
                    } else {
                        PinTiming::End
                    };
                }
            }
            _ => {}
        }
    }
    (
        PlaceTiming {
            pins,
            cell: vec![0.0; n],
            launch,
            capture,
            domain,
            cycles: Vec::new(),
            untimed_from: Vec::new(),
            untimed_to: Vec::new(),
            periods,
            wire: WIRE,
            rounds: 3,
            weight: 8.0,
            exponent: 3,
            reaches,
            frozen: Vec::new(),
            pin_domain,
            pin_delays: PinDelays {
                launch: vec![None; np],
                capture: own_capture,
                through,
                per_output: true,
            },
            // A carry's cascade into the next cell is a wire of the chain,
            // free in [`pip_delay`]'s figures but where it crosses into the
            // tile above, through `ICE_CARRY_IN_MUX`.
            local_wire: Some(0.0),
            local: Vec::new(),
            clock: Vec::new(),
            skew: Vec::new(),
            bel_launch: Vec::new(),
            clock_region: Vec::new(),
            region_margin: (0.0, 0.0),
            region: Vec::new(),
        },
        names,
    )
}

/// Each routing node's delay, in picoseconds, for the router: what entering
/// it costs under [`pip_delay`], taken by its own wire's kind (a span-4 is
/// a `Span4Mux` whatever drives it, so a span-12's way down to one is
/// charged a little less than `Sp12to4`).
#[must_use]
pub fn node_delays(graph: &RoutingGraph, db: &TimingDb) -> Vec<u32> {
    graph
        .nodes
        .iter()
        .map(|w| {
            let ps = pip_delay(db, "", &w.name);
            if ps.is_finite() && ps > 0.0 {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a positive figure of a few nanoseconds"
                )]
                let ps = ps.round().min(f64::from(u32::MAX)) as u32;
                ps
            } else {
                0
            }
        })
        .collect()
}

/// The router's timing for a placement driven by `paths`: each pass timed
/// per clock domain from its connections' routed delays (`node_delay`, from
/// [`node_delays`]), with the settings the other families' timing-driven
/// routes use.
#[must_use]
pub fn route_timing(paths: PlaceTiming, node_delay: Vec<u32>) -> crate::fpga::route::Timing {
    crate::fpga::route::Timing {
        node_delay,
        cell_delay: 0,
        combinational: Vec::new(),
        clock_roles: Vec::new(),
        max_criticality: 0.99,
        cleanups: 2,
        // A span-4 every four tiles: what a tile still to go costs at least.
        tile_delay: 100,
        exponent: 3,
        passes: 4,
        reroute_above: 0.9,
        paths: Some(paths),
        present_cap: f64::INFINITY,
        present_cap_run: u32::MAX,
        search_budget: crate::fpga::route::CRITICAL_SEARCH_BUDGET,
        branch: crate::fpga::route::BranchLoad::NONE,
        claim_inputs: f64::INFINITY,
        critical_astar: 0.0,
        seed_from_placement: false,
        delay_weight: 0.0,
        keep_criticality: false,
        refine_rounds: 0,
        refine_above: 1.0,
        against_longest: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "CELL LocalMux\nIOPATH  I  O  408.09:713.51:1099.3  285.171:498.597:768.183\n\n\
        CELL Span12Mux_v12\nIOPATH  I  O  363.839:636.141:980.096  398.257:696.317:1072.81\n\
        IOPATH  I  O  398.257:696.317:1072.81  457.258:799.475:1231.74\n\n\
        CELL InMux\nIOPATH  I  O  245.837:429.825:662.227  186.836:326.667:503.292\n\n\
        CELL Sp12to4\nIOPATH  I  O  221.254:386.843:596.004  314.672:550.176:847.65\n\n\
        CELL Span4Mux_h4\nIOPATH  I  O  191.753:335.264:516.537  201.587:352.457:543.026\n";

    /// The figure kept is the larger `max` of rise and fall, and of an arc
    /// stated twice the last, as nextpnr reads the file.
    #[test]
    fn a_timing_file_reads_as_nextpnr_reads_it() {
        let db = TimingDb::parse(SAMPLE).unwrap();
        assert_eq!(db.delay("LocalMux", "I", "O"), Some(1099.3));
        assert_eq!(db.delay("Span12Mux_v12", "I", "O"), Some(1231.74));
        assert_eq!(db.delay("LocalMux", "O", "I"), None);
        assert!(TimingDb::parse("IOPATH I O 1:2:3 1:2:3\n").is_err());
    }

    /// A connection's cell is its destination's kind of wire, and for a
    /// span-4 its source's too.
    #[test]
    fn a_connection_is_the_cell_its_wires_say() {
        let db = TimingDb::parse(SAMPLE).unwrap();
        assert_eq!(pip_delay(&db, "sp4_h_r_3", "local_g0_3"), 1099.3);
        assert_eq!(pip_delay(&db, "local_g0_3", "lutff_2/in_1"), 662.227);
        assert_eq!(pip_delay(&db, "sp12_v_b_1", "sp4_h_l_2"), 847.65);
        assert_eq!(pip_delay(&db, "sp4_v_b_1", "sp4_h_l_2"), 543.026);
        assert_eq!(pip_delay(&db, "lutff_0/out", "sp12_v_b_0"), 1231.74);
        assert_eq!(pip_delay(&db, "glb_netwk_3", "glb2local_0"), 0.0);
    }
}
