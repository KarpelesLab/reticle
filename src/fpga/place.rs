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
//! - **Dedicated wiring** makes a macro of its own. A sink pin that the
//!   routing graph can reach from only one site of its driver's kind — a
//!   7-series carry in, fed only from the carry out of the slice below —
//!   ties the two cells' sites together, and the cells so tied move as
//!   one rigid group onto exact sites. It is read off the graph, not
//!   named: see `build_clusters`. Nothing in it is about the 7 series,
//!   and that is measured rather than hoped for — the same code holds an
//!   ECP5 `CCU2C` chain to slices A, B, C, D of a tile and then the tile
//!   one column east, which is a different shape of chain on a different
//!   family. A group it cannot fit anywhere is
//!   [`PlaceError::NoDedicatedRun`], which is not the same thing as
//!   running out of sites.
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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::rc::Rc;

use super::arch::{Arch, NodeId, RoutingGraph};
use super::constraints::{Constraints, matches_glob};
use super::device::{BelRole, Device};
use crate::ir::emit::{BitView, SigBit};
use crate::ir::{CellId, CellKind, Design, ModuleId, ModuleRef};

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
    /// An instance is left in the module. The netlist holds only cells,
    /// so reading on would leave the instance's outputs driven by nothing
    /// and route a design that is not the one written.
    Instance {
        /// The instance's name.
        instance: String,
        /// The module or primitive it instantiates.
        module: String,
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
    /// There are sites of the kind enough for the design, and none left
    /// that this cell may legally take: every free one shares a pin, or a
    /// pool of its tile's own wires, with something already placed that
    /// wants a different signal there.
    NoLegalSite {
        /// The cell.
        cell: String,
        /// The site kind.
        kind: String,
        /// How many the design wants.
        needed: usize,
        /// How many the part has.
        available: usize,
        /// The sites still free that it could have taken, counted by the
        /// rule that refused each, most common first.
        refused: Vec<(String, usize)>,
    },
    /// A package pin the architecture cannot place: either it maps to no
    /// site, or the site it maps to is not one this cell can use.
    NoPinSite {
        /// The cell's name.
        cell: String,
        /// The package pin it is constrained to.
        pin: String,
        /// Why, when the architecture says
        /// ([`Arch::unplaceable_pins`](super::arch::Arch::unplaceable_pins)).
        /// A loader that knows the ball sits in a tile type it has no
        /// tables for says so here, so the message names the tile and
        /// not only the ball.
        why: Option<String>,
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
    /// A group of cells that dedicated wiring holds rigid found nowhere
    /// on the part that fits it.
    ///
    /// This is the rigid group's own error, and it is a different thing
    /// from running out of sites: the part may have thousands free and
    /// still have no run of *this shape* left, because such a group's
    /// cells cannot be spread. The wiring between them has no mux on it,
    /// so each member is on the one site its neighbour reaches or there
    /// is no path at all. A carry chain is the case that occurs: on an
    /// ECP5 a run is four slices of a logic tile and then the tile one
    /// column east, so the longest possible run is one row of the die.
    /// `build_clusters` is where the relationship is read off the fabric.
    NoDedicatedRun {
        /// The group's anchor cell, which names it.
        cell: String,
        /// The kind of site the anchor needs.
        kind: String,
        /// How many cells are in the group.
        members: usize,
        /// How many sites of the anchor's kind the part has.
        available: usize,
    },
    /// The netlist could not be read bit by bit, from the emitter's
    /// bit-level view.
    Netlist(String),
}

impl fmt::Display for PlaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaceError::NoDedicatedRun {
                cell,
                kind,
                members,
                available,
            } => write!(
                f,
                "`{cell}` is one of {members} cell(s) that the fabric's own dedicated wiring \
                 holds in one rigid group, and no run of that shape is free; the part has \
                 {available} `{kind}` site(s), but such a group cannot be spread — the wires \
                 between its members have no mux on them, so each cell must sit on the one site \
                 the cell before it reaches"
            ),
            PlaceError::NoSuchModule => f.write_str("the module is not part of the design"),
            PlaceError::NotAPrimitive { cell, kind } => write!(
                f,
                "`{cell}` is a generic `{kind}` cell, not a primitive: run the target flow first"
            ),
            PlaceError::UnknownPrimitive { cell, primitive } => write!(
                f,
                "`{cell}` instantiates `{primitive}`, which the device database does not declare"
            ),
            PlaceError::Instance { instance, module } => write!(
                f,
                "`{instance}` is an instance of `{module}`, which place and route cannot hold: \
                 the flow does not yet turn an instantiated primitive into a cell, so write the \
                 logic instead, or flatten the design if `{module}` is a module of it"
            ),
            PlaceError::NoSites {
                kind,
                needed,
                available,
            } => write!(
                f,
                "the design needs {needed} `{kind}` site(s) and the part has {available}"
            ),
            PlaceError::NoLegalSite {
                cell,
                kind,
                needed,
                available,
                refused,
            } => {
                write!(
                    f,
                    "`{cell}` found no `{kind}` site it may take: the design needs {needed} of \
                     the {available} the part has, and every one still free shares a pin or a \
                     tile's own wires with something placed that wants another signal there"
                )?;
                let free: usize = refused.iter().map(|(_, n)| n).sum();
                if free > 0 {
                    write!(f, "; of the {free} free:")?;
                    for (i, (why, n)) in refused.iter().enumerate() {
                        let sep = if i == 0 { " " } else { ", " };
                        write!(f, "{sep}{n} because {why}")?;
                    }
                }
                Ok(())
            }
            PlaceError::NoPinSite { cell, pin, why } => match why {
                Some(why) => write!(
                    f,
                    "`{cell}` is constrained to package pin `{pin}`, which the architecture \
                     maps to no usable site: {why}"
                ),
                None => write!(
                    f,
                    "`{cell}` is constrained to package pin `{pin}`, which the architecture maps to no usable site"
                ),
            },
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
    /// [`PlaceError::NotAPrimitive`], [`PlaceError::UnknownPrimitive`] and
    /// [`PlaceError::Instance`] when the netlist is not one the device
    /// could hold, and
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
        if let Some((_, instance)) = m.instances.iter().next() {
            return Err(PlaceError::Instance {
                instance: instance.name.as_str().to_owned(),
                module: match &instance.module {
                    ModuleRef::Resolved(id) => design
                        .modules
                        .get(*id)
                        .map_or_else(String::new, |m| m.name.as_str().to_owned()),
                    ModuleRef::Unresolved(name) => name.as_str().to_owned(),
                },
            });
        }
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
                let mut roles_of_port: Vec<Option<String>> = bases
                    .iter()
                    .filter(|(p, _)| *p == port)
                    .map(|(_, b)| Some(b.clone()))
                    .collect();
                if roles_of_port.is_empty() {
                    roles_of_port.push(None);
                }
                for base in roles_of_port {
                    let width = u32::try_from(bits.len()).unwrap_or(u32::MAX);
                    for (index, bit) in bits.iter().enumerate() {
                        let index = u32::try_from(index).unwrap_or(u32::MAX);
                        let role = match &base {
                            // A one-bit port's pin is usually the base with no
                            // index at all -- a flip-flop's `d`, a block RAM's
                            // `we` -- and the architecture is the only thing
                            // that knows which. Asking it, rather than reading
                            // the width, is the whole of this: a `DP16KD` in
                            // 1-bit mode has a one-bit `DOB` whose pin is
                            // still `p0_dout0`, so dropping the index named a
                            // role the fabric has no wire for, the pin went to
                            // `off_fabric`, and the net was left with nothing
                            // driving it. See [`Netlist::driverless`] for what
                            // that costs on a part.
                            Some(base)
                                if width == 1 && roles.contains(&(kind.clone(), base.clone())) =>
                            {
                                base.clone()
                            }
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

    /// The signals something reads and nothing drives, one sentence each.
    ///
    /// Such a signal is not [routable](Signal::is_routable), so the router
    /// never looks at it, and `routed N of M signal(s)` counts it in the
    /// M beside every signal nothing reads — which is why it has to be
    /// said separately rather than inferred from that line.
    ///
    /// It is not harmless and it is not visible in the bitstream. An
    /// unrouted interconnect input on an ECP5 reads as a **one** at every
    /// sink and a 7-series one reads as a zero, so the pin takes a value
    /// the netlist never asked for while every structural check
    /// downstream still passes: the routing verifies, every set bit
    /// decodes, and the arcs are exactly the arcs the router chose.
    /// `testdata/fpga/cynthion/bram_echo.v` read `ff` for all 32768 of
    /// its bytes on a part because of twenty of these, and the only
    /// number that differed from a build that worked was the M.
    pub fn driverless(&self) -> Vec<String> {
        let mut out = Vec::new();
        for signal in &self.signals {
            if signal.driver.is_some() || signal.sinks.is_empty() {
                continue;
            }
            // The readers, named the way a person reading Verilog would
            // find them. One is the common case and all of them would be
            // a wall of text, so the rest are counted.
            let mut readers: Vec<String> = signal
                .sinks
                .iter()
                .map(|sink| {
                    let pin = &self.pins[*sink];
                    format!(
                        "{}.{}[{}]",
                        self.instances[pin.instance].name, pin.port, pin.bit
                    )
                })
                .collect();
            readers.sort();
            readers.dedup();
            let how_many = readers.len();
            let first = readers.first().map_or_else(String::new, Clone::clone);
            let rest = match how_many {
                0 | 1 => String::new(),
                2 => " and 1 other pin".to_owned(),
                n => format!(" and {} other pin(s)", n - 1),
            };
            out.push(format!(
                "`{}` is read by {first}{rest} and nothing drives it",
                signal.name
            ));
        }
        out
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
                // A port may reach more than one pin of its site: a
                // `BUFGMUX`'s `S` is both `CE0` and `CE1` of the
                // `BUFGCTRL` it is (`port ce0=S ce1=S`), and each pin is
                // routed to on its own.
                if !bases.iter().any(|(p, b)| p == name && *b == base) {
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
    // A clock generator: its `pll` line names the clock ports by role
    // (`ref`, `out`, `fb`, `fbout`, `lock`), and the pins it ties to a
    // constant (a reset, a power-down) keep their own name, lowercased,
    // so that a fabric which can route one says which wire it is on.
    for pll in device
        .clock_resources
        .plls
        .iter()
        .filter(|p| p.name == primitive)
    {
        kind = Some(BelRole::Pll.keyword().to_owned());
        for (role, name) in &pll.ports {
            if !bases.iter().any(|(p, _)| p == name) {
                bases.push((name.clone(), role.clone()));
            }
        }
        for (name, _) in &pll.ties {
            if !bases.iter().any(|(p, _)| p == name) {
                bases.push((name.clone(), name.to_lowercase()));
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
    /// undoes that.
    ///
    /// **It is a floor, not the start.** The window starts at least as
    /// wide as legalisation moved a cell on average
    /// ([`PlacementReport::displacement`]), because a cell the legaliser
    /// sent ten tiles from where it belongs cannot come back one tile at a
    /// time. The limiter was meant to widen a window that is too narrow
    /// and in practice does not: on the ChipWhisperer-Husky — 95 % of its
    /// lookup tables used, 98 pins fixed, the solve's cells piled at the
    /// middle and the legaliser spreading them outward — the window was
    /// one tile at the first temperature and one at the last, and the
    /// anneal took the wirelength from 4,176,953 to 3,948,882. Started at
    /// sixteen tiles it reached 1,752,306, at forty-eight 1,639,339, and at
    /// the whole die 1,710,199. A design the solve placed well is legalised
    /// a tile or less away and starts where it always did.
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
    /// What makes the placement timing-driven, or `None` for one that
    /// minimises wirelength alone; see [`PlaceTiming`].
    pub timing: Option<PlaceTiming>,
    /// The nodes of a fabric's dedicated roads into its global buffers, by
    /// node id, or empty for none: a global buffer (`gb`) whose input a
    /// package pin's buffer drives is confined to the buffer sites that
    /// pad reaches over these alone. See `confine_to_roads`.
    pub clock_roads: Vec<bool>,
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
            timing: None,
            clock_roads: Vec::new(),
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
    /// The site kinds `global_place` spread before legalisation, as
    /// `(kind, cells)`.
    pub spread: Vec<(String, usize)>,
    /// How far legalisation moved the cells that were free to move from
    /// where the analytic solve put them, as `(tiles in all, cells)`. The
    /// anneal starts with a move window at least as wide as the average;
    /// see [`PlaceOptions::start_window`].
    pub displacement: (u64, u64),
    /// Instances held at a package pin.
    pub fixed: usize,
    /// Instances confined to a region, including the ones
    /// `keep_hierarchy` confined.
    pub confined: usize,
    /// Relative-placement macros, and how many instances are in them.
    /// Groups held together by dedicated wiring are counted here too.
    pub macros: (usize, usize),
    /// Of those, the groups held together by dedicated wiring rather than
    /// by a constraint; see `build_clusters`.
    pub dedicated: usize,
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
    /// What each round of a timing-driven placement estimated, the
    /// wirelength placement's first: empty when [`PlaceOptions::timing`] is
    /// `None`.
    pub timing: Vec<TimingRound>,
}

/// One round of [`PlaceOptions::timing`]: the estimate the placement it
/// started from gave.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimingRound {
    /// Per domain of [`PlaceTiming::periods`], the longest estimated path,
    /// in picoseconds.
    pub longest_ps: Vec<u64>,
    /// How many signals had a connection over half critical.
    pub critical: usize,
    /// The half-perimeter wirelength.
    pub hpwl: u64,
}

impl PlacementReport {
    /// [`PlacementReport::displacement`] as an average, in tiles.
    pub fn mean_displacement(&self) -> f64 {
        let (total, cells) = self.displacement;
        if cells == 0 {
            0.0
        } else {
            total as f64 / cells as f64
        }
    }

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
        for (kind, cells) in &self.spread {
            let _ = writeln!(out, "  spread {cells} {kind} cell(s) before legalising");
        }
        let _ = writeln!(
            out,
            "  legalisation moved a cell {:.1} tile(s) on average",
            self.mean_displacement()
        );
        if self.fixed > 0 || self.confined > 0 || self.macros.0 > 0 {
            let _ = writeln!(
                out,
                "  constrained: {} at a package pin, {} in a region, {} macro(s) over {} cell(s)",
                self.fixed, self.confined, self.macros.0, self.macros.1
            );
        }
        if self.dedicated > 0 {
            let _ = writeln!(
                out,
                "  dedicated wiring: {} of those macro(s) nobody asked for",
                self.dedicated
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
    /// Per instance, the signal on each of its pins, as `(role, signal,
    /// whether the instance drives it)`, for the pins that carry one. A
    /// role that is absent carries none.
    per_instance: Vec<Vec<(u32, usize, bool)>>,
    /// Every role name that takes part, so the checks compare integers.
    roles: BTreeMap<String, u32>,
    /// Per site, one entry per pin that draws on a contended pool of the
    /// tile's own wires, as `(role, pool, which wires of it)`. `pool`
    /// indexes [`ControlGroup::pools`] of the site's group; the mask's bit
    /// *n* is that pool's *n*th wire. Empty for every site of every family
    /// whose bels do not share a control wire.
    captive: Vec<Vec<(u32, u32, u32)>>,
    /// Per site, one entry per output pin of another site of its tile
    /// that may not carry a signal other than this one's, as `(the other
    /// site, this site's role, the other's role)`. Both directions are
    /// recorded. See [`exits_of`] for which pins pair.
    exits: Vec<Vec<(usize, u32, u32)>>,
    /// Per instance, the slice-wide mode its primitive needs, or zero for
    /// none; see [`slice_mode`].
    mode: Vec<u8>,
    /// Per signal, whether every reader is in its driver's rigid group
    /// and so never leaves the tile, by signal index; empty for none. Set
    /// once the groups are known: see [`SiteRules::exit_shared`].
    contained: Vec<bool>,
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
    /// Per pool, the wires that are a bel's output — nothing routes into
    /// them, and a pin on them is one the netlist drives, or none on them
    /// is one it reads: a lookup table's `O6`, and its `O5` whether or not
    /// any table in the design uses it — one bit per wire as in
    /// [`SiteRules::captive`]'s masks. Such a wire carries what its bel
    /// drives and nothing else, so only that signal may be given it: a
    /// 7-series flip-flop's data mux reads `O6` or the bypass `<L>X`, and
    /// counted as a wire like any other, an empty lookup table's `O6`
    /// looked like room for a flip-flop fed from elsewhere. Offering the
    /// mux `O5` as well made every single-output table such room, and the
    /// placer filled slices the router then could not.
    sources: Vec<u32>,
    /// Per pool, every pin that draws on it, as `(site, role, which wires
    /// of it)`: [`SiteRules::captive`] turned round, so that a check of one
    /// pool reads its own handful of pins and not every pin of the tile.
    entries: Vec<Vec<(usize, u32, u32)>>,
}

/// How many wires a pool may hold, and how far the walk that finds one may
/// go, before a pin is treated as unconstrained.
///
/// A control wire is one of two or four. A lookup table's input is one of
/// two dozen interconnect wires, and stopping there costs nothing but the
/// look: a pool nobody else in the tile draws on is dropped anyway.
const MAX_POOL: usize = 8;

/// The mode a storage primitive needs of the sites it shares its control
/// pins with, as a small number, or zero for a primitive that needs none.
///
/// A 7-series slice has one `FFSYNC` and one `CLKINV` for all eight of its
/// flip-flops (`xray::sites::slice_pass_throughs`), so an `FDCE` beside an
/// `FDRE` gets a synchronous reset, and an `FDRE_1` beside an `FDRE` an
/// inverted clock: the bits of the two are OR-ed, and one of them is
/// wrong with nothing to say so. NewAE's ChipWhisperer-Husky has 505
/// asynchronous flip-flops and 576 on a falling edge among 27,618. The
/// primitives are named here because the device file does not say which
/// share a field; any other family's flip-flops get zero, and are placed
/// as before.
fn slice_mode(primitive: &str) -> u8 {
    let (base, inverted) = match primitive.strip_suffix("_1") {
        Some(base) => (base, true),
        None => (primitive, false),
    };
    match (base, inverted) {
        ("FDRE" | "FDSE", false) => 1,
        ("FDRE" | "FDSE", true) => 2,
        ("FDCE" | "FDPE", false) => 3,
        ("FDCE" | "FDPE", true) => 4,
        _ => 0,
    }
}

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

/// Where an output pin's signal can leave `tile`: walked forward from the
/// pin through the tile's own wires, the wires with a way out of the tile
/// (the exits), and whether the walk also reaches a bel input inside the
/// tile — a wire nothing leads on from — which an output with somewhere
/// else to go does. `None` for a pin something routes into, which is not
/// an output, and for a walk wider than [`MAX_POOL`].
///
/// A 7-series slice's five-input flip-flop is the case this is for: its
/// `Q` reaches nothing but the output mux `<L>MUX`, and so do a `LUT6_2`'s
/// `O5` and the carry's sum, which have the mux as their only way out of
/// the tile. Two of those in one lane cannot both leave it, and nothing
/// the router does can make them; [`SiteRules::exits`] pairs a pin that
/// can only leave by one wire with every other output whose one way out
/// is that wire. An `O6`, which also leaves by its own wire, pairs with
/// nothing, and two outputs that each have somewhere inside the tile to
/// go are left alone, since either may be read only there: the carry's
/// sum and the `O5` its lane's generate rides on are such a pair.
fn exits_of(graph: &RoutingGraph, tile: (u32, u32), pin: NodeId) -> Option<(Vec<NodeId>, bool)> {
    let local = |node: NodeId| {
        let wire = graph.wire(node);
        !wire.global && wire.tile == tile
    };
    if !local(pin) || !graph.incoming(pin).is_empty() {
        return None;
    }
    let mut seen: Vec<NodeId> = vec![pin];
    let mut work: Vec<NodeId> = vec![pin];
    let mut exits: Vec<NodeId> = Vec::new();
    let mut inside = false;
    while let Some(node) = work.pop() {
        let ways = graph.outgoing(node);
        if ways.is_empty() {
            inside |= node != pin;
            continue;
        }
        if ways.iter().any(|pip| !local(graph.pip(*pip).to)) {
            exits.push(node);
        }
        for pip in ways {
            let to = graph.pip(*pip).to;
            if local(to) && !seen.contains(&to) {
                if seen.len() >= MAX_POOL {
                    return None;
                }
                seen.push(to);
                work.push(to);
            }
        }
    }
    exits.sort_unstable();
    Some((exits, inside))
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
            mode: netlist
                .instances
                .iter()
                .map(|i| slice_mode(&i.primitive))
                .collect(),
            roles: BTreeMap::new(),
            captive: vec![Vec::new(); graph.sites.len()],
            exits: vec![Vec::new(); graph.sites.len()],
            contained: Vec::new(),
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
        // Which `(kind, role)` pins the netlist drives and which it reads:
        // the routing graph says where a pin is and not which way it
        // points, and a wire nothing routes into is a bel's output only
        // when its pins are.
        let mut driven: HashSet<(&str, &str)> = HashSet::new();
        let mut read: HashSet<(&str, &str)> = HashSet::new();
        for pin in &netlist.pins {
            let kind = netlist.instances[pin.instance].kind.as_str();
            if pin.output {
                driven.insert((kind, pin.role.as_str()));
            } else {
                read.insert((kind, pin.role.as_str()));
            }
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
            // Output pins that leave the tile by one wire only: a pin that
            // can do nothing else and another whose one exit is that wire
            // may not carry two signals. See [`exits_of`].
            let mut outputs: Vec<(usize, &str, Vec<NodeId>, bool)> = Vec::new();
            for site in sites {
                let kind = graph.sites[*site].kind.as_str();
                for (role, node) in &graph.sites[*site].pins {
                    if !driven.contains(&(kind, role.as_str())) {
                        continue;
                    }
                    if let Some((exits, inside)) = exits_of(graph, *tile, *node) {
                        outputs.push((*site, role.as_str(), exits, inside));
                    }
                }
            }
            for (site, role, exits, inside) in &outputs {
                if *inside || exits.len() != 1 {
                    continue;
                }
                for (other, theirs, their_exits, _) in &outputs {
                    if other == site || their_exits != exits {
                        continue;
                    }
                    let (mine, theirs) = (
                        role_id(&mut out.roles, role),
                        role_id(&mut out.roles, theirs),
                    );
                    if !out.exits[*site].contains(&(*other, mine, theirs)) {
                        out.exits[*site].push((*other, mine, theirs));
                    }
                    if !out.exits[*other].contains(&(*site, theirs, mine)) {
                        out.exits[*other].push((*site, theirs, mine));
                    }
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
            group.sources = group
                .pools
                .iter()
                .map(|pool| {
                    pool.iter()
                        .enumerate()
                        .filter(|(_, node)| {
                            if !graph.incoming(**node).is_empty() {
                                return false;
                            }
                            let on: Vec<(&str, &str)> = sites
                                .iter()
                                .flat_map(|site| {
                                    let site = &graph.sites[*site];
                                    site.pins
                                        .iter()
                                        .filter(|(_, n)| n == *node)
                                        .map(|(role, _)| (site.kind.as_str(), role.as_str()))
                                })
                                .collect();
                            on.iter().any(|pin| driven.contains(pin))
                                || !on.iter().any(|pin| read.contains(pin))
                        })
                        .fold(0u32, |mask, (bit, _)| mask | 1 << bit)
                })
                .collect();
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
                if group.entries.len() < group.pools.len() {
                    group.entries.resize(group.pools.len(), Vec::new());
                }
                group.entries[which].push((*site, role, mask));
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
                out.per_instance[pin.instance].push((*role, signal, pin.output));
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
            .find(|(r, _, _)| *r == role)
            .map(|(_, s, _)| *s)
    }

    /// Whether two outputs whose one way out of the tile is the same wire
    /// can both be placed: unless one of them carries nothing, they must
    /// carry the same signal.
    ///
    /// A signal that never leaves the tile needs no exit: a `LUT6_2`'s
    /// lower half read only by the five-input flip-flop of its lane, which
    /// [`registered_tables`] put there, shares nothing with that
    /// flip-flop's `Q` on the output mux.
    fn exit_shared(&self, a: usize, a_role: u32, b: usize, b_role: u32) -> bool {
        let inside = |signal: usize| self.contained.get(signal).copied().unwrap_or(false);
        match (self.signal(a, a_role), self.signal(b, b_role)) {
            (Some(x), Some(y)) => x == y || inside(x) || inside(y),
            _ => true,
        }
    }

    /// Whether two instances on sites that share a pin agree on the mode
    /// their sites share: either needs none, or both the same.
    fn same_mode(&self, a: usize, b: usize) -> bool {
        let (a, b) = (self.mode[a], self.mode[b]);
        a == 0 || b == 0 || a == b
    }

    /// Marks each signal whose readers are all in its driver's rigid
    /// group, which [`SiteRules::exit_shared`] lets off.
    fn contain(&mut self, netlist: &Netlist, info: &[Placeable], macros: &[Macro]) {
        self.contained = netlist
            .signals
            .iter()
            .map(|signal| {
                let Some(group) = signal
                    .driver
                    .and_then(|d| info[netlist.pins[d].instance].macro_index)
                else {
                    return false;
                };
                macros[group].exact.is_some()
                    && !signal.sinks.is_empty()
                    && signal
                        .sinks
                        .iter()
                        .all(|p| info[netlist.pins[*p].instance].macro_index == Some(group))
            })
            .collect();
    }

    /// [`SiteRules::signal`], and whether the instance drives it.
    fn signal_driven(&self, instance: usize, role: u32) -> Option<(usize, bool)> {
        self.per_instance[instance]
            .iter()
            .find(|(r, _, _)| *r == role)
            .map(|(_, s, out)| (*s, *out))
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
                        self.same_mode(*instance, neighbour)
                            && self.pairs[run].iter().all(|(mine, theirs)| {
                                self.signal(*instance, *mine) == self.signal(neighbour, *theirs)
                            })
                    })
                })
                && self.exits[*site].iter().all(|(other, mine, theirs)| {
                    after(*other).is_none_or(|neighbour| {
                        self.exit_shared(*instance, *mine, neighbour, *theirs)
                    })
                })
        });
        if !pinned {
            return false;
        }
        // Every pool a moved cell arrives at **or leaves**. Leaving matters
        // as much as arriving: a lookup table that goes takes its `O6` with
        // it, and the flip-flop beside it that read that `O6` now needs the
        // bypass wire, which a carry's generate may hold. Only the tiles
        // arrived at used to be checked, and a whole tile at that — every
        // pool of it, twenty on a 7-series logic tile, for a move that
        // touches two or three.
        let mut touched: [(u32, u32); 16] = [(0, 0); 16];
        let mut many: Vec<(u32, u32)> = Vec::new();
        let mut count_touched = 0usize;
        let origins = moving.iter().filter_map(|(i, _)| placement.site_of(*i));
        for site in moving.iter().map(|(_, s)| *s).chain(origins) {
            let group = self.site_group[site];
            if group == u32::MAX {
                continue;
            }
            for (_, pool, _) in &self.captive[site] {
                let key = (group, *pool);
                if touched[..count_touched].contains(&key) || many.contains(&key) {
                    continue;
                }
                if count_touched < touched.len() {
                    touched[count_touched] = key;
                    count_touched += 1;
                } else {
                    many.push(key);
                }
            }
        }
        touched[..count_touched]
            .iter()
            .chain(&many)
            .all(|(group, pool)| self.pool_fits(&self.groups[*group as usize], *pool, &after))
    }

    /// Whether `instance` on `site` would put a signal on a pin shared
    /// with other sites, none of which is taken: it opens the group, and
    /// sets that signal for the cells that come after. False for a site
    /// that shares nothing, and for a cell that uses none of the pins it
    /// shares.
    fn opens(&self, placement: &Placement, instance: usize, site: usize) -> bool {
        if self.trivial() {
            return false;
        }
        let mut uses = false;
        for (other, start, len) in &self.per_site[site] {
            let run = *start as usize..(*start + *len) as usize;
            if !self.pairs[run]
                .iter()
                .any(|(mine, _)| self.signal(instance, *mine).is_some())
            {
                continue;
            }
            if placement.instance_at(*other).is_some() {
                return false;
            }
            uses = true;
        }
        uses
    }

    /// Why `instance` may not take the free `site`, in words, or `None`
    /// when it may: the first rule of [`SiteRules::allows`] that refuses
    /// it, naming the pin. For the error a legaliser that ran out of
    /// sites reports, which was "every one is refused" and left the
    /// question of *which rule* to a debugger.
    fn refusal(&self, placement: &Placement, instance: usize, site: usize) -> Option<String> {
        if self.trivial() {
            return None;
        }
        let after = |at: usize| -> Option<usize> {
            if at == site {
                return Some(instance);
            }
            placement.instance_at(at).filter(|i| *i != instance)
        };
        let name = |role: u32| -> &str {
            self.roles
                .iter()
                .find(|(_, id)| **id == role)
                .map_or("?", |(name, _)| name.as_str())
        };
        if self.excludes[site]
            .iter()
            .any(|other| after(*other).is_some())
        {
            return Some("a site it excludes is taken".to_owned());
        }
        for (other, start, len) in &self.per_site[site] {
            let Some(neighbour) = after(*other) else {
                continue;
            };
            if !self.same_mode(instance, neighbour) {
                return Some(
                    "a flip-flop of another reset kind or clock edge shares its pins".to_owned(),
                );
            }
            let run = *start as usize..(*start + *len) as usize;
            if let Some((mine, _)) = self.pairs[run].iter().find(|(mine, theirs)| {
                self.signal(instance, *mine) != self.signal(neighbour, *theirs)
            }) {
                return Some(format!(
                    "its `{}` pin is shared and wanted otherwise",
                    name(*mine)
                ));
            }
        }
        for (other, mine, theirs) in &self.exits[site] {
            if let Some(neighbour) = after(*other)
                && !self.exit_shared(instance, *mine, neighbour, *theirs)
            {
                return Some(format!(
                    "its `{}` leaves the tile by a wire another output needs",
                    name(*mine)
                ));
            }
        }
        let group = self.site_group[site];
        if group == u32::MAX {
            return None;
        }
        let group = &self.groups[group as usize];
        for (pool, _) in group.pools.iter().enumerate() {
            let pool = u32::try_from(pool).unwrap_or(u32::MAX);
            if !self.pool_fits(group, pool, &after) {
                let roles: Vec<&str> = self.captive[site]
                    .iter()
                    .filter(|(_, which, _)| *which == pool)
                    .map(|(role, _, _)| name(*role))
                    .collect();
                return Some(format!(
                    "the tile has no wire left for its `{}`",
                    roles.join("`/`")
                ));
            }
        }
        None
    }

    /// Whether one pool of a tile can give each of the signals that want
    /// it a wire of its own.
    ///
    /// The question is a bipartite matching and it is answered as one:
    /// signals on one side, the pool's wires on the other, an edge where a
    /// pin allows that wire. Two pins wanting the *same* signal share a
    /// wire and cost nothing, which is the whole reason a flip-flop may sit
    /// beside a distributed RAM whose write clock it shares; a pin with no
    /// signal on that role — a flip-flop with no reset — asks for nothing.
    fn pool_fits(
        &self,
        group: &ControlGroup,
        pool: u32,
        after: &impl Fn(usize) -> Option<usize>,
    ) -> bool {
        let entries = &group.entries[pool as usize];
        count(
            |work| &mut work.legality_steps,
            u64::try_from(entries.len()).unwrap_or(0),
        );
        // Per signal, the wires every pin reading it would accept, and the
        // source wires it is driven onto; on the stack, since this is asked
        // on every move, and a pool's pins are a handful.
        let mut stack = [(0usize, 0u32, 0u32); 32];
        let mut heap: Vec<(usize, u32, u32)> = Vec::new();
        let mut len = 0usize;
        for (site, role, mask) in entries {
            let Some(instance) = after(*site) else {
                continue;
            };
            let Some((signal, drives)) = self.signal_driven(instance, *role) else {
                continue;
            };
            // A pin the instance drives puts the signal on its wire; one
            // it reads narrows the wires it may come in on. They are kept
            // apart: a lookup table's `O5` and a flip-flop it feeds in the
            // same lane are one signal on two wires, out through the output
            // mux and back through the bypass input.
            let (sinks, driven) = if drives {
                (u32::MAX, *mask)
            } else {
                (*mask, 0)
            };
            let spilled = !heap.is_empty();
            let wanted = if spilled {
                &mut heap[..]
            } else {
                &mut stack[..len]
            };
            if let Some((_, allowed, on)) = wanted.iter_mut().find(|(s, _, _)| *s == signal) {
                *allowed &= sinks;
                *on |= driven;
            } else if !spilled && len < stack.len() {
                stack[len] = (signal, sinks, driven);
                len += 1;
            } else {
                if !spilled {
                    heap.extend_from_slice(&stack[..len]);
                }
                heap.push((signal, sinks, driven));
            }
        }
        let wanted = if heap.is_empty() {
            &stack[..len]
        } else {
            &heap[..]
        };
        fits(
            wanted,
            group.sources[pool as usize],
            group.pools[pool as usize].len(),
        )
    }
}

/// Whether each of `wanted`'s signals, as `(signal, the wires every pin
/// reading it would accept — all of them when none reads it — the source
/// wires it is driven onto)`, can reach its readers in a pool of `wires`
/// wires whose `sources` carry only what is driven onto them.
///
/// A signal its readers can take off the wire it is driven onto costs no
/// wire more: a lookup table and the flip-flop beside it reading its `O6`.
/// Any other signal with a reader needs a wire of its own that is not a
/// source, since a source carries its own driver's signal or nothing.
fn fits(wanted: &[(usize, u32, u32)], sources: u32, wires: usize) -> bool {
    // A pool is at most 32 wires; this is asked on every move, so it
    // stays on the stack.
    let mut need = [(0usize, 0u32); 32];
    let mut count = 0usize;
    for (signal, readers, driven) in wanted {
        if *readers == u32::MAX || readers & driven != 0 {
            continue;
        }
        if count == wires.min(32) {
            return false;
        }
        need[count] = (*signal, readers & !sources);
        count += 1;
    }
    matchable(&need[..count])
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
    /// The members, with their offsets from the anchor. For a group held
    /// together by [`Exact`] wiring the offsets are not used and are zero.
    members: Vec<(usize, i32, i32)>,
    /// When the group is held together by dedicated wiring rather than by
    /// an `rloc` constraint: the exact site of every member follows from
    /// the anchor's site, and this says how.
    exact: Option<Exact>,
}

/// Everything the placer knows about one instance beyond the netlist.
#[derive(Clone)]
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
    report.fixed += follow_fixed_ends(netlist, graph, &sites_by_kind, &mut info);
    apply_regions(netlist, graph, constraints, &mut info);
    // After the pin constraints and the regions, because both of them
    // narrow where the *other* end of a global signal can be, and before
    // legalisation, because the answer is a constraint on every site this
    // pass or the annealer may choose.
    confine_to_reachable(netlist, graph, &sites_by_kind, &mut info, &mut report)?;
    confine_to_roads(
        netlist,
        graph,
        &sites_by_kind,
        &mut info,
        &options.clock_roads,
    );
    let mut macros = build_macros(netlist, constraints, &mut info);
    let clusters = build_clusters(netlist, graph, &sites_by_kind, &mut info, macros.len());
    report.dedicated = clusters.len();
    macros.extend(clusters);
    report.macros = (
        macros.len(),
        macros.iter().map(|m| m.members.len()).sum::<usize>(),
    );

    let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
    let mut shared = SiteRules::find(netlist, graph);
    shared.contain(netlist, &info, &macros);
    let mut positions = solve_analytic(netlist, graph, &info, options);
    let dense = dense_kinds(netlist, graph, &info, &macros, &sites_by_kind, &shared);
    // A design dense enough to spread is placed globally instead: see
    // `global_place` for what one solve and one spread did to it.
    report.spread = if dense.is_empty() {
        dense
    } else {
        let (global, spread) = global_place(netlist, graph, &info, &macros, &sites_by_kind);
        positions = global;
        spread
    };
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
    report.displacement = displacement(graph, &info, &positions, &placement);
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
            &Weights::default(),
            &mut placement,
            &mut report,
        );
        report.hpwl_after = hpwl(netlist, graph, &placement);
    }
    if let Some(timing) = &options.timing {
        let order = timing_order(netlist, timing);
        // Each round refines the last: a narrow window and a cool start,
        // so that the weights move the critical cells and the rest stays
        // where the wirelength placement put it.
        let refine = PlaceOptions {
            start_window: Some(TIMING_WINDOW),
            start_acceptance: Some(TIMING_ACCEPTANCE),
            timing: None,
            ..options.clone()
        };
        let mut held = info.clone();
        for (instance, place) in held.iter_mut().enumerate() {
            if timing.frozen.get(instance).copied().unwrap_or(false)
                && let Some(site) = placement.site_of(instance)
            {
                place.allowed = Some(vec![site]);
            }
        }
        for _ in 0..timing.rounds {
            let (critical, round) = estimate_timing(netlist, graph, &placement, timing, &order);
            report.timing.push(TimingRound {
                hpwl: hpwl(netlist, graph, &placement),
                ..round
            });
            let weights = Weights {
                nets: vec![16; netlist.signals.len()],
                pins: connection_weights(&critical.pins, timing),
            };
            if options.anneal {
                anneal(
                    netlist,
                    graph,
                    &held,
                    &macros,
                    &sites_by_kind,
                    &shared,
                    &refine,
                    &weights,
                    &mut placement,
                    &mut report,
                );
            }
        }
        let (_, round) = estimate_timing(netlist, graph, &placement, timing, &order);
        report.timing.push(TimingRound {
            hpwl: hpwl(netlist, graph, &placement),
            ..round
        });
        report.hpwl_after = hpwl(netlist, graph, &placement);
    }
    report.work = WORK.with(Cell::get);
    Ok((placement, report))
}

/// Fixes every instance whose site follows from a fixed neighbour through
/// wiring nothing else reaches, and returns how many it fixed.
///
/// [`build_clusters`] makes a rigid group of two instances that dedicated
/// wiring ties together, and leaves out an instance a package pin fixes:
/// two rigid shapes cannot both be obeyed. But one fixed end decides the
/// other completely when the free end's pin is dedicated, and it is the
/// common case at a pad: a 7-series `IDDR`'s `D` comes only from its own
/// pad's input buffer, and an `ODDR`'s `Q` goes only to its own pad's
/// output buffer, and the buffer is the instance a `set_io` fixes.
///
/// The test, for a link whose one end is fixed and other is free: walk
/// from the free end's pin, on every site of its kind, **away** from the
/// rest of the design — forwards from a driver, backwards from a sink —
/// for at most [`DEDICATED_WALK`] wires. If every such walk closes, the
/// free end's pin reaches nothing but those wires, and the one site whose
/// walk reaches the fixed end's pin is where it must sit. The walk is not
/// [`dedicated_drivers`]'s: that one starts at the *sink* and asks whether
/// the sink is fed only from a few wires, which an output pad is not — it
/// is also the plain path from the interconnect, through the same `OLOGIC`
/// an `ODDR` would occupy. Asking from the `ODDR`'s side instead, its `OQ`
/// reaches its pad and nothing else.
///
/// An instance whose pin walk does not close — a lookup table, whose
/// output reaches the whole interconnect — is never fixed this way, so a
/// family whose IO logic is on the general interconnect places exactly as
/// before. Repeated until nothing changes, so a chain of such links
/// resolves from its fixed end.
fn follow_fixed_ends(
    netlist: &Netlist,
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    info: &mut [Placeable],
) -> usize {
    // The wires a pin reaches going away from the design, or `None` when
    // that is more than a handful.
    let closure = |site: usize, role: &str, forward: bool| -> Option<Vec<NodeId>> {
        let mut seen: Vec<NodeId> = graph.sites[site].pin_nodes(role).collect();
        if seen.is_empty() {
            return None;
        }
        let mut next = 0;
        while next < seen.len() {
            let node = seen[next];
            next += 1;
            let pips = if forward {
                graph.outgoing(node)
            } else {
                graph.incoming(node)
            };
            for pip in pips {
                let pip = graph.pip(*pip);
                let other = if forward { pip.to } else { pip.from };
                if !seen.contains(&other) {
                    seen.push(other);
                    if seen.len() > DEDICATED_WALK {
                        return None;
                    }
                }
            }
        }
        Some(seen)
    };
    let mut taken: BTreeMap<usize, usize> = info
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.fixed.map(|s| (s, i)))
        .collect();
    let mut fixed = 0usize;
    loop {
        let mut changed = false;
        for signal in &netlist.signals {
            let Some(driver) = signal.driver else {
                continue;
            };
            for sink in &signal.sinks {
                let (dpin, spin) = (&netlist.pins[driver], &netlist.pins[*sink]);
                let (x, y) = (dpin.instance, spin.instance);
                if x == y {
                    continue;
                }
                // Which end is free, and which is fixed where.
                let (free, free_role, forward, anchor_site, anchor_role) =
                    match (info[x].fixed, info[y].fixed) {
                        (Some(site), None) => (y, &spin.role, false, site, &dpin.role),
                        (None, Some(site)) => (x, &dpin.role, true, site, &spin.role),
                        _ => continue,
                    };
                if info[free].macro_index.is_some() {
                    continue;
                }
                let anchor: Vec<NodeId> = graph.sites[anchor_site].pin_nodes(anchor_role).collect();
                if anchor.is_empty() {
                    continue;
                }
                let Some(candidates) = sites_by_kind.get(&netlist.instances[free].kind) else {
                    continue;
                };
                let mut found: Option<usize> = None;
                let mut dedicated = true;
                for &site in candidates {
                    match closure(site, free_role, forward) {
                        None => {
                            dedicated = false;
                            break;
                        }
                        Some(reach) => {
                            if reach.iter().any(|n| anchor.contains(n)) {
                                if found.is_some() {
                                    dedicated = false;
                                    break;
                                }
                                found = Some(site);
                            }
                        }
                    }
                }
                let (true, Some(site)) = (dedicated, found) else {
                    continue;
                };
                if taken.contains_key(&site) || !info[free].may_take(site) {
                    continue;
                }
                info[free].fixed = Some(site);
                taken.insert(site, free);
                fixed += 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    fixed
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
        let why = || arch.why_no_pin(pin).map(str::to_owned);
        let Some(site_name) = arch.site_of_pin(pin) else {
            return Err(PlaceError::NoPinSite {
                cell: instance.name.clone(),
                pin: pin.clone(),
                why: why(),
            });
        };
        let Some(site) = graph.site_index(site_name) else {
            return Err(PlaceError::NoPinSite {
                cell: instance.name.clone(),
                pin: pin.clone(),
                why: why(),
            });
        };
        if graph.sites[site].kind != instance.kind {
            return Err(PlaceError::NoPinSite {
                cell: instance.name.clone(),
                pin: pin.clone(),
                why: Some(format!(
                    "it is site `{site_name}`, which holds a `{}` and not a `{}`",
                    graph.sites[site].kind, instance.kind
                )),
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
        macros.push(Macro {
            anchor,
            members,
            exact: None,
        });
    }
    macros
}

// ---------------------------------------------------------------------------
// Dedicated connections
// ---------------------------------------------------------------------------

/// How far [`dedicated_drivers`] walks back from a pin before it decides
/// the pin is reachable from the general interconnect.
///
/// A dedicated connection is a handful of wires: a 7-series carry chain's
/// is four (`CARRY_CI` ← `CIN` ← the slice below's `COUT_N` ← its `COUT`),
/// five where the chain crosses a clock row, and a lookup table feeding the
/// carry beside it is one. A lookup table input reaches two dozen wires in
/// two steps, so a small bound costs a walk nothing and loses nothing.
const DEDICATED_WALK: usize = 8;

/// Where a sink's driver must be, for one kind of dedicated connection:
/// `(sink kind, sink role, driver kind, driver role)`, as two maps that are
/// each other's inverse.
///
/// A sink site that is absent from [`Relation::driver_of`] has no site of
/// the driver's kind that can reach it at all — the bottom slice of a
/// column has no carry coming in — and a member that needs one cannot sit
/// there.
#[derive(Debug, Default)]
struct Relation {
    /// Sink site to the one driver site that can reach it.
    driver_of: SiteMap,
    /// Driver site to the one sink site it reaches.
    sink_of: SiteMap,
}

/// A map from site to site, as an array over every site of the graph.
///
/// A macro's members are found from its anchor through these on every
/// move the annealer proposes for it, several lookups a candidate site;
/// as hash maps they were a fifth of the Husky's placement. There are a
/// handful of relations, one per kind of dedicated connection, so an
/// array of every site each costs a few megabytes on a whole die.
#[derive(Debug, Default)]
struct SiteMap {
    to: Vec<u32>,
    len: usize,
}

impl SiteMap {
    /// The site `site` maps to.
    fn get(&self, site: usize) -> Option<usize> {
        let at = *self.to.get(site)?;
        (at != u32::MAX).then_some(at as usize)
    }

    /// Maps `site` to `to`, returning what it mapped to before.
    fn insert(&mut self, site: usize, to: usize) -> Option<usize> {
        if site >= self.to.len() {
            self.to.resize(site + 1, u32::MAX);
        }
        let before = self.get(site);
        self.to[site] = u32::try_from(to).unwrap_or(u32::MAX);
        if before.is_none() {
            self.len += 1;
        }
        before
    }

    /// The site the lowest mapped site maps to.
    fn first(&self) -> Option<usize> {
        self.to
            .iter()
            .find(|at| **at != u32::MAX)
            .map(|at| *at as usize)
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A group whose members' sites follow from one another through dedicated
/// wiring: each link says that the driver of a signal must sit on the one
/// site its sink can be reached from.
#[derive(Debug)]
struct Exact {
    /// `(driver instance, sink instance, relation)`.
    links: Vec<(usize, usize, Rc<Relation>)>,
    /// The order to resolve the members in from the anchor: each entry is
    /// an index into `links` and whether the member it resolves is the
    /// link's driver (else its sink).
    order: Vec<(usize, bool)>,
    /// Every site of the anchor's kind, by tile.
    anchor_sites: HashMap<(u32, u32), Vec<usize>>,
}

/// How many wires of its own tile the walk from a carry's output visits
/// looking for the flip-flop beside it; see [`registered_outputs`]. The
/// flip-flop is a hop or two away, so this only bounds a walk that has
/// wandered into a tile's interconnect.
const LOCAL_WIRES: usize = 48;

/// Adds to `links` each flip-flop that registers a carry chain's output
/// from the site beside it, so that it joins the chain's macro.
///
/// A carry's sum reaches the flip-flop of its own lane through the slice's
/// data mux and reaches anything else through the interconnect, so the
/// link is no dedicated wiring and [`dedicated_drivers`] does not find it.
/// Left to itself the flip-flop was placed apart from its chain: on the
/// ChipWhisperer-Husky the 2,946 two-pin nets from a carry to a flip-flop
/// were 36.7 tiles long on average, because a chain is a rigid macro the
/// legaliser places first and the annealer can seldom move on a die 95 %
/// full. Vivado packs such a flip-flop into the carry's slice; this makes
/// it a member of the macro on the site the walk from the carry's pin
/// reaches inside the tile.
///
/// The flip-flops of a slice share one clock, enable and set/reset, so a
/// carry's registered outputs join only when every flip-flop joining that
/// carry agrees on them and on its primitive; otherwise none does, and the
/// macro stays one the legaliser can always place.
fn registered_outputs(
    netlist: &Netlist,
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    info: &[Placeable],
    relations: &mut BTreeMap<(String, String, String, String), Option<Rc<Relation>>>,
    links: &mut Vec<(usize, usize, Rc<Relation>)>,
) {
    let carry = BelRole::Carry.keyword();
    let free = |i: usize| info[i].fixed.is_none() && info[i].macro_index.is_none();
    // Only a chain that is a macro already: a carry with no dedicated link
    // is placed on its own, and so is what it drives.
    let in_macro: HashSet<usize> = links.iter().flat_map(|(x, y, _)| [*x, *y]).collect();
    let mut joining: BTreeMap<usize, Vec<(usize, usize, Rc<Relation>)>> = BTreeMap::new();
    for signal in &netlist.signals {
        let Some(driver) = signal.driver else {
            continue;
        };
        let dpin = &netlist.pins[driver];
        let x = dpin.instance;
        if netlist.instances[x].kind != carry || !in_macro.contains(&x) {
            continue;
        }
        for sink in &signal.sinks {
            let spin = &netlist.pins[*sink];
            let y = spin.instance;
            if y == x || !free(y) || in_macro.contains(&y) {
                continue;
            }
            let key = (
                netlist.instances[y].kind.clone(),
                spin.role.clone(),
                netlist.instances[x].kind.clone(),
                format!("local:{}", dpin.role),
            );
            let relation = relations
                .entry(key)
                .or_insert_with(|| {
                    local_relation(
                        graph,
                        sites_by_kind,
                        &netlist.instances[y].kind,
                        &spin.role,
                        carry,
                        &dpin.role,
                    )
                })
                .clone();
            if let Some(relation) = relation {
                joining.entry(x).or_default().push((x, y, relation));
            }
        }
    }
    // A flip-flop registered twice — two outputs of one chain into one
    // cell — would be two sites for one member.
    let mut taken: HashSet<usize> = HashSet::new();
    for (_, mut candidates) in joining {
        let control = |y: usize| -> Vec<(String, Option<usize>)> {
            let mut out: Vec<(String, Option<usize>)> = netlist.instances[y]
                .pins
                .iter()
                .map(|p| &netlist.pins[*p])
                .filter(|p| !p.output && p.role != "d")
                .map(|p| (p.role.clone(), p.signal))
                .collect();
            out.sort();
            out
        };
        candidates.retain(|(_, y, _)| !taken.contains(y));
        // Two outputs of one lane — its sum and its carry — reach the one
        // flip-flop beside it, and two cells cannot both have that site:
        // the macro would fit nowhere. Compared on one carry site, which
        // stands for them all.
        let mut sites: Vec<Option<usize>> = candidates
            .iter()
            .map(|(_, _, r)| r.sink_of.first())
            .collect();
        let all = sites.len();
        sites.sort_unstable();
        sites.dedup();
        if sites.len() != all || sites.contains(&None) {
            continue;
        }
        let Some((_, first, _)) = candidates.first() else {
            continue;
        };
        let (want, primitive) = (control(*first), &netlist.instances[*first].primitive);
        if candidates
            .iter()
            .all(|(_, y, _)| control(*y) == want && netlist.instances[*y].primitive == *primitive)
        {
            for (x, y, relation) in candidates {
                if taken.insert(y) {
                    links.push((x, y, relation));
                }
            }
        }
    }
}

/// The control pins of a flip-flop, as `(role, signal)` sorted: what the
/// flip-flops of one slice must agree on, with the primitive.
fn control_set(netlist: &Netlist, ff: usize) -> Vec<(String, Option<usize>)> {
    let mut out: Vec<(String, Option<usize>)> = netlist.instances[ff]
        .pins
        .iter()
        .map(|p| &netlist.pins[*p])
        .filter(|p| !p.output && p.role != "d")
        .map(|p| (p.role.clone(), p.signal))
        .collect();
    out.sort();
    out
}

/// Adds to `links` the flip-flop a lookup table's output feeds, on the
/// site that output reaches inside its tile: one per output, so a
/// `LUT6_2` may take two, when they agree on a control set.
///
/// This is the packing Vivado does and this placer did not. A lookup
/// table's `O6` reaches the flip-flop of its own lane through the slice's
/// data mux, and its `O5` the five-input flip-flop beside it; anything
/// else is a trip out to the interconnect and back in through a bypass
/// input. Left to the anneal, the two were placed apart: on NewAE's
/// ChipWhisperer-Husky the flip-flops took 85 % of the four per slice that
/// were offered, wherever there was room, and their bypass inputs and the
/// `BYP_ALT` wires that feed them were what the route ran out of first.
/// In Vivado's bitstream of the same design 8,475 flip-flops read `O6`
/// and 1,963 five-input ones `O5`.
///
/// Like [`registered_outputs`] it joins only cells nothing else holds,
/// and a table's two flip-flops only when they share a clock, enable,
/// set/reset and primitive, since one slice has one of each. An `O5` that
/// also leaves the tile is left alone: its one way out is the output mux
/// the five-input flip-flop's `Q` needs.
fn registered_tables(
    netlist: &Netlist,
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    info: &[Placeable],
    relations: &mut BTreeMap<(String, String, String, String), Option<Rc<Relation>>>,
    links: &mut Vec<(usize, usize, Rc<Relation>)>,
) {
    let lut = BelRole::Lut.keyword();
    let ff = BelRole::Ff.keyword();
    let in_macro: HashSet<usize> = links.iter().flat_map(|(x, y, _)| [*x, *y]).collect();
    let free = |i: usize| {
        info[i].fixed.is_none() && info[i].macro_index.is_none() && !in_macro.contains(&i)
    };
    let mut taken: HashSet<usize> = HashSet::new();
    // A table's flip-flop already joined, which a second must agree with.
    let mut partner: HashMap<usize, usize> = HashMap::new();
    for signal in &netlist.signals {
        let Some(driver) = signal.driver else {
            continue;
        };
        let dpin = &netlist.pins[driver];
        let x = dpin.instance;
        if netlist.instances[x].kind != lut
            || !free(x)
            || (dpin.role == "o5" && signal.sinks.len() != 1)
        {
            continue;
        }
        for sink in &signal.sinks {
            let spin = &netlist.pins[*sink];
            let y = spin.instance;
            if y == x
                || !free(y)
                || taken.contains(&y)
                || netlist.instances[y].kind != ff
                || spin.role != "d"
            {
                continue;
            }
            if let Some(other) = partner.get(&x)
                && (control_set(netlist, *other) != control_set(netlist, y)
                    || netlist.instances[*other].primitive != netlist.instances[y].primitive)
            {
                continue;
            }
            let key = (
                ff.to_owned(),
                spin.role.clone(),
                lut.to_owned(),
                format!("local:{}", dpin.role),
            );
            let relation = relations
                .entry(key)
                .or_insert_with(|| local_relation(graph, sites_by_kind, ff, "d", lut, &dpin.role))
                .clone();
            if let Some(relation) = relation {
                taken.insert(y);
                partner.insert(x, y);
                links.push((x, y, relation));
                break;
            }
        }
    }
}

/// The sites of kind `sink_kind` whose `sink_role` pin each `driver_kind`
/// site's `driver_role` pin reaches through wires of its own tile, when
/// that is exactly one per driver site: [`registered_outputs`]' relation.
fn local_relation(
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    sink_kind: &str,
    sink_role: &str,
    driver_kind: &str,
    driver_role: &str,
) -> Option<Rc<Relation>> {
    let mut pins_at: HashMap<NodeId, usize> = HashMap::new();
    for site in sites_by_kind.get(sink_kind)? {
        for node in graph.sites[*site].pin_nodes(sink_role) {
            pins_at.insert(node, *site);
        }
    }
    let mut relation = Relation::default();
    for site in sites_by_kind.get(driver_kind)? {
        let tile = graph.sites[*site].tile;
        let mut seen: Vec<NodeId> = graph.sites[*site].pin_nodes(driver_role).collect();
        let mut reached: Vec<usize> = Vec::new();
        let mut next = 0;
        while next < seen.len() && seen.len() <= LOCAL_WIRES {
            let node = seen[next];
            next += 1;
            if let Some(sink) = pins_at.get(&node)
                && !reached.contains(sink)
            {
                reached.push(*sink);
            }
            for pip in graph.outgoing(node) {
                let to = graph.pip(*pip).to;
                let wire = graph.wire(to);
                if !wire.global && wire.tile == tile && !seen.contains(&to) {
                    seen.push(to);
                }
            }
        }
        if let [one] = reached.as_slice()
            && (relation.sink_of.insert(*site, *one).is_some()
                || relation.driver_of.insert(*one, *site).is_some())
        {
            return None;
        }
    }
    (!relation.driver_of.is_empty()).then(|| Rc::new(relation))
}

/// The sites of kind `driver_kind` whose `driver_role` pin can reach the
/// `role` pin of `site` **only** through wires nothing else drives, or
/// `None` when the walk back from the pin reaches more than
/// [`DEDICATED_WALK`] wires and the pin is therefore fed from the general
/// interconnect.
///
/// The walk follows every pip backwards, whatever its bits. That is the
/// point: a carry chain's hop from `CIN` costs `PRECYINIT.CIN`, and it is
/// still the only way in.
fn dedicated_drivers(
    graph: &RoutingGraph,
    pins_at: &HashMap<NodeId, Vec<(usize, String)>>,
    site: usize,
    role: &str,
    driver_kind: &str,
    driver_role: &str,
) -> Option<Vec<usize>> {
    let mut seen: Vec<NodeId> = graph.sites[site].pin_nodes(role).collect();
    if seen.is_empty() {
        return Some(Vec::new());
    }
    let mut next = 0;
    while next < seen.len() {
        let node = seen[next];
        next += 1;
        for pip in graph.incoming(node) {
            let from = graph.pip(*pip).from;
            if !seen.contains(&from) {
                seen.push(from);
                if seen.len() > DEDICATED_WALK {
                    return None;
                }
            }
        }
    }
    let mut out: Vec<usize> = Vec::new();
    for node in &seen {
        for (other, other_role) in pins_at.get(node).into_iter().flatten() {
            if *other != site
                && other_role == driver_role
                && graph.sites[*other].kind == driver_kind
                && !out.contains(other)
            {
                out.push(*other);
            }
        }
    }
    Some(out)
}

/// Finds every group of instances that dedicated wiring holds together, and
/// makes each one a rigid macro whose members' **exact sites** follow from
/// its anchor's.
///
/// # Why the placer has to know
///
/// Most of a fabric is general interconnect: a signal leaves a site and
/// can reach any other, at a price the router pays. Some of it is not. A
/// 7-series `CARRY4`'s carry in comes only from the `COUT` of the slice
/// directly below, and its propagate input *is* the output of the lookup
/// table at the same position of the same slice. A placement that puts the
/// two ends of such a connection anywhere else is not a worse placement but
/// an impossible one, and the router finds that out only after the whole
/// design has been placed.
///
/// So it is read off the routing graph, before legalisation, as the
/// relations the router will need: for each signal, walk back from each
/// sink pin through the graph ([`dedicated_drivers`]); if the walk closes
/// within a few wires on every site of the sink's kind, and finds exactly
/// one site of the driver's kind for each, then the driver's site is a
/// function of the sink's. Nothing names a carry chain, a slice or a
/// lookup table: on a family whose every pin is on the interconnect no
/// walk closes, no relation is found, and placement is exactly what it was.
///
/// Instances related this way are grouped, and a group moves as a whole —
/// in legalisation and in the annealer's macro move — to wherever its
/// anchor's kind has a site from which every member's site exists and is
/// free. An instance already fixed to a pin or already in an `rloc` macro
/// is never put in such a group: two rigid shapes cannot both be obeyed,
/// and the constraint the user wrote wins.
///
/// # The two families it has been measured on
///
/// The claim that nothing here names a family is worth only what has been
/// measured, and two chains of two different shapes have been:
///
/// - the 7-series `CARRY4`, which this was written for: the carry in is
///   reached from the slice directly *below*, and the propagate input is
///   the lookup table at the same position of the same slice, so a group
///   is a column of carries with their tables beside them.
/// - the ECP5 `CCU2C`, which this was **not** written for and holds
///   anyway. `FCI_SLICE` is reached only over
///   `FCI <- HFIE0000 <- FCO <- FCO_SLICE`, four `.fixed_conn` hops and
///   well inside [`DEDICATED_WALK`], and every carry site has exactly one
///   candidate driver slice — so the nine-cell chain of
///   `testdata/fpga/ecp5/carry_chain_16.v` comes out as one group of
///   nine and lands on slices A, B, C, D of a tile and then the tile one
///   column *east, in the same row*. A chain that runs along a row and a
///   chain that runs up a column are the same statement to this code,
///   because it asks the graph instead of saying which way is forward.
///   `tests/fpga_trellis.rs`'s
///   `a_carry_chain_places_routes_and_every_bit_of_it_decodes` is the
///   measurement, and it checks each consecutive pair over the fabric's
///   own bitless pips rather than over this function's opinion.
///
/// A family whose carry is on the general interconnect still gets no
/// group, which is not a nicety but the thing that keeps it safe: the
/// iCE40 and the synthetic architecture place exactly as they did before,
/// with a real net for the router to find a path for.
fn build_clusters(
    netlist: &Netlist,
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    info: &mut [Placeable],
    first_index: usize,
) -> Vec<Macro> {
    let free =
        |i: usize, info: &[Placeable]| info[i].fixed.is_none() && info[i].macro_index.is_none();
    // Which site pin sits on which node, built lazily: most designs on most
    // families have no pair of instances worth asking about.
    let mut pins_at: Option<HashMap<NodeId, Vec<(usize, String)>>> = None;
    let mut relations: BTreeMap<(String, String, String, String), Option<Rc<Relation>>> =
        BTreeMap::new();
    let mut links: Vec<(usize, usize, Rc<Relation>)> = Vec::new();

    for signal in &netlist.signals {
        let Some(driver) = signal.driver else {
            continue;
        };
        let dpin = &netlist.pins[driver];
        let x = dpin.instance;
        if !free(x, info) {
            continue;
        }
        for sink in &signal.sinks {
            let spin = &netlist.pins[*sink];
            let y = spin.instance;
            if y == x || !free(y, info) {
                continue;
            }
            let key = (
                netlist.instances[y].kind.clone(),
                spin.role.clone(),
                netlist.instances[x].kind.clone(),
                dpin.role.clone(),
            );
            if !relations.contains_key(&key) {
                let pins_at = pins_at.get_or_insert_with(|| {
                    let mut map: HashMap<NodeId, Vec<(usize, String)>> = HashMap::new();
                    for (index, site) in graph.sites.iter().enumerate() {
                        for (role, node) in &site.pins {
                            map.entry(*node).or_default().push((index, role.clone()));
                        }
                    }
                    map
                });
                let relation = find_relation(graph, pins_at, sites_by_kind, &key);
                relations.insert(key.clone(), relation);
            }
            if let Some(Some(relation)) = relations.get(&key) {
                links.push((x, y, Rc::clone(relation)));
            }
        }
    }
    registered_outputs(
        netlist,
        graph,
        sites_by_kind,
        info,
        &mut relations,
        &mut links,
    );
    registered_tables(
        netlist,
        graph,
        sites_by_kind,
        info,
        &mut relations,
        &mut links,
    );
    if links.is_empty() {
        return Vec::new();
    }

    // Group the linked instances.
    let mut parent: BTreeMap<usize, usize> = BTreeMap::new();
    fn root(parent: &mut BTreeMap<usize, usize>, mut i: usize) -> usize {
        while let Some(&p) = parent.get(&i) {
            if p == i {
                break;
            }
            i = p;
        }
        i
    }
    for (x, y, _) in &links {
        parent.entry(*x).or_insert(*x);
        parent.entry(*y).or_insert(*y);
        let (a, b) = (root(&mut parent, *x), root(&mut parent, *y));
        if a != b {
            let (keep, drop) = (a.min(b), a.max(b));
            parent.insert(drop, keep);
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let members: Vec<usize> = parent.keys().copied().collect();
    for i in members {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }

    let mut out = Vec::new();
    for (_, members) in groups {
        // The anchor is the member whose kind has the fewest sites, so the
        // macro move draws from the scarcest resource; ties go to the
        // lowest index, so the choice is deterministic.
        let anchor = *members
            .iter()
            .min_by_key(|i| {
                let kind = &netlist.instances[**i].kind;
                (sites_by_kind.get(kind).map_or(0, Vec::len), **i)
            })
            .expect("a group has members");
        let mine: Vec<(usize, usize, Rc<Relation>)> = links
            .iter()
            .filter(|(x, _, _)| members.contains(x))
            .map(|(x, y, r)| (*x, *y, Rc::clone(r)))
            .collect();
        // Resolve outwards from the anchor, one link at a time.
        let mut known = vec![anchor];
        let mut order = Vec::new();
        while known.len() < members.len() {
            let before = known.len();
            for (index, (x, y, _)) in mine.iter().enumerate() {
                if known.contains(x) && !known.contains(y) {
                    order.push((index, false));
                    known.push(*y);
                } else if known.contains(y) && !known.contains(x) {
                    order.push((index, true));
                    known.push(*x);
                }
            }
            if known.len() == before {
                break;
            }
        }
        let mut anchor_sites: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for site in sites_by_kind
            .get(&netlist.instances[anchor].kind)
            .into_iter()
            .flatten()
        {
            anchor_sites
                .entry(graph.sites[*site].tile)
                .or_default()
                .push(*site);
        }
        let index = first_index + out.len();
        for member in &members {
            info[*member].macro_index = Some(index);
        }
        out.push(Macro {
            anchor,
            members: members.iter().map(|m| (*m, 0, 0)).collect(),
            exact: Some(Exact {
                links: mine,
                order,
                anchor_sites,
            }),
        });
    }
    out
}

/// The relation for one `(sink kind, sink role, driver kind, driver role)`,
/// or `None` when that connection is not dedicated: some site of the sink's
/// kind is fed from the general interconnect, or can be reached from more
/// than one site of the driver's kind, or two sinks share one driver site.
fn find_relation(
    graph: &RoutingGraph,
    pins_at: &HashMap<NodeId, Vec<(usize, String)>>,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    key: &(String, String, String, String),
) -> Option<Rc<Relation>> {
    let (sink_kind, sink_role, driver_kind, driver_role) = key;
    let mut relation = Relation::default();
    for site in sites_by_kind.get(sink_kind)? {
        if graph.sites[*site].pin(sink_role).is_none() {
            continue;
        }
        let drivers =
            dedicated_drivers(graph, pins_at, *site, sink_role, driver_kind, driver_role)?;
        match drivers.as_slice() {
            [] => {}
            [one] => {
                if relation.sink_of.insert(*one, *site).is_some() {
                    return None;
                }
                relation.driver_of.insert(*site, *one);
            }
            _ => return None,
        }
    }
    (!relation.driver_of.is_empty()).then(|| Rc::new(relation))
}

/// The sites a dedicated-wiring group would take with its anchor on some
/// site of the tile `(ax, ay)`, or `None` when no site of that tile works.
#[allow(clippy::too_many_arguments, reason = "the legaliser's whole state")]
fn exact_sites(
    graph: &RoutingGraph,
    info: &[Placeable],
    m: &Macro,
    exact: &Exact,
    shared: &SiteRules,
    placement: &Placement,
    ax: u32,
    ay: u32,
) -> Option<Vec<(usize, usize)>> {
    let candidates = exact.anchor_sites.get(&(ax, ay))?;
    'anchor: for start in candidates {
        let mut at: BTreeMap<usize, usize> = BTreeMap::new();
        at.insert(m.anchor, *start);
        for (link, driver_side) in &exact.order {
            let (x, y, relation) = &exact.links[*link];
            let site = if *driver_side {
                relation.driver_of.get(*at.get(y)?)
            } else {
                relation.sink_of.get(*at.get(x)?)
            };
            let Some(site) = site else { continue 'anchor };
            at.insert(if *driver_side { *x } else { *y }, site);
        }
        // Every link, including the ones the walk did not use, must hold.
        for (x, y, relation) in &exact.links {
            let (Some(sx), Some(sy)) = (at.get(x), at.get(y)) else {
                continue 'anchor;
            };
            if relation.driver_of.get(*sy) != Some(*sx) {
                continue 'anchor;
            }
        }
        let mut out: Vec<(usize, usize)> = Vec::with_capacity(at.len());
        let mut taken: Vec<usize> = Vec::with_capacity(at.len());
        for (member, site) in &at {
            let (x, y) = graph.sites[*site].tile;
            if placement.instance_at(*site).is_some()
                || taken.contains(site)
                || !info[*member].may_take(*site)
                || info[*member]
                    .region
                    .as_ref()
                    .is_some_and(|(rect, _)| !rect.holds(x, y))
            {
                continue 'anchor;
            }
            taken.push(*site);
            out.push((*member, *site));
        }
        if shared.allows(placement, &out) {
            return Some(out);
        }
    }
    None
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
/// Confines a global buffer a pad drives to the buffer sites the pad's
/// dedicated road reaches ([`PlaceOptions::clock_roads`]).
///
/// General routing reaches every buffer, so nothing here is about whether
/// a site works; it is about how. On a 7-series a clock-capable pad's
/// road climbs the buffer cascade from its own clock row, and so reaches
/// the sixteen buffers of its half of the die and not the other sixteen.
/// A buffer the placer put in the other half is fed over general routing,
/// a clock insertion that moves with every placement: on the
/// ChipWhisperer-Husky Plus the bus clock's buffers went to the half its
/// road does not reach, and their inputs arrived 4.9 to 10.8 ns after the
/// pad. Vivado will not place a pad's buffer there at all.
///
/// The sweep starts at the pad's pin and follows road nodes and the
/// bit-less pips that join a wire's pieces, and stops at a candidate
/// buffer's input. Each input a pad drives narrows the sites the inputs
/// before it left, so a `BUFGMUX` between two pads keeps the buffers both
/// reach; an input whose pad reaches none of them, whether a pad on no
/// road or one whose road and the others' do not meet, narrows nothing.
/// Only global buffers: a clock manager a pad feeds is reached by the
/// road only in the pad's own clock region, and nothing measured here
/// says it should be moved there.
fn confine_to_roads(
    netlist: &Netlist,
    graph: &RoutingGraph,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    info: &mut [Placeable],
    roads: &[bool],
) {
    if roads.is_empty() {
        return;
    }
    let Some(candidates) = sites_by_kind.get("gb") else {
        return;
    };
    for index in 0..netlist.instances.len() {
        if netlist.instances[index].kind != "gb" || info[index].fixed.is_some() {
            continue;
        }
        let mut allowed: Vec<usize> = info[index]
            .allowed
            .clone()
            .unwrap_or_else(|| candidates.clone());
        for pin in &netlist.instances[index].pins {
            let pin = &netlist.pins[*pin];
            if pin.output {
                continue;
            }
            let Some(driver) = pin.signal.and_then(|signal| netlist.signals[signal].driver) else {
                continue;
            };
            let driver = &netlist.pins[driver];
            let Some(pad) = info[driver.instance].fixed else {
                continue;
            };
            let targets: BTreeMap<NodeId, usize> = allowed
                .iter()
                .filter_map(|site| graph.sites[*site].pin(&pin.role).map(|node| (node, *site)))
                .collect();
            let mut seen: HashSet<NodeId> = graph.sites[pad].pin_nodes(&driver.role).collect();
            let mut work: Vec<NodeId> = seen.iter().copied().collect();
            let mut reached: Vec<usize> = Vec::new();
            while let Some(node) = work.pop() {
                for pip in graph.outgoing(node) {
                    let to = graph.pip(*pip).to;
                    if !seen.insert(to) {
                        continue;
                    }
                    if let Some(site) = targets.get(&to) {
                        reached.push(*site);
                    } else if roads.get(to as usize) == Some(&true)
                        || graph.pip_bits(*pip).is_empty()
                    {
                        work.push(to);
                    }
                }
            }
            if reached.is_empty() {
                continue;
            }
            reached.sort_unstable();
            reached.dedup();
            allowed = reached;
        }
        if allowed.len() < candidates.len() {
            info[index].allowed = Some(allowed);
        }
    }
}

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

    /// Adds `value` at `(i, j)` and `(j, i)`, off the diagonal, knowing
    /// neither entry exists yet: a star net's centre is a variable of its
    /// own, so its entries are new, and [`Sparse::add`]'s search along the
    /// row was what made a 13,000-pin clock net quadratic.
    fn add_new_pair(&mut self, i: usize, j: usize, value: f64) {
        self.rows[i].push((j, value));
        self.rows[j].push((i, value));
    }

    /// Grows the matrix by `extra` rows and columns of zeros.
    fn grow(&mut self, extra: usize) {
        let n = self.diagonal.len() + extra;
        self.diagonal.resize(n, 0.0);
        self.rows.resize(n, Vec::new());
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
    solve_analytic_with(netlist, graph, info, options, CLIQUE_PINS)
}

/// [`solve_analytic`] with the clique bound a parameter, so that a test
/// can solve one net both ways.
fn solve_analytic_with(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    options: &PlaceOptions,
    clique_pins: usize,
) -> Vec<(f64, f64)> {
    let n = netlist.instances.len();
    let (a, bx, by) = net_system(netlist, graph, info, clique_pins);
    let x = solve(&a, &bx, options.analytic_iterations);
    let y = solve(&a, &by, options.analytic_iterations);
    x.into_iter().zip(y).take(n).collect()
}

/// The two linear systems [`solve_analytic_with`] solves, one per axis,
/// sharing their matrix: every net as a clique or a star, each instance
/// pulled weakly to the middle, and the fixed ones held.
fn net_system(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    clique_pins: usize,
) -> (Sparse, Vec<f64>, Vec<f64>) {
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
    // Which signal last counted each instance, so that a net's members are
    // found in one pass: `members.contains` was quadratic in the fan-out.
    let mut seen: Vec<usize> = vec![usize::MAX; n];
    let mut members: Vec<usize> = Vec::new();
    for (index, signal) in netlist.signals.iter().enumerate() {
        members.clear();
        for pin in signal.driver.iter().chain(signal.sinks.iter()) {
            let instance = netlist.pins[*pin].instance;
            if seen[instance] != index {
                seen[instance] = index;
                members.push(instance);
            }
        }
        if members.len() < 2 {
            continue;
        }
        // The clique model: every pair, weighted so that the total weight
        // of a k-pin net does not grow with k^2.
        let k = members.len() as f64;
        let weight = 2.0 / (k - 1.0) / k;
        if members.len() > clique_pins {
            // The same net as a star: a centre of its own, joined to each
            // member with weight `k * weight`. Eliminating the centre gives
            // `weight * (k I - 1 1')`, which is the clique's Laplacian, so
            // the system's solution is the clique's — a conjugate gradient
            // cut off early lands near it rather than on it — and the cost
            // is k entries, not k^2.
            let centre = a.len();
            a.grow(1);
            bx.push(0.0);
            by.push(0.0);
            let star = k * weight;
            for i in &members {
                a.add(*i, *i, star);
                a.add(centre, centre, star);
                a.add_new_pair(*i, centre, -star);
            }
            continue;
        }
        for (index, i) in members.iter().enumerate() {
            for j in &members[index + 1..] {
                a.add(*i, *i, weight);
                a.add(*j, *j, weight);
                a.add(*i, *j, -weight);
                a.add(*j, *i, -weight);
            }
        }
    }
    (a, bx, by)
}

/// The pull of the die's middle on every instance.
const ANCHOR_WEIGHT: f64 = 1e-3;

/// Solves `a x = b` by conjugate gradient preconditioned with `a`'s
/// diagonal, starting from `x` and leaving the answer there.
///
/// [`solve`] starts from zero and is unpreconditioned, which is all a
/// small design's one solve needs. [`global_place`] solves the same system
/// thirty times with a little more pull each time, so it starts each solve
/// from the last answer; and a design's diagonal runs from a cell on one
/// two-pin net to a star's centre on a clock of thousands, which is what
/// the preconditioner evens out.
fn solve_from(a: &Sparse, b: &[f64], x: &mut [f64], iterations: u32) {
    let n = a.len();
    if n == 0 {
        return;
    }
    let mut ap = vec![0.0; n];
    a.multiply(x, &mut ap);
    let mut r: Vec<f64> = b.iter().zip(&ap).map(|(b, ax)| b - ax).collect();
    let inverse: Vec<f64> = a
        .diagonal
        .iter()
        .map(|d| if *d > 0.0 { 1.0 / d } else { 1.0 })
        .collect();
    let mut z: Vec<f64> = r.iter().zip(&inverse).map(|(r, m)| r * m).collect();
    let mut p = z.clone();
    let mut rz = dot(&r, &z);
    let tolerance = 1e-14 * dot(b, b).max(1.0);
    for _ in 0..iterations {
        if dot(&r, &r) <= tolerance {
            break;
        }
        a.multiply(&p, &mut ap);
        let denominator = dot(&p, &ap);
        if denominator.abs() < f64::MIN_POSITIVE {
            break;
        }
        let alpha = rz / denominator;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
            z[i] = r[i] * inverse[i];
        }
        let next = dot(&r, &z);
        let beta = next / rz;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
        rz = next;
    }
}

/// How many rounds of spreading and solving [`global_place`] makes.
const GLOBAL_ROUNDS: u32 = 30;

/// How strongly, per round, [`global_place`] pulls a cell to where it was
/// spread, against a two-pin net's pull of one.
const GLOBAL_PULL: f64 = 0.05;

/// The conjugate-gradient iterations of each of [`global_place`]'s solves
/// after the first, which gets four times as many.
const GLOBAL_ITERATIONS: u32 = 300;

/// The fewest cells a kind needs before [`global_place`] spreads it. A
/// handful of clock managers or global buffers are placed by the nets
/// that reach them, as before; spreading four MMCMs over five sites moved
/// them onto interconnect the flow then refused (`fpga_xray_cmt`).
const GLOBAL_SPREAD_MIN: usize = 32;

/// What [`global_place`] returns: where every cell should go, and the
/// kinds it spread as `(kind, cells)`.
type GlobalPlacement = (Vec<(f64, f64)>, Vec<(String, usize)>);

/// One kind [`global_place`] spreads: its name, its movable cells, and the
/// tiles of the sites they may go to.
type SpreadKind = (String, Vec<usize>, Vec<(u32, u32)>);

/// Global placement for a design dense enough that one solve and one
/// spread throw the solve's answer away: what [`dense_kinds`] decides.
/// Returns where every cell should go and the kinds it spread.
///
/// # What it replaced, and why
///
/// One quadratic solve piles a design at the middle of the die; spreading
/// that by bisection keeps the solve's order along each axis but not its
/// neighbourhoods, and a cell the one spread left alone — a flip-flop,
/// whose slice shares its clock — was then solved again around the spread
/// ones, which piled the flip-flops instead. On NewAE's ChipWhisperer-Husky
/// the 17,333 flip-flops in no macro ended 35 tiles from where that solve
/// put them, and `esad`'s 256-stage chain of reference samples had its
/// neighbouring stages 60 to 70 tiles apart: the anneal, whose moves are a
/// few tiles once it is cool, cannot untangle a chain scrambled across the
/// die. Its wirelength after annealing was 964,496.
///
/// This is SimPL (Kim, Lee and Markov, 2012) in its simplest form. Each
/// round spreads every kind of at least [`GLOBAL_SPREAD_MIN`] cells over its
/// own sites by [`bisect`] — an *upper bound*, legal-ish and long — and
/// solves again with each cell drawn towards its spread position — the
/// *lower bound*, short and piled — by a pull that grows each round, so
/// that the two meet. The pull is divided by the square root of the
/// distance between them, which makes a far cell's pull nearer linear in
/// the distance than quadratic, as SimPL's own anchors are. A macro's
/// members other than its anchor follow the anchor. The answer is the last
/// round's spread positions, so every kind is legalised from a place that
/// has room, flip-flops included.
///
/// Measured on the Husky with the same netlist: 30 rounds at a pull of
/// 0.05 took legalisation's displacement from 20.2 tiles to 5.5 and the
/// wirelength after annealing from 964,496 to 636,749, and the route's
/// overuse at the sixth pass from 71,569 nodes to 48,589. A pull of 0.01,
/// 0.1, 0.2 or 0.5 and 50 or 60 rounds all ended between 639,724 and
/// 703,803: the anneal, not the rounds, is what is left.
fn global_place(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    macros: &[Macro],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
) -> GlobalPlacement {
    let n = netlist.instances.len();
    let mut follower = vec![false; n];
    for m in macros {
        for (member, _, _) in &m.members {
            if *member != m.anchor {
                follower[*member] = true;
            }
        }
    }
    let taken: HashSet<usize> = info.iter().filter_map(|p| p.fixed).collect();
    // Each spread kind's movable cells and the sites they may go to.
    let mut kinds: Vec<SpreadKind> = Vec::new();
    for (kind, sites) in sites_by_kind {
        let cells: Vec<usize> = (0..n)
            .filter(|i| {
                netlist.instances[*i].kind == *kind
                    && info[*i].fixed.is_none()
                    && info[*i].region.is_none()
                    && info[*i].allowed.is_none()
                    && !follower[*i]
            })
            .collect();
        if cells.len() < GLOBAL_SPREAD_MIN {
            continue;
        }
        let free: Vec<(u32, u32)> = sites
            .iter()
            .filter(|s| !taken.contains(s))
            .map(|s| graph.sites[*s].tile)
            .collect();
        kinds.push((kind.clone(), cells, free));
    }
    let spread_all = |positions: &[(f64, f64)]| -> Vec<(f64, f64)> {
        let mut upper = positions.to_vec();
        for (_, cells, sites) in &kinds {
            let mut items: Vec<(usize, f64, f64)> = cells
                .iter()
                .map(|i| (*i, positions[*i].0, positions[*i].1))
                .collect();
            let mut sites = sites.clone();
            bisect(&mut items, &mut sites, &mut upper);
        }
        for m in macros {
            let dx = upper[m.anchor].0 - positions[m.anchor].0;
            let dy = upper[m.anchor].1 - positions[m.anchor].1;
            for (member, _, _) in &m.members {
                if *member != m.anchor && info[*member].fixed.is_none() {
                    upper[*member] = (positions[*member].0 + dx, positions[*member].1 + dy);
                }
            }
        }
        upper
    };

    let (nets, bx, by) = net_system(netlist, graph, info, CLIQUE_PINS);
    let centre = (f64::from(graph.width) / 2.0, f64::from(graph.height) / 2.0);
    let mut x = vec![centre.0; nets.len()];
    let mut y = vec![centre.1; nets.len()];
    solve_from(&nets, &bx, &mut x, 4 * GLOBAL_ITERATIONS);
    solve_from(&nets, &by, &mut y, 4 * GLOBAL_ITERATIONS);
    let mut positions: Vec<(f64, f64)> = x.iter().zip(&y).take(n).map(|(a, b)| (*a, *b)).collect();
    for round in 1..=GLOBAL_ROUNDS {
        let upper = spread_all(&positions);
        let pull = GLOBAL_PULL * f64::from(round);
        let mut a = nets.clone();
        let (mut bx, mut by) = (bx.clone(), by.clone());
        for i in 0..n {
            if info[i].fixed.is_some() {
                continue;
            }
            let (ux, uy) = upper[i];
            let (px, py) = positions[i];
            let weight = pull / ((ux - px).abs() + (uy - py).abs()).max(1.0).sqrt();
            a.add(i, i, weight);
            bx[i] += weight * ux;
            by[i] += weight * uy;
        }
        solve_from(&a, &bx, &mut x, GLOBAL_ITERATIONS);
        solve_from(&a, &by, &mut y, GLOBAL_ITERATIONS);
        positions = x.iter().zip(&y).take(n).map(|(a, b)| (*a, *b)).collect();
    }
    let spread = kinds
        .iter()
        .map(|(kind, cells, _)| (kind.clone(), cells.len()))
        .collect();
    (spread_all(&positions), spread)
}

/// The most pins a net may have and still be modelled as a clique by the
/// analytic solve; a larger one is the equivalent star. Small enough that
/// a clock or reset net — 13,000 pins on the ChipWhisperer-Husky, whose
/// clique took the solve hours — is a star, and large enough that every
/// net of the designs the golden placements pin is still the clique it
/// was, summed in the same order.
const CLIQUE_PINS: usize = 64;
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

/// The share of a kind's sites its cells must fill before
/// [`dense_kinds`] counts it, and the design is placed by
/// [`global_place`].
///
/// It was a half, and on NewAE's Husky gateware that was a switch thrown
/// by noise: 16,842 spreadable lookup tables against 32,600 sites were
/// spread, and once the mapper needed fewer, 15,612 were not, so
/// legalisation moved a cell 48.3 tiles where it had moved one 20.9 and
/// the wirelength after annealing was 1,184,807 where it could be
/// 889,405 — and the route was the worse for it from the first pass. At
/// a third those 15,612 are spread. At zero the carry chains are spread
/// too, 1,681 of 8,150 sites, and their macros' flip-flops with them,
/// all over the die: flip-flop legalisation then stops, refused by
/// control sets, which is the reason there is a threshold at all.
///
/// A third was then too high for the Husky Plus, the same gateware on an
/// XC7A100T: its 16,712 spreadable lookup tables fill 26.4 % of 63,400
/// sites, so it was placed by one solve, to a half-perimeter wirelength
/// after annealing of 860,323 against the Husky's 635,591, and the route
/// never converged — at the twentieth pass a pass took seventeen minutes
/// and was getting slower. Placed globally, 691,296, and the route fell to
/// 8,561 overused nodes by the eighth pass where it had been at 31,461.
/// A fifth spreads it. The carry chains' trouble above was the one spread
/// that preceded [`global_place`], which spreads every kind of
/// [`GLOBAL_SPREAD_MIN`] cells once it runs, the Husky's carry chains
/// among them. What a design a tenth full gains or loses by global
/// placement has not been measured.
const SPREAD_FROM: f64 = 0.2;

/// And how many cells it must have. A handful of clock managers on a part
/// with five sites fill most of them, and the legaliser places a handful
/// as well as bisection could; spreading the four MMCMs of
/// `testdata/fpga/husky/clock_managers.v` moved them, and with them where
/// the second solve put the flip-flops they clock, onto general
/// interconnect the flow then refused (`fpga_xray_cmt`).
const SPREAD_MIN: usize = 256;

/// The site kinds of [`SPREAD_MIN`] cells or more that fill more than
/// [`SPREAD_FROM`] of their sites, as `(kind, cells)`: a design with any is
/// placed by [`global_place`], and one with none by one solve, which a
/// small design's legaliser moves a tile or so.
///
/// Until 2026-10-20 this spread those kinds itself, once, and what follows
/// is why that was needed and what it did.
///
/// A quadratic solve puts a design's cells where its nets pull them and
/// nowhere near where they fit: on the ChipWhisperer-Husky, 95 % of whose
/// lookup-table sites are used and 98 of whose cells a pin fixes, the
/// solve piled them about the middle, the legaliser took sites outward
/// from there in the order of the targets, and a cell ended **sixty
/// tiles** on average from where the solve had put it — the solve's
/// answer thrown away, and the anneal left to find a placement from
/// scratch. Here a kind's movable cells and its sites are each sorted
/// along the longer side of the sites' extent, the cells split in half
/// and the sites in the same proportion, and each half recursed into, so
/// every cell ends on a site of its own in the order the solve put them.
/// A kind below the threshold is left alone: a small design's solve is
/// legalised a tile or so away, and spreading it over the die would be
/// a wrong answer, not a better one.
///
/// Left out: cells a pin fixes, cells confined to a region or by
/// reachability, and the members of a macro other than its anchor, whose
/// sites follow from the anchor's.
fn dense_kinds(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    macros: &[Macro],
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    shared: &SiteRules,
) -> Vec<(String, usize)> {
    let mut follower = vec![false; info.len()];
    for m in macros {
        for (member, _, _) in &m.members {
            if *member != m.anchor {
                follower[*member] = true;
            }
        }
    }
    let mut out = Vec::new();
    for (kind, sites) in sites_by_kind {
        // A kind whose sites share pins with each other — a 7-series
        // slice's flip-flops, one clock, enable and reset among four — is
        // legalised by control set, and that needs the freedom a cell
        // spread to a site of its own does not leave: spread too, the
        // Husky's flip-flops took every slice in turn whatever its clock,
        // and legalisation stopped with 6,177 sites free. Such a kind
        // follows the others in a second solve instead. A lookup table
        // shares its output with the carry beside it, which is no control
        // set, and is spread.
        if !shared.trivial()
            && sites.iter().any(|s| {
                shared.per_site[*s]
                    .iter()
                    .any(|(other, _, _)| graph.sites[*other].kind == *kind)
            })
        {
            continue;
        }
        let cells: Vec<usize> = (0..netlist.instances.len())
            .filter(|i| {
                netlist.instances[*i].kind == *kind
                    && info[*i].fixed.is_none()
                    && info[*i].region.is_none()
                    && info[*i].allowed.is_none()
                    && !follower[*i]
            })
            .collect();
        // Sites a fixed cell holds are not there to be spread onto.
        let taken: HashSet<usize> = info.iter().filter_map(|p| p.fixed).collect();
        let free = sites.iter().filter(|s| !taken.contains(s)).count();
        if cells.len() < SPREAD_MIN || (cells.len() as f64) < SPREAD_FROM * free as f64 {
            continue;
        }
        out.push((kind.clone(), cells.len()));
    }
    out
}

/// One step of [`global_place`]'s spreading: `cells` onto `sites`, in
/// proportion.
fn bisect(cells: &mut [(usize, f64, f64)], sites: &mut [(u32, u32)], out: &mut [(f64, f64)]) {
    if cells.is_empty() || sites.is_empty() {
        return;
    }
    let (x0, x1, y0, y1) = sites
        .iter()
        .fold((u32::MAX, 0, u32::MAX, 0), |(a, b, c, d), (x, y)| {
            (a.min(*x), b.max(*x), c.min(*y), d.max(*y))
        });
    let along_x = x1 - x0 >= y1 - y0;
    if cells.len() == 1 || (x0 == x1 && y0 == y1) {
        // Down to one cell, or to one tile: the cells take the sites in
        // turn, which in a tile are all the same place.
        for (n, (cell, _, _)) in cells.iter().enumerate() {
            let (x, y) = sites[n * sites.len() / cells.len()];
            out[*cell] = (f64::from(x), f64::from(y));
        }
        return;
    }
    if along_x {
        cells.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        sites.sort_unstable_by_key(|(x, y)| (*x, *y));
    } else {
        cells.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
        sites.sort_unstable_by_key(|(x, y)| (*y, *x));
    }
    let half = cells.len() / 2;
    let cut = (sites.len() * half / cells.len()).clamp(1, sites.len() - 1);
    let (left_cells, right_cells) = cells.split_at_mut(half);
    let (left_sites, right_sites) = sites.split_at_mut(cut);
    bisect(left_cells, left_sites, out);
    bisect(right_cells, right_sites, out);
}

/// How far, in tiles in all and over how many instances, legalisation
/// moved the instances that were free to move from where the analytic
/// solve wanted them.
fn displacement(
    graph: &RoutingGraph,
    info: &[Placeable],
    positions: &[(f64, f64)],
    placement: &Placement,
) -> (u64, u64) {
    let mut total = 0u64;
    let mut count = 0u64;
    for (index, place) in info.iter().enumerate() {
        if place.fixed.is_some() {
            continue;
        }
        let Some(site) = placement.site_of(index) else {
            continue;
        };
        let (x, y) = positions[index];
        let target = (to_grid(x, graph.width), to_grid(y, graph.height));
        total += manhattan(target, graph.sites[site].tile);
        count += 1;
    }
    (total, count)
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
            None => {
                return Err(no_room(
                    netlist,
                    graph,
                    sites_by_kind,
                    index,
                    info,
                    shared,
                    placement,
                ));
            }
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
    shared: &SiteRules,
    placement: &Placement,
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
            if needed <= available {
                // Every free site this cell could have had, by the rule
                // that refused it, most common first.
                let mut refused: BTreeMap<String, usize> = BTreeMap::new();
                for site in sites {
                    if placement.instance_at(*site).is_some() || !info[index].may_take(*site) {
                        continue;
                    }
                    let why = shared
                        .refusal(placement, index, *site)
                        .unwrap_or_else(|| "nothing refuses it".to_owned());
                    *refused.entry(why).or_default() += 1;
                }
                let mut refused: Vec<(String, usize)> = refused.into_iter().collect();
                refused.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                return PlaceError::NoLegalSite {
                    cell: netlist.instances[index].name.clone(),
                    kind,
                    needed,
                    available,
                    refused,
                };
            }
            PlaceError::NoSites {
                kind,
                needed,
                available,
            }
        }
    }
}

/// How much farther, in tiles, the legaliser will carry a cell to join a
/// group of sites already in use than to open an empty one.
///
/// The group is the sites that share a pin — a 7-series slice's flip-flops,
/// which have one clock, one clock enable and one set/reset among them. A
/// flip-flop that opens a slice closes it to every other control set, and
/// taking the nearest legal site regardless did exactly that on the
/// ChipWhisperer-Husky: placement stopped with 8,926 flip-flop sites free,
/// 6,214 of them in slices whose clock already carried another one. A
/// site that shares no pin with any other opens nothing and is not
/// charged, so lookup tables place as before.
///
/// It was four tiles until the flip-flops were spread before legalising
/// (`global_place`) and packed with the tables that feed them
/// (`registered_tables`). Spread, they reach every slice in turn, and at
/// four the Husky's flip-flops on a clock of their own found every slice
/// already holding another (`O_trace_pattern5`, 36,991 of 37,765 free
/// sites refused for their clock); at sixteen they legalise, and the
/// wirelength after annealing was within one per cent of sixty-four's.
const OPENING: u64 = 16;

/// The free site of `kind` closest to `target`, inside `region` and among
/// `allowed`, that `instance` may legally take, counting a site that opens
/// a group of shared pins [`OPENING`] tiles farther than it is.
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
        let mut distance = manhattan(target, (x, y));
        // The penalty only adds, so a site already no nearer than the best
        // is not worth asking about.
        if best.is_some_and(|(d, _)| distance >= d) {
            continue;
        }
        if shared.opens(placement, instance, *site) {
            distance += OPENING;
        }
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
    // The nearest tile that fits, counting one where a member would open a
    // slice [`OPENING`] tiles farther, as `nearest_free` does for a single
    // cell: a macro is a flip-flop too, and once [`registered_tables`] made
    // thousands of them, taking the nearest fit regardless gave every slice
    // of the Husky a clock before the flip-flops on their own were placed.
    let mut best: Option<(u64, Vec<(usize, usize)>)> = None;
    for (distance, ax, ay) in candidates {
        if best.as_ref().is_some_and(|(cost, _)| distance >= *cost) {
            break;
        }
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
            let opens = sites
                .iter()
                .any(|(member, site)| shared.opens(placement, *member, *site));
            let cost = distance + if opens { OPENING } else { 0 };
            if best.as_ref().is_none_or(|(c, _)| cost < *c) {
                best = Some((cost, sites));
            }
        }
    }
    if let Some((_, sites)) = best {
        for (member, site) in sites {
            placement.place(member, site);
        }
        return Ok(());
    }
    // A rigid group that fits nowhere is not a part that ran out of
    // sites, and saying so would send the reader to the wrong question:
    // the chain of `tests/fpga_trellis.rs` is nine cells on a part with
    // 12 144 of them.
    if m.exact.is_some() {
        let kind = netlist.instances[m.anchor].kind.clone();
        return Err(PlaceError::NoDedicatedRun {
            cell: netlist.instances[m.anchor].name.clone(),
            members: m.members.len(),
            available: sites_by_kind.get(&kind).map_or(0, Vec::len),
            kind,
        });
    }
    Err(no_room(
        netlist,
        graph,
        sites_by_kind,
        m.anchor,
        info,
        shared,
        placement,
    ))
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
    if let Some(exact) = &m.exact {
        return exact_sites(graph, info, m, exact, shared, placement, ax, ay);
    }
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
    /// Per signal, how much its box costs per tile: empty for one each,
    /// which is a wirelength placement; see [`PlaceTiming`].
    weight: Vec<u32>,
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
            weight: Vec::new(),
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

    /// What `signal`'s box costs the anneal: its wirelength, times its
    /// weight when there are weights.
    fn cost_of(&self, signal: usize) -> u64 {
        let weight = self.weight.get(signal).copied().unwrap_or(1);
        self.hpwl_of(signal) * u64::from(weight)
    }

    /// Every signal's cost, summed. Unweighted, this is [`hpwl`] by
    /// another route and has to agree with it.
    fn total(&self) -> u64 {
        (0..self.placed.len()).map(|s| self.cost_of(s)).sum()
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
    /// The critical connections' pull, for a timing-driven placement:
    /// empty for a wirelength one.
    pull: Pull,
}

/// What a timing-driven anneal charges on top of the wirelength
/// ([`PlaceTiming`]): each critical connection's weight times its tile
/// distance, driver to sink. A signal's box is no measure of one late
/// connection on it — the box of a wide bus is the die whatever is moved —
/// and this is, which is why it is per connection, as VPR's timing cost
/// is, and not a weight on the box.
#[derive(Default)]
struct Pull {
    /// Per netlist pin, for a sink: its connection's weight.
    weight: Vec<u32>,
    /// Per instance, the weighted sink pins on it and the weighted sink
    /// pins it drives: the connections a move of it changes.
    touches: Vec<Vec<usize>>,
    /// The connections the move in hand touches, without repeats.
    touched: Vec<usize>,
}

/// The weights an anneal runs with: per signal for its box (empty for one
/// each) and per sink pin for its connection (empty for none).
#[derive(Default)]
struct Weights {
    nets: Vec<u32>,
    pins: Vec<u32>,
}

impl Pull {
    /// The pull of `weight`'s connections over `netlist`.
    fn new(netlist: &Netlist, weight: &[u32]) -> Pull {
        let mut touches = vec![Vec::new(); netlist.instances.len()];
        if weight.iter().any(|w| *w > 0) {
            for signal in &netlist.signals {
                let Some(driver) = signal.driver else {
                    continue;
                };
                for sink in &signal.sinks {
                    if weight.get(*sink).copied().unwrap_or(0) == 0 {
                        continue;
                    }
                    touches[netlist.pins[driver].instance].push(*sink);
                    touches[netlist.pins[*sink].instance].push(*sink);
                }
            }
        }
        Pull {
            weight: weight.to_vec(),
            touches,
            touched: Vec::new(),
        }
    }

    /// What one connection costs where its two ends are now.
    fn cost_of(
        &self,
        netlist: &Netlist,
        graph: &RoutingGraph,
        placement: &Placement,
        sink: usize,
    ) -> u64 {
        let weight = self.weight.get(sink).copied().unwrap_or(0);
        if weight == 0 {
            return 0;
        }
        let Some(driver) = netlist.pins[sink]
            .signal
            .and_then(|s| netlist.signals[s].driver)
        else {
            return 0;
        };
        let tile = |pin: usize| {
            placement
                .site_of(netlist.pins[pin].instance)
                .map(|s| graph.sites[s].tile)
        };
        match (tile(driver), tile(sink)) {
            (Some(a), Some(b)) => u64::from(weight) * manhattan(a, b),
            _ => 0,
        }
    }

    /// Every weighted connection's cost, summed.
    fn total(&self, netlist: &Netlist, graph: &RoutingGraph, placement: &Placement) -> u64 {
        (0..self.weight.len())
            .map(|pin| self.cost_of(netlist, graph, placement, pin))
            .sum()
    }
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
            pull: Pull::default(),
        }
    }

    /// The anneal's cost: the boxes, and the critical connections' pull.
    fn total(&self, netlist: &Netlist, graph: &RoutingGraph, placement: &Placement) -> u64 {
        self.spans.total() + self.pull.total(netlist, graph, placement)
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
    weights: &Weights,
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
        Some(start) => f64::from(start)
            .max(report.mean_displacement().ceil())
            .clamp(1.0, widest),
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
    churn.spans.weight.clone_from(&weights.nets);
    churn.pull = Pull::new(netlist, &weights.pins);
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
            samples.push(apply(netlist, graph, &mut churn, placement, &candidate));
            undo(netlist, graph, &mut churn, placement);
        }
    }
    let mut temperature = start_temperature(&samples, options.start_acceptance);

    let mut current = churn.total(netlist, graph, placement);
    let mut best = placement.clone();
    let mut best_cost = current;
    let mut stalled = 0u32;
    let signals = netlist.signals.len().max(1) as f64;
    report.stop = AnnealStop::Exhausted;
    // Counted here, not in the report, which a timing-driven placement
    // accumulates over several anneals.
    let mut temperatures = 0;
    while temperatures < options.max_temperatures {
        temperatures += 1;
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
            // A global buffer moved within its tile changes no wirelength
            // and nothing the router weighs — which of a 7-series tile's
            // 32 `BUFGCTRL`s it is only picks the clock track — so the
            // move is a random walk among equal sites, and which one
            // blink's clock buffer ended on changed with a flip-flop
            // legalisation rule that had nothing to do with it. Such a
            // move is not made; the site is the legaliser's. Every other
            // cell keeps its moves within a tile: an iCE40 logic cell's
            // position in its tile is invisible to the wirelength and not
            // to the router, and refusing them all cost `blinky_ice40` a
            // tenth of its pips.
            if candidate.iter().all(|(instance, site)| {
                netlist.instances[*instance].kind == BelRole::GlobalBuffer.keyword()
                    && placement
                        .site_of(*instance)
                        .is_some_and(|was| graph.sites[was].tile == graph.sites[*site].tile)
            }) {
                continue;
            }
            step.tried += 1;
            let delta = apply(netlist, graph, &mut churn, placement, &candidate);
            if delta <= 0.0 || rng.unit() * (temperature + delta) < temperature {
                step.accepted += 1;
            } else {
                undo(netlist, graph, &mut churn, placement);
            }
        }
        // Recounted, not accumulated: a few million `+= delta` on an f64
        // drift, and both the exit criterion and the best-so-far compare
        // against this number. The recount is one pass over the signals
        // per temperature, which is nothing beside the moves.
        current = churn.total(netlist, graph, placement);
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
    placement: &mut Placement,
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
        // The members are taken off the placement while their new sites
        // are looked for, and put back before the answer is returned.
        // This used to be a clone of the whole placement per proposal —
        // two vectors the size of the design for a macro of four — and was
        // most of what the Husky's anneal spent.
        let mut lifted: Vec<(usize, usize)> = Vec::with_capacity(m.members.len());
        for (member, _, _) in &m.members {
            if let Some(site) = placement.site_of(*member) {
                lifted.push((*member, site));
                placement.unplace(*member);
            }
        }
        let found = macro_sites(
            netlist,
            graph,
            info,
            m,
            sites_by_kind,
            shared,
            placement,
            ax,
            ay,
        );
        for (member, site) in lifted {
            placement.place(member, site);
        }
        return found;
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
fn apply(
    netlist: &Netlist,
    graph: &RoutingGraph,
    churn: &mut Churn,
    placement: &mut Placement,
    m: &Move,
) -> f64 {
    churn.previous.clear();
    for (instance, _) in m {
        let was = placement.site_of(*instance);
        churn.previous.push((*instance, was));
    }
    retile(
        netlist,
        graph,
        churn,
        placement,
        m.iter().map(|(i, s)| (*i, Some(*s))),
    )
}

/// Puts back what [`apply`] moved.
fn undo(netlist: &Netlist, graph: &RoutingGraph, churn: &mut Churn, placement: &mut Placement) {
    // `previous` is borrowed from the same value `retile` writes into, so
    // the plan is taken out and put back rather than aliased.
    let back = std::mem::take(&mut churn.previous);
    let _ = retile(netlist, graph, churn, placement, back.iter().copied());
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
    netlist: &Netlist,
    graph: &RoutingGraph,
    churn: &mut Churn,
    placement: &mut Placement,
    plan: impl Iterator<Item = (usize, Option<usize>)> + Clone,
) -> f64 {
    // The critical connections the move changes, priced before it.
    churn.pull.touched.clear();
    if !churn.pull.weight.is_empty() {
        for (instance, _) in plan.clone() {
            for index in 0..churn.pull.touches[instance].len() {
                let pin = churn.pull.touches[instance][index];
                if !churn.pull.touched.contains(&pin) {
                    churn.pull.touched.push(pin);
                }
            }
        }
    }
    let pulled_before: u64 = churn
        .pull
        .touched
        .iter()
        .map(|pin| churn.pull.cost_of(netlist, graph, placement, *pin))
        .sum();
    churn.touched.clear();
    for (instance, _) in plan.clone() {
        for (signal, _) in &churn.carried[instance] {
            if !churn.touched.contains(signal) {
                churn.touched.push(*signal);
            }
        }
    }
    let before: u64 = churn.touched.iter().map(|s| churn.spans.cost_of(*s)).sum();
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
    let after: u64 = churn.touched.iter().map(|s| churn.spans.cost_of(*s)).sum();
    count(|work| &mut work.cost_pins, pins);
    let pulled_after: u64 = churn
        .pull
        .touched
        .iter()
        .map(|pin| churn.pull.cost_of(netlist, graph, placement, *pin))
        .sum();
    (after + pulled_after) as f64 - (before + pulled_before) as f64
}

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

/// What one pin does in [`PlaceTiming`]'s analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinTiming {
    /// Not followed: a clock pin, or a pin with no timing worth estimating.
    Ignored,
    /// An input that reaches the instance's `Through` outputs, or an
    /// output those inputs reach, [`PlaceTiming::cell`] later.
    Through,
    /// An output that starts paths, [`PlaceTiming::launch`] after the
    /// instance's clock.
    Start,
    /// An input that ends paths: checked against its domain's period,
    /// [`PlaceTiming::capture`] before the edge.
    End,
}

/// Which clock domain an instance's paths start and end in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathDomain {
    /// None the analysis times.
    Untimed,
    /// One of [`PlaceTiming::periods`].
    Clock(usize),
    /// Every one: a package pin, which an outside device clocks.
    Every,
}

/// What a timing-driven placement needs: how delay is estimated, and what
/// it is measured against.
///
/// **The delays are an estimate.** A connection's is a straight line in
/// its tile distance (`wire`), fitted by the caller to what its router
/// made of a placement; a cell's is a number the caller chose. Paths are
/// timed within a domain only: one that crosses from one clock to another
/// is taken to be synchronised, which is what a design's own constraints
/// usually say of it and what this cannot check.
///
/// Each of [`PlaceTiming::rounds`] estimates every connection's
/// criticality — one less its slack over its domain's period, as VPR's
/// timing-driven placer has it (Marquardt, Betz and Rose, 2000) — weights
/// each signal's box by `1 + weight * criticality^exponent`, and anneals
/// again from where the last round left the cells. A signal's weight is its
/// most critical connection's, which over-weights a wide net for one late
/// sink: net weighting is the cruder relative of VPR's per-connection
/// timing cost, chosen because the annealer's cost is per net.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceTiming {
    /// Per netlist pin, what the analysis does with it.
    pub pins: Vec<PinTiming>,
    /// Per instance, the delay from a `Through` input to a `Through`
    /// output, in nanoseconds.
    pub cell: Vec<f64>,
    /// Per instance, the delay from its clock to a `Start` output.
    pub launch: Vec<f64>,
    /// Per instance, how long before the clock an `End` input must settle:
    /// a flip-flop's setup, or an output buffer's own delay.
    pub capture: Vec<f64>,
    /// Per instance, the domain its paths start and end in.
    pub domain: Vec<PathDomain>,
    /// Per instance, how many periods a path ending at it may take: one,
    /// or what a multicycle constraint says. Empty is one for every
    /// instance.
    pub cycles: Vec<u32>,
    /// Per instance, whether paths starting at it are not timed (a false
    /// path from it), and whether paths ending at it are not. Empty is
    /// every one timed.
    pub untimed_from: Vec<bool>,
    /// See [`PlaceTiming::untimed_from`].
    pub untimed_to: Vec<bool>,
    /// Each domain's period, in nanoseconds.
    pub periods: Vec<f64>,
    /// A connection's delay: `wire.0 + wire.1 * tiles`, plus `wire.2 *
    /// tiles` on a signal of more than [`WIDE_FANOUT`] sinks, whose tree
    /// detours.
    pub wire: (f64, f64, f64),
    /// How many rounds of estimate and anneal follow the wirelength
    /// placement.
    pub rounds: u32,
    /// How much a fully critical signal's box costs over a slack one's.
    pub weight: f64,
    /// How sharply the weight falls off below fully critical.
    pub exponent: i32,
    /// Per netlist pin, for a `Through` input: which of its instance's
    /// `Through` outputs it reaches, bit `j` for the `j`th in the
    /// instance's pin order. Empty, or a pin past its end, reaches them
    /// all. A `LUT6_2` holds two functions, and an input only one of them
    /// reads is no path to the other's output: taken as one, the
    /// analysis finds paths that are not there.
    pub reaches: Vec<u8>,
    /// Per instance, whether the timing rounds leave it on the site the
    /// wirelength placement gave it. For cells whose connections have few
    /// ways to be made at all, which moving can leave with none: see
    /// `xray::place_timing`.
    pub frozen: Vec<bool>,
}

/// The fanout above which [`PlaceTiming::wire`]'s third term applies.
pub const WIDE_FANOUT: usize = 4;

/// The move window a timing round starts with, in tiles.
const TIMING_WINDOW: u32 = 8;

/// The acceptance a timing round's start temperature aims for: cool, so
/// that a round refines the placement rather than redoing it.
const TIMING_ACCEPTANCE: f64 = 0.2;

/// The instances with `Through` outputs, each after every instance one of
/// its `Through` inputs is driven through; an instance on a loop comes
/// after the instances it can, and the loop is cut there.
fn timing_order(netlist: &Netlist, timing: &PlaceTiming) -> Vec<usize> {
    let through = |pin: usize| timing.pins.get(pin) == Some(&PinTiming::Through);
    let n = netlist.instances.len();
    let mut is_comb = vec![false; n];
    for (i, inst) in netlist.instances.iter().enumerate() {
        is_comb[i] = inst
            .pins
            .iter()
            .any(|p| netlist.pins[*p].output && through(*p));
    }
    // Kahn's algorithm over "a Through input driven by a Through output".
    let mut waiting = vec![0usize; n];
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, inst) in netlist.instances.iter().enumerate() {
        if !is_comb[i] {
            continue;
        }
        for pin in &inst.pins {
            let p = &netlist.pins[*pin];
            if p.output || !through(*pin) {
                continue;
            }
            let Some(driver) = p.signal.and_then(|s| netlist.signals[s].driver) else {
                continue;
            };
            let d = netlist.pins[driver].instance;
            if is_comb[d] && through(driver) && d != i {
                waiting[i] += 1;
                readers[d].push(i);
            }
        }
    }
    let mut order: Vec<usize> = (0..n).filter(|i| is_comb[*i] && waiting[*i] == 0).collect();
    let mut done = vec![false; n];
    let mut next = 0;
    loop {
        while next < order.len() {
            let i = order[next];
            next += 1;
            done[i] = true;
            for r in &readers[i] {
                waiting[*r] -= 1;
                if waiting[*r] == 0 {
                    order.push(*r);
                }
            }
        }
        // A loop: take the first instance still waiting, as if its loop
        // inputs were not there.
        let Some(stuck) = (0..n).find(|i| is_comb[*i] && !done[*i] && waiting[*i] > 0) else {
            break;
        };
        waiting[stuck] = 0;
        order.push(stuck);
    }
    order
}

/// Per signal, the criticality of its most critical connection over every
/// domain; and the round's longest path per domain.
fn estimate_timing(
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    timing: &PlaceTiming,
    order: &[usize],
) -> (Criticality, TimingRound) {
    let pins = netlist.pins.len();
    let tile = |instance: usize| placement.site_of(instance).map(|s| graph.sites[s].tile);
    // Each input pin's connection delay, from its driver.
    let mut wire = vec![0.0f64; pins];
    for signal in &netlist.signals {
        let Some(driver) = signal.driver else {
            continue;
        };
        let Some(from) = tile(netlist.pins[driver].instance) else {
            continue;
        };
        let wide = signal.sinks.len() > WIDE_FANOUT;
        for sink in &signal.sinks {
            let Some(to) = tile(netlist.pins[*sink].instance) else {
                continue;
            };
            let tiles = f64::from(from.0.abs_diff(to.0) + from.1.abs_diff(to.1));
            let (a, b, c) = timing.wire;
            wire[*sink] = a + b * tiles + if wide { c * tiles } else { 0.0 };
        }
    }
    let class = |pin: usize| timing.pins.get(pin).copied().unwrap_or(PinTiming::Ignored);
    let reaches = |pin: usize, j: usize| {
        timing
            .reaches
            .get(pin)
            .is_none_or(|mask| j >= 8 || mask & (1 << j) != 0)
    };
    let through_outputs = |inst: &Instance| {
        inst.pins
            .iter()
            .copied()
            .filter(|p| netlist.pins[*p].output && class(*p) == PinTiming::Through)
            .collect::<Vec<usize>>()
            .into_iter()
    };
    let driver = |pin: usize| {
        netlist.pins[pin]
            .signal
            .and_then(|s| netlist.signals[s].driver)
    };
    let mut critical = vec![0.0f64; netlist.signals.len()];
    let mut pin_critical = vec![0.0f64; pins];
    let mut longest_ps = Vec::with_capacity(timing.periods.len());
    for (domain, period) in timing.periods.iter().enumerate() {
        let here = |instance: usize| match timing.domain.get(instance) {
            Some(PathDomain::Clock(d)) => *d == domain,
            Some(PathDomain::Every) => true,
            _ => false,
        };
        let flag = |flags: &[bool], instance: usize| flags.get(instance).copied().unwrap_or(false);
        // Forwards: every output pin's arrival.
        let mut arrival = vec![f64::NEG_INFINITY; pins];
        for (index, inst) in netlist.instances.iter().enumerate() {
            if !here(index) || flag(&timing.untimed_from, index) {
                continue;
            }
            for pin in &inst.pins {
                if netlist.pins[*pin].output && class(*pin) == PinTiming::Start {
                    arrival[*pin] = timing.launch[index];
                }
            }
        }
        let input_arrival = |pin: usize, arrival: &[f64]| -> f64 {
            driver(pin).map_or(f64::NEG_INFINITY, |d| arrival[d] + wire[pin])
        };
        for &index in order {
            let inst = &netlist.instances[index];
            for (j, out) in through_outputs(inst).enumerate() {
                let latest = inst
                    .pins
                    .iter()
                    .filter(|p| !netlist.pins[**p].output && class(**p) == PinTiming::Through)
                    .filter(|p| reaches(**p, j))
                    .map(|p| input_arrival(*p, &arrival))
                    .fold(f64::NEG_INFINITY, f64::max);
                if latest > f64::NEG_INFINITY {
                    arrival[out] = arrival[out].max(latest + timing.cell[index]);
                }
            }
        }
        // The longest path, a path allowed several periods measured in
        // one: what it is late by, in the period's terms.
        let ends = |index: usize| here(index) && !flag(&timing.untimed_to, index);
        let cycles_of =
            |index: usize| f64::from(timing.cycles.get(index).copied().unwrap_or(1).max(1));
        let mut longest = 0.0f64;
        for (index, inst) in netlist.instances.iter().enumerate() {
            if !ends(index) {
                continue;
            }
            for pin in &inst.pins {
                if !netlist.pins[*pin].output && class(*pin) == PinTiming::End {
                    let at = input_arrival(*pin, &arrival) + timing.capture[index];
                    if at.is_finite() {
                        longest = longest.max(at / cycles_of(index));
                    }
                }
            }
        }
        // Backwards: every pin's required time, from the endpoints, against
        // the period or the longest path, whichever is longer. Against the
        // period alone every late connection is equally critical, and on
        // the Husky Plus thousands were: the 15 ns read path to the USB pins
        // was one of them, weighed as no worse than a path two ns late.
        // Against the longest, the latest path is 1 and the rest in
        // proportion, as VPR has it.
        let span = period.max(longest);
        let mut required = vec![f64::INFINITY; pins];
        for (index, inst) in netlist.instances.iter().enumerate() {
            if !ends(index) {
                continue;
            }
            for pin in &inst.pins {
                if !netlist.pins[*pin].output && class(*pin) == PinTiming::End {
                    required[*pin] = cycles_of(index) * span - timing.capture[index];
                }
            }
        }
        let output_required = |pin: usize, required: &[f64]| -> f64 {
            let Some(signal) = netlist.pins[pin].signal else {
                return f64::INFINITY;
            };
            netlist.signals[signal]
                .sinks
                .iter()
                .map(|s| required[*s] - wire[*s])
                .fold(f64::INFINITY, f64::min)
        };
        for &index in order.iter().rev() {
            let inst = &netlist.instances[index];
            let outputs: Vec<f64> = through_outputs(inst)
                .map(|p| output_required(p, &required))
                .collect();
            for pin in &inst.pins {
                if netlist.pins[*pin].output || class(*pin) != PinTiming::Through {
                    continue;
                }
                let earliest = outputs
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| reaches(*pin, *j))
                    .map(|(_, r)| *r)
                    .fold(f64::INFINITY, f64::min);
                if earliest < f64::INFINITY {
                    required[*pin] = required[*pin].min(earliest - timing.cell[index]);
                }
            }
        }
        // Each connection's slack, and from it its criticality.
        for (index, signal) in netlist.signals.iter().enumerate() {
            let Some(d) = signal.driver else { continue };
            if arrival[d] == f64::NEG_INFINITY {
                continue;
            }
            for sink in &signal.sinks {
                if required[*sink] == f64::INFINITY {
                    continue;
                }
                let slack = required[*sink] - (arrival[d] + wire[*sink]);
                let c = (1.0 - slack / span).clamp(0.0, 1.0);
                critical[index] = critical[index].max(c);
                pin_critical[*sink] = pin_critical[*sink].max(c);
            }
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a path of a few microseconds at most, rounded to picoseconds"
        )]
        longest_ps.push((longest * 1000.0).round().max(0.0) as u64);
    }
    let round = TimingRound {
        critical: critical.iter().filter(|c| **c > 0.5).count(),
        longest_ps,
        hpwl: 0,
    };
    (Criticality { pins: pin_critical }, round)
}

