//! Placement: which site each cell of a mapped netlist sits on.
//!
//! Two passes, the usual pair:
//!
//! 1. **Analytic.** The netlist is a weighted graph — every signal a
//!    clique over the pins that touch it, weighted so that a wide net
//!    does not outvote a two-pin one — and the placement that minimises
//!    the sum of squared wire lengths is the solution of two sparse
//!    symmetric systems, one for `x` and one for `y`. They are solved by
//!    a hand-written conjugate-gradient iteration ([`solve`]). Pins tied
//!    to a package pin are the fixed boundary that makes the systems
//!    non-singular; a small pull towards the middle of the die keeps a
//!    design with no fixed pins bounded. The result is a cloud of real
//!    numbers, not a placement: two cells may want the same site.
//! 2. **Legalisation and annealing.** Every cell is assigned the nearest
//!    free site of the kind it needs *and may legally take*, then
//!    simulated annealing improves the result against a half-perimeter
//!    wirelength cost with a move set of swaps, moves to free sites and
//!    relative-placement macro moves.
//!
//! # What the constraints mean here
//!
//! - A **pin constraint** (`set_io`) is hard: the cell lands on the site
//!   the architecture's `pinmap` gives for that package pin, and never
//!   moves. Two cells on one pin is an error, not a warning.
//! - A **region** confines every cell whose name matches the pattern to a
//!   rectangle of tiles, in legalisation and in every annealing move.
//! - **`keep_hierarchy`** is honoured by giving each matched group its
//!   own region: the bounding box its members legalised into, grown by
//!   one tile. The group is then free to improve inside that box and
//!   cannot be scattered across the die. It is a placement constraint,
//!   not a synthesis one; whether the hierarchy survived synthesis is
//!   [`Constraints::keeps_hierarchy`]'s business.
//! - An **`rloc` macro** is rigid: its members keep the exact tile
//!   offsets the constraint states, and the annealer moves the macro, not
//!   its members.
//!
//! # What a legal site is
//!
//! Three things beyond "a site of the right kind that is free":
//!
//! - nothing else in the tile may want a bel pin this cell also wants,
//!   nor a wire out of a pool the tile shares (`SiteRules`);
//! - a cell confined by a region or a macro must stay where it says;
//! - and the fabric has to be able to **carry the cell's signals to and
//!   from the site** (`confine_to_reachable`). That last one only ever
//!   rules anything out on a global wire, whose neighbourhood is not a
//!   function of distance — a clock buffer's — and it is the difference
//!   between a design the router refuses after a whole placement and one
//!   the placer refuses before it starts.
//!
//! # Determinism
//!
//! Everything random here comes from one seeded generator ([`Rng`], an
//! xorshift), whose seed is [`PlaceOptions::seed`]. Every iteration order
//! is over a `Vec` or a `BTreeMap`. Two runs of the same design with the
//! same options produce the same placement, byte for byte, which is what
//! makes the golden tests meaningful.
//!
//! That guarantee has to hold across platforms too, which rules out the
//! two library calls a textbook annealer makes. The acceptance
//! probability of an uphill move is therefore the Cauchy rule
//! `T / (T + delta)` rather than `exp(-delta / T)`, and the moves per
//! temperature are computed with an integer cube root rather than
//! `powf`: both are monotone in exactly the same way, and neither can
//! differ in its last bit between one libm and another.
//!
//! # Failure
//!
//! A design that does not fit is reported, not approximated:
//! [`PlaceError::NoSites`] says which kind of site ran out and by how
//! much, [`PlaceError::PinTaken`] which two cells want one package pin,
//! and [`PlaceError::RegionTooSmall`] which region cannot hold what was
//! assigned to it.
//!
//! [`Constraints::keeps_hierarchy`]: super::Constraints::keeps_hierarchy

use std::cell::Cell;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use super::arch::{Arch, NodeId, RoutingGraph};
use super::constraints::{Constraints, matches_glob};
use super::device::{BelRole, Device};
use crate::ir::emit::{BitView, SigBit};
use crate::ir::{CellId, CellKind, Design, ModuleId};

