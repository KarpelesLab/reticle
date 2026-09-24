//! FPGA targets: device database, primitive mapping, constraints,
//! placement, routing, bitstreams and hand-off to other tools.
//!
//! This is the FPGA side of phase 6 of `ROADMAP.md`, and it now runs the
//! whole way. [`flow::synthesize_for`] takes a design to a netlist of one
//! device's primitives — generic synthesis, primitive mapping, LUT
//! mapping, clean-up, and the rewrite to the family's own cell names —
//! and from there a caller has two ways on:
//!
//! - **out**: [`flow::check_nextpnr_json`] verifies the netlist against
//!   the device database and [`flow::export_nextpnr`] or
//!   [`flow::export_vendor`] hands it to nextpnr, Vivado or Quartus;
//! - **through**: [`flow::place_and_route`] places it ([`mod@place`]), routes
//!   it ([`mod@route`]) and writes its bitstream ([`bitstream`]) against a
//!   routing architecture ([`arch`]). [`flow::implement`] is synthesis
//!   and that in one call, source to bitstream.
//!
//! The second way needs a routing architecture for the part, and the one
//! that ships is **synthetic**: an iCE40-shaped fabric, not an iCE40.
//! [`arch::synthetic`] says exactly what it borrows and what it invents,
//! and [`bitstream`] says exactly what its output is and is not. Nothing
//! in the placer, the router or the bitstream writer knows the
//! difference: a real database derived from Project IceStorm replaces it
//! by parsing a file ([`arch::Arch::parse`], then
//! [`flow::PnrOptions::arch`]), with no code change.
//!
//! # A real fabric, for the Xilinx 7 series
//!
//! That last sentence has been tested. [`xray`] reads Project X-Ray's
//! chip database — public domain, handed in by the caller, never fetched
//! by the library — into the same [`arch::Arch`], and [`xc7`] writes the
//! 7-series configuration container from Xilinx UG470. [`bitstream`] did
//! not change a line.
//!
//! What comes out is a structurally valid Xilinx bitstream whose IDCODE,
//! frame addresses, frame count, packet sequence and CRCs agree with one
//! Vivado made for the same part. The milestone design —
//! `examples/basys3/sw_led.v`, two switches through a lookup table to an
//! LED — places and routes on it, and decoded back into the database's
//! own feature names by [`XrayDatabase::decode`] the configuration it
//! puts on the IO blocks and IO logic is identical to what Vivado put
//! there for the same three pins of the same board.
//!
//! That is the strongest thing that can be said without a cable, and it
//! is not "it works". `docs/fpga-xray.md` sets out what is established,
//! what the fabric measures (18 055 tiles, 30.9 million graph edges and
//! 1386 MiB for the whole die) and what remains — including the 1315
//! bits of Vivado's own bitstream that nothing in the database names.
//! **One design produced by it has run on a part**: a lookup table and
//! three pins on a Digilent Basys 3, confirmed on 2026-09-24 by a person
//! flipping the switches. Nothing larger has been tried, and nothing
//! with a clock, a flip-flop or a memory can be built for a real part
//! yet.
//!
//! # The device-database model
//!
//! A [`Device`] is *data*, not code: LUT size, flip-flop features, the
//! primitives the part offers with their port names, block RAM and DSP
//! shapes, IO standards, banks and package pins, clock resources and a
//! coarse tile grid. Devices are written in the line-oriented `.dev` text
//! format ([`device`]), which round-trips exactly, so a database can be
//! generated, diffed and reviewed like source.
//!
//! Everything downstream is written against that data and against
//! *abstract roles* rather than against family names: the IO buffer is
//! "the primitive with role [`BelRole::Io`], whose `pad`, `din`, `dout`
//! and `oe` ports the database names", the block RAM is "a [`BramShape`]
//! whose width modes fit this memory". Where a family cannot express what
//! a mapping step needs, the step declines with a note instead of
//! guessing.
//!
//! # Adding a family
//!
//! 1. Write a `.dev` file describing one part per `device` block. Start
//!    from `src/fpga/devices/generic.dev`, which is the smallest complete
//!    example, and head the file with where each number came from: a
//!    device file that cannot cite its source is worse than no file.
//! 2. Fill in the port maps for the primitives you want mapped. An IO
//!    buffer needs `pad`, `din`, `dout` and optionally `oe`, plus the
//!    per-direction parameters (`param_in`, `param_out`, `param_inout`).
//!    A global buffer needs `i` and `o`. A carry element needs `ci`,
//!    `i0`, `i1` and `co`. A block RAM needs a `port` line per physical
//!    port naming its `clk`, `en`, `addr`, `din`, `dout` and `we`. A LUT
//!    needs its input pins in order (`i=I0,I1,I2,I3`), its output, and
//!    the parameter it takes its truth table in, declared with a default
//!    as wide as the family expects (`param LUT_INIT=16'h0000`).
//! 3. Declare one `bel ... ff` line per flip-flop the family offers,
//!    each with a `mode` clause saying exactly what it does (see
//!    [`FfVariant`]) and the parameters it needs. A behaviour with no
//!    line is one Reticle will report rather than approximate, so the
//!    completeness of this list is the completeness of the family's
//!    flip-flop support.
//! 4. Parse it with [`DeviceDb::parse`] and use it directly, or add it to
//!    `src/fpga/devices/` and list it in [`BUILTIN_FILES`] to compile it
//!    in. No other Rust changes are needed.
//! 5. Teach [`flow`] the command line of the place-and-route tool for the
//!    family, which is the one piece that cannot be data.
//!
//! # Pipeline
//!
//! ```no_run
//! # use reticle::diag::Diagnostics;
//! # use reticle::ir::{Design, ModuleId};
//! # use reticle::fpga::{self, Constraints, FpgaOptions};
//! # fn go(design: &mut Design, top: ModuleId, rcf: &str, diags: &mut Diagnostics) {
//! # let file = unimplemented!();
//! let device = fpga::target("ice40-hx1k-tq144").expect("built-in device");
//! let mut constraints = Constraints::parse(rcf, file, diags);
//! constraints.merge_attrs(design, top, diags);
//! constraints.check(design, device, diags);
//!
//! // Synthesis, block RAM / DSP / carry / IO / clock mapping, LUT
//! // mapping, clean-up, and the device's own cell names.
//! let report =
//!     fpga::synthesize_for(design, top, device, &constraints, &FpgaOptions::default(), diags)
//!         .expect("the flow");
//!
//! // Nothing generic is left, every port exists, every net has one
//! // driver: what comes back is what nextpnr would have complained about.
//! let problems = fpga::check_nextpnr_json(design, top, device, &constraints);
//! assert!(problems.is_empty());
//!
//! let inputs = fpga::export_nextpnr(design, top, device, &constraints).unwrap();
//! # let _ = (report, inputs);
//! # }
//! ```
//!
//! The steps are public one by one ([`map`] for the primitives,
//! [`map_cells`] for the LUT and flip-flop rename), so a caller who wants
//! a different pipeline builds it; `synthesize_for` is the common path.
//!
//! The library returns the files and the argument list; running the tool
//! is the caller's business, as everywhere else in Reticle.
//!
//! # All the way to a bitstream
//!
//! ```no_run
//! # use reticle::diag::Diagnostics;
//! # use reticle::ir::{Design, ModuleId};
//! # use reticle::fpga::{self, Constraints, FpgaOptions, PnrOptions};
//! # fn go(design: &mut Design, top: ModuleId, constraints: &Constraints,
//! #       diags: &mut Diagnostics) {
//! let device = fpga::target("ice40-hx1k-tq144").expect("built-in device");
//! let done = fpga::implement(
//!     design,
//!     top,
//!     device,
//!     constraints,
//!     &FpgaOptions::default(),
//!     &PnrOptions::new(),
//!     diags,
//! )
//! .expect("the flow");
//!
//! // What every stage did: cells, wirelength before and after
//! // annealing, the overuse of each routing iteration, and the bits.
//! print!("{}", done.to_text());
//!
//! // The routing really implements the netlist: every sink walks back
//! // to its driver.
//! assert!(done.pnr.verify().is_empty());
//!
//! let asc = done.bitstream().expect("a bitstream").write_asc();
//! # let _ = asc;
//! # }
//! ```
//!
//! See `docs/fpga.md` for the `.arch` format and for what a real iCE40
//! database would have to supply.

