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
//!    free site of the kind it needs, then simulated annealing improves
//!    the result against a half-perimeter wirelength cost with a move set
//!    of swaps, moves to free sites and relative-placement macro moves.
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

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use super::arch::{Arch, RoutingGraph};
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
    /// Geometric cooling factor, in `(0, 1)`.
    pub cooling: f64,
    /// Upper bound on temperature steps, so a pathological design stops.
    pub max_temperatures: u32,
    /// Moves tried per temperature, or `None` for the usual
    /// `10 * n^(4/3)`.
    pub moves_per_temperature: Option<usize>,
}

impl Default for PlaceOptions {
    fn default() -> Self {
        PlaceOptions {
            seed: 0x5EED_1CE4_0000_0001,
            analytic_iterations: 200,
            anneal: true,
            cooling: 0.9,
            max_temperatures: 120,
            moves_per_temperature: None,
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
    /// Pins the architecture gives no wire, which are not routed.
    pub off_fabric: usize,
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
        if self.off_fabric > 0 {
            let _ = writeln!(out, "  off-fabric pins: {}", self.off_fabric);
        }
        out
    }
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
        })
        .collect();
    fix_pins(netlist, arch, graph, &mut info, &mut report)?;
    apply_regions(netlist, graph, constraints, &mut info);
    let macros = build_macros(netlist, constraints, &mut info);
    report.macros = (
        macros.len(),
        macros.iter().map(|m| m.members.len()).sum::<usize>(),
    );

    let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
    let positions = solve_analytic(netlist, graph, &info, options);
    legalise(
        netlist,
        graph,
        &info,
        &macros,
        &positions,
        &sites_by_kind,
        &mut placement,
    )?;
    confine_hierarchy(netlist, graph, constraints, &placement, &mut info);
    report.confined = info.iter().filter(|i| i.region.is_some()).count();
    report.hpwl_before = hpwl(netlist, graph, &placement);
    report.hpwl_after = report.hpwl_before;

    if options.anneal {
        let stats = anneal(
            netlist,
            graph,
            &info,
            &macros,
            &sites_by_kind,
            options,
            &mut placement,
        );
        report.temperatures = stats.0;
        report.moves = (stats.1, stats.2);
        report.hpwl_after = hpwl(netlist, graph, &placement);
    }
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
fn legalise(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    macros: &[Macro],
    positions: &[(f64, f64)],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
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
        legalise_macro(netlist, graph, info, m, positions, sites_by_kind, placement)?;
    }
    for index in order {
        let (x, y) = positions[index];
        let kind = &netlist.instances[index].kind;
        let target = (to_grid(x, graph.width), to_grid(y, graph.height));
        let region = info[index].region.as_ref().map(|(rect, _)| rect);
        let site = nearest_free(graph, sites_by_kind, kind, region, target, placement);
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
            PlaceError::NoSites {
                kind,
                needed,
                available: sites.len(),
            }
        }
    }
}

/// The free site of `kind` closest to `target`, inside `region`.
fn nearest_free(
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    kind: &str,
    region: Option<&Rect>,
    target: (u32, u32),
    placement: &Placement,
) -> Option<usize> {
    let sites = sites_by_kind.get(kind)?;
    let mut best: Option<(u64, usize)> = None;
    for site in sites {
        if placement.instance_at(*site).is_some() {
            continue;
        }
        let (x, y) = graph.sites[*site].tile;
        if let Some(rect) = region
            && !rect.holds(x, y)
        {
            continue;
        }
        let distance = manhattan(target, (x, y));
        if best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, *site));
        }
    }
    best.map(|(_, site)| site)
}

/// Places a rigid macro: the anchor goes to the tile closest to its
/// target from which every member finds a site.
fn legalise_macro(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    m: &Macro,
    positions: &[(f64, f64)],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
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
        if let Some(sites) = macro_sites(netlist, graph, info, m, sites_by_kind, placement, ax, ay)
        {
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
        })?;
        taken.push(*site);
        out.push((*member, *site));
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