/// What [`estimate_timing`] found critical: each connection, by its sink
/// pin.
struct Criticality {
    /// Per netlist pin, for a sink; zero for anything else.
    pins: Vec<f64>,
}

/// Each connection's pull for [`Pull`], by its sink pin, from its
/// criticality: in sixteenths of a tile of wirelength, `16 * weight *
/// criticality^exponent`, so that a fully critical connection costs
/// `weight` times what a tile of a box does, and a slack one nothing.
fn connection_weights(critical: &[f64], timing: &PlaceTiming) -> Vec<u32> {
    critical
        .iter()
        .map(|c| {
            let w = 16.0 * timing.weight * c.powi(timing.exponent);
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a weight of zero to a few thousand, clamped"
            )]
            let w = w.round().clamp(0.0, 1.0e6) as u32;
            w
        })
        .collect()
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

    /// `driverless` names the signals something reads and nothing drives,
    /// and stays quiet about the two shapes that are fine: a signal with
    /// both ends, and a signal with a driver that nothing reads.
    ///
    /// Built by hand rather than compiled, because the condition comes of
    /// a pin whose role the architecture has no wire for — `build` drops
    /// those into [`Netlist::off_fabric`] and carries on — so no source
    /// text produces it on the generic device. `tests/fpga_trellis.rs`'s
    /// `a_one_bit_memory_reaches_the_narrowest_mode_with_every_pin_driven`
    /// is the end-to-end half, on the shape that really did it.
    ///
    /// What this would not catch: a pin that reaches the fabric on the
    /// *wrong* wire. Both ends exist there, and nothing in this file can
    /// tell a right wire from a wrong one.
    #[test]
    fn a_signal_something_reads_and_nothing_drives_is_named() {
        let pin = |instance: usize, port: &str, bit: u32, output: bool, signal: usize| NetPin {
            instance,
            port: port.to_owned(),
            bit,
            role: port.to_owned(),
            output,
            signal: Some(signal),
            constant: None,
        };
        let instance = |name: &str| Instance {
            cell: CellId::from_index(0),
            name: name.to_owned(),
            primitive: "LUT4".to_owned(),
            kind: "lut".to_owned(),
            pins: Vec::new(),
            pin: None,
        };
        // Three signals: one with both ends, one read by two pins and
        // driven by none, one driven and unread.
        let netlist = Netlist {
            instances: vec![instance("u_drive"), instance("u_read"), instance("u_other")],
            pins: vec![
                pin(0, "o", 0, true, 0),
                pin(1, "i0", 0, false, 0),
                pin(1, "i1", 0, false, 1),
                pin(2, "i0", 0, false, 1),
                pin(2, "o", 0, true, 2),
            ],
            signals: vec![
                Signal {
                    name: "whole".to_owned(),
                    driver: Some(0),
                    sinks: vec![1],
                },
                Signal {
                    name: "orphan".to_owned(),
                    driver: None,
                    sinks: vec![2, 3],
                },
                Signal {
                    name: "unread".to_owned(),
                    driver: Some(4),
                    sinks: Vec::new(),
                },
            ],
            off_fabric: Vec::new(),
        };
        let found = netlist.driverless();
        assert_eq!(
            found,
            vec![
                "`orphan` is read by u_other.i0[0] and 1 other pin and nothing drives it"
                    .to_owned()
            ],
            "only the signal with sinks and no driver, naming a reader and counting the rest"
        );
        // And the pair that is the whole point of the distinction: a
        // signal nothing reads is not routed either, and is not a fault.
        assert_eq!(netlist.routable(), vec![0]);
    }

    /// An instance left in the module is refused by name. The netlist
    /// used to read only the cells, so an instantiated `LUT2` vanished and
    /// the signal it drove reached its readers driven by nothing; a
    /// network of 1 355 such instances was routed, written and would have
    /// been loaded, computing something else. This catches any leftover
    /// instance; it would not catch a cell whose pins are wrongly read.
    #[test]
    fn an_instance_left_in_the_module_is_refused() {
        let text = "top t\nmodule t\n  net %a u1 wire\n  net %t u1 wire\n  \
                    port a in %a\n  port y out %t\n  \
                    instance u of LUT2 (I0=%a, I1=%a, O=%t)\nend\n";
        let mut sources = crate::source::SourceMap::new();
        let file = sources.add("t.rtl", text).unwrap();
        let design = Design::parse_text(text, file).unwrap();
        let device = crate::fpga::target("generic").unwrap();
        let (_, graph) = grid(2, 2);
        let error = Netlist::build(&design, design.top.unwrap(), device, &graph).unwrap_err();
        assert_eq!(
            error,
            PlaceError::Instance {
                instance: "u".to_owned(),
                module: "LUT2".to_owned()
            }
        );
        assert!(error.to_string().contains("`u` is an instance of `LUT2`"));
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

    /// The four-cell chain of [`chain`] on a row of ten tiles, at
    /// columns 0, 3, 4 and 9, timed from its first cell to its last.
    fn timed_chain() -> (RoutingGraph, Netlist, Placement, PlaceTiming) {
        let (_, graph) = grid(10, 1);
        let netlist = chain(4);
        let mut placement = Placement::new(4, graph.sites.len());
        for (instance, x) in [0u32, 3, 4, 9].iter().enumerate() {
            let site = graph.sites.iter().position(|s| s.tile == (*x, 0)).unwrap();
            placement.place(instance, site);
        }
        let mut pins = vec![PinTiming::Through; netlist.pins.len()];
        for (index, pin) in netlist.pins.iter().enumerate() {
            if pin.instance == 0 {
                pins[index] = PinTiming::Start;
            }
            if pin.instance == 3 {
                pins[index] = PinTiming::End;
            }
        }
        let timing = PlaceTiming {
            pins,
            cell: vec![0.0, 0.1, 0.1, 0.0],
            launch: vec![0.3, 0.0, 0.0, 0.0],
            capture: vec![0.0, 0.0, 0.0, 0.05],
            domain: vec![
                PathDomain::Clock(0),
                PathDomain::Untimed,
                PathDomain::Untimed,
                PathDomain::Clock(0),
            ],
            periods: vec![5.0],
            wire: (0.2, 0.1, 0.0),
            rounds: 1,
            weight: 8.0,
            exponent: 3,
            reaches: Vec::new(),
            cycles: Vec::new(),
            untimed_from: Vec::new(),
            untimed_to: Vec::new(),
            frozen: vec![false; 4],
        };
        (graph, netlist, placement, timing)
    }

    /// **A path's estimate is its cells and its wires, summed.** The
    /// chain's first cell launches at 0.3 ns; its three connections cross
    /// 3, 1 and 5 tiles at 0.2 ns and 0.1 ns a tile; the two cells between
    /// add 0.1 ns each and the last needs 0.05 ns before the edge: 2.05 ns
    /// in all, by hand. Every connection is on that one path, so each has
    /// its slack, 5 - 2.05 = 2.95 ns, and a criticality of 1 - 2.95 / 5.
    /// Would catch a cell's delay or a connection's left out or counted
    /// twice, the capture time forgotten, and the backward pass's slack
    /// taken from the wrong end; would not catch a delay model that is
    /// wrong about the fabric, which is the caller's numbers.
    #[test]
    fn a_paths_estimate_is_its_cells_and_wires_summed() {
        let (graph, netlist, placement, timing) = timed_chain();
        let order = timing_order(&netlist, &timing);
        assert_eq!(order, vec![1, 2]);
        let (critical, round) = estimate_timing(&netlist, &graph, &placement, &timing, &order);
        assert_eq!(round.longest_ps, vec![2050]);
        for signal in &netlist.signals {
            let c = critical.pins[signal.sinks[0]];
            assert!((c - (1.0 - 2.95 / 5.0)).abs() < 1e-9, "{:?}", critical.pins);
        }
        assert_eq!(round.critical, 0);
        // Out of its domain, nothing is timed.
        let mut other = timing.clone();
        other.domain[0] = PathDomain::Clock(1);
        other.periods.push(5.0);
        let (critical, round) = estimate_timing(&netlist, &graph, &placement, &other, &order);
        assert_eq!(round.longest_ps, vec![0, 0]);
        assert!(
            critical.pins.iter().all(|c| *c == 0.0),
            "{:?}",
            critical.pins
        );
    }

    /// **Criticality is against the longest path when it is past the
    /// period.** The chain split at its third cell, which ends one path
    /// and starts another: cells 0, 1, 2 at columns 0, 3, 4 make 1.25 ns,
    /// cells 2, 3 at 4 and 9 make 1.05 ns, and the period is 0.5. Both are
    /// late; against the period both would be fully critical, and on the
    /// Husky Plus thousands of connections were, a path two ns late
    /// weighed as heavily as the read path to the USB pins. Against the
    /// longest, the first path is 1 and the second `1 - 0.2 / 1.25`. Would
    /// catch the normalisation back on the period (the second is 1 too);
    /// would not catch a domain whose longest path is wrongly computed,
    /// which the test above pins.
    #[test]
    fn criticality_is_against_the_longest_path_past_the_period() {
        let (graph, netlist, placement, mut timing) = timed_chain();
        for (index, pin) in netlist.pins.iter().enumerate() {
            if pin.instance == 2 {
                timing.pins[index] = if pin.output {
                    PinTiming::Start
                } else {
                    PinTiming::End
                };
            }
        }
        timing.domain[2] = PathDomain::Clock(0);
        timing.launch[2] = 0.3;
        timing.capture[2] = 0.05;
        timing.periods = vec![0.5];
        let order = timing_order(&netlist, &timing);
        let (critical, round) = estimate_timing(&netlist, &graph, &placement, &timing, &order);
        assert_eq!(round.longest_ps, vec![1250]);
        let into = |instance: usize| {
            netlist.instances[instance]
                .pins
                .iter()
                .copied()
                .find(|p| !netlist.pins[*p].output)
                .unwrap()
        };
        assert!(
            (critical.pins[into(1)] - 1.0).abs() < 1e-9,
            "{:?}",
            critical.pins
        );
        assert!(
            (critical.pins[into(2)] - 1.0).abs() < 1e-9,
            "{:?}",
            critical.pins
        );
        let second = 1.0 - 0.2 / 1.25;
        assert!(
            (critical.pins[into(3)] - second).abs() < 1e-9,
            "{:?}",
            critical.pins
        );
    }

    /// **An input reaches only the outputs its mask names.** The same
    /// chain, with the middle cell's input marked as reaching none of its
    /// outputs: there is no path from the first cell to the last, so the
    /// longest path is nothing and no connection is critical; with the
    /// mask reaching its one output, it is the 2.05 ns above. Would catch
    /// the forward pass or the backward pass ignoring the mask (the path
    /// or the slack would come back); would not catch a wrong mask, which
    /// is `xray::timing`'s to compute and test.
    #[test]
    fn an_input_reaches_only_the_outputs_its_mask_names() {
        let (graph, netlist, placement, mut timing) = timed_chain();
        let order = timing_order(&netlist, &timing);
        let input = netlist.instances[1]
            .pins
            .iter()
            .copied()
            .find(|p| !netlist.pins[*p].output)
            .unwrap();
        timing.reaches = vec![u8::MAX; netlist.pins.len()];
        timing.reaches[input] = 0;
        let (critical, round) = estimate_timing(&netlist, &graph, &placement, &timing, &order);
        assert_eq!(round.longest_ps, vec![0]);
        assert!(
            critical.pins.iter().all(|c| *c == 0.0),
            "{:?}",
            critical.pins
        );
        timing.reaches[input] = 1;
        let (_, round) = estimate_timing(&netlist, &graph, &placement, &timing, &order);
        assert_eq!(round.longest_ps, vec![2050]);
    }

    /// **A critical connection pulls, and a slack one does not.**
    /// Weights are sixteenths of a tile of box: criticality 0 is nothing,
    /// 1 is `16 * weight`, and half is `16 * weight / 8` at an exponent of
    /// three. Would catch the exponent or the weight dropped, and a slack
    /// connection pulling (the placement would no longer be the
    /// wirelength one where nothing is critical).
    #[test]
    fn a_connections_pull_grows_with_its_criticality() {
        let (_, _, _, timing) = timed_chain();
        assert_eq!(
            connection_weights(&[0.0, 1.0, 0.5], &timing),
            vec![0, 128, 16]
        );
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

    /// **A move's cost change is what the cost changed by, pull
    /// included.** The timed chain, its boxes weighted as a timing round
    /// weights them and every connection pulling: moves of one cell and
    /// swaps of two, each of whose deltas must equal the difference of the
    /// recounted totals, and each undone to the total it started from.
    /// Would catch a connection priced only on one side of the move (its
    /// driver's or its sink's), a connection counted twice when both ends
    /// move, and an undo that leaves the pull behind; would not catch a
    /// pull that is consistent and wrong, which the weights' own test is
    /// for.
    #[test]
    fn a_moves_cost_change_includes_the_pull() {
        let (graph, netlist, mut placement, _) = timed_chain();
        let mut churn = Churn::new(&netlist, &graph, &placement);
        churn.spans.weight = vec![16; netlist.signals.len()];
        let mut weight = vec![0; netlist.pins.len()];
        for (index, pin) in netlist.pins.iter().enumerate() {
            if !pin.output {
                weight[index] = 40 + 10 * u32::try_from(index).unwrap();
            }
        }
        churn.pull = Pull::new(&netlist, &weight);
        let start = churn.total(&netlist, &graph, &placement);
        let site = |x: u32| graph.sites.iter().position(|s| s.tile == (x, 0)).unwrap();
        let moves: Vec<Move> = vec![
            vec![(1, site(8))],
            vec![(0, site(5)), (3, site(0))],
            vec![(2, site(9)), (3, site(4))],
            vec![(1, site(1)), (2, site(2))],
        ];
        for m in &moves {
            let before = churn.total(&netlist, &graph, &placement);
            let delta = apply(&netlist, &graph, &mut churn, &mut placement, m);
            let after = churn.total(&netlist, &graph, &placement);
            assert_eq!(delta, after as f64 - before as f64, "{m:?}");
            undo(&netlist, &graph, &mut churn, &mut placement);
            assert_eq!(
                churn.total(&netlist, &graph, &placement),
                start,
                "{m:?} undone"
            );
        }
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

    /// A fabric with a carry chain in miniature: each tile has a lookup
    /// table and a carry element whose propagate pin **is** the table's
    /// output wire, and whose carry in is fed only from the carry out of
    /// the tile one row down (`dy = +1`), the way `tileconn` joins a
    /// 7-series `CIN` to the `COUT_N` below it. Two other lookup tables
    /// per tile are free-standing.
    fn chained(width: u32, height: u32) -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("t", "test", width, height);
        let mut tile = TileType::new("logic", "logic_tile", 4, 4);
        for name in ["i", "o", "j", "q", "cin", "cout"] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        let mut lut = BelDecl::new("lut", "lut");
        lut.pins.push(("i0".to_owned(), WireRef::local("i")));
        lut.pins.push(("o".to_owned(), WireRef::local("o")));
        tile.bels.push(lut);
        let mut other = BelDecl::new("lut2", "lut");
        other.pins.push(("i0".to_owned(), WireRef::local("j")));
        other.pins.push(("o".to_owned(), WireRef::local("q")));
        tile.bels.push(other);
        let mut carry = BelDecl::new("cy", "carry");
        carry.pins.push(("p0".to_owned(), WireRef::local("o")));
        carry.pins.push(("ci".to_owned(), WireRef::local("cin")));
        carry.pins.push(("co3".to_owned(), WireRef::local("cout")));
        tile.bels.push(carry);
        tile.pips.push(PipDecl {
            from: WireRef::at("cout", 0, 1),
            to: WireRef::local("cin"),
            bits: Vec::new(),
        });
        arch.tile_types.push(tile);
        for y in 0..height {
            for x in 0..width {
                arch.set_tile(x, y, 0);
            }
        }
        let graph = arch.build_graph();
        (arch, graph)
    }

    /// `n` carry elements chained `co3` to `ci`, each with its own lookup
    /// table on `p0`.
    fn carry_chain(n: usize) -> Netlist {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: Vec::new(),
            off_fabric: Vec::new(),
        };
        let pin = |netlist: &mut Netlist, instance, role: &str, output, signal: usize| {
            let index = netlist.pins.len();
            netlist.pins.push(NetPin {
                instance,
                port: role.to_owned(),
                bit: 0,
                role: role.to_owned(),
                output,
                signal: Some(signal),
                constant: None,
            });
            if output {
                netlist.signals[signal].driver = Some(index);
            } else {
                netlist.signals[signal].sinks.push(index);
            }
            netlist.instances[instance].pins.push(index);
        };
        for k in 0..n {
            for (name, kind) in [(format!("lut{k}"), "lut"), (format!("cy{k}"), "carry")] {
                netlist.instances.push(Instance {
                    cell: CellId::from_index(netlist.instances.len()),
                    name,
                    primitive: kind.to_owned(),
                    kind: kind.to_owned(),
                    pins: Vec::new(),
                    pin: None,
                });
            }
            netlist.signals.push(Signal {
                name: format!("s{k}"),
                driver: None,
                sinks: Vec::new(),
            });
            let s = netlist.signals.len() - 1;
            pin(&mut netlist, 2 * k, "o", true, s);
            pin(&mut netlist, 2 * k + 1, "p0", false, s);
            if k > 0 {
                netlist.signals.push(Signal {
                    name: format!("c{k}"),
                    driver: None,
                    sinks: Vec::new(),
                });
                let c = netlist.signals.len() - 1;
                pin(&mut netlist, 2 * (k - 1) + 1, "co3", true, c);
                pin(&mut netlist, 2 * k + 1, "ci", false, c);
            }
        }
        netlist
    }

    /// **Dedicated wiring is read off the graph and obeyed.** Nothing here
    /// names a carry chain: the placer finds that `p0` can be reached only
    /// from the lookup table of the same tile and `ci` only from the `co3`
    /// one row down, and keeps the whole chain rigid through legalisation
    /// and annealing.
    ///
    /// Would catch: the walk stopping too early or too late, the relation
    /// inverted (the chain stacked downwards), the annealer moving one
    /// member of the group on its own, and a group that fits nowhere being
    /// reported as a part that ran out of sites. Would not catch: a real
    /// fabric whose dedicated path is longer than `DEDICATED_WALK` wires —
    /// the 7-series one is four, which `tests/fpga_xray_carry.rs`
    /// exercises, and the ECP5's is five, which
    /// `tests/fpga_trellis.rs` does.
    #[test]
    fn dedicated_wiring_holds_a_chain_together() {
        let (arch, graph) = chained(4, 6);
        let netlist = carry_chain(4);
        let (placement, report) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .unwrap();
        assert_eq!(report.dedicated, 1, "one group");
        assert_eq!(report.macros, (1, 8));
        let site = |i: usize| &graph.sites[placement.site_of(i).unwrap()];
        for k in 0..4 {
            assert_eq!(
                site(2 * k).tile,
                site(2 * k + 1).tile,
                "lut{k} beside cy{k}"
            );
            assert_eq!(
                site(2 * k).bel,
                "lut",
                "lut{k} on the table whose output is p0"
            );
            if k > 0 {
                let (below, here) = (site(2 * k - 1).tile, site(2 * k + 1).tile);
                assert_eq!(here.0, below.0, "the chain leaves its column");
                assert_eq!(here.1 + 1, below.1, "cy{k} is not one row up");
            }
        }
        // And a chain too tall for the fabric is refused, not bent — with
        // the rigid group's own error and not "the part is full", because
        // this fabric has 24 carry sites and the chain wants 7 of them.
        let netlist = carry_chain(7);
        let error = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .expect_err("a seven-tall chain on a six-tall die");
        match &error {
            PlaceError::NoDedicatedRun {
                members, available, ..
            } => {
                assert_eq!(*members, 14, "seven carries and their seven tables");
                assert_eq!(*available, 24, "and the part is nowhere near full");
            }
            other => panic!("the rigid group reported `{other}` instead"),
        }
        // A design with no dedicated wiring makes no group at all.
        let (arch, graph) = grid(4, 4);
        let (_, report) = place(
            &chain(6),
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .unwrap();
        assert_eq!(report.dedicated, 0);
    }

    /// [`chained`] with a flip-flop beside each carry that its sum `s0`
    /// reaches through the tile's own `dmux`, and that a `track` the
    /// interconnect drives reaches too: the link is local and preferred,
    /// not dedicated.
    fn chained_with_flip_flops(width: u32, height: u32) -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("t", "test", width, height);
        let mut tile = TileType::new("logic", "logic_tile", 4, 4);
        for name in [
            "i", "o", "j", "q", "cin", "cout", "sum", "dmux", "track", "clk",
        ] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        let mut lut = BelDecl::new("lut", "lut");
        lut.pins.push(("i0".to_owned(), WireRef::local("i")));
        lut.pins.push(("o".to_owned(), WireRef::local("o")));
        tile.bels.push(lut);
        let mut carry = BelDecl::new("cy", "carry");
        carry.pins.push(("p0".to_owned(), WireRef::local("o")));
        carry.pins.push(("ci".to_owned(), WireRef::local("cin")));
        carry.pins.push(("co3".to_owned(), WireRef::local("cout")));
        carry.pins.push(("s0".to_owned(), WireRef::local("sum")));
        tile.bels.push(carry);
        let mut ff = BelDecl::new("ff", "ff");
        ff.pins.push(("d".to_owned(), WireRef::local("dmux")));
        ff.pins.push(("clk".to_owned(), WireRef::local("clk")));
        tile.bels.push(ff);
        for (from, to) in [
            (WireRef::at("cout", 0, 1), WireRef::local("cin")),
            (WireRef::local("sum"), WireRef::local("dmux")),
            (WireRef::local("sum"), WireRef::at("track", 1, 0)),
            (WireRef::at("track", -1, 0), WireRef::local("dmux")),
        ] {
            tile.pips.push(PipDecl {
                from,
                to,
                bits: Vec::new(),
            });
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

    /// **A flip-flop that registers a carry's sum joins the carry's
    /// macro, beside it.** A chain of four, each carry's `s0` read by a
    /// flip-flop of one clock: the four flip-flops end in their carries'
    /// tiles, as members of the one macro. On the ChipWhisperer-Husky the
    /// two-pin nets from a carry to a flip-flop went from 36.7 tiles long
    /// on average to 0.2. A carry no macro holds draws nothing in.
    ///
    /// Would catch the link not being made (the flip-flops would place on
    /// their own, and the annealer on this tiny die might still put them
    /// close — the macro count is the check), or a carry outside any
    /// macro drawing its flip-flop in; would not catch two flip-flops of
    /// one carry that disagree on their clock joining, which this fixture
    /// has no second output to show.
    #[test]
    fn a_flip_flop_that_registers_a_carry_joins_its_macro() {
        let (arch, graph) = chained_with_flip_flops(4, 6);
        let mut netlist = carry_chain(4);
        netlist.signals.push(Signal {
            name: "clk".to_owned(),
            driver: None,
            sinks: Vec::new(),
        });
        let clk = netlist.signals.len() - 1;
        let push =
            |netlist: &mut Netlist, instance: usize, role: &str, output: bool, signal: usize| {
                let index = netlist.pins.len();
                netlist.pins.push(NetPin {
                    instance,
                    port: role.to_owned(),
                    bit: 0,
                    role: role.to_owned(),
                    output,
                    signal: Some(signal),
                    constant: None,
                });
                if output {
                    netlist.signals[signal].driver = Some(index);
                } else {
                    netlist.signals[signal].sinks.push(index);
                }
                netlist.instances[instance].pins.push(index);
            };
        for k in 0..4 {
            netlist.signals.push(Signal {
                name: format!("sum{k}"),
                driver: None,
                sinks: Vec::new(),
            });
            let sum = netlist.signals.len() - 1;
            let ff = netlist.instances.len();
            netlist.instances.push(Instance {
                cell: CellId::from_index(ff),
                name: format!("ff{k}"),
                primitive: "FDRE".to_owned(),
                kind: "ff".to_owned(),
                pins: Vec::new(),
                pin: None,
            });
            push(&mut netlist, 2 * k + 1, "s0", true, sum);
            push(&mut netlist, ff, "d", false, sum);
            push(&mut netlist, ff, "clk", false, clk);
        }
        let (placement, report) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .unwrap();
        assert_eq!(
            report.macros,
            (1, 12),
            "four carries, their tables and flip-flops"
        );
        let tile = |i: usize| graph.sites[placement.site_of(i).unwrap()].tile;
        for k in 0..4 {
            assert_eq!(tile(8 + k), tile(2 * k + 1), "ff{k} beside cy{k}");
        }

        // A lone carry, which no macro holds: its flip-flop is its own.
        let mut lone = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: vec![
                Signal {
                    name: "sum".to_owned(),
                    driver: None,
                    sinks: Vec::new(),
                },
                Signal {
                    name: "clk".to_owned(),
                    driver: None,
                    sinks: Vec::new(),
                },
            ],
            off_fabric: Vec::new(),
        };
        for (name, kind) in [("cy", "carry"), ("ff", "ff")] {
            lone.instances.push(Instance {
                cell: CellId::from_index(lone.instances.len()),
                name: name.to_owned(),
                primitive: kind.to_owned(),
                kind: kind.to_owned(),
                pins: Vec::new(),
                pin: None,
            });
        }
        push(&mut lone, 0, "s0", true, 0);
        push(&mut lone, 1, "d", false, 0);
        push(&mut lone, 1, "clk", false, 1);
        let (_, report) = place(
            &lone,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .unwrap();
        assert_eq!(report.macros, (0, 0));
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

    /// One lane of a 7-series slice, reduced to what the pool rule is
    /// about: a lookup table driving `o6` and `o5`, a flip-flop whose data
    /// mux reads `o6` or the bypass `ax`, and a carry generate whose mux
    /// reads `ax` or `o5`. `ax` is the only one of them the interconnect
    /// reaches, from the next tile, so the tile at `(0, 0)` is the one to
    /// ask about.
    fn lane() -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("t", "test", 2, 1);
        let mut tile = TileType::new("clb", "logic_tile", 8, 8);
        for name in ["o6", "o5", "ax", "d", "di", "track"] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        for (from, to) in [("o6", "d"), ("ax", "d"), ("ax", "di"), ("o5", "di")] {
            tile.pips.push(PipDecl {
                from: WireRef::local(from),
                to: WireRef::local(to),
                bits: Vec::new(),
            });
        }
        tile.pips.push(PipDecl {
            from: WireRef::at("track", 1, 0),
            to: WireRef::local("ax"),
            bits: Vec::new(),
        });
        let mut lut = BelDecl::new("lut", "lut");
        lut.pins.push(("o".to_owned(), WireRef::local("o6")));
        lut.pins.push(("o5".to_owned(), WireRef::local("o5")));
        tile.bels.push(lut);
        let mut ff = BelDecl::new("ff", "ff");
        ff.pins.push(("d".to_owned(), WireRef::local("d")));
        tile.bels.push(ff);
        let mut carry = BelDecl::new("carry", "carry");
        carry.pins.push(("di".to_owned(), WireRef::local("di")));
        tile.bels.push(carry);
        arch.tile_types.push(tile);
        for x in 0..2 {
            arch.set_tile(x, 0, 0);
        }
        let graph = arch.build_graph();
        (arch, graph)
    }

    /// One lookup table and two flip-flops a tile, sharing one clock: the
    /// table's `o` reaches the first flip-flop's `d` inside the tile and
    /// its `o5` the second's, as a 7-series lane's `O6` reaches `AFF` and
    /// its `O5` `A5FF`, and either `d` can also come from the tile to the
    /// east, so that neither is dedicated wiring `build_clusters` would
    /// have tied already.
    fn registered_lane() -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("t", "test", 2, 1);
        let mut tile = TileType::new("clb", "logic_tile", 8, 8);
        for name in ["o6", "o5", "da", "db", "clk", "track"] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        for (from, to) in [
            (WireRef::local("o6"), WireRef::local("da")),
            (WireRef::local("o5"), WireRef::local("db")),
            (WireRef::at("track", 1, 0), WireRef::local("da")),
            (WireRef::at("track", 1, 0), WireRef::local("db")),
            (WireRef::local("o6"), WireRef::local("track")),
            (WireRef::local("o5"), WireRef::local("track")),
        ] {
            tile.pips.push(PipDecl {
                from,
                to,
                bits: Vec::new(),
            });
        }
        let mut lut = BelDecl::new("lut", "lut");
        lut.pins.push(("o".to_owned(), WireRef::local("o6")));
        lut.pins.push(("o5".to_owned(), WireRef::local("o5")));
        tile.bels.push(lut);
        for (name, d) in [("ffa", "da"), ("ffb", "db")] {
            let mut ff = BelDecl::new(name, "ff");
            ff.pins.push(("d".to_owned(), WireRef::local(d)));
            ff.pins.push(("clk".to_owned(), WireRef::local("clk")));
            tile.bels.push(ff);
        }
        arch.tile_types.push(tile);
        for x in 0..2 {
            arch.set_tile(x, 0, 0);
        }
        let graph = arch.build_graph();
        (arch, graph)
    }

    /// **A lookup table takes the flip-flops it feeds into its macro, one
    /// an output, and a second only on the first one's control set.** A
    /// table's `o` and `o5` each register into a flip-flop of `
    /// registered_lane`: on one clock both join the table, on two clocks
    /// only the first does, and an `o5` someone else reads too is left
    /// alone, since its one way out of the tile is the output mux the
    /// second flip-flop's `Q` needs. Would catch `registered_tables`
    /// gone (no macro), one flip-flop per table (two members where three
    /// are wanted), the control set unchecked (three on two clocks, which
    /// no slice can hold) and the shared `o5` taken; would not catch a
    /// relation found on the wrong site, which `local_relation`'s own
    /// walk decides and `registered_outputs`' tests pin.
    #[test]
    fn a_lookup_table_takes_the_flip_flops_it_feeds_on_one_control_set() {
        let (_, graph) = registered_lane();
        let mut sites_by_kind: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, site) in graph.sites.iter().enumerate() {
            sites_by_kind
                .entry(site.kind.clone())
                .or_default()
                .push(index);
        }
        // Signals: 0 = o, 1 = o5, 2 and 3 = clocks.
        let members = |b_clock: usize, o5_elsewhere: bool| -> Vec<Vec<usize>> {
            let mut netlist = lane_netlist([
                &[("o", 0, true), ("o5", 1, true)],
                &[("d", 0, false), ("clk", 2, false)],
                &[("d", 1, false), ("clk", b_clock, false)],
            ]);
            netlist.instances[2].kind = "ff".to_owned();
            netlist.instances[2].primitive = "FF".to_owned();
            if o5_elsewhere {
                // A fourth instance, a lookup table that reads `o5` too.
                let pin = netlist.pins.len();
                netlist.pins.push(NetPin {
                    instance: 3,
                    port: "I0".to_owned(),
                    bit: 0,
                    role: "i0".to_owned(),
                    output: false,
                    signal: Some(1),
                    constant: None,
                });
                netlist.signals[1].sinks.push(pin);
                netlist.instances.push(Instance {
                    cell: CellId::from_index(3),
                    name: "reader".to_owned(),
                    primitive: "LUT".to_owned(),
                    kind: "lut".to_owned(),
                    pins: vec![pin],
                    pin: None,
                });
            }
            let mut info: Vec<Placeable> = (0..netlist.instances.len())
                .map(|_| Placeable {
                    fixed: None,
                    region: None,
                    macro_index: None,
                    allowed: None,
                })
                .collect();
            build_clusters(&netlist, &graph, &sites_by_kind, &mut info, 0)
                .iter()
                .map(|m| {
                    let mut members: Vec<usize> = m.members.iter().map(|(i, _, _)| *i).collect();
                    members.sort_unstable();
                    members
                })
                .collect()
        };
        assert_eq!(members(2, false), vec![vec![0, 1, 2]]);
        assert_eq!(members(3, false), vec![vec![0, 1]]);
        assert_eq!(members(2, true), vec![vec![0, 1]]);
    }

    /// A lookup table, a flip-flop and a carry, each pin `(role, signal,
    /// whether the instance drives it)`.
    fn lane_netlist(pins: [&[(&str, usize, bool)]; 3]) -> Netlist {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: (0..4)
                .map(|n| Signal {
                    name: format!("s{n}"),
                    driver: None,
                    sinks: Vec::new(),
                })
                .collect(),
            off_fabric: Vec::new(),
        };
        for (instance, (kind, pins)) in ["lut", "ff", "carry"].iter().zip(pins).enumerate() {
            let mut own = Vec::new();
            for (role, signal, output) in pins {
                let pin = netlist.pins.len();
                netlist.pins.push(NetPin {
                    instance,
                    port: role.to_uppercase(),
                    bit: 0,
                    role: (*role).to_owned(),
                    output: *output,
                    signal: Some(*signal),
                    constant: None,
                });
                if *output {
                    netlist.signals[*signal].driver = Some(pin);
                } else {
                    netlist.signals[*signal].sinks.push(pin);
                }
                own.push(pin);
            }
            netlist.instances.push(Instance {
                cell: CellId::from_index(instance),
                name: format!("{kind}{instance}"),
                primitive: (*kind).to_uppercase(),
                kind: (*kind).to_owned(),
                pins: own,
                pin: None,
            });
        }
        netlist
    }

    /// **A wire nothing routes into carries what its bel drives, or
    /// nothing.** The rule `SiteRules::fits` adds to the pool matching,
    /// asked of one lane in its three cases.
    ///
    /// Would catch: an idle `O5` counted as room for any signal, which is
    /// what offering it to the flip-flop's data mux did — the placer
    /// filled slices on it, and the router then had one bypass wire for
    /// two signals (66 nodes overused on the ChipWhisperer-Husky); and a
    /// signal driven onto `O5` held to `O5` for its readers too, which
    /// refused a flip-flop reading its own lane's `O5` (it goes out
    /// through the output mux and back in on `AX`) and stopped the Husky's
    /// placement with every site of the kind free. Also that
    /// `SiteRules::refusal`, which the legaliser's error is made of, says
    /// no exactly when `allows` does and names the carry's `di`. Would not
    /// catch a pool wrongly drawn, which is `captive_pool`'s and not this.
    #[test]
    fn a_wire_nothing_routes_into_carries_only_what_drives_it() {
        let (_, graph) = lane();
        let here: Vec<usize> = (0..graph.sites.len())
            .filter(|s| graph.sites[*s].tile == (0, 0))
            .collect();
        let site = |kind: &str| {
            *here
                .iter()
                .find(|s| graph.sites[**s].kind == kind)
                .unwrap_or_else(|| panic!("no `{kind}` site"))
        };
        let (lut, ff, carry) = (site("lut"), site("ff"), site("carry"));
        let fits = |netlist: &Netlist| -> bool {
            let rules = SiteRules::find(netlist, &graph);
            let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
            placement.place(0, lut);
            placement.place(1, ff);
            let allowed = rules.allows(&placement, &[(2, carry)]);
            // What the legaliser's error reports agrees with the check,
            // and names the pin that ran out.
            let why = rules.refusal(&placement, 2, carry);
            assert_eq!(allowed, why.is_none(), "{why:?}");
            if let Some(why) = why {
                assert!(why.contains("`di`"), "{why}");
            }
            allowed
        };
        // A plain table, a flip-flop and a generate both fed from
        // elsewhere: one bypass wire for two signals, and `O5` is no help
        // because nothing drives it.
        let plain = lane_netlist([&[("o", 0, true)], &[("d", 1, false)], &[("di", 2, false)]]);
        assert!(!fits(&plain), "an idle `O5` counted as a wire");
        // The generate off the table's own `O5`: the bypass is the
        // flip-flop's.
        let paired = lane_netlist([
            &[("o", 0, true), ("o5", 2, true)],
            &[("d", 1, false)],
            &[("di", 2, false)],
        ]);
        assert!(fits(&paired), "a generate off its own table's `O5`");
        // A flip-flop reading the table's `O5`, with no generate in the
        // way: out through the output mux and back in on the bypass.
        let looped = lane_netlist([&[("o", 0, true), ("o5", 1, true)], &[("d", 1, false)], &[]]);
        assert!(fits(&looped), "a flip-flop reading its own lane's `O5`");
        // And beside a generate that needs the bypass too, it cannot.
        let crowded = lane_netlist([
            &[("o", 0, true), ("o5", 1, true)],
            &[("d", 1, false)],
            &[("di", 2, false)],
        ]);
        assert!(!fits(&crowded), "two signals on one bypass wire");
    }

    /// **A cell that leaves a tile can break it.** A lookup table whose
    /// `O6` the flip-flop beside it reads, and a carry whose generate
    /// comes in on the bypass: legal, the flip-flop off `O6` and the carry
    /// off `ax`. Moving the table to the next tile leaves the flip-flop
    /// wanting `ax` too, and two signals on one wire.
    ///
    /// Would catch the legality check looking only at the tiles a move
    /// arrives at, which is what it did: the annealer would accept the
    /// move and the router find the conflict. Would not catch a pool drawn
    /// wrongly in the first place.
    #[test]
    fn a_cell_that_leaves_a_tile_can_break_it() {
        let (_, graph) = lane();
        let site = |kind: &str, x: u32| {
            (0..graph.sites.len())
                .find(|s| graph.sites[*s].kind == kind && graph.sites[*s].tile == (x, 0))
                .unwrap_or_else(|| panic!("no `{kind}` site at {x}"))
        };
        let netlist = lane_netlist([&[("o", 0, true)], &[("d", 0, false)], &[("di", 2, false)]]);
        let rules = SiteRules::find(&netlist, &graph);
        let mut placement = Placement::new(3, graph.sites.len());
        placement.place(0, site("lut", 0));
        placement.place(1, site("ff", 0));
        assert!(
            rules.allows(&placement, &[(2, site("carry", 0))]),
            "the flip-flop reads `O6` and the carry has `ax`"
        );
        placement.place(2, site("carry", 0));
        assert!(
            !rules.allows(&placement, &[(0, site("lut", 1))]),
            "with the table gone, the flip-flop and the carry both want `ax`"
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

    /// A row of three tiles, each with a pad and a global buffer, joined
    /// by general `track` both ways and by a `road` that runs east only
    /// from a pad to the buffers: a clock pad's own way up the buffer
    /// cascade, which reaches some buffers and not others.
    fn roads() -> RoutingGraph {
        let mut arch = Arch::new("t", "test", 3, 1);
        let mut tile = TileType::new("t", "t_tile", 8, 8);
        for name in ["track", "road", "pad_o", "gin"] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        let mut pad = BelDecl::new("pad", "io");
        pad.pins.push(("din".to_owned(), WireRef::local("pad_o")));
        tile.bels.push(pad);
        let mut buffer = BelDecl::new("buf", "gb");
        buffer.pins.push(("i".to_owned(), WireRef::local("gin")));
        tile.bels.push(buffer);
        for (from, to) in [
            (WireRef::local("pad_o"), "track"),
            (WireRef::at("track", -1, 0), "track"),
            (WireRef::at("track", 1, 0), "track"),
            (WireRef::local("track"), "gin"),
            (WireRef::local("pad_o"), "road"),
            (WireRef::at("road", -1, 0), "road"),
            (WireRef::local("road"), "gin"),
        ] {
            tile.pips.push(PipDecl {
                from,
                to: WireRef::local(to),
                bits: vec![crate::fpga::arch::ConfigBit::new(0, 0)],
            });
        }
        arch.tile_types.push(tile);
        for x in 0..3 {
            arch.set_tile(x, 0, 0);
        }
        arch.build_graph()
    }

    /// **A pad's buffer is confined to the buffers its road reaches.** The
    /// pad in the middle tile: general routing reaches all three buffers
    /// and the road the middle and east ones, so those two are what the
    /// buffer may take. With no roads named, nothing is confined; and a
    /// buffer whose driver is not fixed is left alone. Would catch the
    /// sweep following general routing (all three allowed), not following
    /// the road past its first wire (the east one missing), or confining a
    /// buffer no pad fixes; would not catch a real fabric's road named
    /// wrongly, which is `xray::XrayFabric::clock_roads`' and the Husky
    /// Plus build's business.
    #[test]
    fn a_pads_buffer_is_confined_to_the_buffers_its_road_reaches() {
        let graph = roads();
        let netlist = Netlist {
            instances: vec![
                Instance {
                    cell: CellId::from_index(0),
                    name: "pad".to_owned(),
                    primitive: "IBUF".to_owned(),
                    kind: "io".to_owned(),
                    pins: vec![0],
                    pin: None,
                },
                Instance {
                    cell: CellId::from_index(1),
                    name: "buffer".to_owned(),
                    primitive: "BUFG".to_owned(),
                    kind: "gb".to_owned(),
                    pins: vec![1],
                    pin: None,
                },
            ],
            pins: vec![
                NetPin {
                    instance: 0,
                    port: "O".to_owned(),
                    bit: 0,
                    role: "din".to_owned(),
                    output: true,
                    signal: Some(0),
                    constant: None,
                },
                NetPin {
                    instance: 1,
                    port: "I".to_owned(),
                    bit: 0,
                    role: "i".to_owned(),
                    output: false,
                    signal: Some(0),
                    constant: None,
                },
            ],
            signals: vec![Signal {
                name: "clk".to_owned(),
                driver: Some(0),
                sinks: vec![1],
            }],
            off_fabric: Vec::new(),
        };
        let mut sites_by_kind: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, site) in graph.sites.iter().enumerate() {
            sites_by_kind
                .entry(site.kind.clone())
                .or_default()
                .push(index);
        }
        let site = |name: &str| graph.site_index(name).expect("the bel exists");
        let info = |pad: Option<usize>| -> Vec<Placeable> {
            [pad, None]
                .into_iter()
                .map(|fixed| Placeable {
                    fixed,
                    region: None,
                    macro_index: None,
                    allowed: None,
                })
                .collect()
        };
        let road: Vec<bool> = graph.nodes.iter().map(|w| w.name == "road").collect();

        let mut fixed = info(Some(site("X1Y0/pad")));
        confine_to_roads(&netlist, &graph, &sites_by_kind, &mut fixed, &road);
        let mut want = vec![site("X1Y0/buf"), site("X2Y0/buf")];
        want.sort_unstable();
        assert_eq!(fixed[1].allowed, Some(want));

        let mut unnamed = info(Some(site("X1Y0/pad")));
        confine_to_roads(&netlist, &graph, &sites_by_kind, &mut unnamed, &[]);
        assert_eq!(unnamed[1].allowed, None, "no roads, no confinement");

        let mut loose = info(None);
        confine_to_roads(&netlist, &graph, &sites_by_kind, &mut loose, &road);
        assert_eq!(loose[1].allowed, None, "a pad no pin fixes has no road");
    }

    /// A row of tiles, each one slice of two flip-flops sharing a clock.
    fn clocked(width: u32) -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("t", "test", width, 1);
        let mut tile = TileType::new("slice", "logic_tile", 8, 8);
        tile.wires.push(WireDecl {
            name: "clk".to_owned(),
            dx: 0,
            dy: 0,
        });
        for n in 0..2 {
            let mut ff = BelDecl::new(format!("ff{n}"), "ff");
            ff.pins.push(("clk".to_owned(), WireRef::local("clk")));
            tile.bels.push(ff);
        }
        arch.tile_types.push(tile);
        for x in 0..width {
            arch.set_tile(x, 0, 0);
        }
        let graph = arch.build_graph();
        (arch, graph)
    }

    /// Flip-flops clocked by `clocks[i]`, each its own instance.
    fn clocked_by(clocks: &[usize]) -> Netlist {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: (0..=clocks.iter().copied().max().unwrap_or(0))
                .map(|n| Signal {
                    name: format!("clk{n}"),
                    driver: None,
                    sinks: Vec::new(),
                })
                .collect(),
            off_fabric: Vec::new(),
        };
        for (instance, clock) in clocks.iter().enumerate() {
            let pin = netlist.pins.len();
            netlist.pins.push(NetPin {
                instance,
                port: "C".to_owned(),
                bit: 0,
                role: "clk".to_owned(),
                output: false,
                signal: Some(*clock),
                constant: None,
            });
            netlist.signals[*clock].sinks.push(pin);
            netlist.instances.push(Instance {
                cell: CellId::from_index(instance),
                name: format!("ff{instance}"),
                primitive: "FD".to_owned(),
                kind: "ff".to_owned(),
                pins: vec![pin],
                pin: None,
            });
        }
        netlist
    }

    /// **Flip-flops that share a slice's pins agree on its reset kind and
    /// clock edge.** Two flip-flops on one clock in one two-site slice:
    /// an `FDRE` beside an `FDSE` is allowed, since both are synchronous
    /// and `ZRST` is each one's own; beside an `FDCE`, an `FDPE` or an
    /// `FDRE_1` it is refused, and the refusal says why. Would catch the mode ignored, which is a
    /// bitstream whose asynchronous resets are synchronous with nothing
    /// to say so; would not catch a primitive missing from `slice_mode`,
    /// which places as it always did.
    #[test]
    fn flip_flops_sharing_a_slice_agree_on_reset_kind_and_clock_edge() {
        let (_, graph) = clocked(1);
        for (other, allowed) in [
            ("FDSE", true),
            ("FDRE", true),
            ("FDCE", false),
            ("FDPE", false),
            ("FDRE_1", false),
        ] {
            let mut netlist = clocked_by(&[0, 0]);
            netlist.instances[0].primitive = "FDRE".to_owned();
            netlist.instances[1].primitive = other.to_owned();
            let shared = SiteRules::find(&netlist, &graph);
            let mut placement = Placement::new(2, graph.sites.len());
            placement.place(0, 0);
            assert_eq!(
                shared.allows(&placement, &[(1, 1)]),
                allowed,
                "an FDRE beside an {other}"
            );
            assert_eq!(
                shared.refusal(&placement, 1, 1).is_none(),
                allowed,
                "{other}"
            );
        }
    }

    /// **The legaliser joins a slice before it opens one.**
    ///
    /// Two slices of two flip-flops, two clocks with two flip-flops each:
    /// exactly full. The second flip-flop of clock 0 wants the right-hand
    /// tile. Taking the nearest legal site, it opened that slice, and the
    /// two flip-flops of clock 1 then had no slice left — the shape of the
    /// ChipWhisperer-Husky's stop, in four cells. Would catch an
    /// [`OPENING`] of zero, or `SiteRules::opens` answering for a slice
    /// that is in use. One would pass by the tie going to the earlier
    /// site. Would not catch a penalty so large it drags
    /// cells across the die, which is a wirelength question and not a
    /// legality one.
    /// **The anneal starts no narrower than legalisation moved a cell.**
    /// Twenty-five sinks of one net on an eight-by-eight grid — under
    /// half its sites, so `dense_kinds` does not count them — : the solve puts
    /// them all on the driver, the legaliser spreads them over several
    /// tiles, and the first temperature's window has to reach that far.
    /// Would catch the floor being dropped, which left the ChipWhisperer-
    /// Husky annealing one tile at a time from a placement legalisation
    /// had scattered; would not catch a floor set too high, which costs
    /// time and not correctness.
    #[test]
    fn the_anneal_starts_as_wide_as_legalisation_moved_a_cell() {
        let (arch, graph) = grid(8, 8);
        let netlist = fan_out(25);
        let (_, report) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .unwrap();
        let mean = report.mean_displacement();
        assert!(mean > 1.0, "the fixture should displace: {mean}");
        assert!(report.spread.is_empty(), "{:?}", report.spread);
        let first = report.schedule.first().expect("an anneal ran");
        assert!(
            f64::from(first.range_limit) >= mean.ceil(),
            "first window {} for a mean displacement of {mean}",
            first.range_limit
        );
        assert!(report.to_text().contains("legalisation moved a cell"));
    }

    /// **Bisection keeps the solve's order and gives each cell a site.**
    /// Four cells piled near the origin in a known left-to-right order,
    /// and four sites in a row: each cell takes its own site, in that
    /// order. Would catch the cut taking sites out of proportion (two
    /// cells on one site) or the sort losing the order; would not catch
    /// a poor choice of axis on a square, which only costs wirelength.
    #[test]
    fn bisection_keeps_the_solves_order_and_gives_each_cell_a_site() {
        let mut cells = vec![
            (2usize, 0.3, 0.0),
            (0, 0.1, 0.0),
            (3, 0.4, 0.0),
            (1, 0.2, 0.0),
        ];
        let mut sites = vec![(3u32, 0u32), (0, 0), (2, 0), (1, 0)];
        let mut out = vec![(0.0, 0.0); 4];
        bisect(&mut cells, &mut sites, &mut out);
        assert_eq!(out, vec![(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0)]);
    }

    /// **A kind that fills most of its sites is spread before it is
    /// legalised.** Three hundred sinks of one net on a twenty-by-twenty
    /// grid, three quarters of its sites: piled on the driver by the solve,
    /// spread, and legalised close to where spreading put them. Would catch the spread
    /// not running above the threshold, or its targets ignored; the
    /// threshold itself is the previous test's.
    #[test]
    fn a_full_kind_is_spread_before_it_is_legalised() {
        let (arch, graph) = grid(20, 20);
        let netlist = fan_out(300);
        let (_, report) = place(
            &netlist,
            &arch,
            &graph,
            &Constraints::new(),
            &PlaceOptions::default(),
        )
        .unwrap();
        assert_eq!(report.spread, vec![("lut".to_owned(), 301)]);
        assert!(
            report.mean_displacement() < 1.0,
            "spread cells moved {} tiles in legalisation",
            report.mean_displacement()
        );
    }

    /// A [`SiteMap`] answers as the hash map it replaced: nothing for a
    /// site never mapped or past the end, the previous value from an
    /// `insert` that replaces one — which is how the relation builders
    /// see a site claimed twice — and, from `first`, what the lowest
    /// mapped site maps to.
    #[test]
    fn a_site_map_answers_as_a_map() {
        let mut map = SiteMap::default();
        assert!(map.is_empty());
        assert_eq!(map.get(3), None);
        assert_eq!(map.insert(7, 2), None);
        assert_eq!(map.insert(4, 9), None);
        assert_eq!(map.get(7), Some(2));
        assert_eq!(map.get(5), None);
        assert_eq!(map.get(1000), None);
        assert_eq!(map.first(), Some(9));
        assert_eq!(map.insert(7, 0), Some(2));
        assert_eq!(map.get(7), Some(0));
        assert!(!map.is_empty());
    }

    /// **The threshold is a fifth of a kind's sites.** The same three
    /// hundred sinks fill 48 per cent of a twenty-five-by-twenty-five grid
    /// and 24.5 per cent of a thirty-five-by-thirty-five one, and both are
    /// spread — the second is the Husky Plus's share, which at a third was
    /// not; on a fifty-by-fifty grid, 12 per cent, they are left to the
    /// legaliser. Would catch the threshold back at a third or a half (the
    /// second spreads nothing) or gone (the third spreads); what spreading
    /// does is the previous test's.
    #[test]
    fn a_kind_a_fifth_full_is_spread_and_a_tenth_full_is_not() {
        for (side, spread) in [(25, true), (35, true), (50, false)] {
            let (arch, graph) = grid(side, side);
            let netlist = fan_out(300);
            let (_, report) = place(
                &netlist,
                &arch,
                &graph,
                &Constraints::new(),
                &PlaceOptions::default(),
            )
            .unwrap();
            let want = if spread {
                vec![("lut".to_owned(), 301)]
            } else {
                Vec::new()
            };
            assert_eq!(report.spread, want, "on a {side}x{side} grid");
        }
    }

    /// A chain of `n` lookup tables whose order along the chain is not the
    /// order of their indices: link `t` joins instance `(t * 7) % n` to
    /// instance `((t + 1) * 7) % n`.
    fn scrambled_chain(n: usize) -> Netlist {
        let mut netlist = chain(n);
        let at = |t: usize| (t * 7) % n;
        for (t, signal) in netlist.signals.iter().enumerate() {
            let (driver, sink) = (signal.driver.unwrap(), signal.sinks[0]);
            netlist.pins[driver].instance = at(t);
            netlist.pins[sink].instance = at(t + 1);
        }
        for instance in &mut netlist.instances {
            instance.pins.clear();
        }
        for (index, pin) in netlist.pins.iter().enumerate() {
            netlist.instances[pin.instance].pins.push(index);
        }
        netlist
    }

    /// **Global placement keeps a chain's neighbours together.** Five
    /// hundred lookup tables in a chain on a twenty-five-by-twenty-five
    /// grid, 80 % full, numbered out of the chain's order: the stages one
    /// link apart end 2.42 tiles apart on average, where one solve and one
    /// spread — `GLOBAL_ROUNDS` or `GLOBAL_PULL` at zero — leave them 6.16
    /// apart. The shape is `esad`'s 256-stage chain of reference samples on
    /// the ChipWhisperer-Husky. Would catch the rounds or the pull gone;
    /// would not catch a slower convergence that still ends under three
    /// and a half tiles, which only the Husky's wirelength shows.
    #[test]
    fn global_placement_keeps_a_chains_neighbours_together() {
        let n = 500;
        let (_, graph) = grid(25, 25);
        let netlist = scrambled_chain(n);
        let info: Vec<Placeable> = (0..n)
            .map(|_| Placeable {
                fixed: None,
                region: None,
                macro_index: None,
                allowed: None,
            })
            .collect();
        let mut sites_by_kind: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, site) in graph.sites.iter().enumerate() {
            sites_by_kind
                .entry(site.kind.clone())
                .or_default()
                .push(index);
        }
        let (positions, spread) = global_place(&netlist, &graph, &info, &[], &sites_by_kind);
        assert_eq!(spread, vec![("lut".to_owned(), n)]);
        let links: f64 = netlist
            .signals
            .iter()
            .map(|signal| {
                let a = positions[netlist.pins[signal.driver.unwrap()].instance];
                let b = positions[netlist.pins[signal.sinks[0]].instance];
                (a.0 - b.0).abs() + (a.1 - b.1).abs()
            })
            .sum();
        let mean = links / (n - 1) as f64;
        assert!(mean < 3.5, "chain links {mean:.2} tiles long on average");
    }

    /// **A star is the clique it replaces.** A fan-out of 100 from a cell
    /// fixed at one corner, its sinks held between it and the weak pull to
    /// the middle every cell has, solved with the net as a clique and as a
    /// star: the positions agree to well under a tile. Would catch a star weight that is not `k`
    /// times the clique's, or a centre counted among the cells; would not
    /// catch the two agreeing and both being wrong, which
    /// `the_solver_finds_the_obvious_answer` is for.
    #[test]
    fn a_star_net_solves_to_the_clique_it_replaces() {
        let (_, graph) = grid(16, 16);
        let netlist = fan_out(100);
        let mut info: Vec<Placeable> = (0..netlist.instances.len())
            .map(|_| Placeable {
                fixed: None,
                region: None,
                macro_index: None,
                allowed: None,
            })
            .collect();
        info[0].fixed = Some(0);
        let options = PlaceOptions {
            analytic_iterations: 2000,
            ..PlaceOptions::default()
        };
        let clique = solve_analytic_with(&netlist, &graph, &info, &options, usize::MAX);
        let star = solve_analytic_with(&netlist, &graph, &info, &options, 2);
        assert_eq!(clique.len(), netlist.instances.len());
        assert_eq!(star.len(), clique.len(), "a centre leaked into the answer");
        for (i, (c, s)) in clique.iter().zip(&star).enumerate() {
            assert!(
                (c.0 - s.0).abs() < 0.05 && (c.1 - s.1).abs() < 0.05,
                "cell {i}: clique {c:?}, star {s:?}"
            );
        }
    }

    #[test]
    fn the_legaliser_joins_a_slice_before_it_opens_one() {
        let (_, graph) = clocked(2);
        let netlist = clocked_by(&[0, 0, 1, 1]);
        let mut sites_by_kind: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, site) in graph.sites.iter().enumerate() {
            sites_by_kind
                .entry(site.kind.clone())
                .or_default()
                .push(index);
        }
        let info: Vec<Placeable> = (0..4)
            .map(|_| Placeable {
                fixed: None,
                region: None,
                macro_index: None,
                allowed: None,
            })
            .collect();
        // The order is by target, then by name: ff0 at the left, then
        // ff1, ff2 and ff3 at the right.
        let positions = [(0.0, 0.0), (1.0, 0.0), (1.0, 0.0), (1.0, 0.0)];
        let shared = SiteRules::find(&netlist, &graph);
        let mut placement = Placement::new(4, graph.sites.len());
        legalise(
            &netlist,
            &graph,
            &info,
            &[],
            &positions,
            &sites_by_kind,
            &shared,
            &mut placement,
        )
        .expect("two clocks fit two slices");
        let tile = |i: usize| graph.sites[placement.site_of(i).unwrap()].tile;
        assert_eq!(tile(0), tile(1), "clock 0 shares a slice");
        assert_eq!(tile(2), tile(3), "clock 1 shares a slice");
        assert_ne!(tile(0), tile(2));
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