#[cfg(feature = "apicula")]
pub mod apicula;
pub mod arch;
pub mod bitstream;
pub mod constraints;
pub mod device;
pub mod flow;
pub mod gowin;
pub mod place;
pub mod pll;
pub mod primitives;
pub mod route;
pub mod techcells;
mod text;
pub mod xc7;
pub mod xray;

use std::sync::OnceLock;

pub use arch::{
    Arch, ArchSite, BelDecl, ConfigBit, ConfigEntry, NodeId, Pip, PipDecl, PipId, RoutingGraph,
    TileType, Wire, WireDecl, WireRef, architecture_for, builtin_architectures, builtin_graph_for,
};
pub use bitstream::{Bitstream, BitstreamError, BitstreamFormat, TileFormat};
pub use constraints::{
    ClockDef, ClockDomain, Constraints, IoAttrs, MulticyclePath, Origin, PathSpec, PinAssignment,
    Region, RegionAssignment, Rloc, matches_glob,
};
pub use device::{
    BelKind, BelRole, BramPort, BramPortRole, BramShape, ClockRegion, ClockResources, Device,
    DeviceDb, DspShape, FfFeatures, FfReset, FfVariant, Grid, IoBank, IoStandard, Pin, PinKind,
    PinName, PllDivider, PllDividerRole, PllFeedback, PllShape, Site, WideCarry,
};
pub use flow::{
    FlowError, FlowReport, NetlistProblem, NextpnrInputs, PnrOptions, PnrResult, PnrRoute,
    VendorInputs, check_nextpnr_json, constant_convention, export_nextpnr, export_vendor,
    place_and_route, pnr_route,
};
#[cfg(feature = "synth")]
pub use flow::{FpgaOptions, Implementation, implement, synthesize_for};
pub use place::{
    Instance, NetPin, Netlist, PlaceError, PlaceOptions, Placement, PlacementReport, Signal, hpwl,
    place,
};
pub use pll::PllSolution;
pub use primitives::{
    BramFallback, BramMapping, CarryMapping, ClockMapping, DspMapping, IoMapping, MapOptions,
    MapReport, PllMapping, map,
};
pub use route::{Iteration, Route, RouteError, RouteOptions, Routing, RoutingReport, route};
pub use techcells::{CellMapReport, map_cells};
pub use xc7::{
    BitHeader, FrameAddress, FrameData, FrameLayout, FrameMap, Part, TileBits, Xc7Error,
};
pub use xray::{
    Decoded, Ppip, PpipKind, SiteCoverage, XrayDatabase, XrayError, XrayFabric, XrayOptions,
    XrayStats,
};