/// A seeded xorshift generator.
///
/// Reticle has no dependencies, and a placer needs reproducible
/// randomness rather than good randomness: this is `xorshift64*`, which
/// is four lines, passes the tests a simulated annealer cares about, and
/// gives the same sequence on every platform.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator seeded with `seed`; zero is replaced, since xorshift
    /// has a fixed point there.
    pub fn new(seed: u64) -> Self {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0 .. n`, or 0 when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let v = self.next_u64() % (n as u64);
        usize::try_from(v).unwrap_or(0)
    }

    /// A number in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        // 53 bits is exactly what an f64 mantissa holds, so the division
        // is exact and the result is uniform.
        let bits = self.next_u64() >> 11;
        bits as f64 / (1u64 << 53) as f64
    }
}

/// Why a design could not be placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceError {
    /// The module id does not belong to the design.
    NoSuchModule,
    /// The netlist holds something that is not a device primitive, so the
    /// flow was not run to the end.
    NotAPrimitive {
        /// The cell's name.
        cell: String,
        /// What it is instead.
        kind: String,
    },
    /// A primitive the device database does not declare.
    UnknownPrimitive {
        /// The cell's name.
        cell: String,
        /// The primitive it instantiates.
        primitive: String,
    },
    /// The architecture has no site of a kind the design needs, or too
    /// few of them.
    NoSites {
        /// The site kind (`lut`, `ff`, `io`, ...).
        kind: String,
        /// How many the design wants.
        needed: usize,
        /// How many the part has.
        available: usize,
    },
    /// A package pin the architecture cannot place: either it maps to no
    /// site, or the site it maps to is not one this cell can use.
    NoPinSite {
        /// The cell's name.
        cell: String,
        /// The package pin it is constrained to.
        pin: String,
    },
    /// Two cells were constrained to one package pin.
    PinTaken {
        /// The cell that could not be placed.
        cell: String,
        /// The pin both want.
        pin: String,
        /// The cell that got there first.
        by: String,
    },
    /// A region cannot hold what was assigned to it.
    RegionTooSmall {
        /// The region's name.
        region: String,
        /// The site kind that ran out inside it.
        kind: String,
        /// How many cells of that kind were assigned to the region.
        needed: usize,
        /// How many sites of that kind the region holds.
        available: usize,
    },
    /// No site of the kind a cell needs can carry one of its signals: the
    /// fabric has no path between that pin and the other end of the net,
    /// wherever either of them is put.
    ///
    /// This is the global network's error. A clock buffer's input and
    /// output are global wires, and a global wire's neighbourhood is not a
    /// function of distance: on this ECP5 seven of the part's fifty-six
    /// buffer sites cannot be reached from a given top-edge pad at all,
    /// and on the iCE40-like architecture a block RAM's clock pin cannot
    /// be reached from a global buffer at all. Both used to be found by
    /// the router, one signal at a time and after the whole placement was
    /// finished. See `confine_to_reachable`.
    NoReachableSite {
        /// The pin that cannot be connected, as `<cell>.<role>` — the
        /// same spelling the router's own unroutable-sink message uses.
        pin: String,
        /// The kind of site the cell needs.
        kind: String,
        /// How many sites of that kind the part has.
        available: usize,
        /// The pin at the other end of that signal, spelled the same way.
        other: String,
        /// True when the signal arrives on `pin`, false when it leaves.
        inbound: bool,
    },
    /// The netlist could not be read bit by bit, from the emitter's
    /// bit-level view.
    Netlist(String),
}

impl fmt::Display for PlaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaceError::NoSuchModule => f.write_str("the module is not part of the design"),
            PlaceError::NotAPrimitive { cell, kind } => write!(
                f,
                "`{cell}` is a generic `{kind}` cell, not a primitive: run the target flow first"
            ),
            PlaceError::UnknownPrimitive { cell, primitive } => write!(
                f,
                "`{cell}` instantiates `{primitive}`, which the device database does not declare"
            ),
            PlaceError::NoSites {
                kind,
                needed,
                available,
            } => write!(
                f,
                "the design needs {needed} `{kind}` site(s) and the part has {available}"
            ),
            PlaceError::NoPinSite { cell, pin } => write!(
                f,
                "`{cell}` is constrained to package pin `{pin}`, which the architecture maps to no usable site"
            ),
            PlaceError::PinTaken { cell, pin, by } => write!(
                f,
                "`{cell}` and `{by}` are both constrained to package pin `{pin}`"
            ),
            PlaceError::RegionTooSmall {
                region,
                kind,
                needed,
                available,
            } => write!(
                f,
                "region `{region}` holds {available} `{kind}` site(s) and {needed} cell(s) were assigned to it"
            ),
            PlaceError::NoReachableSite {
                pin,
                kind,
                available,
                other,
                inbound,
            } => {
                let (from, to) = if *inbound { (other, pin) } else { (pin, other) };
                write!(
                    f,
                    "none of the part's {available} `{kind}` site(s) has a path from `{from}` to `{to}`"
                )
            }
            PlaceError::Netlist(message) => write!(f, "the netlist cannot be read: {message}"),
        }
    }
}

impl Error for PlaceError {}

// ---------------------------------------------------------------------------
// The netlist the placer and the router share
// ---------------------------------------------------------------------------

/// One cell to place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    /// The cell in the module's arena.
    pub cell: CellId,
    /// The cell's name.
    pub name: String,
    /// The primitive it instantiates.
    pub primitive: String,
    /// The kind of site it needs, which is the device's role keyword:
    /// `lut`, `ff`, `carry`, `io`, `gb`, `bram`.
    pub kind: String,
    /// Its pins, as indices into [`Netlist::pins`].
    pub pins: Vec<usize>,
    /// The package pin it is tied to, when a constraint says so.
    pub pin: Option<String>,
}

/// One pin of one instance, already reduced to a single bit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetPin {
    /// The instance it belongs to.
    pub instance: usize,
    /// The cell port it comes from.
    pub port: String,
    /// Which bit of that port.
    pub bit: u32,
    /// The pin role the architecture knows it by (`i0`, `clk`,
    /// `p0_addr3`).
    pub role: String,
    /// True when the instance drives the pin.
    pub output: bool,
    /// The signal it carries, or `None` for a constant.
    pub signal: Option<usize>,
    /// The constant driving it, when [`NetPin::signal`] is `None` because
    /// the pin is tied rather than wired.
    ///
    /// The router does not route a constant — tying one is a property of
    /// the fabric, not a connection to find — but a fabric that *can* tie
    /// a pin has to know which constant to tie it to. An ECP5's
    /// interconnect can drive a wire to a fixed zero or one
    /// (`CIB.<wire>MUX`), which is how an output pad with a constant
    /// behind it is configured, and before this field the netlist recorded
    /// that a pin was tied without recording to what.
    pub constant: Option<crate::logic::Bit>,
}

/// One bit of one net: what the router has to connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signal {
    /// A name for reports: the net's name, with the bit when it is a
    /// vector.
    pub name: String,
    /// The pin that drives it, if one does.
    pub driver: Option<usize>,
    /// The pins that read it, in netlist order.
    pub sinks: Vec<usize>,
}

impl Signal {
    /// True when the signal has a driver and at least one sink, which is
    /// what makes it worth routing.
    pub fn is_routable(&self) -> bool {
        self.driver.is_some() && !self.sinks.is_empty()
    }
}

/// A mapped netlist, reduced to what placement and routing need.
///
/// Built from the IR's bit-level view ([`BitView`]), so continuous
/// assignments, slices and concatenations are already resolved and every
/// pin is one bit of one signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Netlist {
    /// The cells to place, in arena order.
    pub instances: Vec<Instance>,
    /// Every pin of every instance.
    pub pins: Vec<NetPin>,
    /// Every signal, in the order their driving bits were first seen.
    pub signals: Vec<Signal>,
    /// Pins the architecture has no wire for, by
    /// `instance.port[bit]`. A package pad is the usual one: it reaches
    /// the outside world, not the fabric.
    pub off_fabric: Vec<String>,
}

impl Netlist {
    /// Reads `module` into the form the placer works on.
    ///
    /// `graph` decides which pins are routable: a pin role no bel of the
    /// right kind offers a wire for is recorded in
    /// [`Netlist::off_fabric`] and left alone.
    ///
    /// # Errors
    ///
    /// [`PlaceError::NotAPrimitive`] and [`PlaceError::UnknownPrimitive`]
    /// when the netlist is not one the device could hold, and
    /// [`PlaceError::Netlist`] when a cell connection is not structural.
    pub fn build(
        design: &Design,
        module: ModuleId,
        device: &Device,
        graph: &RoutingGraph,
    ) -> Result<Netlist, PlaceError> {
        let Some(m) = design.modules.get(module) else {
            return Err(PlaceError::NoSuchModule);
        };
        let view = BitView::new(m).map_err(|e| PlaceError::Netlist(e.to_string()))?;
        let mut out = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: Vec::new(),
            off_fabric: Vec::new(),
        };
        let mut by_slot: BTreeMap<usize, usize> = BTreeMap::new();
        let roles = pin_roles(graph);

        for (id, cell) in m.cells.iter() {
            let CellKind::Blackbox(primitive) = &cell.kind else {
                return Err(PlaceError::NotAPrimitive {
                    cell: cell.name.as_str().to_owned(),
                    kind: cell.kind.keyword().to_owned(),
                });
            };
            let primitive = primitive.as_str().to_owned();
            let Some((kind, bases)) = describe(device, &primitive) else {
                return Err(PlaceError::UnknownPrimitive {
                    cell: cell.name.as_str().to_owned(),
                    primitive,
                });
            };
            let instance = out.instances.len();
            let mut pins = Vec::new();
            let mut connections: Vec<(String, bool, Vec<SigBit>)> = Vec::new();
            for (port, expr) in &cell.inputs {
                let bits = view
                    .expr_bits(*expr)
                    .map_err(|e| PlaceError::Netlist(e.to_string()))?;
                connections.push((port.as_str().to_owned(), false, bits));
            }
            for (port, net) in &cell.outputs {
                let bits = view
                    .net_bits(*net, cell.span)
                    .map_err(|e| PlaceError::Netlist(e.to_string()))?;
                connections.push((port.as_str().to_owned(), true, bits));
            }
            for (port, output, bits) in connections {
                let base = bases
                    .iter()
                    .find(|(p, _)| *p == port)
                    .map(|(_, b)| b.clone());
                let width = u32::try_from(bits.len()).unwrap_or(u32::MAX);
                for (index, bit) in bits.iter().enumerate() {
                    let index = u32::try_from(index).unwrap_or(u32::MAX);
                    let role = match &base {
                        Some(base) if width == 1 => base.clone(),
                        Some(base) => format!("{base}{index}"),
                        None => format!("{port}{index}"),
                    };
                    if !roles.contains(&(kind.clone(), role.clone())) {
                        out.off_fabric
                            .push(format!("{}.{port}[{index}]", cell.name));
                        continue;
                    }
                    let mut constant = None;
                    let signal = match view.canonical(*bit) {
                        SigBit::Slot(slot) => {
                            let next = out.signals.len();
                            let index = *by_slot.entry(slot).or_insert(next);
                            if index == next {
                                let (net, netbit) = view.owner(slot);
                                let name = if m.nets[net].ty.width().unwrap_or(1) == 1 {
                                    m.nets[net].name.as_str().to_owned()
                                } else {
                                    format!("{}[{netbit}]", m.nets[net].name)
                                };
                                out.signals.push(Signal {
                                    name,
                                    driver: None,
                                    sinks: Vec::new(),
                                });
                            }
                            Some(index)
                        }
                        SigBit::Const(value) => {
                            constant = Some(value);
                            None
                        }
                    };
                    let pin = out.pins.len();
                    out.pins.push(NetPin {
                        instance,
                        port: port.clone(),
                        bit: index,
                        role,
                        output,
                        signal,
                        constant,
                    });
                    pins.push(pin);
                    if let Some(signal) = signal {
                        if output {
                            out.signals[signal].driver = Some(pin);
                        } else {
                            out.signals[signal].sinks.push(pin);
                        }
                    }
                }
            }
            out.instances.push(Instance {
                cell: id,
                name: cell.name.as_str().to_owned(),
                primitive,
                kind,
                pins,
                pin: cell
                    .attrs
                    .get("pin")
                    .and_then(|v| v.as_str().map(str::to_owned)),
            });
        }
        Ok(out)
    }

    /// How many instances of each site kind, sorted by kind.
    pub fn kind_counts(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for inst in &self.instances {
            *counts.entry(inst.kind.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    /// The signals worth routing, as indices.
    pub fn routable(&self) -> Vec<usize> {
        (0..self.signals.len())
            .filter(|s| self.signals[*s].is_routable())
            .collect()
    }
}

/// Every `(site kind, pin role)` the architecture offers a wire for.
fn pin_roles(graph: &RoutingGraph) -> std::collections::BTreeSet<(String, String)> {
    let mut out = std::collections::BTreeSet::new();
    for site in &graph.sites {
        for (role, _) in &site.pins {
            out.insert((site.kind.clone(), role.clone()));
        }
    }
    out
}

/// The site kind and the port-to-role map of one primitive.
///
/// Roles are the architecture's names for pins. A device port that is one
/// of several under a role (a LUT's `i=I0,I1,I2,I3`) takes that role plus
/// its position; a block RAM's ports take their physical port's index as
/// a prefix, since both ports call their clock `clk`.
///
/// One role is renamed on the way through: an IO buffer's `oen` — the
/// active-low spelling of its enable, see
/// [`BelKind::enable_port`](super::device::BelKind::enable_port) — becomes
/// `oe`. The two are the same *pin*, differing only in which way round
/// its logic is, and the routing graph knows nothing about logic: every
/// architecture declares the wire under `oe` and a fabric that had to
/// declare it twice would be describing metal that does not differ.
/// Whether the pin is inverted is decided once, where the buffer is built.
fn describe(device: &Device, primitive: &str) -> Option<(String, Vec<(String, String)>)> {
    let mut bases: Vec<(String, String)> = Vec::new();
    let mut kind: Option<String> = None;
    for bel in device.bels.iter().filter(|b| b.name == primitive) {
        kind = Some(bel.role.keyword().to_owned());
        for (role, names) in &bel.ports {
            let role = if role == "oen" { "oe" } else { role.as_str() };
            let names: Vec<&str> = names.split(',').collect();
            for (index, name) in names.iter().enumerate() {
                let base = if names.len() == 1 {
                    role.to_owned()
                } else {
                    format!("{role}{index}")
                };
                if !bases.iter().any(|(p, _)| p == name) {
                    bases.push(((*name).to_owned(), base));
                }
            }
        }
    }
    for bram in device.block_rams.iter().filter(|b| b.name == primitive) {
        kind = Some(BelRole::Bram.keyword().to_owned());
        for (index, port) in bram.port_map.iter().enumerate() {
            for (role, name) in &port.signals {
                if !bases.iter().any(|(p, _)| p == name) {
                    bases.push((name.clone(), format!("p{index}_{role}")));
                }
            }
        }
    }
    for dsp in device.dsps.iter().filter(|d| d.name == primitive) {
        kind = Some(BelRole::Dsp.keyword().to_owned());
        for (role, name) in &dsp.ports {
            if !bases.iter().any(|(p, _)| p == name) {
                bases.push((name.clone(), role.clone()));
            }
        }
    }
    kind.map(|kind| (kind, bases))
}

// ---------------------------------------------------------------------------
// The placement itself
// ---------------------------------------------------------------------------

/// Which site every instance sits on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    /// Instance to site index, parallel to [`Netlist::instances`].
    of_instance: Vec<Option<usize>>,
    /// Site index to instance, parallel to [`RoutingGraph::sites`].
    of_site: Vec<Option<usize>>,
}

impl Placement {
    /// An empty placement for a netlist and a graph.
    pub fn new(instances: usize, sites: usize) -> Self {
        Placement {
            of_instance: vec![None; instances],
            of_site: vec![None; sites],
        }
    }

    /// The site of an instance.
    pub fn site_of(&self, instance: usize) -> Option<usize> {
        self.of_instance.get(instance).copied().flatten()
    }

    /// The instance on a site.
    pub fn instance_at(&self, site: usize) -> Option<usize> {
        self.of_site.get(site).copied().flatten()
    }

    /// Puts `instance` on `site`, taking it off whatever it was on.
    ///
    /// The site must be free; putting two instances on one site is a
    /// programming error, which is why this asserts rather than reports.
    pub fn place(&mut self, instance: usize, site: usize) {
        assert!(
            self.of_site[site].is_none(),
            "site {site} already holds an instance"
        );
        if let Some(old) = self.of_instance[instance] {
            self.of_site[old] = None;
        }
        self.of_instance[instance] = Some(site);
        self.of_site[site] = Some(instance);
    }

    /// Takes `instance` off whatever site it is on.
    pub fn unplace(&mut self, instance: usize) {
        if let Some(site) = self.of_instance[instance].take() {
            self.of_site[site] = None;
        }
    }

    /// How many instances are placed.
    pub fn placed(&self) -> usize {
        self.of_instance.iter().filter(|s| s.is_some()).count()
    }

    /// The placement as `instance site` lines, sorted by instance name.
    pub fn to_text(&self, netlist: &Netlist, graph: &RoutingGraph) -> String {
        let mut lines: Vec<String> = Vec::new();
        for (index, instance) in netlist.instances.iter().enumerate() {
            let site = match self.site_of(index) {
                Some(site) => graph.sites[site].name.clone(),
                None => "(unplaced)".to_owned(),
            };
            lines.push(format!("{} {site}\n", instance.name));
        }
        lines.sort();
        lines.concat()
    }
}

/// How much work a placement cost, counted rather than timed.
///
/// A wall-clock number means nothing on another machine and cannot be
/// asserted on; these can be. They are what the annealer actually does,
/// so a change that makes a move cheaper shows up here as a smaller
/// number per move and a change that makes the *schedule* shorter shows
/// up as fewer moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaceWork {
    /// Candidate moves offered to the legality check.
    pub legality_tests: u64,
    /// Shared-pin entries and control-pool slots those checks looked at.
    /// This is the one that was three figures per move before the pools
    /// were visited once each instead of once per pool.
    pub legality_steps: u64,
    /// Pin positions the wirelength cost read. One net of a thousand
    /// sinks read from scratch on every move is what made a big design
    /// grow faster than its move count.
    pub cost_pins: u64,
    /// Times the best placement so far was recorded.
    pub snapshots: u64,
    /// Wires the reachability sweeps took off their queue, which is what
    /// `confine_to_reachable` costs. It is paid once, before
    /// legalisation, and no move pays any of it: a move's share of this
    /// check is one binary search in a list of at most a few dozen sites.
    pub reach_steps: u64,
}

thread_local! {
    /// [`PlaceWork`] for the placement running on this thread, read into
    /// [`PlacementReport::work`] when [`place`] returns.
    ///
    /// A thread-local rather than an argument: the cost function and the
    /// legality check are called from closures several layers inside the
    /// legaliser, and threading a counter through all of them would bury
    /// the code it is there to measure. Nothing reads it but [`place`],
    /// which zeroes it on the way in.
    static WORK: Cell<PlaceWork> = const { Cell::new(PlaceWork {
        legality_tests: 0,
        legality_steps: 0,
        cost_pins: 0,
        snapshots: 0,
        reach_steps: 0,
    }) };
}

/// Adds to one counter of [`WORK`].
fn count(what: fn(&mut PlaceWork) -> &mut u64, by: u64) {
    WORK.with(|cell| {
        let mut work = cell.get();
        *what(&mut work) += by;
        cell.set(work);
    });
}

/// Knobs for [`place`].
#[derive(Clone, Debug)]
pub struct PlaceOptions {
    /// The seed every random choice comes from.
    pub seed: u64,
    /// How many conjugate-gradient iterations the analytic pass runs.
    pub analytic_iterations: u32,
    /// Run the annealing pass. Turning it off leaves the legalised
    /// analytic placement, which is useful for comparing the two.
    pub anneal: bool,
    /// A fixed geometric cooling factor in `(0, 1)`, instead of the
    /// adaptive one.
    ///
    /// `None`, the default, is VPR's acceptance-rate schedule: the step is
    /// chosen per temperature from `cooling_factor`. This is here so the
    /// two can be compared on one design without recompiling, and because
    /// a test that pins what the fixed schedule did needs to ask for it.
    pub cooling: Option<f64>,
    /// How the start temperature is chosen.
    ///
    /// `None` is the paper's rule, `20 x` the standard deviation of the
    /// cost change over a random move sequence, which is picked so that
    /// "initially virtually any move is accepted at the start of the
    /// anneal". That is right when the anneal starts from a *random*
    /// placement, which is what VPR does and what this placer does not:
    /// there is an analytic solve and a legalisation in front of it, and
    /// a temperature hot enough to accept anything throws their answer
    /// away. Measured on `clock_blink.v`, it does exactly that — the
    /// first temperature takes a wirelength of 240 to 5793, and a
    /// hundred and twenty temperatures later the walk is at 353 and has
    /// still never been back under where it started.
    ///
    /// `Some(rate)`, the default, instead solves for the temperature at
    /// which `rate` of the sampled moves would be accepted, so the walk
    /// begins where the schedule is trying to hold it rather than far
    /// above it. See `start_temperature`.
    pub start_acceptance: Option<f64>,
    /// The move window the anneal starts with, in tiles, or `None` for
    /// the whole die.
    ///
    /// `None` is the paper's rule — "initially, `D_limit` is set to the
    /// entire chip" — and it is right for the same reason the hot start
    /// is: VPR anneals a *random* placement, which has nothing worth
    /// keeping and everything worth mixing. Here an analytic solve and a
    /// legaliser have already put every cell roughly where it belongs,
    /// and a die-wide move at any temperature warm enough to accept one
    /// undoes that. The limiter will widen the window on its own if the
    /// acceptance rate asks it to.
    pub start_window: Option<u32>,
    /// The acceptance rate the move window aims to hold, 0.44 by default.
    ///
    /// Not a cooling parameter: it is the setpoint of the range limiter,
    /// which widens the window when more than this fraction of moves is
    /// accepted and narrows it when less is. See [`PlaceOptions::range_limit`].
    pub target_acceptance: f64,
    /// Shrink the move window with the acceptance rate.
    ///
    /// With this off, a move may send a cell to any site of its kind on
    /// the die, which is what this placer did before and is why its
    /// annealer could not improve on legalisation at all: at a
    /// temperature low enough to be selective, a die-wide move is always
    /// a large uphill one and is always rejected, so the walk spends its
    /// whole budget being refused.
    pub range_limit: bool,
    /// Consecutive temperature steps that fail to improve the best
    /// placement, once the acceptance rate has fallen into the quench
    /// band (`QUENCHED`, 0.15), after which the anneal stops. Zero never
    /// stops for this reason.
    ///
    /// This is the exit criterion the fixed schedule did not have. VPR's
    /// own — stop when the temperature is below `0.005 * cost / nets` —
    /// is still there and still fires first on most designs; this one
    /// catches a walk that has converged while the temperature is still
    /// nominally warm.
    pub stall_limit: u32,
    /// Upper bound on temperature steps, so a pathological design stops.
    pub max_temperatures: u32,
    /// Moves tried per temperature, or `None` for
    /// [`PlaceOptions::move_effort`] `* n^(4/3)`.
    pub moves_per_temperature: Option<usize>,
    /// What multiplies `n^(4/3)` to get the moves tried per temperature,
    /// when [`PlaceOptions::moves_per_temperature`] does not say.
    ///
    /// **This is the knob that decides how long a placement takes.** The
    /// schedule is VPR's, and its `inner_num` is exactly this number; 10 is
    /// the high-quality setting and 1 is VPR's own default. On a design of
    /// 4300 cells, 10 is 70.7 million moves and 135 seconds, which is most
    /// of what a bitstream costs. A caller iterating on hardware can trade
    /// that: see `docs/fpga-trellis.md` for what the wirelength does when it
    /// is lowered, measured rather than guessed.
    ///
    /// Zero is treated as one, and the result is clamped to at least 20
    /// moves and at most a million, so a tiny design still gets a walk and
    /// a huge one does not get an unbounded one.
    pub move_effort: usize,
}

impl Default for PlaceOptions {
    fn default() -> Self {
        PlaceOptions {
            seed: 0x5EED_1CE4_0000_0001,
            analytic_iterations: 200,
            anneal: true,
            cooling: None,
            start_acceptance: Some(0.44),
            start_window: Some(1),
            target_acceptance: 0.44,
            range_limit: true,
            stall_limit: 4,
            max_temperatures: 120,
            moves_per_temperature: None,
            move_effort: 10,
        }
    }
}

/// One temperature step of the annealing schedule, as it ran.
///
/// Nothing here is a wall-clock number: a step is described by the work
/// it did and by the two quantities the schedule steers on, so a test
/// may assert on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TemperatureStep {
    /// Moves proposed at this temperature. A proposal whose window held
    /// no site of the right kind is not one.
    pub tried: u64,
    /// How many of those were accepted. `accepted / tried` is the
    /// `R_accept` the range limiter and the cooling factor both read.
    pub accepted: u64,
    /// The half-side of the move window, in tiles, during this step.
    pub range_limit: u32,
    /// The whole placement's half-perimeter wirelength when the step
    /// ended, recounted rather than accumulated.
    pub cost: u64,
}

/// Why the annealer stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnnealStop {
    /// It did not run: no movable cell, or
    /// [`PlaceOptions::anneal`] was off.
    #[default]
    NotRun,
    /// The temperature fell below `0.005 * cost / nets`, VPR's criterion:
    /// below that, an uphill move is unlikely to be accepted at all.
    Cold,
    /// [`PlaceOptions::stall_limit`] steps in a row improved nothing
    /// while the acceptance rate was in the quench band.
    Stalled,
    /// [`PlaceOptions::max_temperatures`] steps ran. This is the
    /// backstop, not an outcome to be pleased about.
    Exhausted,
}

impl AnnealStop {
    /// The reason, as a clause that follows "stopped because".
    fn clause(self) -> &'static str {
        match self {
            AnnealStop::NotRun => "it did not run",
            AnnealStop::Cold => "the temperature fell below 0.005 of the cost per net",
            AnnealStop::Stalled => "the best placement stopped improving",
            AnnealStop::Exhausted => "it ran out of temperature steps",
        }
    }
}

/// What [`place`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlacementReport {
    /// How many instances of each site kind, and how many sites the part
    /// has, sorted by kind.
    pub usage: Vec<(String, usize, usize)>,
    /// Half-perimeter wirelength after legalising the analytic pass.
    pub hpwl_before: u64,
    /// Half-perimeter wirelength after annealing.
    pub hpwl_after: u64,
    /// Instances held at a package pin.
    pub fixed: usize,
    /// Instances confined to a region, including the ones
    /// `keep_hierarchy` confined.
    pub confined: usize,
    /// Relative-placement macros, and how many instances are in them.
    pub macros: (usize, usize),
    /// Temperature steps the annealer ran.
    pub temperatures: u32,
    /// Moves tried and moves accepted.
    pub moves: (u64, u64),
    /// One entry per temperature step, in order.
    ///
    /// This is the schedule as it actually ran rather than as it was
    /// configured, which is the only way to see whether the acceptance
    /// rate went where the range limiter was trying to put it.
    pub schedule: Vec<TemperatureStep>,
    /// Why the annealer stopped.
    pub stop: AnnealStop,
    /// Pins the architecture gives no wire, which are not routed.
    pub off_fabric: usize,
    /// One entry per cell whose site set reachability narrowed, as
    /// `(the cell, sites left, sites of its kind)`, sorted by name.
    ///
    /// Empty for every design on a family whose bels have no global pin,
    /// and empty for a design whose clock buffers can go anywhere. A
    /// non-empty entry is the placer saying which cells it may not place
    /// by distance alone; see `confine_to_reachable`.
    pub reach: Vec<(String, usize, usize)>,
    /// What the pass cost, in work rather than in seconds.
    pub work: PlaceWork,
}

impl PlacementReport {
    /// The report as plain text.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from("placement:\n");
        for (kind, used, available) in &self.usage {
            let _ = writeln!(out, "  {used} of {available} {kind} sites");
        }
        let _ = writeln!(
            out,
            "  wirelength: {} before annealing, {} after",
            self.hpwl_before, self.hpwl_after
        );
        if self.fixed > 0 || self.confined > 0 || self.macros.0 > 0 {
            let _ = writeln!(
                out,
                "  constrained: {} at a package pin, {} in a region, {} macro(s) over {} cell(s)",
                self.fixed, self.confined, self.macros.0, self.macros.1
            );
        }
        let _ = writeln!(
            out,
            "  annealing: {} temperature(s), {} move(s), {} accepted",
            self.temperatures, self.moves.0, self.moves.1
        );
        if let (Some(first), Some(last)) = (self.schedule.first(), self.schedule.last()) {
            let _ = writeln!(
                out,
                "  schedule: acceptance {} then {}, window {} then {} tile(s), stopped because {}",
                percent(first.accepted, first.tried),
                percent(last.accepted, last.tried),
                first.range_limit,
                last.range_limit,
                self.stop.clause(),
            );
        }
        let _ = writeln!(
            out,
            "  work: {} legality test(s) over {} step(s), {} cost pin read(s), {} snapshot(s)",
            self.work.legality_tests,
            self.work.legality_steps,
            self.work.cost_pins,
            self.work.snapshots
        );
        if !self.reach.is_empty() {
            let _ = writeln!(
                out,
                "  reachable sites: {}",
                self.reach
                    .iter()
                    .map(|(cell, left, all)| format!("{cell} {left} of {all}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if self.work.reach_steps > 0 {
            let _ = writeln!(
                out,
                "  reachability: {} wire(s) swept",
                self.work.reach_steps
            );
        }
        if self.off_fabric > 0 {
            let _ = writeln!(out, "  off-fabric pins: {}", self.off_fabric);
        }
        out
    }

    /// The whole schedule, one line per temperature step.
    ///
    /// [`PlacementReport::to_text`] prints the two ends of it; this is
    /// what shows whether the acceptance rate was held near the target
    /// in between, which is the thing the schedule exists to do.
    pub fn schedule_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from("schedule:\n");
        let _ = writeln!(out, "  step   window     tried  accepted  rate   cost");
        for (step, s) in self.schedule.iter().enumerate() {
            let _ = writeln!(
                out,
                "  {step:>4}   {:>6}  {:>8}  {:>8}  {:>4}  {:>6}",
                s.range_limit,
                s.tried,
                s.accepted,
                percent(s.accepted, s.tried),
                s.cost,
            );
        }
        out
    }
}

/// `part / whole` as a whole-number percentage, `"-"` when `whole` is 0.
fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "-".to_owned();
    }
    format!("{}%", part.saturating_mul(100) / whole)
}

/// A rectangle of tiles a cell must stay inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Rect {
    fn holds(self, x: u32, y: u32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

/// The two ways two sites of one tile are not independent, and what each
/// of them means for a placement.
///
/// # A pin that is the same wire
///
/// A fabric does not always give every bel its own pins. An ECP5 slice is
/// two flip-flops, and its `CLK<c>_SLICE`, `CE<c>_SLICE` and `LSR<c>_SLICE`
/// are one wire each for the pair — `src/fpga/trellis/sites.rs` declares
/// both `SLICE<l>.FF0` and `SLICE<l>.FF1` pointing at them, because that is
/// what the silicon is. A wire carries one signal, so two cells whose pins
/// reach it **must want the same signal on it**, and a placement that puts
/// two flip-flops with different clock enables in one slice is not a
/// placement at all: the router is then asked to carry two signals on one
/// node, which is where this was found — 52 oversubscribed nodes, every one
/// of them a `CE`, on a design with several clock enables.
///
/// This is a legality constraint and not a cost, so it belongs to the
/// legaliser and to every move the annealer proposes rather than to the
/// wirelength. Two things are worth knowing about it:
///
/// - **it is read off the architecture, not hard-coded.** Any two pins of
///   any two bels in one tile that resolve to one node are a constraint, on
///   every family, which is how it can be true of an ECP5's control wires
///   without anything here naming one.
/// - **agreeing means the same signal, including no signal at all.** A
///   flip-flop with no enable is not "indifferent" to the enable wire: on
///   this family the mux that ignores it, `SLICE<l>.CEMUX`, is a setting of
///   the *slice* and not of the flip-flop, so a pair that disagrees cannot
///   be configured either way round. Requiring equality is what both halves
///   of the pair can be written to do.
///
/// # A bel that is the other bel
///
/// Sharing a wire is the weaker case. Sometimes two bels are not two
/// pieces of silicon at all: an ECP5 slice in `DPRAM` mode **is** its two
/// lookup tables, and a distributed RAM therefore takes six of a logic
/// tile's eight. Those six are not "shared" — they are gone, and a lookup
/// table placed on one of them would write its truth table into the RAM's
/// contents. [`ArchSite::blocks`](super::arch::ArchSite::blocks) says so,
/// read off the architecture like everything else here, and the rule is
/// the simple one: a site with something on it excludes every site it
/// blocks and every site that blocks it.
///
/// # A wire that is one of a tile's few
///
/// Both of the cases above are about *one* wire or *one* bel. The third
/// is about a **budget**, and it is what a control wire really is.
///
/// An ECP5 logic tile has four slices and exactly two set/reset wires,
/// `LSR0` and `LSR1`: each slice's `MUXLSR<s>` picks one of the two, and
/// nothing else can reach a flip-flop's reset pin. Two wires for four
/// slices is a budget of two distinct reset signals per tile — and a
/// distributed RAM **spends one of them**, because `WRE0_SLICE` and
/// `WRE1_SLICE`, its write enable, are joined to `LSR1` by a
/// `.fixed_conn` and have no mux to choose with. The same shape holds for
/// the clock (`CLK0`/`CLK1`, with a RAM's `WCK<n>_SLICE` fixed to
/// `CLK1`); it does **not** hold for the clock enable, where `CE0`..`CE3`
/// are four wires for four slices and a distributed RAM uses none.
///
/// A placer that does not know this produces placements the router must
/// reject: `oversubscribed node X24Y3/LSR1`, which is where this was
/// found. The rule it needs is the weak one and not the strong one — a
/// flip-flop with **no** reset shares a RAM's tile freely, and so does one
/// whose reset is a signal the tile already carries. `ecppack`'s own
/// output says exactly that: of the 111 distributed RAMs in Great Scott
/// Gadgets' three Cynthion bitstreams, 79 share their tile with a
/// flip-flop; 51 of those tiles' slices drive a reset, and **all 51 take
/// `LSR0`** — not one takes the `LSR1` the RAM spent. On the clock the
/// vendor does the *other* thing: 158 of those slices clock off `CLK1`,
/// the RAM's own write-clock wire, against 8 off `CLK0`, because a write
/// clock and a flip-flop's clock are usually the same net and a wire that
/// already carries a signal is free to the signal it carries.
///
/// So the constraint is not "a RAM sterilises a tile's flip-flops". It is
/// a **system of distinct representatives**: every distinct signal wanting
/// a wire out of one pool needs a wire of its own, and a placement is
/// legal exactly when such an assignment exists. [`SiteRules::pools`]
/// derives the pools from the routing graph rather than naming a wire:
/// a pin's pool is the set of the tile's **own** wires its signal must
/// pass through, found by walking back from the pin while every way in
/// comes from inside the tile. For a flip-flop's `rst` that walk ends at
/// `{LSR0, LSR1}`, for a RAM's `we` at `{LSR1}`, for a lookup table's
/// input at a wire of its own — and on a family whose bels do not share
/// control wires it ends nowhere and costs nothing.
#[derive(Debug, Default)]
struct SiteRules {
    /// Per site, one entry per *other site* it shares a pin with, as
    /// `(the other site, where its role pairs start in
    /// [`SiteRules::pairs`], how many)`. Both directions are recorded, so
    /// placing something on a site only needs that site's own list.
    ///
    /// It is grouped by the other site and not one entry per pin because
    /// the question asked of each group is "is that site occupied at
    /// all", and the answer is usually no: an ECP5 logic tile's
    /// distributed-RAM bel shares pins with all eight of its lookup
    /// tables, so a flat list made a legality check read a hundred and
    /// fifty entries to decide something sixteen looks settle.
    per_site: Vec<Vec<(usize, u32, u32)>>,
    /// The `(this site's role, the other site's role)` pairs that
    /// [`SiteRules::per_site`]'s runs index. Roles are interned indices
    /// into [`SiteRules::roles`].
    pairs: Vec<(u32, u32)>,
    /// Per site, the sites it cannot be occupied at the same time as, in
    /// both directions. Empty for every site of every family whose
    /// architecture declares no exclusion.
    excludes: Vec<Vec<usize>>,
    /// Per instance, the signal on each of its pins, as `(role, signal)`,
    /// for the pins that carry one. A role that is absent carries none.
    per_instance: Vec<Vec<(u32, usize)>>,
    /// Every role name that takes part, so the checks compare integers.
    roles: BTreeMap<String, u32>,
    /// Per site, one entry per pin that draws on a contended pool of the
    /// tile's own wires, as `(role, pool, which wires of it)`. `pool`
    /// indexes [`ControlGroup::pools`] of the site's group; the mask's bit
    /// *n* is that pool's *n*th wire. Empty for every site of every family
    /// whose bels do not share a control wire.
    captive: Vec<Vec<(u32, u32, u32)>>,
    /// Per site, the group it belongs to, or `u32::MAX` for none.
    site_group: Vec<u32>,
    /// One per tile that has a contended pool at all.
    groups: Vec<ControlGroup>,
}

/// The contended control wires of one tile, and the sites that draw on
/// them. See [`SiteRules`]' third section.
#[derive(Debug, Default)]
struct ControlGroup {
    /// Every site of the tile with at least one entry in
    /// [`SiteRules::captive`].
    sites: Vec<usize>,
    /// The pools, each the wires of one *independent* budget: two pins
    /// whose wires overlap are in one pool and two whose wires do not are
    /// in two. An ECP5 logic tile has one pool for `{LSR0, LSR1}`, one for
    /// `{CLK0, CLK1}` and one per slice for its `CE`.
    pools: Vec<Vec<NodeId>>,
}

/// How many wires a pool may hold, and how far the walk that finds one may
/// go, before a pin is treated as unconstrained.
///
/// A control wire is one of two or four. A lookup table's input is one of
/// two dozen interconnect wires, and stopping there costs nothing but the
/// look: a pool nobody else in the tile draws on is dropped anyway.
const MAX_POOL: usize = 8;

/// How many occupied sites of one tile [`SiteRules::control_fits`] keeps on
/// the stack before it falls back to
/// [`SiteRules::control_fits_unbounded`].
///
/// An ECP5 logic tile has seventeen sites in all — eight lookup tables,
/// eight flip-flops and the distributed RAM — so this is never reached
/// today and is a bound on the scratch rather than on what is legal.
const OCCUPANTS: usize = 32;

/// The wires of `tile` that a signal arriving at `pin` must pass through,
/// or `None` when it can also arrive from outside the tile.
///
/// The walk is backwards, and the rule is "expand a wire every way into
/// which is inside this tile". An ECP5 flip-flop's `rst` pin is
/// `LSR<s>_SLICE`, whose one source is `MUXLSR<s>`, whose two sources are
/// `LSR0` and `LSR1` — and those two are fed from the global network and
/// from neighbouring tiles, so the walk stops and the answer is the pair.
/// A distributed RAM's `we` is `WRE<n>_SLICE`, whose one source is `LSR1`,
/// so the answer is that one wire and the RAM has no choice at all.
///
/// The set is always a **cut**: every path from any driver to the pin
/// crosses exactly one of its wires, which is what lets the caller treat
/// it as a budget. Returning `None` is always safe — it says only that
/// this pin adds no constraint.
fn captive_pool(graph: &RoutingGraph, tile: (u32, u32), pin: NodeId) -> Option<Vec<NodeId>> {
    let local = |node: NodeId| {
        let wire = graph.wire(node);
        !wire.global && wire.tile == tile
    };
    if !local(pin) {
        return None;
    }
    let mut set: Vec<NodeId> = vec![pin];
    let mut expanded: Vec<NodeId> = Vec::new();
    for _ in 0..MAX_POOL {
        let next = set.iter().position(|node| {
            let ways = graph.incoming(*node);
            !ways.is_empty() && ways.iter().all(|pip| local(graph.pip(*pip).from))
        });
        let Some(at) = next else { return Some(set) };
        let node = set.remove(at);
        expanded.push(node);
        for pip in graph.incoming(node) {
            let from = graph.pip(*pip).from;
            // A cycle among a tile's own wires would make the walk
            // meaningless; so would a pool wider than a control wire's.
            if expanded.contains(&from) || set.len() >= MAX_POOL {
                return None;
            }
            if !set.contains(&from) {
                set.push(from);
            }
        }
    }
    None
}

impl SiteRules {
    /// Finds every pair of bel pins in one tile that are one wire, every
    /// pair of sites that exclude each other, and every pool of a tile's
    /// own control wires that more than one of its bels draws on.
    ///
    /// Cost is one pass over the sites grouped by tile, and within a tile
    /// the pins are compared pairwise — sixteen sites of five pins for an
    /// ECP5 logic tile, so the square is small and the grouping keeps it
    /// local. The pool walk is bounded by [`MAX_POOL`] and gives up on the
    /// first wire that can be reached from outside the tile, which is
    /// every interconnect wire, so it costs one look at most pins.
    fn find(netlist: &Netlist, graph: &RoutingGraph) -> SiteRules {
        let mut out = SiteRules {
            per_site: vec![Vec::new(); graph.sites.len()],
            pairs: Vec::new(),
            excludes: vec![Vec::new(); graph.sites.len()],
            per_instance: vec![Vec::new(); netlist.instances.len()],
            roles: BTreeMap::new(),
            captive: vec![Vec::new(); graph.sites.len()],
            site_group: vec![u32::MAX; graph.sites.len()],
            groups: Vec::new(),
        };
        let mut exclusions = 0usize;
        for (index, site) in graph.sites.iter().enumerate() {
            for other in &site.blocks {
                if *other == index || *other >= graph.sites.len() {
                    continue;
                }
                if !out.excludes[index].contains(other) {
                    out.excludes[index].push(*other);
                    exclusions += 1;
                }
                if !out.excludes[*other].contains(&index) {
                    out.excludes[*other].push(index);
                }
            }
        }
        let mut by_tile: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
        for (index, site) in graph.sites.iter().enumerate() {
            by_tile.entry(site.tile).or_default().push(index);
        }
        let role_id = |roles: &mut BTreeMap<String, u32>, name: &str| -> u32 {
            if let Some(id) = roles.get(name) {
                return *id;
            }
            let id = u32::try_from(roles.len()).unwrap_or(u32::MAX);
            roles.insert(name.to_owned(), id);
            id
        };
        for (tile, sites) in &by_tile {
            for (i, left) in sites.iter().enumerate() {
                for right in &sites[i + 1..] {
                    let start = out.pairs.len();
                    for (lrole, lnode) in &graph.sites[*left].pins {
                        for (rrole, rnode) in &graph.sites[*right].pins {
                            if lnode != rnode {
                                continue;
                            }
                            let l = role_id(&mut out.roles, lrole);
                            let r = role_id(&mut out.roles, rrole);
                            out.pairs.push((l, r));
                        }
                    }
                    let shared = out.pairs.len() - start;
                    if shared == 0 {
                        continue;
                    }
                    // The other direction wants the pairs the other way
                    // round, which is a second run over the same answers.
                    let flipped = out.pairs.len();
                    for index in start..start + shared {
                        let (l, r) = out.pairs[index];
                        out.pairs.push((r, l));
                    }
                    let (start, shared, flipped) = (
                        u32::try_from(start).unwrap_or(0),
                        u32::try_from(shared).unwrap_or(0),
                        u32::try_from(flipped).unwrap_or(0),
                    );
                    out.per_site[*left].push((*right, start, shared));
                    out.per_site[*right].push((*left, flipped, shared));
                }
            }
            // Every pin of the tile whose signal has to come through the
            // tile's own wires, and then the pools those wires form.
            let mut drawn: Vec<(usize, &str, Vec<NodeId>)> = Vec::new();
            for site in sites {
                for (role, node) in &graph.sites[*site].pins {
                    let Some(pool) = captive_pool(graph, *tile, *node) else {
                        continue;
                    };
                    // A role naming several wires is one connection, and
                    // on an ECP5 both of a RAM's `we` wires come off the
                    // same `LSR1`; recording it once is what it is.
                    if !drawn
                        .iter()
                        .any(|(s, r, p)| *s == *site && *r == role && *p == pool)
                    {
                        drawn.push((*site, role.as_str(), pool));
                    }
                }
            }
            // A wire only one bel can reach is that bel's own business.
            let shared: Vec<bool> = drawn
                .iter()
                .map(|(site, _, pool)| {
                    drawn.iter().any(|(other, _, theirs)| {
                        *other != *site && theirs.iter().any(|n| pool.contains(n))
                    })
                })
                .collect();
            let mut keep = shared.iter();
            drawn.retain(|_| *keep.next().unwrap_or(&false));
            if drawn.is_empty() {
                continue;
            }
            let mut group = ControlGroup::default();
            let mut too_wide = false;
            for (_, _, pool) in &drawn {
                let mut merged = pool.clone();
                group.pools.retain(|other| {
                    if other.iter().any(|n| pool.contains(n)) {
                        for node in other {
                            if !merged.contains(node) {
                                merged.push(*node);
                            }
                        }
                        false
                    } else {
                        true
                    }
                });
                // The mask below is one bit per wire of a pool, so a pool
                // this wide is not a control budget and is left alone.
                too_wide |= merged.len() > 32;
                merged.sort_unstable();
                group.pools.push(merged);
            }
            if too_wide {
                continue;
            }
            let index = u32::try_from(out.groups.len()).unwrap_or(u32::MAX);
            for (site, role, pool) in &drawn {
                let which = group
                    .pools
                    .iter()
                    .position(|p| p.iter().any(|n| pool.contains(n)))
                    .expect("every pool was merged into one of them");
                let mut mask = 0u32;
                for (bit, node) in group.pools[which].iter().enumerate() {
                    if pool.contains(node) {
                        mask |= 1 << bit;
                    }
                }
                let role = role_id(&mut out.roles, role);
                out.captive[*site].push((role, u32::try_from(which).unwrap_or(0), mask));
                if !group.sites.contains(site) {
                    group.sites.push(*site);
                }
                out.site_group[*site] = index;
            }
            out.groups.push(group);
        }
        if out.roles.is_empty() && exclusions == 0 {
            return SiteRules::default();
        }
        for pin in &netlist.pins {
            if let Some(signal) = pin.signal
                && let Some(role) = out.roles.get(pin.role.as_str())
            {
                out.per_instance[pin.instance].push((*role, signal));
            }
        }
        out
    }

    /// Nothing in this architecture shares a pin or excludes a site, so
    /// nothing has to be checked. Every family but the ECP5 is in this
    /// case today, and the checks below then cost one `is_empty`.
    fn trivial(&self) -> bool {
        self.per_site.is_empty()
    }

    /// The signal an instance wants on the wire its `role` pin reaches.
    fn signal(&self, instance: usize, role: u32) -> Option<usize> {
        self.per_instance[instance]
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, s)| *s)
    }

    /// Whether `moving` can be applied: every shared pin of every site it
    /// touches is wanted by one signal only, nothing occupies a site it
    /// excludes, and every control pool of every tile it touches can still
    /// give each of its signals a wire of its own.
    ///
    /// `moving` is the whole move, so a swap is judged after both of its
    /// halves have happened rather than against the placement it is leaving.
    fn allows(&self, placement: &Placement, moving: &[(usize, usize)]) -> bool {
        if self.trivial() {
            return true;
        }
        count(|work| &mut work.legality_tests, 1);
        count(
            |work| &mut work.legality_steps,
            moving
                .iter()
                .map(|(_, site)| {
                    u64::try_from(self.excludes[*site].len() + self.per_site[*site].len())
                        .unwrap_or(0)
                })
                .sum(),
        );
        let after = |site: usize| -> Option<usize> {
            if let Some((instance, _)) = moving.iter().find(|(_, s)| *s == site) {
                return Some(*instance);
            }
            placement
                .instance_at(site)
                .filter(|i| !moving.iter().any(|(j, _)| j == i))
        };
        let pinned = moving.iter().all(|(instance, site)| {
            self.excludes[*site]
                .iter()
                .all(|other| after(*other).is_none())
                && self.per_site[*site].iter().all(|(other, start, len)| {
                    after(*other).is_none_or(|neighbour| {
                        let run = *start as usize..(*start + *len) as usize;
                        self.pairs[run].iter().all(|(mine, theirs)| {
                            self.signal(*instance, *mine) == self.signal(neighbour, *theirs)
                        })
                    })
                })
        });
        if !pinned {
            return false;
        }
        let mut seen: Vec<u32> = Vec::new();
        for (_, site) in moving {
            let group = self.site_group[*site];
            if group == u32::MAX || seen.contains(&group) {
                continue;
            }
            seen.push(group);
            if !self.control_fits(&self.groups[group as usize], &after) {
                return false;
            }
        }
        true
    }

    /// Whether every pool of one tile can give each of the signals that
    /// want it a wire of its own.
    ///
    /// The question is a bipartite matching and it is answered as one:
    /// signals on one side, the pool's wires on the other, an edge where a
    /// pin allows that wire. Two pins wanting the *same* signal share a
    /// wire and cost nothing, which is the whole reason a flip-flop may sit
    /// beside a distributed RAM whose write clock it shares; a pin with no
    /// signal on that role — a flip-flop with no reset — asks for nothing.
    fn control_fits(&self, group: &ControlGroup, after: &impl Fn(usize) -> Option<usize>) -> bool {
        // Who is on each site of the group, worked out **once**. `after`
        // is a scan of the move and a look in the placement, and asking
        // it again for every pool — six of them on an ECP5 logic tile —
        // was most of what a legality check cost. The sites of one tile
        // are few and mostly empty, so what the pools below walk is the
        // occupied handful rather than all of them.
        let mut occupied: [(usize, usize); OCCUPANTS] = [(0, 0); OCCUPANTS];
        let mut taken = 0usize;
        count(
            |work| &mut work.legality_steps,
            u64::try_from(group.sites.len()).unwrap_or(0),
        );
        for site in &group.sites {
            let Some(instance) = after(*site) else {
                continue;
            };
            if taken == OCCUPANTS {
                return self.control_fits_unbounded(group, after);
            }
            occupied[taken] = (*site, instance);
            taken += 1;
        }
        for (pool, wires) in group.pools.iter().enumerate() {
            let pool = u32::try_from(pool).unwrap_or(u32::MAX);
            // Per signal, the wires every pin wanting it would accept.
            let mut wanted: Vec<(usize, u32)> = Vec::new();
            for (site, instance) in &occupied[..taken] {
                for (role, which, mask) in &self.captive[*site] {
                    if *which != pool {
                        continue;
                    }
                    let Some(signal) = self.signal(*instance, *role) else {
                        continue;
                    };
                    match wanted.iter_mut().find(|(s, _)| *s == signal) {
                        Some((_, allowed)) => *allowed &= *mask,
                        None => wanted.push((signal, *mask)),
                    }
                }
            }
            if wanted.len() > wires.len() || !matchable(&wanted) {
                return false;
            }
        }
        true
    }

    /// [`SiteRules::control_fits`] for a tile with more occupied sites
    /// than [`OCCUPANTS`], which no family here has. It asks `after` once
    /// per pool per site, which is what the fast path above exists to
    /// avoid; it is here so that the bound is a performance choice and
    /// never a correctness one.
    fn control_fits_unbounded(
        &self,
        group: &ControlGroup,
        after: &impl Fn(usize) -> Option<usize>,
    ) -> bool {
        for (pool, wires) in group.pools.iter().enumerate() {
            let pool = u32::try_from(pool).unwrap_or(u32::MAX);
            let mut wanted: Vec<(usize, u32)> = Vec::new();
            for site in &group.sites {
                let Some(instance) = after(*site) else {
                    continue;
                };
                for (role, which, mask) in &self.captive[*site] {
                    if *which != pool {
                        continue;
                    }
                    let Some(signal) = self.signal(instance, *role) else {
                        continue;
                    };
                    match wanted.iter_mut().find(|(s, _)| *s == signal) {
                        Some((_, allowed)) => *allowed &= *mask,
                        None => wanted.push((signal, *mask)),
                    }
                }
            }
            if wanted.len() > wires.len() || !matchable(&wanted) {
                return false;
            }
        }
        true
    }
}

