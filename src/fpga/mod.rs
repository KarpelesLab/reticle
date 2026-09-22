//! FPGA targets: device database, primitive mapping, constraints and
//! hand-off to a place-and-route tool.
//!
//! This is the front half of the FPGA side of phase 6 of `ROADMAP.md`:
//! everything between a synthesised netlist and a placer. The placer and
//! router themselves are the next phase; until they land, [`flow`] hands
//! the design to nextpnr or to a vendor tool.
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
//!    port naming its `clk`, `en`, `addr`, `din`, `dout` and `we`.
//! 3. Parse it with [`DeviceDb::parse`] and use it directly, or add it to
//!    `src/fpga/devices/` and list it in [`BUILTIN_FILES`] to compile it
//!    in. No other Rust changes are needed.
//! 4. Teach [`flow`] the command line of the place-and-route tool for the
//!    family, which is the one piece that cannot be data.
//!
//! # Pipeline
//!
//! ```no_run
//! # use reticle::diag::Diagnostics;
//! # use reticle::ir::{Design, ModuleId};
//! # use reticle::fpga::{self, MapOptions, Constraints};
//! # fn go(design: &mut Design, top: ModuleId, rcf: &str, diags: &mut Diagnostics) {
//! # let file = unimplemented!();
//! let device = fpga::target("ice40-hx1k-tq144").expect("built-in device");
//! let mut constraints = Constraints::parse(rcf, file, diags);
//! constraints.merge_attrs(design, top, diags);
//! constraints.check(design, device, diags);
//! let report = fpga::map(design, top, device, &constraints, &MapOptions::default(), diags);
//! let inputs = fpga::export_nextpnr(design, top, device, &constraints).unwrap();
//! # let _ = (report, inputs);
//! # }
//! ```
//!
//! The library returns the files and the argument list; running the tool
//! is the caller's business, as everywhere else in Reticle.

pub mod constraints;
pub mod device;
pub mod flow;
pub mod primitives;
mod text;

use std::sync::OnceLock;

pub use constraints::{
    ClockDef, ClockDomain, Constraints, IoAttrs, MulticyclePath, Origin, PathSpec, PinAssignment,
    Region, RegionAssignment, Rloc, matches_glob,
};
pub use device::{
    BelKind, BelRole, BramPort, BramPortRole, BramShape, ClockRegion, ClockResources, Device,
    DeviceDb, DspShape, FfFeatures, Grid, IoBank, IoStandard, Pin, PinKind, PinName, PllShape,
    Site,
};
pub use flow::{FlowError, NextpnrInputs, VendorInputs, export_nextpnr, export_vendor};
pub use primitives::{
    BramFallback, BramMapping, CarryMapping, ClockMapping, DspMapping, IoMapping, MapOptions,
    MapReport, map,
};

use crate::diag::Diagnostics;
use crate::source::SourceMap;

/// The built-in device files, as `(name, contents)` pairs.
///
/// The name is only used in diagnostics, which a well-formed file never
/// produces. Adding a family means adding a file here.
pub const BUILTIN_FILES: [(&str, &str); 3] = [
    ("ice40.dev", include_str!("devices/ice40.dev")),
    ("ecp5.dev", include_str!("devices/ecp5.dev")),
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
            let io = device
                .bel(BelRole::Io)
                .unwrap_or_else(|| panic!("{} has no IO buffer", device.name));
            assert!(
                io.has_ports(&["pad", "din", "dout"]),
                "{} IO buffer lacks a port map",
                device.name
            );
            let gb = device
                .bel(BelRole::GlobalBuffer)
                .unwrap_or_else(|| panic!("{} has no global buffer", device.name));
            assert!(gb.has_ports(&["i", "o"]), "{}", device.name);
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
            }
            for pin in &device.pins {
                if let Some(bank) = &pin.bank {
                    assert!(device.io_bank(bank).is_some(), "{}", device.name);
                }
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
        // The ECP5 carry unit is recorded but has no port map, which is
        // what makes carry mapping decline for the family.
        assert!(!ecp5.bel(BelRole::Carry).unwrap().has_ports(&["ci"]));
    }
}