use crate::diag::Diagnostics;
use crate::source::SourceMap;

/// The built-in device files, as `(name, contents)` pairs.
///
/// The name is only used in diagnostics, which a well-formed file never
/// produces. Adding a family means adding a file here.
pub const BUILTIN_FILES: [(&str, &str); 5] = [
    ("ice40.dev", include_str!("devices/ice40.dev")),
    ("ecp5.dev", include_str!("devices/ecp5.dev")),
    ("xc7.dev", include_str!("devices/xc7.dev")),
    ("gowin.dev", include_str!("devices/gowin.dev")),
    ("generic.dev", include_str!("devices/generic.dev")),
];

static BUILTINS: OnceLock<DeviceDb> = OnceLock::new();

/// The database of devices compiled into the crate.
///
/// The files are parsed once, on first use. They are constants checked by
/// the test suite, so a parse error in one of them is an internal
/// invariant violation and panics rather than producing a half database.
pub fn builtin_devices() -> &'static DeviceDb {
    BUILTINS.get_or_init(|| {
        let mut map = SourceMap::new();
        let mut diags = Diagnostics::new();
        let mut db = DeviceDb::new();
        for (name, text) in BUILTIN_FILES {
            let file = map
                .add(name, text)
                .expect("built-in device file fits in a source map");
            for device in DeviceDb::parse(text, file, &mut diags).devices() {
                db.insert(device.clone());
            }
        }
        assert!(
            !diags.has_errors(),
            "built-in device database is malformed:\n{}",
            diags.render(&map)
        );
        db
    })
}