/// Whether each signal can be given a wire of its own, as `(signal, the
/// wires it would accept)` with one bit per wire.
///
/// Kuhn's algorithm, which is the textbook one and is exact. The sizes are
/// a tile's: two wires and two signals on an ECP5 logic tile, so the
/// augmenting search never goes more than a step or two deep.
fn matchable(wanted: &[(usize, u32)]) -> bool {
    fn assign(at: usize, wanted: &[(usize, u32)], taken: &mut [usize], seen: &mut u32) -> bool {
        // Only the wires this pin will actually take, not all thirty-two
        // of the mask. A control pool has two wires; walking the whole
        // word was most of what a legality check spent here, and it is
        // asked once per pool per move.
        let mut left = wanted[at].1 & !*seen;
        while left != 0 {
            let wire = left.trailing_zeros();
            let bit = 1u32 << wire;
            left &= !bit;
            *seen |= bit;
            let wire = wire as usize;
            if taken[wire] == usize::MAX || assign(taken[wire], wanted, taken, seen) {
                taken[wire] = at;
                return true;
            }
            // A deeper call may have claimed more wires; honour that.
            left &= !*seen;
        }
        false
    }
    // One signal needs one wire it allows, which is a bit test, and no
    // signals need nothing. Nearly every tile is one of these two.
    match wanted {
        [] => return true,
        [(_, mask)] => return *mask != 0,
        _ => {}
    }
    let mut taken = [usize::MAX; 32];
    for at in 0..wanted.len() {
        let mut seen = 0u32;
        if !assign(at, wanted, &mut taken, &mut seen) {
            return false;
        }
    }
    true
}

/// A rigid group of instances: the anchor and the tile offsets.
struct Macro {
    /// The instance every offset is measured from.
    anchor: usize,
    /// The members, with their offsets from the anchor.
    members: Vec<(usize, i32, i32)>,
}