/// The half-perimeter wirelength of one signal, in tiles.
fn signal_hpwl(
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    signal: usize,
) -> u64 {
    let s = &netlist.signals[signal];
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

/// The signals one instance touches, without repeats.
fn signals_of(netlist: &Netlist, instance: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for pin in &netlist.instances[instance].pins {
        if let Some(signal) = netlist.pins[*pin].signal
            && !out.contains(&signal)
        {
            out.push(signal);
        }
    }
    out
}

/// One candidate move: which instances go where.
type Move = Vec<(usize, usize)>;

/// Runs the annealing pass, returning `(temperatures, moves, accepted)`.
fn anneal(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    macros: &[Macro],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    options: &PlaceOptions,
    placement: &mut Placement,
) -> (u32, u64, u64) {
    let movable: Vec<usize> = (0..netlist.instances.len())
        .filter(|i| info[*i].fixed.is_none())
        .collect();
    if movable.is_empty() {
        return (0, 0, 0);
    }
    let mut rng = Rng::new(options.seed);
    let inner = options
        .moves_per_temperature
        .unwrap_or_else(|| moves_for(movable.len()));

    // The starting temperature is the spread of the cost changes a
    // random walk sees, which is the standard way of making one schedule
    // fit every design size.
    let mut samples = Vec::new();
    let mut probe = placement.clone();
    for _ in 0..inner.min(100) {
        if let Some(candidate) = propose(
            netlist,
            graph,
            info,
            macros,
            sites_by_kind,
            &movable,
            &mut rng,
            &probe,
        ) {
            let (delta, _) = apply(netlist, graph, &mut probe, &candidate);
            samples.push(delta);
        }
    }
    let mut temperature = 20.0 * stddev(&samples).max(1.0);

    let mut current = hpwl(netlist, graph, placement) as f64;
    // Annealing accepts uphill moves on purpose, so where it stops is not
    // where it was best. Keeping the best placement seen makes the pass
    // monotone: it can only improve on what legalisation produced.
    let mut best = placement.clone();
    let mut best_cost = current;
    let mut tried = 0u64;
    let mut accepted = 0u64;
    let mut steps = 0u32;
    while steps < options.max_temperatures {
        let signals = netlist.signals.len().max(1) as f64;
        if temperature < 0.005 * current / signals {
            break;
        }
        for _ in 0..inner {
            let Some(candidate) = propose(
                netlist,
                graph,
                info,
                macros,
                sites_by_kind,
                &movable,
                &mut rng,
                placement,
            ) else {
                continue;
            };
            tried += 1;
            let (delta, previous) = apply(netlist, graph, placement, &candidate);
            if delta <= 0.0 || rng.unit() * (temperature + delta) < temperature {
                accepted += 1;
                current += delta;
                if current < best_cost {
                    best_cost = current;
                    best = placement.clone();
                }
            } else {
                undo(placement, &previous);
            }
        }
        temperature *= options.cooling;
        steps += 1;
    }
    *placement = best;
    (steps, tried, accepted)
}

/// Moves to try per temperature: the usual `10 * n^(4/3)`, computed in
/// integers.
///
/// `powf` is not guaranteed to give the same last bit on every platform,
/// and a golden placement has to, so the exponent is taken as an integer
/// fourth power and an integer cube root instead.
fn moves_for(instances: usize) -> usize {
    let fourth = (instances as u128).pow(4);
    let root = cube_root(fourth);
    usize::try_from(root.saturating_mul(10))
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

/// Proposes a move: a macro relocation, a move to a free site, or a swap.
#[allow(clippy::too_many_arguments, reason = "the annealer's whole state")]
fn propose(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    macros: &[Macro],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    movable: &[usize],
    rng: &mut Rng,
    placement: &Placement,
) -> Option<Move> {
    let instance = movable[rng.below(movable.len())];
    if let Some(index) = info[instance].macro_index {
        let m = &macros[index];
        let ax = u32::try_from(rng.below(graph.width as usize)).unwrap_or(0);
        let ay = u32::try_from(rng.below(graph.height as usize)).unwrap_or(0);
        let mut free = placement.clone();
        for (member, _, _) in &m.members {
            free.unplace(*member);
        }
        return macro_sites(netlist, graph, info, m, sites_by_kind, &free, ax, ay);
    }
    let kind = &netlist.instances[instance].kind;
    let sites = sites_by_kind.get(kind)?;
    let target = sites[rng.below(sites.len())];
    if placement.site_of(instance) == Some(target) {
        return None;
    }
    let (x, y) = graph.sites[target].tile;
    if let Some((rect, _)) = &info[instance].region
        && !rect.holds(x, y)
    {
        return None;
    }
    match placement.instance_at(target) {
        None => Some(vec![(instance, target)]),
        Some(other) => {
            if info[other].fixed.is_some() || info[other].macro_index.is_some() {
                return None;
            }
            let back = placement.site_of(instance)?;
            let (bx, by) = graph.sites[back].tile;
            if let Some((rect, _)) = &info[other].region
                && !rect.holds(bx, by)
            {
                return None;
            }
            Some(vec![(instance, target), (other, back)])
        }
    }
}

/// The cost of the signals a move touches, before it is applied.
fn cost_of(netlist: &Netlist, graph: &RoutingGraph, placement: &Placement, m: &Move) -> f64 {
    let mut signals: Vec<usize> = Vec::new();
    for (instance, _) in m {
        for signal in signals_of(netlist, *instance) {
            if !signals.contains(&signal) {
                signals.push(signal);
            }
        }
    }
    signals
        .iter()
        .map(|s| signal_hpwl(netlist, graph, placement, *s) as f64)
        .sum()
}

/// Applies a move, returning the change in wirelength and where each
/// instance it touched came from.
fn apply(
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &mut Placement,
    m: &Move,
) -> (f64, Vec<(usize, Option<usize>)>) {
    let before = cost_of(netlist, graph, placement, m);
    let previous: Vec<(usize, Option<usize>)> = m
        .iter()
        .map(|(instance, _)| (*instance, placement.site_of(*instance)))
        .collect();
    for (instance, _) in m {
        placement.unplace(*instance);
    }
    for (instance, site) in m {
        placement.place(*instance, *site);
    }
    (cost_of(netlist, graph, placement, m) - before, previous)
}

/// Puts back what [`apply`] moved.
fn undo(placement: &mut Placement, previous: &[(usize, Option<usize>)]) {
    for (instance, _) in previous {
        placement.unplace(*instance);
    }
    for (instance, site) in previous {
        if let Some(site) = site {
            placement.place(*instance, *site);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::arch::{Arch, BelDecl, TileType, WireDecl, WireRef};
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