/// The built-in device with the given name.
///
/// ```
/// let device = reticle::fpga::target("ice40-hx1k-tq144").unwrap();
/// assert_eq!(device.family, "ice40");
/// assert_eq!(device.lut_size, 4);
/// assert!(reticle::fpga::target("no-such-part").is_none());
/// ```
pub fn target(name: &str) -> Option<&'static Device> {
    builtin_devices().get(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_parse_and_round_trip() {
        let db = builtin_devices();
        assert_eq!(
            db.names(),
            vec![
                "ice40-lp1k-tq144",
                "ice40-hx1k-tq144",
                "ice40-hx8k-ct256",
                "ecp5-25f-CABGA381",
                "ecp5-45f-CABGA381",
                "xc7a35t-cpg236",
                "gw2a-18-pg256",
                "generic",
                "generic-k6",
            ]
        );
        let text = db.to_text();
        let mut map = SourceMap::new();
        let file = map.add("builtins.dev", &text).unwrap();
        let mut diags = Diagnostics::new();
        let again = DeviceDb::parse(&text, file, &mut diags);
        assert_eq!(diags.render(&map), "");
        assert_eq!(*db, again);
    }

    #[test]
    fn built_ins_describe_what_mapping_needs() {
        for device in builtin_devices().devices() {
            assert!(!device.family.is_empty(), "{}", device.name);
            assert!(device.lut_size >= 3, "{}", device.name);
            // The buffer for each direction, which is one primitive on a
            // family with a configurable buffer and three on Xilinx.
            // An input needs a way in, an output a way out, and both
            // need the pad.
            //
            // A BIDIRECTIONAL buffer and a GLOBAL CLOCK BUFFER are
            // required of every family *but Gowin*, and that exception is
            // deliberate rather than an omission. `devices/gowin.dev`
            // gives both reasons in full:
            //
            //   * a Gowin `IOBUF`'s output enable is `OEN` and it is
            //     ACTIVE LOW, and the `.dev` `io` line has no way to say
            //     so, so declaring it would give every tristate design an
            //     inverted enable — a bus that drives when it should
            //     listen. It is declared `other` instead, and a tristate
            //     port is reported as unbuildable;
            //   * a Gowin clock enters the global network by being
            //     *routed* onto it and not by instantiating a buffer, so
            //     there is no `BUFG` bel on a GW2A-18 at all.
            //
            // Keeping the requirement for every other family is what
            // makes this test still catch a file that simply forgot one.
            let excused = device.family == "gowin";
            for (direction, roles) in [
                ("in", &["pad", "din"][..]),
                ("out", &["pad", "dout"][..]),
                ("inout", &["pad", "din", "dout", "oe"][..]),
            ] {
                let Some(io) = device.io_bel(direction) else {
                    assert!(
                        excused && direction == "inout",
                        "{} has no {direction} IO buffer",
                        device.name
                    );
                    continue;
                };
                assert!(
                    io.has_ports(roles),
                    "{}: the {direction} buffer `{}` lacks {roles:?}",
                    device.name,
                    io.name
                );
            }
            match device.bel(BelRole::GlobalBuffer) {
                Some(gb) => assert!(gb.has_ports(&["i", "o"]), "{}", device.name),
                None => assert!(excused, "{} has no global buffer", device.name),
            }
            for bram in &device.block_rams {
                assert!(!bram.width_modes.is_empty(), "{}", device.name);
                // A narrow mode may hold fewer bits than the widest one
                // (an ECP5 block loses its parity bits below 9), but none
                // may hold more.
                let bits = bram.bits();
                for (w, d) in &bram.width_modes {
                    assert!(u64::from(*w) * u64::from(*d) <= bits, "{}", device.name);
                }
                assert!(bram.addr_width() > 0, "{}", device.name);
                assert!(bram.read_port().is_some(), "{}", device.name);
                assert!(bram.write_port().is_some(), "{}", device.name);
                // A layout the database states must add up: the words of
                // a row times the rows is the depth, no two bits share a
                // row bit, every bit fits a slot, and the word address
                // fits the address port (and fills it, when it starts
                // above bit 0, which is the point of starting there).
                let Some(params) = &bram.init_params else {
                    continue;
                };
                for &(width, depth) in &bram.width_modes {
                    let layout = bram.layout_for_mode((width, depth));
                    let init = layout.init.as_ref().unwrap_or_else(|| {
                        panic!("{}: the {width}x{depth} mode has no layout", device.name)
                    });
                    let per_row = u64::try_from(init.words.len()).unwrap();
                    assert_eq!(per_row * params.total_rows(), u64::from(depth));
                    let mut seen: Vec<u32> = init.words.iter().flatten().copied().collect();
                    let all = seen.len();
                    seen.sort_unstable();
                    seen.dedup();
                    assert_eq!(seen.len(), all, "{}: a row bit is used twice", device.name);
                    assert!(seen.iter().all(|bit| *bit < params.slot));
                    let word_bits = u64::BITS - (u64::from(depth) - 1).leading_zeros();
                    assert!(layout.addr_low + word_bits <= bram.addr_width());
                    if layout.addr_low > 0 {
                        assert_eq!(layout.addr_low + word_bits, bram.addr_width());
                    }
                    if let Some(pad) = &layout.addr_pad {
                        assert_eq!(pad.width(), layout.addr_low, "{}", device.name);
                    }
                }
            }
            for pin in &device.pins {
                if let Some(bank) = &pin.bank {
                    assert!(device.io_bank(bank).is_some(), "{}", device.name);
                }
            }
        }
    }

    /// Every built-in family can build the flip-flops synthesis infers,
    /// and says with its `mode` clauses exactly which ones.
    #[test]
    fn built_ins_declare_their_flip_flops() {
        for device in builtin_devices().devices() {
            let variants: Vec<(String, FfVariant)> = device
                .ff_variants()
                .map(|(bel, variant)| (bel.name.clone(), variant))
                .collect();
            assert!(
                !variants.is_empty(),
                "{} declares no flip-flop",
                device.name
            );
            // The four shapes every synthesised design produces.
            for (clk_pos, has_enable) in [(true, false), (true, true), (false, false)] {
                let plain = FfVariant {
                    clk_pos,
                    has_enable,
                    enable_active_high: true,
                    reset: None,
                };
                assert!(
                    device.ff_variant(plain).is_some(),
                    "{} has no flip-flop with {}",
                    device.name,
                    plain.describe()
                );
                for asynchronous in [false, true] {
                    for sets in [false, true] {
                        let variant = FfVariant {
                            reset: Some(FfReset {
                                asynchronous,
                                sets,
                                active_high: true,
                            }),
                            ..plain
                        };
                        assert!(
                            device.ff_variant(variant).is_some(),
                            "{} has no flip-flop with {}",
                            device.name,
                            variant.describe()
                        );
                    }
                }
            }
            // No two lines may claim the same behaviour, or which one
            // mapping picks would depend on file order for no reason.
            for (index, (name, variant)) in variants.iter().enumerate() {
                assert!(
                    !variants[..index].iter().any(|(_, other)| other == variant),
                    "{}: `{name}` repeats the mode {}",
                    device.name,
                    variant.flags()
                );
                let bel = device.ff_variant(*variant).expect("just listed");
                assert!(
                    bel.has_ports(&["clk", "d", "q"]),
                    "{}: `{name}` has no (clk, d, q) port map",
                    device.name
                );
                assert!(
                    !variant.has_enable || bel.port("en").is_some(),
                    "{}: `{name}` has a clock enable but no `en` port",
                    device.name
                );
                assert!(
                    variant.reset.is_none() || bel.port("rst").is_some(),
                    "{}: `{name}` has a set/reset but no `rst` port",
                    device.name
                );
            }
        }
    }

    #[test]
    fn ice40_matches_its_datasheet_figures() {
        let hx1k = target("ice40-hx1k-tq144").unwrap();
        assert_eq!(hx1k.bel(BelRole::Lut).unwrap().count, Some(1280));
        assert_eq!(hx1k.clock_resources.global_buffers, 8);
        assert_eq!(hx1k.block_rams[0].bits(), 4096);
        assert!(hx1k.dsps.is_empty(), "iCE40 LP/HX have no DSP blocks");
        assert_eq!(hx1k.pin("21").unwrap().kind, PinKind::Clock);
        let ecp5 = target("ecp5-45f-CABGA381").unwrap();
        assert_eq!(ecp5.block_rams[0].bits(), 18432);
        assert_eq!(ecp5.dsps[0].p_width, 36);
        assert!(ecp5.tile_grid.is_none());
        // The ECP5 carry unit is recorded but has no port map of either
        // shape, which is what makes carry mapping decline for the
        // family: CCU2C is neither the one-bit element nor the wide one.
        let ccu2c = ecp5.bel(BelRole::Carry).unwrap();
        assert!(!ccu2c.has_ports(&["ci"]));
        assert!(ccu2c.wide_carry().is_none());
    }
}