/// Everything the placer knows about one instance beyond the netlist.
struct Placeable {
    /// The site it must be on, from a package pin constraint.
    fixed: Option<usize>,
    /// The rectangle it must stay inside.
    region: Option<(Rect, String)>,
    /// The macro it belongs to.
    macro_index: Option<usize>,
    /// The sites it may take, when reachability rules some of them out:
    /// sorted, so membership is a binary search. `None` means every site
    /// of its kind, which is what every cell of every family whose bels
    /// have no global pin gets. See [`confine_to_reachable`].
    allowed: Option<Vec<usize>>,
}

impl Placeable {
    /// True when this cell may sit on `site`, as far as reachability is
    /// concerned. Free for a cell that is not reachability-constrained.
    fn may_take(&self, site: usize) -> bool {
        self.allowed
            .as_ref()
            .is_none_or(|sites| sites.binary_search(&site).is_ok())
    }
}

/// Places `netlist` on `graph`.
///
/// The steps are the ones in the module docs: analytic solve, legalise,
/// anneal. `constraints` decides what is fixed and what is confined;
/// `device` is only needed to relate a package pin to the architecture's
/// pin map.
///
/// # Errors
///
/// Every variant of [`PlaceError`] except the netlist ones, which
/// [`Netlist::build`] has already reported.
pub fn place(
    netlist: &Netlist,
    arch: &Arch,
    graph: &RoutingGraph,
    constraints: &Constraints,
    options: &PlaceOptions,
) -> Result<(Placement, PlacementReport), PlaceError> {
    let mut report = PlacementReport {
        off_fabric: netlist.off_fabric.len(),
        ..PlacementReport::default()
    };
    WORK.with(|cell| cell.set(PlaceWork::default()));
    let mut sites_by_kind: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, site) in graph.sites.iter().enumerate() {
        sites_by_kind
            .entry(site.kind.clone())
            .or_default()
            .push(index);
    }
    for (kind, needed) in netlist.kind_counts() {
        let available = sites_by_kind.get(&kind).map_or(0, Vec::len);
        report.usage.push((kind.clone(), needed, available));
        if needed > available {
            return Err(PlaceError::NoSites {
                kind,
                needed,
                available,
            });
        }
    }

    let mut info: Vec<Placeable> = (0..netlist.instances.len())
        .map(|_| Placeable {
            fixed: None,
            region: None,
            macro_index: None,
            allowed: None,
        })
        .collect();
    fix_pins(netlist, arch, graph, &mut info, &mut report)?;
    apply_regions(netlist, graph, constraints, &mut info);
    // After the pin constraints and the regions, because both of them
    // narrow where the *other* end of a global signal can be, and before
    // legalisation, because the answer is a constraint on every site this
    // pass or the annealer may choose.
    confine_to_reachable(netlist, graph, &sites_by_kind, &mut info, &mut report)?;
    let macros = build_macros(netlist, constraints, &mut info);
    report.macros = (
        macros.len(),
        macros.iter().map(|m| m.members.len()).sum::<usize>(),
    );

    let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
    let shared = SiteRules::find(netlist, graph);
    let positions = solve_analytic(netlist, graph, &info, options);
    legalise(
        netlist,
        graph,
        &info,
        &macros,
        &positions,
        &sites_by_kind,
        &shared,
        &mut placement,
    )?;
    confine_hierarchy(netlist, graph, constraints, &placement, &mut info);
    report.confined = info.iter().filter(|i| i.region.is_some()).count();
    report.hpwl_before = hpwl(netlist, graph, &placement);
    report.hpwl_after = report.hpwl_before;

    if options.anneal {
        anneal(
            netlist,
            graph,
            &info,
            &macros,
            &sites_by_kind,
            &shared,
            options,
            &mut placement,
            &mut report,
        );
        report.hpwl_after = hpwl(netlist, graph, &placement);
    }
    report.work = WORK.with(Cell::get);
    Ok((placement, report))
}

/// Ties every pin-constrained instance to its site.
fn fix_pins(
    netlist: &Netlist,
    arch: &Arch,
    graph: &RoutingGraph,
    info: &mut [Placeable],
    report: &mut PlacementReport,
) -> Result<(), PlaceError> {
    let mut taken: BTreeMap<usize, String> = BTreeMap::new();
    for (index, instance) in netlist.instances.iter().enumerate() {
        let Some(pin) = &instance.pin else { continue };
        let Some(site_name) = arch.site_of_pin(pin) else {
            return Err(PlaceError::NoPinSite {
                cell: instance.name.clone(),
                pin: pin.clone(),
            });
        };
        let Some(site) = graph.site_index(site_name) else {
            return Err(PlaceError::NoPinSite {
                cell: instance.name.clone(),
                pin: pin.clone(),
            });
        };
        if graph.sites[site].kind != instance.kind {
            return Err(PlaceError::NoPinSite {
                cell: instance.name.clone(),
                pin: pin.clone(),
            });
        }
        if let Some(by) = taken.get(&site) {
            return Err(PlaceError::PinTaken {
                cell: instance.name.clone(),
                pin: pin.clone(),
                by: by.clone(),
            });
        }
        taken.insert(site, instance.name.clone());
        info[index].fixed = Some(site);
        report.fixed += 1;
    }
    Ok(())
}

/// Applies the region constraints to the instances their patterns match.
fn apply_regions(
    netlist: &Netlist,
    graph: &RoutingGraph,
    constraints: &Constraints,
    info: &mut [Placeable],
) {
    for assignment in &constraints.region_assignments {
        let Some(region) = constraints
            .regions
            .iter()
            .find(|r| r.name == assignment.region)
        else {
            continue;
        };
        let rect = Rect {
            x0: region.x0.min(graph.width.saturating_sub(1)),
            y0: region.y0.min(graph.height.saturating_sub(1)),
            x1: region.x1.min(graph.width.saturating_sub(1)),
            y1: region.y1.min(graph.height.saturating_sub(1)),
        };
        for (index, instance) in netlist.instances.iter().enumerate() {
            if matches_glob(&assignment.pattern, &instance.name) {
                info[index].region = Some((rect, region.name.clone()));
            }
        }
    }
}

/// Gives each `keep_hierarchy` group the box it legalised into, grown by
/// one tile, so that annealing cannot scatter it.
fn confine_hierarchy(
    netlist: &Netlist,
    graph: &RoutingGraph,
    constraints: &Constraints,
    placement: &Placement,
    info: &mut [Placeable],
) {
    for (pattern, _, _) in &constraints.keep_hierarchy {
        let members: Vec<usize> = (0..netlist.instances.len())
            .filter(|i| matches_glob(pattern, &netlist.instances[*i].name))
            .collect();
        if members.len() < 2 {
            continue;
        }
        let mut rect: Option<Rect> = None;
        for member in &members {
            let Some(site) = placement.site_of(*member) else {
                continue;
            };
            let (x, y) = graph.sites[site].tile;
            rect = Some(match rect {
                None => Rect {
                    x0: x,
                    y0: y,
                    x1: x,
                    y1: y,
                },
                Some(r) => Rect {
                    x0: r.x0.min(x),
                    y0: r.y0.min(y),
                    x1: r.x1.max(x),
                    y1: r.y1.max(y),
                },
            });
        }
        let Some(rect) = rect else { continue };
        let grown = Rect {
            x0: rect.x0.saturating_sub(1),
            y0: rect.y0.saturating_sub(1),
            x1: (rect.x1 + 1).min(graph.width.saturating_sub(1)),
            y1: (rect.y1 + 1).min(graph.height.saturating_sub(1)),
        };
        for member in members {
            if info[member].region.is_none() && info[member].fixed.is_none() {
                info[member].region = Some((grown, format!("keep_hierarchy {pattern}")));
            }
        }
    }
}

/// Builds the rigid macros the `rloc` constraints describe.
fn build_macros(
    netlist: &Netlist,
    constraints: &Constraints,
    info: &mut [Placeable],
) -> Vec<Macro> {
    let mut groups: BTreeMap<String, Vec<(usize, i32, i32)>> = BTreeMap::new();
    for rloc in &constraints.rlocs {
        let group = rloc.group.clone().unwrap_or_else(|| rloc.pattern.clone());
        for (index, instance) in netlist.instances.iter().enumerate() {
            if matches_glob(&rloc.pattern, &instance.name) {
                let members = groups.entry(group.clone()).or_default();
                if !members.iter().any(|(i, _, _)| *i == index) {
                    members.push((index, rloc.dx, rloc.dy));
                }
            }
        }
    }
    let mut macros = Vec::new();
    for (_, mut members) in groups {
        if members.len() < 2 {
            continue;
        }
        members.sort_by_key(|(i, dx, dy)| (*dy, *dx, *i));
        let (anchor, ax, ay) = members[0];
        let members: Vec<(usize, i32, i32)> = members
            .iter()
            .map(|(i, dx, dy)| (*i, dx - ax, dy - ay))
            .collect();
        let index = macros.len();
        for (member, _, _) in &members {
            info[*member].macro_index = Some(index);
        }
        macros.push(Macro { anchor, members });
    }
    macros
}

// ---------------------------------------------------------------------------
// Analytic placement
// ---------------------------------------------------------------------------

/// A reusable breadth-first sweep over the routing graph.
///
/// Only [`confine_to_reachable`] uses it, and the reason it is a struct is
/// the stamp: a sweep marks a million wires on a real part, and a design
/// with two clocks asks for several sweeps, so the mark is a generation
/// number rather than a `bool` that would have to be cleared each time.
struct Sweep {
    /// The generation each wire was last reached in.
    stamp: Vec<u32>,
    /// The generation the current sweep is marking with. Starts at 1, so
    /// a zero stamp means "never reached".
    now: u32,
    /// The frontier, as a queue with a read cursor rather than a
    /// `VecDeque`: it is drained once per sweep and never wraps.
    queue: Vec<NodeId>,
}

impl Sweep {
    /// A sweep over a graph of `nodes` wires, with nothing marked.
    fn new(nodes: usize) -> Sweep {
        Sweep {
            stamp: vec![0; nodes],
            now: 0,
            queue: Vec::new(),
        }
    }

    /// Marks every wire reachable from `seeds` — forwards along the pips
    /// when `forward`, backwards against them otherwise — and returns how
    /// many wires of `wanted` it reached, stopping as soon as that count
    /// reaches `enough`.
    ///
    /// `wanted` must be sorted. The early exit is what makes the usual
    /// answer cheap: a sweep that has its answer has no reason to carry
    /// on, and only a sweep whose answer is "no" has to exhaust the
    /// graph. The cost is one visit per wire and one look per pip,
    /// counted into [`PlaceWork::reach_steps`].
    fn mark(
        &mut self,
        graph: &RoutingGraph,
        seeds: &[NodeId],
        forward: bool,
        wanted: &[NodeId],
        enough: usize,
    ) -> usize {
        self.now += 1;
        self.queue.clear();
        let mut found = 0usize;
        for seed in seeds {
            if self.stamp[*seed as usize] != self.now {
                self.stamp[*seed as usize] = self.now;
                self.queue.push(*seed);
                found += usize::from(wanted.binary_search(seed).is_ok());
            }
        }
        let mut head = 0usize;
        let mut steps = 0u64;
        while head < self.queue.len() && found < enough {
            let node = self.queue[head];
            head += 1;
            steps += 1;
            let pips = if forward {
                graph.outgoing(node)
            } else {
                graph.incoming(node)
            };
            for pip in pips {
                let pip = graph.pip(*pip);
                let next = if forward { pip.to } else { pip.from };
                if self.stamp[next as usize] != self.now {
                    self.stamp[next as usize] = self.now;
                    self.queue.push(next);
                    found += usize::from(wanted.binary_search(&next).is_ok());
                }
            }
        }
        count(|work| &mut work.reach_steps, steps);
        found
    }

    /// True when the last [`Sweep::mark`] reached `node`.
    fn reached(&self, node: NodeId) -> bool {
        self.stamp[node as usize] == self.now
    }
}

/// Narrows every cell's site set to the sites the fabric can actually
/// carry its signals to and from, and reports a cell for which that set
/// is empty.
///
/// # Why this exists, and why it is not asked of every cell
///
/// A site's legality was "is it free, and does nothing in the tile want
/// the same bel pin" ([`SiteRules`]). That is enough on general
/// interconnect, where the fabric is uniform and a path exists between
/// any two tiles; it is not enough on a **global** wire, whose
/// neighbourhood is not a function of distance. Two defects came of that,
/// one from each direction:
///
/// - **Nothing can drive it.** An ECP5 clock buffer's input is the global
///   wire `G_CLKI_<name>`, and the part has fifty-six buffers. Measured on
///   an `LFE5U-12F` in caBGA-256, a pad on the top edge (ball A2, site
///   `X4Y0/PIOA`) reaches **49** of them and not the other **7** —
///   `LDCC3`, `LDCC4`, `LDCC8`, `LDCC11` and `LDCC13` at `X3Y25`, and
///   `RDCC11` and `RDCC13` at `X69Y25`. All fourteen `LDCC` sites are in
///   one tile, so to a placer that judges a site by distance they are
///   interchangeable, and five of them cannot be driven.
/// - **It can drive nothing.** On the iCE40-like architecture this crate
///   carries, a block RAM's clock pin is fed from its tile's local tracks
///   and from nothing else, so a global buffer's output cannot reach it
///   from *any* site. A round that put a memory's clock on a buffer made
///   `ram_ice40` stop routing for that reason.
///
/// Both were found by the router, after a whole placement, as an
/// unroutable sink. This asks the question before legalisation instead,
/// and the answer constrains every move.
///
/// # What is asked, and what it costs
///
/// Only a pin the architecture puts on a global wire is asked about, and
/// only against the other end of the signal it carries. **Which end the
/// sweep starts from is decided by which end has one wire**:
///
/// - a counterpart a **package pin fixes** has exactly one, so the sweep
///   runs from there towards the candidate sites — forwards for a pin the
///   signal arrives on, backwards for one it leaves — and one sweep
///   answers the question for every candidate at once, exactly. This is
///   the ECP5 case above: a clock enters the die at a pad, and the pad is
///   `set_io`'d;
/// - a **movable** counterpart has one wire per site of its kind, 24 288
///   for a flip-flop. Seeding with all of them costs thirty times as much
///   and narrows nothing, because the sites of a kind are
///   interchangeable: if one is reachable they all are. So that direction
///   is swept from *this cell's* candidate pins outwards, and answers the
///   question that has actually been wrong — whether a path to that kind
///   of site exists **at all**. This is the iCE40 case above.
///
/// Either way it is one pass over the graph per globally-wired pin per
/// counterpart group, counted in [`PlaceWork::reach_steps`], and a sweep
/// stops as soon as it has its answer. It is paid once, before
/// legalisation; a move pays nothing but a binary search in
/// [`Placeable::allowed`], which
/// `the_reachability_check_does_not_grow_with_the_move_count` asserts by
/// running one design at two move efforts in one process.
///
/// # What it would and would not catch
///
/// Seeding with the union over where a movable counterpart *may* go makes
/// the answer a relaxation: a site is kept when **some** placement of the
/// other end could reach it, so the check never rejects a site that could
/// have worked, and it does not catch a buffer that reaches some sites of
/// a kind but not the ones the rest of the placement wants. It catches
/// exactly the two cases above. A counterpart a package pin fixes — the
/// pad, which is how a clock enters the die — is one seed and the answer
/// is exact; and a kind no site of which is reachable is rejected
/// whatever the relaxation.
fn confine_to_reachable(
    netlist: &Netlist,
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    info: &mut [Placeable],
    report: &mut PlacementReport,
) -> Result<(), PlaceError> {
    // Which kinds have a bel pin on a global wire at all. One pass over
    // the sites, so a family whose bels have none — every one but the
    // ECP5 and the iCE40-like fabric's `gb` — leaves at the first `if`.
    let mut global_pins: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for site in &graph.sites {
        if global_pins.contains_key(site.kind.as_str()) {
            continue;
        }
        let roles: Vec<&str> = site
            .pins
            .iter()
            .filter(|(_, node)| graph.wire(*node).global)
            .map(|(role, _)| role.as_str())
            .collect();
        if !roles.is_empty() {
            global_pins.insert(site.kind.as_str(), roles);
        }
    }
    if global_pins.is_empty() {
        return Ok(());
    }

    let mut sweep = Sweep::new(graph.nodes.len());
    for index in 0..netlist.instances.len() {
        let kind = netlist.instances[index].kind.clone();
        let Some(roles) = global_pins.get(kind.as_str()) else {
            continue;
        };
        let Some(candidates) = sites_by_kind.get(&kind) else {
            continue;
        };
        let mut allowed: Vec<usize> = candidates.clone();
        let mut narrowed = false;
        for pin_index in netlist.instances[index].pins.clone() {
            let role = netlist.pins[pin_index].role.clone();
            let output = netlist.pins[pin_index].output;
            if !roles.contains(&role.as_str()) {
                continue;
            }
            let Some(signal) = netlist.pins[pin_index].signal else {
                continue;
            };
            let others: Vec<usize> = if output {
                netlist.signals[signal].sinks.clone()
            } else {
                netlist.signals[signal].driver.into_iter().collect()
            };
            // One sweep per *group* of counterparts that share a seed set:
            // every flip-flop's `clk` asks the same question of the same
            // wires, and a design has sixteen of them.
            let mut asked: Vec<String> = Vec::new();
            for other in others {
                let op = &netlist.pins[other];
                if op.instance == index {
                    continue;
                }
                let key = match info[op.instance].fixed {
                    Some(site) => format!("={site}:{}", op.role),
                    None => format!("~{}:{}", netlist.instances[op.instance].kind, op.role),
                };
                if asked.contains(&key) {
                    continue;
                }
                asked.push(key);
                let theirs = seed_wires(netlist, graph, sites_by_kind, info, op);
                if theirs.is_empty() {
                    continue;
                }
                let mut ours: Vec<NodeId> = allowed
                    .iter()
                    .filter_map(|site| graph.sites[*site].pin(&role))
                    .collect();
                ours.sort_unstable();
                ours.dedup();
                let fail = || PlaceError::NoReachableSite {
                    pin: format!("{}.{role}", netlist.instances[index].name),
                    kind: kind.clone(),
                    available: candidates.len(),
                    other: format!("{}.{}", netlist.instances[op.instance].name, op.role),
                    inbound: !output,
                };
                // Which end to sweep from is decided by which end has one
                // wire. A counterpart a package pin fixes has exactly one,
                // so the sweep starts there and the answer is exact for
                // every candidate site at once. A movable counterpart has
                // one per site of its kind — twenty-four thousand for a
                // flip-flop — and seeding with all of them answers the
                // same question at thirty times the cost and narrows
                // nothing, because the sites of a kind are
                // interchangeable: if one is reachable they all are. So
                // that direction is swept from *our* candidate pins
                // instead, and the question it answers is the one that has
                // been wrong — whether a path to that kind exists at all.
                if info[op.instance].fixed.is_some() {
                    sweep.mark(graph, &theirs, !output, &ours, ours.len());
                    let before = allowed.len();
                    allowed.retain(|site| {
                        graph.sites[*site]
                            .pin(&role)
                            .is_some_and(|node| sweep.reached(node))
                    });
                    narrowed |= allowed.len() != before;
                    if allowed.is_empty() {
                        return Err(fail());
                    }
                    // A cell a package pin fixes has one site rather than a
                    // choice of them, so a constraint that asks for an
                    // unreachable one is refused here too — with the
                    // counterpart that ruled it out named, which is what a
                    // later check could not have said.
                    if info[index]
                        .fixed
                        .is_some_and(|site| allowed.binary_search(&site).is_err())
                    {
                        return Err(fail());
                    }
                } else if sweep.mark(graph, &ours, output, &theirs, 1) == 0 {
                    return Err(fail());
                }
            }
        }
        // A cell a package pin fixes keeps the site the pin names; the
        // loop above has already refused one that cannot be connected.
        if !narrowed || info[index].fixed.is_some() {
            continue;
        }
        report.reach.push((
            netlist.instances[index].name.clone(),
            allowed.len(),
            candidates.len(),
        ));
        info[index].allowed = Some(allowed);
    }
    report.reach.sort();
    Ok(())
}

/// Every wire the other end of a signal could be on: the one its package
/// pin fixes it to, or that role's wire on every site of its kind inside
/// its region.
///
/// Sorted and deduplicated, so the seeds do not depend on the order the
/// netlist happened to give the pins.
fn seed_wires(
    netlist: &Netlist,
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    info: &[Placeable],
    pin: &NetPin,
) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = Vec::new();
    if let Some(site) = info[pin.instance].fixed {
        out.extend(graph.sites[site].pin(&pin.role));
    } else if let Some(sites) = sites_by_kind.get(&netlist.instances[pin.instance].kind) {
        let region = info[pin.instance].region.as_ref().map(|(rect, _)| rect);
        for site in sites {
            let (x, y) = graph.sites[*site].tile;
            if region.is_some_and(|rect| !rect.holds(x, y)) {
                continue;
            }
            out.extend(graph.sites[*site].pin(&pin.role));
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// A sparse symmetric matrix in the shape a conjugate gradient wants.
#[derive(Clone, Debug, Default)]
pub struct Sparse {
    /// The diagonal.
    diagonal: Vec<f64>,
    /// Off-diagonal entries per row, as `(column, value)`.
    rows: Vec<Vec<(usize, f64)>>,
}

impl Sparse {
    /// An `n` by `n` zero matrix.
    pub fn new(n: usize) -> Self {
        Sparse {
            diagonal: vec![0.0; n],
            rows: vec![Vec::new(); n],
        }
    }

    /// Adds `value` to entry `(i, j)`.
    pub fn add(&mut self, i: usize, j: usize, value: f64) {
        if i == j {
            self.diagonal[i] += value;
            return;
        }
        match self.rows[i].iter_mut().find(|(c, _)| *c == j) {
            Some(slot) => slot.1 += value,
            None => self.rows[i].push((j, value)),
        }
    }

    /// The order of the matrix.
    pub fn len(&self) -> usize {
        self.diagonal.len()
    }

    /// True when the matrix is empty.
    pub fn is_empty(&self) -> bool {
        self.diagonal.is_empty()
    }

    /// `out = self * v`.
    pub fn multiply(&self, v: &[f64], out: &mut [f64]) {
        for i in 0..self.diagonal.len() {
            let mut sum = self.diagonal[i] * v[i];
            for (j, value) in &self.rows[i] {
                sum += value * v[*j];
            }
            out[i] = sum;
        }
    }
}

/// Solves `a x = b` by conjugate gradient, from `x = 0`.
///
/// `a` must be symmetric and positive definite, which the net model plus
/// the pull towards the middle of the die makes it. The iteration stops
/// when the residual is small or after `iterations` steps, whichever
/// comes first; an analytic placement is a hint, so an approximate
/// solution is a fine one.
///
/// ```
/// use reticle::fpga::place::{Sparse, solve};
/// let mut a = Sparse::new(2);
/// a.add(0, 0, 2.0);
/// a.add(1, 1, 2.0);
/// a.add(0, 1, -1.0);
/// a.add(1, 0, -1.0);
/// let x = solve(&a, &[1.0, 1.0], 50);
/// assert!((x[0] - 1.0).abs() < 1e-9 && (x[1] - 1.0).abs() < 1e-9);
/// ```
pub fn solve(a: &Sparse, b: &[f64], iterations: u32) -> Vec<f64> {
    let n = a.len();
    let mut x = vec![0.0; n];
    if n == 0 {
        return x;
    }
    let mut r = b.to_vec();
    let mut p = r.clone();
    let mut ap = vec![0.0; n];
    let mut rs = dot(&r, &r);
    let tolerance = 1e-12 * rs.max(1.0);
    for _ in 0..iterations {
        if rs <= tolerance {
            break;
        }
        a.multiply(&p, &mut ap);
        let denominator = dot(&p, &ap);
        if denominator.abs() < f64::MIN_POSITIVE {
            break;
        }
        let alpha = rs / denominator;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        let next = dot(&r, &r);
        let beta = next / rs;
        for i in 0..n {
            p[i] = r[i] + beta * p[i];
        }
        rs = next;
    }
    x
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The analytic pass: builds the two systems and solves them.
fn solve_analytic(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    options: &PlaceOptions,
) -> Vec<(f64, f64)> {
    let n = netlist.instances.len();
    let mut a = Sparse::new(n);
    let mut bx = vec![0.0; n];
    let mut by = vec![0.0; n];
    // A weak pull to the middle keeps a design with no fixed pin from
    // collapsing onto one point and the matrix from being singular.
    let centre = (f64::from(graph.width) / 2.0, f64::from(graph.height) / 2.0);
    for i in 0..n {
        a.add(i, i, ANCHOR_WEIGHT);
        bx[i] += ANCHOR_WEIGHT * centre.0;
        by[i] += ANCHOR_WEIGHT * centre.1;
    }
    for i in 0..n {
        if let Some(site) = info[i].fixed {
            let (x, y) = graph.sites[site].tile;
            a.add(i, i, FIXED_WEIGHT);
            bx[i] += FIXED_WEIGHT * f64::from(x);
            by[i] += FIXED_WEIGHT * f64::from(y);
        }
    }
    for signal in &netlist.signals {
        let mut members: Vec<usize> = Vec::new();
        for pin in signal.driver.iter().chain(signal.sinks.iter()) {
            let instance = netlist.pins[*pin].instance;
            if !members.contains(&instance) {
                members.push(instance);
            }
        }
        if members.len() < 2 {
            continue;
        }
        // The clique model: every pair, weighted so that the total weight
        // of a k-pin net does not grow with k^2.
        let weight = 2.0 / (members.len() as f64 - 1.0) / members.len() as f64;
        for (index, i) in members.iter().enumerate() {
            for j in &members[index + 1..] {
                a.add(*i, *i, weight);
                a.add(*j, *j, weight);
                a.add(*i, *j, -weight);
                a.add(*j, *i, -weight);
            }
        }
    }
    let x = solve(&a, &bx, options.analytic_iterations);
    let y = solve(&a, &by, options.analytic_iterations);
    x.into_iter().zip(y).collect()
}

/// The pull of the die's middle on every instance.
const ANCHOR_WEIGHT: f64 = 1e-3;
/// The pull of a package pin on the instance tied to it, which has to
/// dominate every net the instance is on.
const FIXED_WEIGHT: f64 = 1e3;

// ---------------------------------------------------------------------------
// Legalisation
// ---------------------------------------------------------------------------

/// Rounds an analytic coordinate onto the grid.
///
/// The solver works in tile coordinates but has no idea the grid is
/// finite, so a strongly pulled instance can land outside it. Clamping
/// first makes the conversion exact: after the clamp the value is in
/// `0 ..= limit`, which every grid this crate can hold fits in a `u32`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped into the grid first, so the value is a small non-negative integer"
)]
fn to_grid(value: f64, size: u32) -> u32 {
    let limit = f64::from(size.saturating_sub(1));
    let clamped = value.round().clamp(0.0, limit);
    clamped as u32
}

/// Assigns every instance a site.
#[allow(clippy::too_many_arguments, reason = "the legaliser's whole state")]
fn legalise(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    macros: &[Macro],
    positions: &[(f64, f64)],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    shared: &SiteRules,
    placement: &mut Placement,
) -> Result<(), PlaceError> {
    // Fixed instances first: their sites are not negotiable.
    for (index, place) in info.iter().enumerate() {
        if let Some(site) = place.fixed {
            placement.place(index, site);
        }
    }
    // Then the macros as units, then everything else. Within each group
    // the order is by target position, which keeps the result stable.
    let mut order: Vec<usize> = (0..netlist.instances.len())
        .filter(|i| info[*i].fixed.is_none() && info[*i].macro_index.is_none())
        .collect();
    order.sort_by_key(|i| {
        let (x, y) = positions[*i];
        (
            to_grid(y, graph.height),
            to_grid(x, graph.width),
            netlist.instances[*i].name.clone(),
        )
    });

    for m in macros {
        legalise_macro(
            netlist,
            graph,
            info,
            m,
            positions,
            sites_by_kind,
            shared,
            placement,
        )?;
    }
    for index in order {
        let (x, y) = positions[index];
        let kind = &netlist.instances[index].kind;
        let target = (to_grid(x, graph.width), to_grid(y, graph.height));
        let region = info[index].region.as_ref().map(|(rect, _)| rect);
        let site = nearest_free(
            graph,
            sites_by_kind,
            kind,
            region,
            info[index].allowed.as_deref(),
            target,
            shared,
            index,
            placement,
        );
        match site {
            Some(site) => placement.place(index, site),
            None => return Err(no_room(netlist, graph, sites_by_kind, index, info)),
        }
    }
    Ok(())
}

/// The error that fits an instance that found no site.
fn no_room(
    netlist: &Netlist,
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    index: usize,
    info: &[Placeable],
) -> PlaceError {
    let kind = netlist.instances[index].kind.clone();
    let empty = Vec::new();
    let sites = sites_by_kind.get(&kind).unwrap_or(&empty);
    match &info[index].region {
        Some((rect, name)) => {
            let available = sites
                .iter()
                .filter(|s| {
                    let (x, y) = graph.sites[**s].tile;
                    rect.holds(x, y)
                })
                .count();
            let needed = netlist
                .instances
                .iter()
                .enumerate()
                .filter(|(i, inst)| {
                    inst.kind == kind
                        && info[*i]
                            .region
                            .as_ref()
                            .is_some_and(|(_, other)| other == name)
                })
                .count();
            PlaceError::RegionTooSmall {
                region: name.clone(),
                kind,
                needed,
                available,
            }
        }
        None => {
            let needed = netlist
                .instances
                .iter()
                .filter(|inst| inst.kind == kind)
                .count();
            // What ran out is what this cell could have taken, so a cell
            // reachability confined counts its own sites and not the
            // part's. See `confine_to_reachable`.
            let available = info[index]
                .allowed
                .as_ref()
                .map_or_else(|| sites.len(), Vec::len);
            PlaceError::NoSites {
                kind,
                needed,
                available,
            }
        }
    }
}

/// The free site of `kind` closest to `target`, inside `region` and among
/// `allowed`, that `instance` may legally take.
///
/// `allowed` is [`Placeable::allowed`]: the sites reachability left, or
/// `None` for every site of the kind.
#[allow(clippy::too_many_arguments, reason = "the legaliser's whole state")]
fn nearest_free(
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    kind: &str,
    region: Option<&Rect>,
    allowed: Option<&[usize]>,
    target: (u32, u32),
    shared: &SiteRules,
    instance: usize,
    placement: &Placement,
) -> Option<usize> {
    let sites = sites_by_kind.get(kind)?;
    let mut best: Option<(u64, usize)> = None;
    for site in sites {
        if placement.instance_at(*site).is_some() {
            continue;
        }
        if allowed.is_some_and(|ok| ok.binary_search(site).is_err()) {
            continue;
        }
        let (x, y) = graph.sites[*site].tile;
        if let Some(rect) = region
            && !rect.holds(x, y)
        {
            continue;
        }
        let distance = manhattan(target, (x, y));
        if best.is_none_or(|(d, _)| distance < d) && shared.allows(placement, &[(instance, *site)])
        {
            best = Some((distance, *site));
        }
    }
    best.map(|(_, site)| site)
}

/// Places a rigid macro: the anchor goes to the tile closest to its
/// target from which every member finds a site.
#[allow(clippy::too_many_arguments, reason = "the legaliser's whole state")]
fn legalise_macro(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    m: &Macro,
    positions: &[(f64, f64)],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    shared: &SiteRules,
    placement: &mut Placement,
) -> Result<(), PlaceError> {
    let (tx, ty) = positions[m.anchor];
    let target = (to_grid(tx, graph.width), to_grid(ty, graph.height));
    let mut candidates: Vec<(u64, u32, u32)> = Vec::new();
    for y in 0..graph.height {
        for x in 0..graph.width {
            candidates.push((manhattan(target, (x, y)), x, y));
        }
    }
    candidates.sort();
    for (_, ax, ay) in candidates {
        if let Some(sites) = macro_sites(
            netlist,
            graph,
            info,
            m,
            sites_by_kind,
            shared,
            placement,
            ax,
            ay,
        ) {
            for (member, site) in sites {
                placement.place(member, site);
            }
            return Ok(());
        }
    }
    Err(no_room(netlist, graph, sites_by_kind, m.anchor, info))
}

/// The sites a macro anchored at `(ax, ay)` would take, or `None` when
/// one of its members finds none.
#[allow(clippy::too_many_arguments, reason = "the legaliser's whole state")]
fn macro_sites(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    m: &Macro,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    shared: &SiteRules,
    placement: &Placement,
    ax: u32,
    ay: u32,
) -> Option<Vec<(usize, usize)>> {
    let mut taken: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for (member, dx, dy) in &m.members {
        let x = u32::try_from(i64::from(ax) + i64::from(*dx)).ok()?;
        let y = u32::try_from(i64::from(ay) + i64::from(*dy)).ok()?;
        if x >= graph.width || y >= graph.height {
            return None;
        }
        if let Some((rect, _)) = &info[*member].region
            && !rect.holds(x, y)
        {
            return None;
        }
        let kind = &netlist.instances[*member].kind;
        let sites = sites_by_kind.get(kind)?;
        let site = sites.iter().find(|s| {
            graph.sites[**s].tile == (x, y)
                && placement.instance_at(**s).is_none()
                && !taken.contains(*s)
                && info[*member].may_take(**s)
                && shared.allows(placement, &[(*member, **s)])
        })?;
        taken.push(*site);
        out.push((*member, *site));
    }
    // A macro's members can share a pin with each other as well as with
    // whatever is placed already, and `placement` does not hold them yet,
    // so the group is judged once more as a whole.
    if !shared.allows(placement, &out) {
        return None;
    }
    Some(out)
}

/// The Manhattan distance between two tiles.
fn manhattan(a: (u32, u32), b: (u32, u32)) -> u64 {
    let dx = u64::from(a.0.abs_diff(b.0));
    let dy = u64::from(a.1.abs_diff(b.1));
    dx + dy
}

// ---------------------------------------------------------------------------
// Simulated annealing
// ---------------------------------------------------------------------------

/// The annealer's wirelength bookkeeping: every signal's bounding box,
/// kept up to date as cells move rather than recomputed from its pins.
///
/// This is the structure the placer did not have, and not having it is
/// what made a big design cost more than its move count. The cost of a
/// move is the change in half-perimeter wirelength of the signals it
/// touches, and that was worked out by reading **every pin of every one of
/// those signals, twice, for every move**. A design with a thousand
/// flip-flops has a clock net with a thousand sinks, a quarter of the
/// moves touch it, and a move of one cell cannot change where the other
/// nine hundred and ninety-nine pins are. `usb_host_target.v` read
/// 24 billion pin positions to place 4304 cells.
///
/// What is kept instead, per signal, is **how many of its pins are in each
/// tile column and in each tile row**, with the lowest and highest
/// occupied index of each. Moving one pin is two decrements, two
/// increments, and a walk of as many columns and rows as the box really
/// shrank by — nothing in the net's fan-out at all.
///
/// The numbers are **exactly** the ones [`signal_hpwl`] produces, because a
/// bounding box is decided by which columns and rows hold a pin and by
/// nothing else. So the same moves are accepted in the same order and the
/// placement does not change.
///
/// The counts cost `signals * (width + height)` `u32`s: 2 MiB for that
/// design on this die.
struct Spans {
    /// Tile columns of the die.
    width: usize,
    /// Tile rows of the die.
    height: usize,
    /// Per signal, how many of its placed pins are in each column, at
    /// `signal * width + x`.
    columns: Vec<u32>,
    /// Per signal, the same by row, at `signal * height + y`.
    rows: Vec<u32>,
    /// Per signal, `(x0, x1, y0, y1)`: the lowest and highest occupied
    /// column and row. Meaningless while `placed` is zero.
    extent: Vec<(u32, u32, u32, u32)>,
    /// Per signal, how many of its pins are on a placed instance.
    placed: Vec<u32>,
}

impl Spans {
    /// The bounding boxes of `placement`, counted from scratch.
    fn new(netlist: &Netlist, graph: &RoutingGraph, placement: &Placement) -> Spans {
        let signals = netlist.signals.len();
        let width = (graph.width as usize).max(1);
        let height = (graph.height as usize).max(1);
        let mut out = Spans {
            width,
            height,
            columns: vec![0; signals * width],
            rows: vec![0; signals * height],
            extent: vec![(0, 0, 0, 0); signals],
            placed: vec![0; signals],
        };
        for (signal, s) in netlist.signals.iter().enumerate() {
            for pin in s.driver.iter().chain(s.sinks.iter()) {
                let Some(site) = placement.site_of(netlist.pins[*pin].instance) else {
                    continue;
                };
                let (x, y) = graph.sites[site].tile;
                out.add(signal, x, y);
            }
        }
        out
    }

    /// Where in `columns` and `rows` a position of `signal` lives.
    fn slots(&self, signal: usize, x: u32, y: u32) -> (usize, usize) {
        let x = (x as usize).min(self.width - 1);
        let y = (y as usize).min(self.height - 1);
        (signal * self.width + x, signal * self.height + y)
    }

    /// Records one more pin of `signal` at `(x, y)`, growing its box.
    fn add(&mut self, signal: usize, x: u32, y: u32) {
        let (column, row) = self.slots(signal, x, y);
        self.columns[column] += 1;
        self.rows[row] += 1;
        if self.placed[signal] == 0 {
            self.extent[signal] = (x, x, y, y);
        } else {
            let (x0, x1, y0, y1) = self.extent[signal];
            self.extent[signal] = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        }
        self.placed[signal] += 1;
    }

    /// Takes one pin of `signal` at `(x, y)` away again.
    ///
    /// The box is left alone: it may now be wider than the pins that are
    /// left, and [`Spans::settle`] is what narrows it once a whole batch of
    /// removals and additions is in.
    fn remove(&mut self, signal: usize, x: u32, y: u32) {
        let (column, row) = self.slots(signal, x, y);
        self.columns[column] = self.columns[column].saturating_sub(1);
        self.rows[row] = self.rows[row].saturating_sub(1);
        self.placed[signal] = self.placed[signal].saturating_sub(1);
    }

    /// Narrows `signal`'s box onto the columns and rows that still hold a
    /// pin.
    ///
    /// The invariant [`Spans::add`] keeps is that no column below `x0` and
    /// none above `x1` holds a pin, so this walk only crosses what the box
    /// actually lost and never the width of the die.
    fn settle(&mut self, signal: usize) {
        if self.placed[signal] == 0 {
            self.extent[signal] = (0, 0, 0, 0);
            return;
        }
        let (mut x0, mut x1, mut y0, mut y1) = self.extent[signal];
        let base = signal * self.width;
        while x0 < x1 && self.columns[base + x0 as usize] == 0 {
            x0 += 1;
        }
        while x1 > x0 && self.columns[base + x1 as usize] == 0 {
            x1 -= 1;
        }
        let base = signal * self.height;
        while y0 < y1 && self.rows[base + y0 as usize] == 0 {
            y0 += 1;
        }
        while y1 > y0 && self.rows[base + y1 as usize] == 0 {
            y1 -= 1;
        }
        self.extent[signal] = (x0, x1, y0, y1);
    }

    /// `signal`'s half-perimeter wirelength, in tiles.
    fn hpwl_of(&self, signal: usize) -> u64 {
        if self.placed[signal] == 0 {
            return 0;
        }
        let (x0, x1, y0, y1) = self.extent[signal];
        u64::from(x1 - x0) + u64::from(y1 - y0)
    }

    /// Every signal's, summed. This is [`hpwl`] by another route and has to
    /// agree with it.
    fn total(&self) -> u64 {
        (0..self.placed.len()).map(|s| self.hpwl_of(s)).sum()
    }
}

/// The buffers one annealing move reuses, so that a move allocates
/// nothing.
///
/// `carried` is the part that is not scratch: every instance's pins that
/// carry a signal, worked out once. The annealer used to ask
/// [`signals_of`] for that on every move, which allocated a `Vec` and
/// deduplicated it by scanning, for an answer that is a property of the
/// netlist and never changes.
struct Churn {
    /// The bounding boxes; see [`Spans`].
    spans: Spans,
    /// Per instance, `(signal, pin)` for each of its pins that carries
    /// one.
    carried: Vec<Vec<(usize, usize)>>,
    /// The signals the move in hand touches, without repeats.
    touched: Vec<usize>,
    /// Where each instance of the move in hand came from, for [`undo`].
    previous: Vec<(usize, Option<usize>)>,
}

impl Churn {
    /// The buffers for annealing `placement`.
    fn new(netlist: &Netlist, graph: &RoutingGraph, placement: &Placement) -> Churn {
        Churn {
            spans: Spans::new(netlist, graph, placement),
            carried: netlist
                .instances
                .iter()
                .map(|inst| {
                    inst.pins
                        .iter()
                        .filter_map(|pin| netlist.pins[*pin].signal.map(|signal| (signal, *pin)))
                        .collect()
                })
                .collect(),
            touched: Vec::new(),
            previous: Vec::new(),
        }
    }
}

/// The half-perimeter wirelength of one signal, in tiles.
fn signal_hpwl(
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    signal: usize,
) -> u64 {
    let s = &netlist.signals[signal];
    count(
        |work| &mut work.cost_pins,
        (1 + s.sinks.len()).try_into().unwrap_or(u64::MAX),
    );
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for pin in s.driver.iter().chain(s.sinks.iter()) {
        let Some(site) = placement.site_of(netlist.pins[*pin].instance) else {
            continue;
        };
        let (x, y) = graph.sites[site].tile;
        bounds = Some(match bounds {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }
    match bounds {
        Some((x0, y0, x1, y1)) => u64::from(x1 - x0) + u64::from(y1 - y0),
        None => 0,
    }
}

/// The half-perimeter wirelength of the whole placement, in tiles.
pub fn hpwl(netlist: &Netlist, graph: &RoutingGraph, placement: &Placement) -> u64 {
    (0..netlist.signals.len())
        .map(|s| signal_hpwl(netlist, graph, placement, s))
        .sum()
}

/// One candidate move: which instances go where.
type Move = Vec<(usize, usize)>;

/// Where the sites of one kind are, so that a move can be drawn from a
/// window of tiles rather than from the whole die.
///
/// The move generator used to pick uniformly from every site of the kind,
/// wherever it was. That is fine while the temperature is hot enough to
/// accept anything and useless afterwards: a cell sent to a random site
/// on a 46-by-56 die changes the wirelength by tens of tiles, so once the
/// temperature is small compared with that, every single move is refused
/// and the remaining budget buys nothing. Betz and Rose's answer is a
/// *range limit*: only sites within `D_limit` tiles in x and y are
/// offered, and `D_limit` is driven by the acceptance rate, so the
/// window shrinks exactly as fast as the walk becomes selective.
///
/// Drawing from a window needs the sites indexed by position, which is
/// what this is: the columns that hold a site of the kind, and within
/// each, its sites ordered by row. Both searches are binary, so a draw
/// costs two `partition_point`s and two random numbers however big the
/// die is.
struct KindSites {
    /// The tile columns holding at least one site of this kind,
    /// ascending.
    columns: Vec<u32>,
    /// Parallel to [`KindSites::columns`]: that column's sites as
    /// `(tile y, site index)`, ascending by `y`.
    rows: Vec<Vec<(u32, usize)>>,
}

impl KindSites {
    /// Indexes `sites`, which are all of one kind.
    fn build(graph: &RoutingGraph, sites: &[usize]) -> KindSites {
        let mut by_column: BTreeMap<u32, Vec<(u32, usize)>> = BTreeMap::new();
        for site in sites {
            let (x, y) = graph.sites[*site].tile;
            by_column.entry(x).or_default().push((y, *site));
        }
        let mut columns = Vec::with_capacity(by_column.len());
        let mut rows = Vec::with_capacity(by_column.len());
        for (x, mut column) in by_column {
            column.sort_unstable();
            columns.push(x);
            rows.push(column);
        }
        KindSites { columns, rows }
    }

    /// A site of this kind inside the tile rectangle, or `None` when the
    /// rectangle holds none.
    ///
    /// A column is drawn first and a row inside it second, which is not
    /// quite uniform over the sites of the rectangle — a sparse column is
    /// over-weighted. That is deliberate and matches VPR, which draws a
    /// *location* in the window and then asks what is there: what the
    /// window has to do is bound the distance a cell moves, and no part
    /// of the schedule reads the shape of the distribution inside it.
    fn pick(&self, rng: &mut Rng, rect: Rect) -> Option<usize> {
        let first = self.columns.partition_point(|c| *c < rect.x0);
        let last = self.columns.partition_point(|c| *c <= rect.x1);
        if first == last {
            return None;
        }
        let column = &self.rows[first + rng.below(last - first)];
        let low = column.partition_point(|(y, _)| *y < rect.y0);
        let high = column.partition_point(|(y, _)| *y <= rect.y1);
        if low == high {
            return None;
        }
        Some(column[low + rng.below(high - low)].1)
    }
}

/// The acceptance rate below which the walk is quenching rather than
/// searching.
///
/// It is the bottom band of [`cooling_factor`]'s table, and the paper's
/// reason for that band is the reason this constant exists: "if very few
/// moves are being accepted ... there is also little improvement in
/// cost". A stall above this rate is not a stall, it is a walk that has
/// not finished exploring — which is why
/// [`PlaceOptions::stall_limit`] is not allowed to fire until the rate
/// is here.
const QUENCHED: f64 = 0.15;

/// The range limit as a whole number of tiles, at least one.
///
/// Same shape as [`to_grid`] and for the same reason: the clamp comes
/// before the conversion, so what is converted is a small non-negative
/// integer and the cast cannot lose anything. `round` and `clamp` are
/// exactly specified by IEEE 754, unlike `exp` and `powf`, so the window
/// is the same integer on every platform — which it has to be, because
/// the placement is a golden.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped into 1 ..= u32::MAX first, so the value is a non-negative integer"
)]
fn window_tiles(window: f64) -> u32 {
    window.round().clamp(1.0, f64::from(u32::MAX)) as u32
}

/// The move window for the next temperature.
///
/// `D_limit_new = D_limit_old * (1 - 0.44 + R_accept_old)`, clamped to
/// `1 ..= max FPGA dimension` — Betz and Rose again, and the whole
/// mechanism is in the sign of the bracket: above the target rate the
/// window grows, below it the window shrinks, and at the target it
/// holds. Nothing else in the schedule is a feedback loop; this is the
/// one that keeps the acceptance rate near 0.44 "for as long as
/// possible", which is what the cooling table alone cannot do.
fn next_window(window: f64, rate: f64, target: f64, widest: f64) -> f64 {
    (window * (1.0 - target + rate)).clamp(1.0, widest)
}

/// The cooling factor for an acceptance rate, from VPR's table.
///
/// Betz and Rose, *VPR: a new packing, placement and routing tool for
/// FPGA research* (FPL 1997), Table 1. The shape of it is the whole
/// point: cool fast while almost everything is accepted, because nothing
/// is being learned there; cool slowly in the band where some moves are
/// accepted and some are not, because that is where the search happens;
/// cool fast again once almost nothing is accepted, because the walk is
/// finished and only the budget is left.
///
/// | `R_accept` | factor |
/// |---|---|
/// | `> 0.96` | 0.5 |
/// | `0.8 ..= 0.96` | 0.9 |
/// | `0.15 ..= 0.8` | 0.95 |
/// | `<= 0.15` | 0.8 |
fn cooling_factor(accepted: f64) -> f64 {
    if accepted > 0.96 {
        0.5
    } else if accepted > 0.8 {
        0.9
    } else if accepted > 0.15 {
        0.95
    } else {
        0.8
    }
}

/// Runs the annealing pass, filling in the schedule part of `report`.
///
/// The schedule is Betz and Rose's, in all four of its parts:
///
/// 1. **Start temperature** `20 ×` the standard deviation of the cost
///    change over `N` random moves, where `N` is the number of movable
///    cells — the paper's `N_blocks`, not a fixed hundred. A hundred
///    samples of a quantity whose own spread is what is being measured
///    is not a sample of a 4000-cell design at all.
/// 2. **`effort · n^(4/3)` moves per temperature**, as before.
/// 3. **An adaptive cooling factor** read off the acceptance rate of the
///    step that just ended: [`cooling_factor`].
/// 4. **A range limit** on the move generator, updated as
///    `D' = D · (1 - 0.44 + R_accept)` and clamped to
///    `1 ..= max(width, height)`, which is what holds the acceptance
///    rate near 0.44 instead of letting it collapse.
///
/// Plus one thing the paper does not have: a stall exit, so a placement
/// that has converged stops rather than spending the rest of
/// [`PlaceOptions::max_temperatures`] proving it.
///
/// The best placement seen is kept, so the pass is monotone — it cannot
/// return something worse than legalisation handed it. That snapshot is
/// taken at a temperature boundary rather than on every improving move:
/// a move is now accepted a hundred times more often than it was, and
/// cloning the placement on each one would cost more than the move does.
/// The end of a temperature is within a move or two of the best point
/// inside it once the walk is cold, which is where it matters.
#[allow(clippy::too_many_arguments, reason = "the annealer's whole state")]
fn anneal(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    macros: &[Macro],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    shared: &SiteRules,
    options: &PlaceOptions,
    placement: &mut Placement,
    report: &mut PlacementReport,
) {
    let movable: Vec<usize> = (0..netlist.instances.len())
        .filter(|i| info[*i].fixed.is_none())
        .collect();
    if movable.is_empty() {
        return;
    }
    let windows: BTreeMap<String, KindSites> = sites_by_kind
        .iter()
        .map(|(kind, sites)| (kind.clone(), KindSites::build(graph, sites)))
        .collect();
    let mut rng = Rng::new(options.seed);
    let inner = options
        .moves_per_temperature
        .unwrap_or_else(|| moves_for(movable.len(), options.move_effort));
    let widest = f64::from(graph.width.max(graph.height).max(1));
    let mut window = match options.start_window {
        Some(start) => f64::from(start).clamp(1.0, widest),
        None => widest,
    };
    // The probe samples the same window the walk will start in, so that
    // the temperature it yields is a temperature for the moves that will
    // actually be made.
    let probe_reach = if options.range_limit {
        window_tiles(window)
    } else {
        u32::MAX
    };

    // The sample the start temperature is read off. The paper takes
    // `N_blocks` moves, so the count is the number of movable cells and
    // not a fixed hundred: a hundred draws from a distribution whose own
    // spread is the thing being measured is not a sample of a four
    // thousand cell design.
    //
    // Each probe move is undone. The paper's probe accepts every move,
    // but it starts from a random placement and so stays on one; here
    // the placement to be annealed is the legaliser's, and a probe that
    // wanders off it measures the neighbourhood of a placement that will
    // never be visited. What the schedule needs to know is how big a
    // cost change a move makes *here*.
    let mut samples = Vec::new();
    let mut churn = Churn::new(netlist, graph, placement);
    for _ in 0..movable.len().clamp(20, 100_000) {
        if let Some(candidate) = propose(
            netlist,
            graph,
            info,
            macros,
            sites_by_kind,
            &windows,
            shared,
            &movable,
            &mut rng,
            placement,
            probe_reach,
        ) {
            samples.push(apply(graph, &mut churn, placement, &candidate));
            undo(graph, &mut churn, placement);
        }
    }
    let mut temperature = start_temperature(&samples, options.start_acceptance);

    let mut current = churn.spans.total();
    let mut best = placement.clone();
    let mut best_cost = current;
    let mut stalled = 0u32;
    let signals = netlist.signals.len().max(1) as f64;
    report.stop = AnnealStop::Exhausted;
    while report.temperatures < options.max_temperatures {
        let reach = if options.range_limit {
            window_tiles(window)
        } else {
            u32::MAX
        };
        let mut step = TemperatureStep {
            range_limit: reach.min(graph.width.max(graph.height)),
            ..TemperatureStep::default()
        };
        for _ in 0..inner {
            let Some(candidate) = propose(
                netlist,
                graph,
                info,
                macros,
                sites_by_kind,
                &windows,
                shared,
                &movable,
                &mut rng,
                placement,
                reach,
            ) else {
                continue;
            };
            step.tried += 1;
            let delta = apply(graph, &mut churn, placement, &candidate);
            if delta <= 0.0 || rng.unit() * (temperature + delta) < temperature {
                step.accepted += 1;
            } else {
                undo(graph, &mut churn, placement);
            }
        }
        // Recounted, not accumulated: a few million `+= delta` on an f64
        // drift, and both the exit criterion and the best-so-far compare
        // against this number. The recount is one pass over the signals
        // per temperature, which is nothing beside the moves.
        current = churn.spans.total();
        step.cost = current;
        let rate = if step.tried == 0 {
            0.0
        } else {
            step.accepted as f64 / step.tried as f64
        };
        let improved = current < best_cost;
        if improved {
            best_cost = current;
            best.clone_from(placement);
            count(|work| &mut work.snapshots, 1);
            stalled = 0;
        } else if rate <= QUENCHED {
            stalled += 1;
        }
        report.moves.0 += step.tried;
        report.moves.1 += step.accepted;
        report.schedule.push(step);
        report.temperatures += 1;
        if options.stall_limit > 0 && stalled >= options.stall_limit {
            report.stop = AnnealStop::Stalled;
            break;
        }
        temperature *= options.cooling.unwrap_or_else(|| cooling_factor(rate));
        window = next_window(window, rate, options.target_acceptance, widest);
        // VPR's exit: "the anneal is terminated when
        // T < 0.005 * Cost / N_nets", because below that an uphill move
        // is unlikely to be accepted at all.
        //
        // Checked after a step, and only when that step improved
        // nothing. Both of those are needed because this anneal does not
        // start from a random placement: the start temperature is solved
        // for the target acceptance rate at the legalised placement, and
        // on a design whose neighbourhood is mostly level it comes out
        // below the threshold immediately. The criterion describes a walk
        // that has finished, not one that has not begun, and a step that
        // still found an improvement has not finished.
        if temperature < 0.005 * current as f64 / signals && !improved {
            report.stop = AnnealStop::Cold;
            break;
        }
    }
    *placement = best;
}

/// Moves to try per temperature: `effort * n^(4/3)`, computed in integers.
///
/// `powf` is not guaranteed to give the same last bit on every platform,
/// and a golden placement has to, so the exponent is taken as an integer
/// fourth power and an integer cube root instead.
fn moves_for(instances: usize, effort: usize) -> usize {
    let fourth = (instances as u128).pow(4);
    let root = cube_root(fourth);
    usize::try_from(root.saturating_mul(effort.max(1) as u128))
        .unwrap_or(usize::MAX)
        .clamp(20, 1_000_000)
}

/// The integer cube root of `value`, by bisection.
fn cube_root(value: u128) -> u128 {
    let (mut low, mut high) = (0u128, 1u128 << 43);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if mid.saturating_mul(mid).saturating_mul(mid) <= value {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    low
}

/// The temperature the walk starts at, from a sample of cost changes.
///
/// `want` is `None` for the paper's rule, `20 x` the standard deviation
/// of the sample, and `Some(rate)` to solve instead for the temperature
/// at which `rate` of the sample would be accepted.
///
/// The solve is a bisection on
/// `A(T) = mean over the sample of (delta <= 0 ? 1 : T / (T + delta))`,
/// which is the placer's own acceptance rule — the Cauchy one, for the
/// reason in the module docs — and is increasing in `T`, so sixty steps
/// of bisection pin it far below anything that matters. Nothing here is
/// a libm call, so the answer is the same on every platform.
///
/// `A` counts a level or downhill move as accepted, because the rate the
/// schedule later steers on counts it too: the point of solving is to
/// start where the feedback loop is trying to sit, and a definition that
/// disagreed with the loop's would not do that. It is why the answer can
/// be the floor — a neighbourhood more than `rate` of which is already
/// level or downhill has no positive temperature that accepts only
/// `rate` of it, and the right reading of that is "descend, do not warm
/// up". Something has to stop the walk ending there after one step, and
/// it is the exit criterion, which asks for a step that improved nothing
/// as well as for a low temperature.
fn start_temperature(samples: &[f64], want: Option<f64>) -> f64 {
    let Some(rate) = want else {
        return 20.0 * stddev(samples).max(1.0);
    };
    if samples.is_empty() {
        return 1.0;
    }
    let n = samples.len() as f64;
    let acceptance = |t: f64| {
        samples
            .iter()
            .map(|d| if *d <= 0.0 { 1.0 } else { t / (t + d) })
            .sum::<f64>()
            / n
    };
    let (mut low, mut high) = (1e-9_f64, 1e9_f64);
    for _ in 0..60 {
        let mid = 0.5 * (low + high);
        if acceptance(mid) < rate {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

/// The standard deviation of a sample, 0 for fewer than two values.
fn stddev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    variance.sqrt()
}

/// The tiles a cell at `(x, y)` may move to: the `reach`-tile window
/// around it, clipped to the die and to the cell's region.
///
/// `None` when the two do not overlap, which cannot happen for a cell
/// legalisation put inside its own region but is checked rather than
/// assumed.
fn window_of(
    graph: &RoutingGraph,
    region: Option<&Rect>,
    x: u32,
    y: u32,
    reach: u32,
) -> Option<Rect> {
    let mut rect = Rect {
        x0: x.saturating_sub(reach),
        y0: y.saturating_sub(reach),
        x1: x.saturating_add(reach).min(graph.width.saturating_sub(1)),
        y1: y.saturating_add(reach).min(graph.height.saturating_sub(1)),
    };
    if let Some(region) = region {
        rect.x0 = rect.x0.max(region.x0);
        rect.y0 = rect.y0.max(region.y0);
        rect.x1 = rect.x1.min(region.x1);
        rect.y1 = rect.y1.min(region.y1);
    }
    (rect.x0 <= rect.x1 && rect.y0 <= rect.y1).then_some(rect)
}

/// Proposes a move: a macro relocation, a move to a free site, or a swap.
///
/// `reach` is the range limit in tiles: the destination is drawn from the
/// square of that half-side around where the cell is now. `u32::MAX`
/// means the whole die, which is what the start-temperature probe uses
/// and what [`PlaceOptions::range_limit`] turns off to.
#[allow(clippy::too_many_arguments, reason = "the annealer's whole state")]
fn propose(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    macros: &[Macro],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    windows: &BTreeMap<String, KindSites>,
    shared: &SiteRules,
    movable: &[usize],
    rng: &mut Rng,
    placement: &Placement,
    reach: u32,
) -> Option<Move> {
    let instance = movable[rng.below(movable.len())];
    let here = placement.site_of(instance)?;
    let (hx, hy) = graph.sites[here].tile;
    if let Some(index) = info[instance].macro_index {
        let m = &macros[index];
        // A macro moves as a whole, so the window is around its anchor
        // and not around whichever member was drawn.
        let (ax0, ay0) = match placement.site_of(m.anchor) {
            Some(site) => graph.sites[site].tile,
            None => (hx, hy),
        };
        let rect = window_of(graph, None, ax0, ay0, reach)?;
        let ax = rect.x0 + u32::try_from(rng.below((rect.x1 - rect.x0 + 1) as usize)).unwrap_or(0);
        let ay = rect.y0 + u32::try_from(rng.below((rect.y1 - rect.y0 + 1) as usize)).unwrap_or(0);
        let mut free = placement.clone();
        for (member, _, _) in &m.members {
            free.unplace(*member);
        }
        return macro_sites(
            netlist,
            graph,
            info,
            m,
            sites_by_kind,
            shared,
            &free,
            ax,
            ay,
        );
    }
    let kind = &netlist.instances[instance].kind;
    let rect = window_of(
        graph,
        info[instance].region.as_ref().map(|(rect, _)| rect),
        hx,
        hy,
        reach,
    )?;
    let target = windows.get(kind)?.pick(rng, rect)?;
    if target == here {
        return None;
    }
    // Reachability, which for everything but a cell on the global network
    // is one `is_none` and free: see `confine_to_reachable`. Without it
    // the annealer would move a clock buffer between the sites of one
    // tile for nothing — all fourteen of this ECP5's `LDCC` buffers are
    // in `X3Y25` and five of them cannot be driven from a top-edge pad —
    // because such a move changes the tile-granular wirelength by exactly
    // zero and is accepted at any temperature.
    if !info[instance].may_take(target) {
        return None;
    }
    // A move is only proposed if it is legal, which for a swap means legal
    // after both halves have happened: see `SiteRules`. The annealer
    // therefore never has to undo an illegal placement, and a design whose
    // cells share no pin — every family but the ECP5, today — takes exactly
    // the moves it took before this check existed.
    let candidate = match placement.instance_at(target) {
        None => vec![(instance, target)],
        Some(other) => {
            if info[other].fixed.is_some() || info[other].macro_index.is_some() {
                return None;
            }
            if let Some((rect, _)) = &info[other].region
                && !rect.holds(hx, hy)
            {
                return None;
            }
            if !info[other].may_take(here) {
                return None;
            }
            vec![(instance, target), (other, here)]
        }
    };
    shared.allows(placement, &candidate).then_some(candidate)
}

/// Applies a move, returning the change in wirelength.
///
/// Where each instance came from is recorded in [`Churn::previous`], which
/// is what [`undo`] puts back; the wirelength comes out of [`Spans`] and
/// not out of a rescan of the nets.
fn apply(graph: &RoutingGraph, churn: &mut Churn, placement: &mut Placement, m: &Move) -> f64 {
    churn.previous.clear();
    for (instance, _) in m {
        let was = placement.site_of(*instance);
        churn.previous.push((*instance, was));
    }
    retile(
        graph,
        churn,
        placement,
        m.iter().map(|(i, s)| (*i, Some(*s))),
    )
}

/// Puts back what [`apply`] moved.
fn undo(graph: &RoutingGraph, churn: &mut Churn, placement: &mut Placement) {
    // `previous` is borrowed from the same value `retile` writes into, so
    // the plan is taken out and put back rather than aliased.
    let back = std::mem::take(&mut churn.previous);
    let _ = retile(graph, churn, placement, back.iter().copied());
    churn.previous = back;
}

/// Moves every instance of `plan` onto the site it names, keeping
/// [`Churn::spans`] exact, and returns the change in half-perimeter
/// wirelength.
///
/// The order matters: every affected pin leaves its old column and row
/// before any of them arrives at a new one, because two cells of one move
/// may be swapping places and a box told of the arrival first would
/// believe a column still holds a pin it has lost.
fn retile(
    graph: &RoutingGraph,
    churn: &mut Churn,
    placement: &mut Placement,
    plan: impl Iterator<Item = (usize, Option<usize>)> + Clone,
) -> f64 {
    churn.touched.clear();
    for (instance, _) in plan.clone() {
        for (signal, _) in &churn.carried[instance] {
            if !churn.touched.contains(signal) {
                churn.touched.push(*signal);
            }
        }
    }
    let before: u64 = churn.touched.iter().map(|s| churn.spans.hpwl_of(*s)).sum();
    let mut pins = 0u64;
    for (instance, _) in plan.clone() {
        if let Some(site) = placement.site_of(instance) {
            let (x, y) = graph.sites[site].tile;
            for index in 0..churn.carried[instance].len() {
                let signal = churn.carried[instance][index].0;
                churn.spans.remove(signal, x, y);
                pins += 1;
            }
        }
        placement.unplace(instance);
    }
    for (instance, site) in plan {
        let Some(site) = site else { continue };
        placement.place(instance, site);
        let (x, y) = graph.sites[site].tile;
        for index in 0..churn.carried[instance].len() {
            let signal = churn.carried[instance][index].0;
            churn.spans.add(signal, x, y);
            pins += 1;
        }
    }
    for index in 0..churn.touched.len() {
        let signal = churn.touched[index];
        churn.spans.settle(signal);
    }
    let after: u64 = churn.touched.iter().map(|s| churn.spans.hpwl_of(*s)).sum();
    count(|work| &mut work.cost_pins, pins);
    after as f64 - before as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::arch::{Arch, BelDecl, PipDecl, TileType, WireDecl, WireRef};
    use crate::ir::Id;

    /// A grid of one-LUT tiles, small enough to reason about.
    fn grid(width: u32, height: u32) -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("t", "test", width, height);
        let mut tile = TileType::new("logic", "logic_tile", 4, 4);
        for name in ["i", "o"] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        let mut bel = BelDecl::new("lut", "lut");
        bel.pins.push(("i0".to_owned(), WireRef::local("i")));
        bel.pins.push(("o".to_owned(), WireRef::local("o")));
        tile.bels.push(bel);
        arch.tile_types.push(tile);
        for y in 0..height {
            for x in 0..width {
                arch.set_tile(x, y, 0);
            }
        }
        let graph = arch.build_graph();
        (arch, graph)
    }

    /// A chain of `n` LUTs, each feeding the next.
    fn chain(n: usize) -> Netlist {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: Vec::new(),
            off_fabric: Vec::new(),
        };
        for i in 0..n {
            let instance = netlist.instances.len();
            let mut pins = Vec::new();
            if i > 0 {
                let pin = netlist.pins.len();
                netlist.pins.push(NetPin {
                    instance,
                    port: "I0".to_owned(),
                    bit: 0,
                    role: "i0".to_owned(),
                    output: false,
                    signal: Some(i - 1),
                    constant: None,
                });
                netlist.signals[i - 1].sinks.push(pin);
                pins.push(pin);
            }
            if i + 1 < n {
                let pin = netlist.pins.len();
                netlist.pins.push(NetPin {
                    instance,
                    port: "O".to_owned(),
                    bit: 0,
                    role: "o".to_owned(),
                    output: true,
                    signal: Some(i),
                    constant: None,
                });
                netlist.signals.push(Signal {
                    name: format!("n{i}"),
                    driver: Some(pin),
                    sinks: Vec::new(),
                });
                pins.push(pin);
            }
            netlist.instances.push(Instance {
                cell: CellId::from_index(i),
                name: format!("lut{i}"),
                primitive: "L".to_owned(),
                kind: "lut".to_owned(),
                pins,
                pin: None,
            });
        }
        netlist
    }

    #[test]
    fn the_solver_finds_the_obvious_answer() {
        // Two nodes pulled to 0 and 10 with a spring between them settle
        // at the thirds.
        let mut a = Sparse::new(2);
        a.add(0, 0, 2.0);
        a.add(1, 1, 2.0);
        a.add(0, 1, -1.0);
        a.add(1, 0, -1.0);
        let x = solve(&a, &[0.0, 10.0], 100);
        assert!((x[0] - 10.0 / 3.0).abs() < 1e-9, "{x:?}");
        assert!((x[1] - 20.0 / 3.0).abs() < 1e-9, "{x:?}");
        assert_eq!(solve(&Sparse::new(0), &[], 10), Vec::<f64>::new());
        assert!(Sparse::new(0).is_empty());
    }

    /// One driver and `n` sinks: a clock net's shape, with nothing else in
    /// the design to confuse the measurement.
    fn fan_out(n: usize) -> Netlist {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: vec![Signal {
                name: "net".to_owned(),
                driver: None,
                sinks: Vec::new(),
            }],
            off_fabric: Vec::new(),
        };
        for i in 0..=n {
            let instance = netlist.instances.len();
            let pin = netlist.pins.len();
            let output = i == 0;
            netlist.pins.push(NetPin {
                instance,
                port: if output { "O" } else { "I0" }.to_owned(),
                bit: 0,
                role: if output { "o" } else { "i0" }.to_owned(),
                output,
                signal: Some(0),
                constant: None,
            });
            if output {
                netlist.signals[0].driver = Some(pin);
            } else {
                netlist.signals[0].sinks.push(pin);
            }
            netlist.instances.push(Instance {
                cell: CellId::from_index(i),
                name: format!("lut{i}"),
                primitive: "L".to_owned(),
                kind: "lut".to_owned(),
                pins: vec![pin],
                pin: None,
            });
        }
        netlist
    }

    /// **A move costs what the cells it moves have on them, not what the
    /// nets they are on have on them.** The guard on [`Spans`], counted
    /// rather than timed.
    ///
    /// This is the regression that cost the most: the annealer worked out a
    /// move's cost by reading every pin of every signal it touched, twice,
    /// so a design with a thousand-sink clock net paid a thousand pin reads
    /// per move. `usb_host_target.v` read 84.5 **billion** pin positions to
    /// place 4304 cells, which was 489 of its 694 seconds.
    ///
    /// A number of seconds cannot be asserted — CI runs on slower machines —
    /// so this compares two placements in one process. Both have the same
    /// number of cells and the same number of moves to make; one net has
    /// eight sinks and the other two hundred. If the cost of a move scales
    /// with fan-out, the second does twenty-five times the reading. What it
    /// must do is **the same**: a bounding box kept per column and per row
    /// is updated by the pins that moved and by nothing else.
    ///
    /// What it would catch: any return to recomputing a signal's bounding
    /// box from its pins inside the move loop, which is the shape of the
    /// bug and not a particular number. What it would **not** catch: a
    /// bounding box that is updated cheaply and *wrongly* — that is
    /// `the_annealers_boxes_agree_with_a_full_recount`'s job.
    #[test]
    fn a_move_costs_the_cells_it_moves_and_not_the_fan_out_of_their_nets() {
        let (arch, graph) = grid(16, 16);
        let options = PlaceOptions {
            // The same number of moves either way, so the only thing that
            // can differ is what one move costs.
            moves_per_temperature: Some(2_000),
            max_temperatures: 4,
            ..PlaceOptions::default()
        };
        let cost_per_move = |sinks: usize| -> f64 {
            let netlist = fan_out(sinks);
            let (_, report) =
                place(&netlist, &arch, &graph, &Constraints::default(), &options).expect("it fits");
            assert!(
                report.moves.0 > 1_000,
                "the annealer ran: {:?}",
                report.moves
            );
            report.work.cost_pins as f64 / report.moves.0 as f64
        };
        let narrow = cost_per_move(8);
        let wide = cost_per_move(200);
        assert!(
            wide < 4.0 * narrow,
            "a move on a 200-sink net costs {wide:.1} pin read(s) against {narrow:.1} on an \
             8-sink one, which is the fan-out being read again per move"
        );
    }

    /// The bounding boxes the annealer keeps are the ones a full recount
    /// gives, which is what makes [`Spans`] a speed-up and not a different
    /// cost function.
    ///
    /// It is the other half of the test above: that one says the update is
    /// cheap, this one says it is right. Without it a box that was updated
    /// wrongly would still be fast, and the placer would quietly be
    /// optimising something else.
    #[test]
    fn the_annealers_boxes_agree_with_a_full_recount() {
        let (arch, graph) = grid(8, 8);
        let netlist = chain(12);
        let (placement, report) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::default(),
            &PlaceOptions::default(),
        )
        .expect("it fits");
        let spans = Spans::new(&netlist, &graph, &placement);
        for signal in 0..netlist.signals.len() {
            assert_eq!(
                spans.hpwl_of(signal),
                signal_hpwl(&netlist, &graph, &placement, signal),
                "signal {signal}"
            );
        }
        assert_eq!(spans.total(), hpwl(&netlist, &graph, &placement));
        assert_eq!(report.hpwl_after, spans.total());
    }

    #[test]
    fn legalisation_gives_every_cell_its_own_site() {
        let (arch, graph) = grid(4, 4);
        let netlist = chain(6);
        let (placement, report) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .unwrap();
        assert_eq!(placement.placed(), 6);
        let mut sites: Vec<usize> = (0..6).map(|i| placement.site_of(i).unwrap()).collect();
        sites.sort_unstable();
        sites.dedup();
        assert_eq!(sites.len(), 6, "two cells share a site");
        assert_eq!(report.usage, vec![("lut".to_owned(), 6, 16)]);
        // A chain of six on a four by four grid routes to a short path.
        assert!(report.hpwl_after <= report.hpwl_before, "{report:?}");
        assert!(report.hpwl_after <= 8, "{}", report.to_text());
        assert!(report.to_text().contains("6 of 16 lut sites"));
    }

    #[test]
    fn a_design_that_does_not_fit_says_so() {
        let (arch, graph) = grid(2, 2);
        let netlist = chain(9);
        let err = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            PlaceError::NoSites {
                kind: "lut".to_owned(),
                needed: 9,
                available: 4,
            }
        );
        assert!(err.to_string().contains("needs 9 `lut` site(s)"));
    }

    #[test]
    fn a_region_confines_what_it_matches() {
        use crate::fpga::{Origin, Region, RegionAssignment};
        use crate::source::{SourceMap, Span};

        let (arch, graph) = grid(6, 6);
        let netlist = chain(4);
        let mut sources = SourceMap::new();
        let file = sources.add("c.rcf", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut constraints = Constraints::new();
        constraints.regions.push(Region {
            name: "corner".to_owned(),
            x0: 4,
            y0: 4,
            x1: 5,
            y1: 5,
            span,
        });
        constraints.region_assignments.push(RegionAssignment {
            pattern: "lut*".to_owned(),
            region: "corner".to_owned(),
            span,
            origin: Origin::File,
        });
        let (placement, report) = place(
            &netlist,
            &arch,
            &graph,
            &constraints,
            &PlaceOptions::default(),
        )
        .unwrap();
        for i in 0..4 {
            let (x, y) = graph.sites[placement.site_of(i).unwrap()].tile;
            assert!((4..=5).contains(&x) && (4..=5).contains(&y), "({x}, {y})");
        }
        assert_eq!(report.confined, 4);

        // One more cell than the region holds is reported as such.
        let netlist = chain(5);
        let err = place(
            &netlist,
            &arch,
            &graph,
            &constraints,
            &PlaceOptions::default(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            PlaceError::RegionTooSmall {
                region: "corner".to_owned(),
                kind: "lut".to_owned(),
                needed: 5,
                available: 4,
            }
        );
        assert!(err.to_string().contains("region `corner` holds 4"));
    }

    #[test]
    fn an_rloc_macro_keeps_its_shape() {
        use crate::fpga::{Origin, Rloc};
        use crate::source::{SourceMap, Span};

        let (arch, graph) = grid(6, 6);
        let netlist = chain(3);
        let mut sources = SourceMap::new();
        let file = sources.add("c.rcf", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut constraints = Constraints::new();
        for (index, (dx, dy)) in [(0, 0), (1, 0), (2, 0)].into_iter().enumerate() {
            constraints.rlocs.push(Rloc {
                pattern: format!("lut{index}"),
                dx,
                dy,
                group: Some("row".to_owned()),
                span,
                origin: Origin::File,
            });
        }
        let (placement, report) = place(
            &netlist,
            &arch,
            &graph,
            &constraints,
            &PlaceOptions::default(),
        )
        .unwrap();
        assert_eq!(report.macros, (1, 3));
        let tiles: Vec<(u32, u32)> = (0..3)
            .map(|i| graph.sites[placement.site_of(i).unwrap()].tile)
            .collect();
        assert_eq!(tiles[1].0, tiles[0].0 + 1);
        assert_eq!(tiles[2].0, tiles[0].0 + 2);
        assert_eq!(tiles[0].1, tiles[1].1);
        assert_eq!(tiles[0].1, tiles[2].1);
    }

    #[test]
    fn the_same_seed_gives_the_same_placement() {
        let (arch, graph) = grid(5, 5);
        let netlist = chain(8);
        let options = PlaceOptions::default();
        let (a, _) = place(&netlist, &arch, &graph, &Constraints::new(), &options).unwrap();
        let (b, _) = place(&netlist, &arch, &graph, &Constraints::new(), &options).unwrap();
        assert_eq!(a.to_text(&netlist, &graph), b.to_text(&netlist, &graph));
        let other = PlaceOptions {
            seed: 12345,
            ..options
        };
        let (c, _) = place(&netlist, &arch, &graph, &Constraints::new(), &other).unwrap();
        // A different seed is allowed to differ; what matters is that it
        // is still a legal placement.
        assert_eq!(c.placed(), 8);
    }

    /// The cooling factor is the one in VPR's Table 1, at every boundary.
    ///
    /// A table is the kind of thing that gets transcribed with a `<` where
    /// a `<=` belongs, and the bands are what the whole schedule is, so
    /// each is checked on both sides of its edge.
    #[test]
    fn the_cooling_factor_follows_the_published_table() {
        assert_eq!(cooling_factor(1.0), 0.5);
        assert_eq!(cooling_factor(0.97), 0.5);
        assert_eq!(cooling_factor(0.96), 0.9);
        assert_eq!(cooling_factor(0.81), 0.9);
        assert_eq!(cooling_factor(0.80), 0.95);
        assert_eq!(cooling_factor(0.16), 0.95);
        assert_eq!(cooling_factor(0.15), 0.8);
        assert_eq!(cooling_factor(0.0), 0.8);
    }

    /// The range limiter is a feedback loop around the target rate: it
    /// widens above, narrows below, holds at it, and never leaves
    /// `1 ..= widest`.
    #[test]
    fn the_move_window_tracks_the_acceptance_rate() {
        assert_eq!(next_window(10.0, 0.44, 0.44, 40.0), 10.0);
        assert!(next_window(10.0, 0.9, 0.44, 40.0) > 10.0);
        assert!(next_window(10.0, 0.1, 0.44, 40.0) < 10.0);
        // Both clamps, which are what stop a run of refusals from driving
        // the window to zero and a run of acceptances from making it
        // bigger than the die.
        assert_eq!(next_window(1.0, 0.0, 0.44, 40.0), 1.0);
        assert_eq!(next_window(40.0, 1.0, 0.44, 40.0), 40.0);
    }

    /// The start temperature is solved for, not guessed: at the
    /// temperature it returns, the sample it was given would be accepted
    /// at the rate that was asked for.
    ///
    /// The second half is the degenerate case the solve has to get right
    /// — a neighbourhood that is already mostly downhill cannot be made
    /// *less* acceptable by cooling, so the answer is the floor and not a
    /// number that overshoots.
    #[test]
    fn the_start_temperature_is_the_one_that_accepts_what_was_asked() {
        let samples: Vec<f64> = (1..=100).map(|i| f64::from(i) - 40.0).collect();
        let t = start_temperature(&samples, Some(0.44));
        let got = samples
            .iter()
            .map(|d| if *d <= 0.0 { 1.0 } else { t / (t + d) })
            .sum::<f64>()
            / samples.len() as f64;
        assert!((got - 0.44).abs() < 1e-6, "{got} at T = {t}");

        // A neighbourhood already more than 44% downhill wants no
        // temperature at all, and the bisection has to say so rather than
        // overshoot: the answer is its floor, and what keeps that from
        // ending the anneal after one step is the exit criterion.
        assert!(start_temperature(&[-1.0; 10], Some(0.44)) < 1e-6);

        // And the paper's rule is still there, unchanged, for the run
        // that wants to start from scratch.
        let spread = vec![-10.0, 10.0];
        assert_eq!(start_temperature(&spread, None), 200.0);
    }

    /// A move window holds: every site the generator offers is inside the
    /// rectangle it was given, and a rectangle with nothing in it offers
    /// nothing rather than something far away.
    #[test]
    fn the_move_generator_stays_inside_its_window() {
        let (_, graph) = grid(16, 16);
        let sites: Vec<usize> = (0..graph.sites.len())
            .filter(|s| graph.sites[*s].kind == "lut")
            .collect();
        let index = KindSites::build(&graph, &sites);
        let mut rng = Rng::new(7);
        let rect = Rect {
            x0: 4,
            y0: 5,
            x1: 6,
            y1: 9,
        };
        let mut seen = 0;
        for _ in 0..2_000 {
            let Some(site) = index.pick(&mut rng, rect) else {
                continue;
            };
            let (x, y) = graph.sites[site].tile;
            assert!(rect.holds(x, y), "({x}, {y}) is outside {rect:?}");
            seen += 1;
        }
        assert!(seen > 1_000, "the window offered only {seen} site(s)");
        assert_eq!(
            index.pick(
                &mut rng,
                Rect {
                    x0: 40,
                    y0: 40,
                    x1: 44,
                    y1: 44
                }
            ),
            None
        );
    }

    /// **The annealer improves on what legalisation hands it, at the
    /// lowest effort.** The regression this whole schedule exists for.
    ///
    /// The placer used to end at *exactly* the wirelength legalisation
    /// produced on `usb_host_target.v` at any effort below 10, and on
    /// `clock_blink.v` at every effort including 10: seven million moves
    /// for nothing. The cause was the move generator, not the move count
    /// — a die-wide move is a large uphill one, so once the temperature
    /// is low enough to be selective every move is refused, and while it
    /// is high enough to accept one the placement is scattered.
    ///
    /// What this would catch: a schedule that goes back to offering
    /// die-wide moves, or a start temperature hot enough to throw the
    /// analytic placement away, both of which show up as
    /// `hpwl_after == hpwl_before`. What it would **not** catch: a
    /// schedule that improves the wirelength a little when it could
    /// improve it a lot; only a measurement on a real design says that,
    /// and `docs/fpga-trellis.md` has it.
    #[test]
    fn the_annealer_improves_on_legalisation_at_the_lowest_effort() {
        let (arch, graph) = grid(12, 12);
        let netlist = chain(96);
        let options = PlaceOptions {
            move_effort: 1,
            ..PlaceOptions::default()
        };
        let (_, report) =
            place(&netlist, &arch, &graph, &Constraints::new(), &options).expect("it fits");
        assert!(
            report.hpwl_after < report.hpwl_before,
            "{} move(s) left the wirelength at {}\n{}",
            report.moves.0,
            report.hpwl_after,
            report.schedule_text()
        );
    }

    /// And the range limit is *why*: the same design and the same budget,
    /// annealed the way this placer used to, ends worse.
    ///
    /// This is a comparison and not an absolute number, because what is
    /// being claimed is a mechanism. A placer whose moves may go anywhere
    /// has a neighbourhood whose cost changes are enormous beside any
    /// temperature that would accept them.
    #[test]
    fn the_range_limit_is_what_makes_the_annealing_work() {
        let (arch, graph) = grid(12, 12);
        let netlist = chain(96);
        let options = PlaceOptions {
            move_effort: 1,
            ..PlaceOptions::default()
        };
        let (_, limited) =
            place(&netlist, &arch, &graph, &Constraints::new(), &options).expect("it fits");
        let wide = PlaceOptions {
            range_limit: false,
            start_window: None,
            start_acceptance: None,
            cooling: Some(0.9),
            stall_limit: 0,
            ..options
        };
        let (_, old) = place(&netlist, &arch, &graph, &Constraints::new(), &wide).expect("it fits");
        assert!(
            limited.hpwl_after < old.hpwl_after,
            "range-limited {} against die-wide {}",
            limited.hpwl_after,
            old.hpwl_after
        );
    }

    /// A placement that has converged stops, rather than spending the
    /// rest of its budget.
    ///
    /// Either exit is a pass — the temperature criterion is VPR's and
    /// fires first on most designs, the stall criterion catches the rest.
    /// What must not happen is `Exhausted`, which means the schedule ran
    /// to its backstop and the budget, rather than the search, decided
    /// when to stop.
    #[test]
    fn a_converged_placement_stops_before_its_budget() {
        let (arch, graph) = grid(12, 12);
        let netlist = chain(96);
        let (_, report) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .expect("it fits");
        assert!(
            matches!(report.stop, AnnealStop::Cold | AnnealStop::Stalled),
            "stopped because {}, after {} of {} temperature(s)\n{}",
            report.stop.clause(),
            report.temperatures,
            PlaceOptions::default().max_temperatures,
            report.schedule_text()
        );
        assert_eq!(report.temperatures as usize, report.schedule.len());
        assert_eq!(
            report.moves,
            report
                .schedule
                .iter()
                .fold((0, 0), |(t, a), s| (t + s.tried, a + s.accepted))
        );
    }

    /// The schedule that is reported is the one that ran, and the
    /// placement that comes back is the best step of it.
    ///
    /// `TemperatureStep::cost` is recounted from the bounding boxes at
    /// each temperature rather than accumulated from the move deltas,
    /// which is what keeps the exit criterion honest over tens of
    /// millions of `+=` on an `f64`; this checks that recount against
    /// [`hpwl`], which reads the pins.
    #[test]
    fn the_reported_schedule_is_the_one_that_ran() {
        let (arch, graph) = grid(12, 12);
        let netlist = chain(96);
        let (placement, report) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .expect("it fits");
        let best = report
            .schedule
            .iter()
            .map(|s| s.cost)
            .min()
            .expect("the annealer ran");
        assert_eq!(best.min(report.hpwl_before), report.hpwl_after);
        assert_eq!(hpwl(&netlist, &graph, &placement), report.hpwl_after);
        assert!(report.to_text().contains("schedule: acceptance"));
    }

    /// A grid of tiles holding **two** flip-flops each, whose enable pin is
    /// one wire for the pair — an ECP5 slice in miniature, and the shape
    /// `SiteRules` is for.
    fn pairs(width: u32, height: u32) -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("t", "test", width, height);
        let mut tile = TileType::new("logic", "logic_tile", 4, 4);
        for name in ["d0", "d1", "q0", "q1", "ce"] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        for half in 0..2 {
            let mut bel = BelDecl::new(format!("ff{half}"), "ff");
            bel.pins
                .push(("d".to_owned(), WireRef::local(format!("d{half}"))));
            bel.pins
                .push(("q".to_owned(), WireRef::local(format!("q{half}"))));
            // The whole point: both halves name one wire for the enable.
            bel.pins.push(("en".to_owned(), WireRef::local("ce")));
            tile.bels.push(bel);
        }
        arch.tile_types.push(tile);
        for y in 0..height {
            for x in 0..width {
                arch.set_tile(x, y, 0);
            }
        }
        let graph = arch.build_graph();
        (arch, graph)
    }

    /// `n` flip-flops whose enables come from `groups` distinct signals,
    /// round-robin, and nothing else — the shape a state machine with
    /// several clock enables has.
    fn enabled(n: usize, groups: usize) -> Netlist {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: Vec::new(),
            off_fabric: Vec::new(),
        };
        for g in 0..groups {
            netlist.signals.push(Signal {
                name: format!("en{g}"),
                driver: None,
                sinks: Vec::new(),
            });
        }
        for i in 0..n {
            let instance = netlist.instances.len();
            let pin = netlist.pins.len();
            let signal = i % groups;
            netlist.pins.push(NetPin {
                instance,
                port: "CE".to_owned(),
                bit: 0,
                role: "en".to_owned(),
                output: false,
                signal: Some(signal),
                constant: None,
            });
            netlist.signals[signal].sinks.push(pin);
            netlist.instances.push(Instance {
                cell: CellId::from_index(i),
                name: format!("ff{i}"),
                primitive: "F".to_owned(),
                kind: "ff".to_owned(),
                pins: vec![pin],
                pin: None,
            });
        }
        netlist
    }

    /// Two cells in one tile whose pins are one wire must want the same
    /// signal on it.
    ///
    /// This is the constraint an ECP5 slice's `CE`, `CLK` and `LSR` wires
    /// impose, and without it a design with several clock enables places
    /// happily and then asks the router to carry two signals on one node —
    /// which is how it was found, as 52 oversubscribed `CE` nodes on the
    /// eight-bit ULPI device core.
    #[test]
    fn two_cells_that_share_a_pin_must_agree_about_it() {
        // Nothing to check when nothing is shared, which is every family
        // but the ECP5 today and is why the check costs them nothing.
        let (_, plain) = grid(4, 4);
        assert!(
            SiteRules::find(&chain(6), &plain).trivial(),
            "a one-bel tile has no shared pin"
        );

        // Four tiles, eight flip-flops, one enable between them: every pair
        // agrees, so both halves of every tile are used.
        let (arch, graph) = pairs(2, 2);
        let netlist = enabled(8, 1);
        let (placement, _) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .expect("eight flip-flops with one enable fit in four pairs");
        assert_eq!(placement.placed(), 8);
        let mut tiles: Vec<(u32, u32)> = (0..8)
            .map(|i| graph.sites[placement.site_of(i).unwrap()].tile)
            .collect();
        tiles.sort_unstable();
        tiles.dedup();
        assert_eq!(
            tiles.len(),
            4,
            "agreeing flip-flops must still be allowed to pair up, or this check is just              spreading everything out"
        );

        // Eight flip-flops, eight enables, on a grid with room: no tile may
        // hold two of them.
        let (arch, graph) = pairs(4, 4);
        let netlist = enabled(8, 8);
        let (placement, _) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .expect("eight enables and sixteen tiles");
        let mut tiles: Vec<(u32, u32)> = (0..8)
            .map(|i| graph.sites[placement.site_of(i).unwrap()].tile)
            .collect();
        tiles.sort_unstable();
        let before = tiles.len();
        tiles.dedup();
        assert_eq!(
            tiles.len(),
            before,
            "two flip-flops with different enables share a tile, so the wire they share would              have to carry two signals"
        );

        // And the predicate itself, since the placements above could pass by
        // luck: the two halves of one tile, with disagreeing enables, are
        // refused, and with agreeing ones allowed.
        let shared = SiteRules::find(&netlist, &graph);
        assert!(!shared.trivial());
        let pair: Vec<usize> = (0..graph.sites.len())
            .filter(|s| graph.sites[*s].tile == (0, 0))
            .collect();
        assert_eq!(pair.len(), 2, "two flip-flops a tile");
        let mut empty = Placement::new(netlist.instances.len(), graph.sites.len());
        empty.place(0, pair[0]);
        assert!(
            !shared.allows(&empty, &[(1, pair[1])]),
            "`ff1` wants `en1` where `ff0` wants `en0`"
        );
        let same = enabled(2, 1);
        let shared_same = SiteRules::find(&same, &graph);
        let mut both = Placement::new(same.instances.len(), graph.sites.len());
        both.place(0, pair[0]);
        assert!(
            shared_same.allows(&both, &[(1, pair[1])]),
            "two flip-flops with one enable belong in one tile"
        );
    }

    /// A tile with **two** control wires for four flip-flops, and a
    /// memory whose own control pin is joined to one of them: the shape of
    /// an ECP5 logic tile's `LSR0`/`LSR1` with a `TRELLIS_DPR16X4` in it.
    ///
    /// `feed` is where the two wires come from. A wire the interconnect
    /// can drive is a **cut** the pool walk stops at; one that nothing
    /// outside the tile drives is not a control wire at all, which is the
    /// case the second half of the test below covers.
    fn budget(width: u32, height: u32, feed: bool) -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("t", "test", width, height);
        let mut tile = TileType::new("logic", "logic_tile", 8, 8);
        for name in ["lsr0", "lsr1", "wre", "track"] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        // The memory's control pin, wired to `lsr1` and to nothing else.
        tile.pips.push(PipDecl {
            from: WireRef::local("lsr1"),
            to: WireRef::local("wre"),
            bits: Vec::new(),
        });
        let mut ram = BelDecl::new("ram", "lutram");
        ram.pins.push(("we".to_owned(), WireRef::local("wre")));
        tile.bels.push(ram);
        // Four flip-flops, each with a mux of its own over the two wires.
        for slice in 0..4 {
            let mux = format!("mux{slice}");
            tile.wires.push(WireDecl {
                name: mux.clone(),
                dx: 0,
                dy: 0,
            });
            for wire in ["lsr0", "lsr1"] {
                tile.pips.push(PipDecl {
                    from: WireRef::local(wire),
                    to: WireRef::local(&mux),
                    bits: Vec::new(),
                });
            }
            let mut ff = BelDecl::new(format!("ff{slice}"), "ff");
            ff.pins.push(("rst".to_owned(), WireRef::local(&mux)));
            tile.bels.push(ff);
        }
        if feed {
            // What makes `lsr0` and `lsr1` the frontier: the interconnect
            // reaches them, so the walk stops there instead of going on.
            for wire in ["lsr0", "lsr1"] {
                tile.pips.push(PipDecl {
                    from: WireRef::at("track", 1, 0),
                    to: WireRef::local(wire),
                    bits: Vec::new(),
                });
            }
        }
        arch.tile_types.push(tile);
        for y in 0..height {
            for x in 0..width {
                arch.set_tile(x, y, 0);
            }
        }
        let graph = arch.build_graph();
        (arch, graph)
    }

    /// `n` flip-flops whose resets come from `groups` distinct signals,
    /// round-robin, with a `lutram` in front of them whose write enable is
    /// a signal of its own.
    fn reset_by(n: usize, groups: usize) -> Netlist {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: Vec::new(),
            off_fabric: Vec::new(),
        };
        for g in 0..groups {
            netlist.signals.push(Signal {
                name: format!("rst{g}"),
                driver: None,
                sinks: Vec::new(),
            });
        }
        netlist.signals.push(Signal {
            name: "we".to_owned(),
            driver: None,
            sinks: Vec::new(),
        });
        let cell = |netlist: &mut Netlist, kind: &str, role: &str, signal: usize| {
            let instance = netlist.instances.len();
            let pin = netlist.pins.len();
            netlist.pins.push(NetPin {
                instance,
                port: role.to_uppercase(),
                bit: 0,
                role: role.to_owned(),
                output: false,
                signal: Some(signal),
                constant: None,
            });
            netlist.signals[signal].sinks.push(pin);
            netlist.instances.push(Instance {
                cell: CellId::from_index(instance),
                name: format!("{kind}{instance}"),
                primitive: "P".to_owned(),
                kind: kind.to_owned(),
                pins: vec![pin],
                pin: None,
            });
        };
        cell(&mut netlist, "lutram", "we", groups);
        for i in 0..n {
            cell(&mut netlist, "ff", "rst", i % groups);
        }
        netlist
    }

    /// A tile's control wires are a **budget**, and a cell that is joined
    /// to one of them has spent it.
    ///
    /// This is the ECP5's `LSR0`/`LSR1` with a distributed RAM in the tile,
    /// which is where it was found: `oversubscribed node X24Y3/LSR1`, on a
    /// design with 32 `TRELLIS_DPR16X4` and two reset nets. The rule is the
    /// weak one — a flip-flop with no reset, or with the *same* reset as
    /// the tile already carries, is welcome — and the wires it applies to
    /// are read off the graph rather than named.
    #[test]
    fn a_control_wire_is_a_budget_and_a_memory_spends_one() {
        // One reset and one write enable: two signals, two wires, and the
        // memory may share its tile with all four flip-flops.
        let (arch, graph) = budget(2, 2, true);
        let netlist = reset_by(4, 1);
        let (placement, _) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .expect("one reset beside a memory fits");
        let ram = graph.sites[placement.site_of(0).unwrap()].tile;
        assert_eq!(
            (1..5)
                .filter(|i| graph.sites[placement.site_of(*i).unwrap()].tile == ram)
                .count(),
            4,
            "a memory must not sterilise its tile's flip-flops: that is the strong rule, and it \
             is not what `ecppack` does"
        );

        // Two resets and a write enable is three signals for two wires, so
        // the tile that holds the memory can hold only one of the two
        // reset domains.
        let netlist = reset_by(4, 2);
        let (placement, _) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .expect("four tiles is room enough to separate them");
        let ram = graph.sites[placement.site_of(0).unwrap()].tile;
        let mut wanted: Vec<usize> = (1..5)
            .filter(|i| graph.sites[placement.site_of(*i).unwrap()].tile == ram)
            .map(|i| (i - 1) % 2)
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        assert!(
            wanted.len() <= 1,
            "the memory's tile holds flip-flops of {} reset domains, and it has one wire left",
            wanted.len()
        );

        // And the predicate, since a placement could pass by luck.
        let shared = SiteRules::find(&netlist, &graph);
        assert!(!shared.trivial());
        let sites: Vec<usize> = (0..graph.sites.len())
            .filter(|s| graph.sites[*s].tile == (0, 0))
            .collect();
        let mut held = Placement::new(netlist.instances.len(), graph.sites.len());
        held.place(0, sites[0]);
        held.place(1, sites[1]);
        assert!(
            shared.allows(&held, &[(3, sites[2])]),
            "`ff3` resets from `rst0`, which this tile already carries"
        );
        assert!(
            !shared.allows(&held, &[(2, sites[2])]),
            "`ff2` resets from `rst1`, and the memory has spent the other wire"
        );

        // The pool is the same whether or not the interconnect reaches the
        // two wires: a wire nothing drives stops the walk too, because it
        // is a source and not a choice. What it is *not* is a wire only one
        // bel can reach — that is the bel's own business, and a tile of one
        // bel has no pool at all, which is every family but the ECP5 today
        // and is why the check costs them nothing.
        let (_, loose) = budget(2, 2, false);
        assert_eq!(
            SiteRules::find(&reset_by(4, 2), &loose).groups.len(),
            4,
            "one pool per tile either way"
        );
        let (_, plain) = grid(2, 2);
        assert!(
            SiteRules::find(&chain(2), &plain).groups.is_empty(),
            "a one-bel tile shares nothing, so there is no budget to keep"
        );
    }

    #[test]
    fn the_generator_is_uniform_enough_and_never_sticks() {
        let mut rng = Rng::new(0);
        let mut seen = [0usize; 4];
        for _ in 0..4000 {
            seen[rng.below(4)] += 1;
            let u = rng.unit();
            assert!((0.0..1.0).contains(&u));
        }
        assert!(seen.iter().all(|c| *c > 800), "{seen:?}");
        assert_eq!(Rng::new(7).below(0), 0);
    }
}
