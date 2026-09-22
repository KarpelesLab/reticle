//! The device database: what an FPGA part offers, as plain data.
//!
//! A [`Device`] describes one part: its logic cell (LUT size and flip-flop
//! features), the primitives it provides ([`BelKind`]), the shapes of its
//! block RAMs ([`BramShape`]) and DSP blocks ([`DspShape`]), its IO
//! standards, banks and package pins, its clock resources, and a coarse
//! site list and tile grid. Nothing here is Rust-specific: a device is
//! written in the line-oriented `.dev` text format, parsed with
//! [`Device::parse`] and written back with [`Device::to_text`], so a new
//! family is added by adding a file, not by changing code.
//!
//! The database is deliberately *coarse*. This phase uses it to check
//! constraints and to guide primitive mapping, not to place and route: the
//! tile grid is a bounding box, sites are optional, and routing is absent
//! entirely. The placer and router of the next phase will extend the model
//! rather than replace it.
//!
//! # The `.dev` format
//!
//! One directive per line, whitespace separated, `#` or `//` comments,
//! two-space indentation for readability (indentation is not significant).
//! A file holds one or more `device` blocks, each closed by `end`:
//!
//! ```text
//! device ice40-hx1k-tq144
//!   family ice40
//!   package tq144
//!   speed 1
//!   lut_size 4
//!   ff enable sync_reset async_reset shared_reset init 0
//!   bel SB_LUT4 lut port i=I0,I1,I2,I3 o=O param LUT_INIT=16'h0000
//!   bel SB_IO io port pad=PACKAGE_PIN din=D_IN_0 dout=D_OUT_0 oe=OUTPUT_ENABLE
//!   bram SB_RAM40_4K
//!     ports 2
//!     mode 16 256
//!     mode 8 512
//!     flags dual_port init
//!     port read clk=RCLK en=RE addr=RADDR dout=RDATA
//!     port write clk=WCLK en=WE addr=WADDR din=WDATA
//!   end
//!   dsp MULT18X18D a 18 b 18 p 36 accumulator stages 3 port a=A b=B p=P
//!   io_standard LVCMOS33 vccio 3.3 drive 4,8,12 slew slow,fast
//!   bank io vccio 3.3,2.5,1.8
//!   pin 21 clock bank io
//!   pin 99 io bank io
//!   global_buffers 8
//!   clock_region global 0 0 13 17
//!   pll SB_PLL40_CORE input 10 133 vco 533 1066 outputs 2
//!   site X1Y1 lc 1 1
//!   grid 14 18
//! end
//! ```
//!
//! Every directive is optional except `family`; a missing one leaves its
//! field empty. [`Device::to_text`] writes the directives in the order
//! above, one object per line, which makes a database diff-friendly and
//! makes `parse(to_text(d)) == d` hold exactly (comments excepted, since
//! the model does not keep them).
//!
//! # Port maps
//!
//! Primitives differ in what they call their pins, so the database carries
//! the names: a [`BelKind`] maps abstract roles (`pad`, `din`, `dout`,
//! `oe`, `i`, `o`, `ci`, `co`) onto the primitive's port names, and a
//! [`BramShape`] and [`DspShape`] do the same for their ports. Primitive
//! mapping ([`super::primitives`]) is written against the roles, so it
//! works for any family whose file fills them in, and it declines to map
//! (with a note) when a role it needs is missing.

use std::fmt;

use super::text::{Line, Token, quote, tokenize};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::AttrValue;
use crate::ir::types::Const;
use crate::source::{SourceId, Span};

/// Diagnostic code for a malformed line in a `.dev` file.
pub const DEV_SYNTAX: &str = "F0100";
/// Diagnostic code for a `.dev` construct that is well formed but unknown.
pub const DEV_UNKNOWN: &str = "F0101";

/// The name of a package pin, as printed on the datasheet (`A3`, `21`).
pub type PinName = String;

/// What a flip-flop of the device's logic cell can do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FfFeatures {
    /// A clock enable input exists.
    pub has_enable: bool,
    /// A synchronous set/reset input exists.
    pub has_sync_reset: bool,
    /// An asynchronous set/reset input exists.
    pub has_async_reset: bool,
    /// The set/reset (and usually the clock and enable) is shared by every
    /// flip-flop of one logic tile, so flops with different resets cannot
    /// be packed together.
    pub reset_is_shared: bool,
    /// The value the flop holds after configuration, when it is fixed;
    /// `None` when each flop can be initialised independently.
    pub init_value: Option<bool>,
}

/// What a [`BelKind`] is for.
///
/// Mapping looks a device's primitives up by role, so an unknown family
/// only has to say which of its primitives plays each part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BelRole {
    /// A lookup table.
    Lut,
    /// A flip-flop.
    Ff,
    /// A carry-chain element.
    Carry,
    /// An IO buffer at a package pin.
    Io,
    /// A global (low-skew) clock buffer.
    GlobalBuffer,
    /// A LUT used as a small distributed RAM.
    LutRam,
    /// A block RAM; its shape is in [`BramShape`].
    Bram,
    /// A DSP block; its shape is in [`DspShape`].
    Dsp,
    /// A PLL or other clock generator; its shape is in [`PllShape`].
    Pll,
    /// Anything else the family wants recorded.
    Other,
}

impl BelRole {
    /// Every role, in a fixed order.
    pub const ALL: [BelRole; 10] = [
        BelRole::Lut,
        BelRole::Ff,
        BelRole::Carry,
        BelRole::Io,
        BelRole::GlobalBuffer,
        BelRole::LutRam,
        BelRole::Bram,
        BelRole::Dsp,
        BelRole::Pll,
        BelRole::Other,
    ];

    /// The keyword used in the text format.
    pub fn keyword(self) -> &'static str {
        match self {
            BelRole::Lut => "lut",
            BelRole::Ff => "ff",
            BelRole::Carry => "carry",
            BelRole::Io => "io",
            BelRole::GlobalBuffer => "gb",
            BelRole::LutRam => "lutram",
            BelRole::Bram => "bram",
            BelRole::Dsp => "dsp",
            BelRole::Pll => "pll",
            BelRole::Other => "other",
        }
    }

    /// The role with the given keyword.
    pub fn from_keyword(word: &str) -> Option<BelRole> {
        BelRole::ALL.into_iter().find(|r| r.keyword() == word)
    }
}

impl fmt::Display for BelRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.keyword())
    }
}

/// One kind of basic element (BEL) the device provides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BelKind {
    /// The primitive name a netlist instantiates (`SB_LUT4`, `TRELLIS_IO`).
    pub name: String,
    /// What the primitive is for.
    pub role: BelRole,
    /// How many the device has, when the number is known.
    pub count: Option<u32>,
    /// Abstract role to port name, in file order: `pad`, `din`, `dout`,
    /// `oe` for an IO buffer, `i`/`o` for a buffer or LUT, `ci`/`i0`/`i1`/
    /// `co` for a carry element. A role whose primitive has several ports
    /// (a LUT's inputs) lists them comma-separated in one entry.
    ///
    /// The two data roles of an IO buffer are named from the fabric's
    /// point of view: `din` is the port that delivers the pad's value *to*
    /// the design, `dout` the port that takes the design's value *to* the
    /// pad. On `SB_IO` they are `D_IN_0` and `D_OUT_0`, on `TRELLIS_IO`
    /// they are `O` and `I`.
    pub ports: Vec<(String, String)>,
    /// Parameters every instance of the primitive carries, in file order.
    pub params: Vec<(String, AttrValue)>,
    /// Parameters added only in a given situation, keyed by a condition
    /// word, in file order. The conditions mapping uses are `in`, `out`
    /// and `inout` for the direction of the port an IO buffer serves, and
    /// `pullup` for a pulled-up input. This is how a family states its
    /// IO conventions (`PIN_TYPE` on iCE40, `DIR` and `PULLMODE` on ECP5)
    /// as data rather than as code.
    pub cond_params: Vec<(String, Vec<(String, AttrValue)>)>,
}

impl BelKind {
    /// A primitive with a name and a role and nothing else.
    pub fn new(name: impl Into<String>, role: BelRole) -> Self {
        BelKind {
            name: name.into(),
            role,
            count: None,
            ports: Vec::new(),
            params: Vec::new(),
            cond_params: Vec::new(),
        }
    }

    /// The port name playing `role`, if the database records one.
    pub fn port(&self, role: &str) -> Option<&str> {
        self.ports
            .iter()
            .find(|(r, _)| r == role)
            .map(|(_, n)| n.as_str())
    }

    /// The parameters this primitive takes in the situation `condition`
    /// (`in`, `out`, `inout`, `pullup`); empty when the database records
    /// none for it.
    pub fn params_when(&self, condition: &str) -> &[(String, AttrValue)] {
        self.cond_params
            .iter()
            .find(|(c, _)| c == condition)
            .map_or(&[][..], |(_, params)| params.as_slice())
    }

    /// True when every role in `roles` has a port name.
    pub fn has_ports(&self, roles: &[&str]) -> bool {
        roles.iter().all(|r| self.port(r).is_some())
    }
}

/// Which side of a block RAM one physical port is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BramPortRole {
    /// Reads only.
    Read,
    /// Writes only.
    Write,
    /// Reads and writes (a true dual-port block).
    ReadWrite,
}

impl BramPortRole {
    /// The keyword used in the text format.
    pub fn keyword(self) -> &'static str {
        match self {
            BramPortRole::Read => "read",
            BramPortRole::Write => "write",
            BramPortRole::ReadWrite => "rw",
        }
    }

    /// The role with the given keyword.
    pub fn from_keyword(word: &str) -> Option<BramPortRole> {
        match word {
            "read" => Some(BramPortRole::Read),
            "write" => Some(BramPortRole::Write),
            "rw" => Some(BramPortRole::ReadWrite),
            _ => None,
        }
    }

    /// True when the port can read.
    pub fn reads(self) -> bool {
        matches!(self, BramPortRole::Read | BramPortRole::ReadWrite)
    }

    /// True when the port can write.
    pub fn writes(self) -> bool {
        matches!(self, BramPortRole::Write | BramPortRole::ReadWrite)
    }
}

/// One physical port of a block RAM, with its pin names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BramPort {
    /// What the port can do.
    pub role: BramPortRole,
    /// Abstract signal role to port name, in file order: `clk`, `en`,
    /// `ce`, `addr`, `din`, `dout`, `we`, `rst`.
    pub signals: Vec<(String, String)>,
}

impl BramPort {
    /// The port name playing `role`, if the database records one.
    pub fn signal(&self, role: &str) -> Option<&str> {
        self.signals
            .iter()
            .find(|(r, _)| r == role)
            .map(|(_, n)| n.as_str())
    }
}

/// The shape of one kind of block RAM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BramShape {
    /// The primitive name (`SB_RAM40_4K`, `DP16KD`).
    pub name: String,
    /// The `(data_width, depth)` pairs the block can be configured as, in
    /// file order. Every pair of one block holds the same number of bits.
    pub width_modes: Vec<(u32, u32)>,
    /// The parameters that select each mode, parallel to `width_modes`:
    /// `mode_params[i]` configures `width_modes[i]` (`READ_MODE` and
    /// `WRITE_MODE` on iCE40, `DATA_WIDTH_A` and `DATA_WIDTH_B` on ECP5).
    /// Use [`BramShape::params_for_mode`] rather than indexing it.
    pub mode_params: Vec<Vec<(String, AttrValue)>>,
    /// Number of physical ports.
    pub ports: u32,
    /// The block has per-byte write enables.
    pub has_byte_enable: bool,
    /// The block has two independent ports (true or simple dual port; see
    /// [`BramPort::role`] for which).
    pub dual_port: bool,
    /// The block's contents can be initialised by the bitstream.
    pub init_supported: bool,
    /// The physical ports and their pin names, in file order.
    pub port_map: Vec<BramPort>,
}

impl BramShape {
    /// The largest number of bits the block holds, over its width modes.
    pub fn bits(&self) -> u64 {
        self.width_modes
            .iter()
            .map(|(w, d)| u64::from(*w) * u64::from(*d))
            .max()
            .unwrap_or(0)
    }

    /// The narrowest mode at least `width` bits wide, or the widest mode
    /// when none is wide enough.
    ///
    /// Mapping a memory uses this to pick the mode that wastes least:
    /// a 12-bit memory goes into a 16-bit mode, a 40-bit one is split
    /// across blocks in the widest mode available.
    pub fn mode_for_width(&self, width: u32) -> Option<(u32, u32)> {
        let fitting = self
            .width_modes
            .iter()
            .filter(|(w, _)| *w >= width)
            .min_by_key(|(w, _)| *w);
        fitting
            .or_else(|| self.width_modes.iter().max_by_key(|(w, _)| *w))
            .copied()
    }

    /// The parameters selecting `mode`, empty when the database records
    /// none.
    pub fn params_for_mode(&self, mode: (u32, u32)) -> &[(String, AttrValue)] {
        self.width_modes
            .iter()
            .position(|m| *m == mode)
            .and_then(|index| self.mode_params.get(index))
            .map_or(&[][..], Vec::as_slice)
    }

    /// The number of address bits the primitive's address port takes,
    /// which is what its deepest mode needs.
    pub fn addr_width(&self) -> u32 {
        let deepest = self
            .width_modes
            .iter()
            .map(|(_, depth)| u64::from(*depth))
            .max()
            .unwrap_or(0);
        if deepest <= 1 {
            0
        } else {
            u64::BITS - (deepest - 1).leading_zeros()
        }
    }

    /// The first port that can read.
    pub fn read_port(&self) -> Option<&BramPort> {
        self.port_map.iter().find(|p| p.role.reads())
    }

    /// The first port that can write.
    pub fn write_port(&self) -> Option<&BramPort> {
        self.port_map.iter().find(|p| p.role.writes())
    }
}

/// The shape of one kind of DSP block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DspShape {
    /// The primitive name (`MULT18X18D`).
    pub name: String,
    /// Width of the first multiplier operand.
    pub a_width: u32,
    /// Width of the second multiplier operand.
    pub b_width: u32,
    /// Width of the product (and of the accumulator, when there is one).
    pub p_width: u32,
    /// An adder in front of the multiplier exists.
    pub has_preadder: bool,
    /// An adder or accumulator after the multiplier exists.
    pub has_accumulator: bool,
    /// How many pipeline registers the block can use.
    pub pipeline_stages: u32,
    /// Abstract role to port name, in file order: `a`, `b`, `c`, `p`,
    /// `clk`, `ce`, `rst`.
    pub ports: Vec<(String, String)>,
}

impl DspShape {
    /// The port name playing `role`, if the database records one.
    pub fn port(&self, role: &str) -> Option<&str> {
        self.ports
            .iter()
            .find(|(r, _)| r == role)
            .map(|(_, n)| n.as_str())
    }

    /// True when an `a_width` x `b_width` multiply of the given operand
    /// widths fits this block.
    pub fn fits_multiply(&self, a: u32, b: u32, p: u32) -> bool {
        a <= self.a_width && b <= self.b_width && p <= self.p_width
    }
}

/// One IO standard the device supports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoStandard {
    /// The standard's name as constraints spell it (`LVCMOS33`).
    pub name: String,
    /// Drive strengths in milliamperes, in file order; empty when the
    /// standard has no choice.
    pub drive_strengths: Vec<u32>,
    /// Slew rate names, in file order (`slow`, `fast`); empty when the
    /// standard has no choice.
    pub slew_rates: Vec<String>,
    /// The standard is differential and uses a pin pair.
    pub differential: bool,
    /// The bank supply voltage the standard needs, as written on the
    /// datasheet (`3.3`), or `None` when it does not constrain it.
    pub vccio: Option<String>,
}

impl IoStandard {
    /// A standard with a name and nothing else.
    pub fn new(name: impl Into<String>) -> Self {
        IoStandard {
            name: name.into(),
            drive_strengths: Vec::new(),
            slew_rates: Vec::new(),
            differential: false,
            vccio: None,
        }
    }
}

/// One IO bank: a group of pins sharing a supply voltage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoBank {
    /// The bank name as constraints spell it (`0`, `bottom`).
    pub name: String,
    /// The pins in the bank, filled in from the [`Pin`] list when a device
    /// is parsed.
    pub pins: Vec<PinName>,
    /// The supply voltages the bank accepts, in file order.
    pub vccio_options: Vec<String>,
}

/// What a package pin is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PinKind {
    /// A general purpose IO pin.
    Io,
    /// Ground.
    Gnd,
    /// A supply pin.
    Vcc,
    /// A configuration pin (`CDONE`, `CRESET`, the SPI pins).
    Config,
    /// An IO pin that also reaches a global clock buffer directly.
    Clock,
}

impl PinKind {
    /// Every kind, in a fixed order.
    pub const ALL: [PinKind; 5] = [
        PinKind::Io,
        PinKind::Gnd,
        PinKind::Vcc,
        PinKind::Config,
        PinKind::Clock,
    ];

    /// The keyword used in the text format.
    pub fn keyword(self) -> &'static str {
        match self {
            PinKind::Io => "io",
            PinKind::Gnd => "gnd",
            PinKind::Vcc => "vcc",
            PinKind::Config => "config",
            PinKind::Clock => "clock",
        }
    }

    /// The kind with the given keyword.
    pub fn from_keyword(word: &str) -> Option<PinKind> {
        PinKind::ALL.into_iter().find(|k| k.keyword() == word)
    }

    /// True when a design may drive or read the pin: [`PinKind::Io`] and
    /// [`PinKind::Clock`].
    pub fn is_io(self) -> bool {
        matches!(self, PinKind::Io | PinKind::Clock)
    }
}

impl fmt::Display for PinKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.keyword())
    }
}

/// One package pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The pin name as printed on the datasheet (`A3`, `21`).
    pub name: PinName,
    /// The site inside the die the pin reaches, when known.
    pub site: Option<String>,
    /// The IO bank the pin belongs to, when it has one.
    pub bank: Option<String>,
    /// What the pin is for.
    pub kind: PinKind,
}

/// The shape of one PLL or clock generator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PllShape {
    /// The primitive name (`SB_PLL40_CORE`, `EHXPLLL`).
    pub name: String,
    /// The reference frequency range in MHz, when known.
    pub input_mhz: Option<(u32, u32)>,
    /// The VCO frequency range in MHz, when known.
    pub vco_mhz: Option<(u32, u32)>,
    /// How many independent outputs the block has.
    pub outputs: u32,
}

/// A rectangular part of the tile grid a clock network covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockRegion {
    /// The region name.
    pub name: String,
    /// Left edge, inclusive.
    pub x0: u32,
    /// Bottom edge, inclusive.
    pub y0: u32,
    /// Right edge, inclusive.
    pub x1: u32,
    /// Top edge, inclusive.
    pub y1: u32,
}

/// What the device offers for distributing clocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockResources {
    /// How many global (low-skew) clock networks the device has.
    pub global_buffers: u32,
    /// The parts of the die a clock network covers, in file order; empty
    /// when the whole device is one region.
    pub regions: Vec<ClockRegion>,
    /// The clock generators the device has, in file order.
    pub plls: Vec<PllShape>,
}

/// One placeable location inside the die.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    /// The site name as a constraint spells it.
    pub name: String,
    /// What can be placed here (`lc`, `io`, `ram`, `pll`), free-form so a
    /// family can name its own.
    pub kind: String,
    /// Column in the tile grid.
    pub x: u32,
    /// Row in the tile grid.
    pub y: u32,
}

/// The extent of the tile grid, used to check placement regions.
///
/// The grid is a bounding box only: `x` runs `0 .. width` and `y` runs
/// `0 .. height`. What is in each tile is not modelled in this phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    /// Number of tile columns.
    pub width: u32,
    /// Number of tile rows.
    pub height: u32,
}

impl Grid {
    /// True when `(x, y)` is inside the grid.
    pub fn contains(self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height
    }
}

/// One FPGA part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The name this device is looked up by (`ice40-hx1k-tq144`).
    pub name: String,
    /// The family it belongs to (`ice40`, `ecp5`, `generic`), which
    /// selects the vendor conventions of the exporters.
    pub family: String,
    /// The package (`tq144`, `CABGA381`), or empty when unspecified.
    pub package: String,
    /// The speed grade as the vendor writes it, or empty.
    pub speed_grade: String,
    /// Number of inputs of one lookup table.
    pub lut_size: u32,
    /// What the logic cell's flip-flop can do.
    pub ff: FfFeatures,
    /// The primitives the device provides, in file order.
    pub bels: Vec<BelKind>,
    /// The block RAM shapes, in file order.
    pub block_rams: Vec<BramShape>,
    /// The DSP shapes, in file order.
    pub dsps: Vec<DspShape>,
    /// The IO standards, in file order.
    pub io_standards: Vec<IoStandard>,
    /// The IO banks, in file order.
    pub io_banks: Vec<IoBank>,
    /// The package pins, in file order.
    pub pins: Vec<Pin>,
    /// Clock buffers, clock regions and PLLs.
    pub clock_resources: ClockResources,
    /// Placeable sites, in file order; may be empty.
    pub sites: Vec<Site>,
    /// The extent of the tile grid, when the device records one.
    pub tile_grid: Option<Grid>,
}

impl Device {
    /// An empty device with the given name and family.
    pub fn new(name: impl Into<String>, family: impl Into<String>) -> Self {
        Device {
            name: name.into(),
            family: family.into(),
            package: String::new(),
            speed_grade: String::new(),
            lut_size: 4,
            ff: FfFeatures::default(),
            bels: Vec::new(),
            block_rams: Vec::new(),
            dsps: Vec::new(),
            io_standards: Vec::new(),
            io_banks: Vec::new(),
            pins: Vec::new(),
            clock_resources: ClockResources::default(),
            sites: Vec::new(),
            tile_grid: None,
        }
    }

    /// The first primitive with the given role.
    pub fn bel(&self, role: BelRole) -> Option<&BelKind> {
        self.bels.iter().find(|b| b.role == role)
    }

    /// The primitive with the given name.
    pub fn bel_named(&self, name: &str) -> Option<&BelKind> {
        self.bels.iter().find(|b| b.name == name)
    }

    /// The package pin with the given name.
    pub fn pin(&self, name: &str) -> Option<&Pin> {
        self.pins.iter().find(|p| p.name == name)
    }

    /// The IO bank with the given name.
    pub fn io_bank(&self, name: &str) -> Option<&IoBank> {
        self.io_banks.iter().find(|b| b.name == name)
    }

    /// The IO standard with the given name, compared case-insensitively
    /// because constraint files are inconsistent about case.
    pub fn io_standard(&self, name: &str) -> Option<&IoStandard> {
        self.io_standards
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// The site with the given name.
    pub fn site(&self, name: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.name == name)
    }

    /// Parses exactly one `device` block from `text`.
    ///
    /// Problems are reported through `diags` with spans into `file`;
    /// `None` means the text held no usable device. A file with several
    /// devices is a [`DeviceDb`]; this returns its first device and
    /// reports the rest as an error.
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> Option<Device> {
        let mut db = DeviceDb::parse(text, file, diags);
        if db.len() > 1 {
            let span = Span::new(file, 0, 0);
            diags.push(
                Diagnostic::error("expected a single device in this file")
                    .with_code(DEV_SYNTAX)
                    .with_span(span)
                    .with_note(format!("the file describes {} devices", db.len())),
            );
        }
        if db.devices.is_empty() {
            None
        } else {
            Some(db.devices.remove(0))
        }
    }

    /// Renders the device in the `.dev` text format.
    ///
    /// The result parses back to an equal device, so a database can be
    /// round-tripped through a file and diffed line by line.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write_text(&mut out);
        out
    }

    fn write_text(&self, out: &mut String) {
        out.push_str(&format!("device {}\n", quote(&self.name)));
        out.push_str(&format!("  family {}\n", quote(&self.family)));
        if !self.package.is_empty() {
            out.push_str(&format!("  package {}\n", quote(&self.package)));
        }
        if !self.speed_grade.is_empty() {
            out.push_str(&format!("  speed {}\n", quote(&self.speed_grade)));
        }
        out.push_str(&format!("  lut_size {}\n", self.lut_size));
        out.push_str(&format!("  {}\n", write_ff(&self.ff)));
        for bel in &self.bels {
            out.push_str(&format!("  {}\n", write_bel(bel)));
        }
        for bram in &self.block_rams {
            write_bram(out, bram);
        }
        for dsp in &self.dsps {
            out.push_str(&format!("  {}\n", write_dsp(dsp)));
        }
        for std in &self.io_standards {
            out.push_str(&format!("  {}\n", write_io_standard(std)));
        }
        for bank in &self.io_banks {
            let mut line = format!("  bank {}", quote(&bank.name));
            if !bank.vccio_options.is_empty() {
                line.push_str(&format!(" vccio {}", join(&bank.vccio_options)));
            }
            out.push_str(&line);
            out.push('\n');
        }
        for pin in &self.pins {
            let mut line = format!("  pin {} {}", quote(&pin.name), pin.kind.keyword());
            if let Some(bank) = &pin.bank {
                line.push_str(&format!(" bank {}", quote(bank)));
            }
            if let Some(site) = &pin.site {
                line.push_str(&format!(" site {}", quote(site)));
            }
            out.push_str(&line);
            out.push('\n');
        }
        let clocks = &self.clock_resources;
        if clocks.global_buffers > 0 {
            out.push_str(&format!("  global_buffers {}\n", clocks.global_buffers));
        }
        for region in &clocks.regions {
            out.push_str(&format!(
                "  clock_region {} {} {} {} {}\n",
                quote(&region.name),
                region.x0,
                region.y0,
                region.x1,
                region.y1
            ));
        }
        for pll in &clocks.plls {
            let mut line = format!("  pll {}", quote(&pll.name));
            if let Some((lo, hi)) = pll.input_mhz {
                line.push_str(&format!(" input {lo} {hi}"));
            }
            if let Some((lo, hi)) = pll.vco_mhz {
                line.push_str(&format!(" vco {lo} {hi}"));
            }
            line.push_str(&format!(" outputs {}", pll.outputs));
            out.push_str(&line);
            out.push('\n');
        }
        for site in &self.sites {
            out.push_str(&format!(
                "  site {} {} {} {}\n",
                quote(&site.name),
                quote(&site.kind),
                site.x,
                site.y
            ));
        }
        if let Some(grid) = self.tile_grid {
            out.push_str(&format!("  grid {} {}\n", grid.width, grid.height));
        }
        out.push_str("end\n");
    }
}

fn join(items: &[String]) -> String {
    items
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",")
}

fn write_ff(ff: &FfFeatures) -> String {
    let mut line = String::from("ff");
    if ff.has_enable {
        line.push_str(" enable");
    }
    if ff.has_sync_reset {
        line.push_str(" sync_reset");
    }
    if ff.has_async_reset {
        line.push_str(" async_reset");
    }
    if ff.reset_is_shared {
        line.push_str(" shared_reset");
    }
    if let Some(value) = ff.init_value {
        line.push_str(&format!(" init {}", u8::from(value)));
    }
    line
}

fn write_pairs(kind: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let mut line = format!(" {kind}");
    for (role, name) in pairs {
        line.push_str(&format!(" {}={}", quote(role), quote(name)));
    }
    line
}

fn write_bel(bel: &BelKind) -> String {
    let mut line = format!("bel {} {}", quote(&bel.name), bel.role.keyword());
    if let Some(count) = bel.count {
        line.push_str(&format!(" count {count}"));
    }
    line.push_str(&write_pairs("port", &bel.ports));
    write_params(&mut line, "param", &bel.params);
    for (condition, params) in &bel.cond_params {
        write_params(&mut line, &format!("param_{condition}"), params);
    }
    line
}

fn write_params(line: &mut String, keyword: &str, params: &[(String, AttrValue)]) {
    if params.is_empty() {
        return;
    }
    line.push(' ');
    line.push_str(keyword);
    for (key, value) in params {
        line.push_str(&format!(" {}={}", quote(key), write_value(value)));
    }
}

fn write_value(value: &AttrValue) -> String {
    match value {
        AttrValue::Int(v) => v.to_string(),
        AttrValue::Const(c) => c.to_verilog_literal(),
        AttrValue::String(s) => quote(s),
    }
}

fn write_bram(out: &mut String, bram: &BramShape) {
    out.push_str(&format!("  bram {}\n", quote(&bram.name)));
    out.push_str(&format!("    ports {}\n", bram.ports));
    for (index, (width, depth)) in bram.width_modes.iter().enumerate() {
        let mut line = format!("    mode {width} {depth}");
        if let Some(params) = bram.mode_params.get(index)
            && !params.is_empty()
        {
            write_params(&mut line, "param", params);
        }
        out.push_str(&line);
        out.push('\n');
    }
    let mut flags = String::new();
    if bram.has_byte_enable {
        flags.push_str(" byte_enable");
    }
    if bram.dual_port {
        flags.push_str(" dual_port");
    }
    if bram.init_supported {
        flags.push_str(" init");
    }
    if !flags.is_empty() {
        out.push_str(&format!("    flags{flags}\n"));
    }
    for port in &bram.port_map {
        let mut line = format!("    port {}", port.role.keyword());
        for (role, name) in &port.signals {
            line.push_str(&format!(" {}={}", quote(role), quote(name)));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("  end\n");
}

fn write_dsp(dsp: &DspShape) -> String {
    let mut line = format!(
        "dsp {} a {} b {} p {}",
        quote(&dsp.name),
        dsp.a_width,
        dsp.b_width,
        dsp.p_width
    );
    if dsp.has_preadder {
        line.push_str(" preadder");
    }
    if dsp.has_accumulator {
        line.push_str(" accumulator");
    }
    if dsp.pipeline_stages > 0 {
        line.push_str(&format!(" stages {}", dsp.pipeline_stages));
    }
    line.push_str(&write_pairs("port", &dsp.ports));
    line
}

fn write_io_standard(std: &IoStandard) -> String {
    let mut line = format!("io_standard {}", quote(&std.name));
    if let Some(vccio) = &std.vccio {
        line.push_str(&format!(" vccio {}", quote(vccio)));
    }
    if !std.drive_strengths.is_empty() {
        let drives: Vec<String> = std.drive_strengths.iter().map(u32::to_string).collect();
        line.push_str(&format!(" drive {}", join(&drives)));
    }
    if !std.slew_rates.is_empty() {
        line.push_str(&format!(" slew {}", join(&std.slew_rates)));
    }
    if std.differential {
        line.push_str(" differential");
    }
    line
}

/// Several devices, looked up by name.
///
/// The built-in database is [`super::builtin_devices`]; a project adds its
/// own parts by parsing more `.dev` text into the same value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceDb {
    devices: Vec<Device>,
}

impl DeviceDb {
    /// An empty database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses every `device` block in `text`.
    ///
    /// Malformed lines are reported through `diags` and skipped, so one bad
    /// directive does not lose the rest of the file.
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> DeviceDb {
        let lines = tokenize(text, file);
        let mut parser = Parser {
            lines: &lines,
            pos: 0,
            diags,
        };
        let mut db = DeviceDb::new();
        while let Some(line) = parser.peek() {
            if line.keyword() == "device" {
                let span = line.span;
                if let Some(device) = parser.device() {
                    if db.get(&device.name).is_some() {
                        parser.diags.push(
                            Diagnostic::error(format!("device `{}` is defined twice", device.name))
                                .with_code(DEV_SYNTAX)
                                .with_span(span),
                        );
                    } else {
                        db.insert(device);
                    }
                }
            } else {
                let span = line.span;
                let keyword = line.keyword().to_owned();
                parser.bump();
                parser.error(span, format!("expected `device`, found `{keyword}`"));
            }
        }
        db
    }

    /// Adds a device, replacing one of the same name.
    pub fn insert(&mut self, device: Device) {
        match self.devices.iter_mut().find(|d| d.name == device.name) {
            Some(slot) => *slot = device,
            None => self.devices.push(device),
        }
    }

    /// The device with the given name.
    pub fn get(&self, name: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.name == name)
    }

    /// Every device, in insertion order.
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Every device name, in insertion order.
    pub fn names(&self) -> Vec<&str> {
        self.devices.iter().map(|d| d.name.as_str()).collect()
    }

    /// Number of devices.
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// True when the database holds no device.
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// Renders every device in the `.dev` text format, in insertion order.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, device) in self.devices.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            device.write_text(&mut out);
        }
        out
    }
}

/// The recursive-descent parser over tokenized lines.
struct Parser<'a> {
    lines: &'a [Line],
    pos: usize,
    diags: &'a mut Diagnostics,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Line> {
        self.lines.get(self.pos)
    }

    fn bump(&mut self) -> Option<&'a Line> {
        let line = self.lines.get(self.pos);
        self.pos += 1;
        line
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(
            Diagnostic::error(message)
                .with_code(DEV_SYNTAX)
                .with_span(span),
        );
    }

    fn unknown(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(
            Diagnostic::error(message)
                .with_code(DEV_UNKNOWN)
                .with_span(span),
        );
    }

    fn word(&mut self, line: &'a Line, index: usize, what: &str) -> Option<&'a str> {
        match line.get(index) {
            Some(token) => Some(token.as_str()),
            None => {
                self.error(line.span, format!("expected {what}"));
                None
            }
        }
    }

    fn number(&mut self, token: &Token) -> Option<u32> {
        match token.as_str().parse::<u32>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.error(token.span, format!("expected a number, found `{token}`"));
                None
            }
        }
    }

    fn number_at(&mut self, line: &'a Line, index: usize, what: &str) -> Option<u32> {
        match line.get(index) {
            Some(token) => self.number(token),
            None => {
                self.error(line.span, format!("expected {what}"));
                None
            }
        }
    }

    /// Parses one `device` block; the cursor is on its header line.
    fn device(&mut self) -> Option<Device> {
        let header = self.bump()?;
        let name = self.word(header, 1, "a device name")?.to_owned();
        let mut device = Device::new(name, "");
        let mut seen_family = false;
        loop {
            let Some(line) = self.peek() else {
                self.error(header.span, "unterminated `device` block");
                break;
            };
            if line.keyword() == "end" {
                self.bump();
                break;
            }
            if line.keyword() == "device" {
                self.error(header.span, "unterminated `device` block");
                break;
            }
            self.bump();
            seen_family |= line.keyword() == "family";
            self.directive(line, &mut device);
        }
        if !seen_family {
            self.error(
                header.span,
                format!("device `{}` has no `family` line", device.name),
            );
        }
        for pin in &device.pins {
            if let Some(bank) = pin.bank.clone()
                && let Some(bank) = device.io_banks.iter_mut().find(|b| b.name == bank)
            {
                bank.pins.push(pin.name.clone());
            }
        }
        Some(device)
    }

    fn directive(&mut self, line: &'a Line, device: &mut Device) {
        match line.keyword() {
            "family" => {
                if let Some(word) = self.word(line, 1, "a family name") {
                    device.family = word.to_owned();
                }
            }
            "package" => {
                if let Some(word) = self.word(line, 1, "a package name") {
                    device.package = word.to_owned();
                }
            }
            "speed" => {
                if let Some(word) = self.word(line, 1, "a speed grade") {
                    device.speed_grade = word.to_owned();
                }
            }
            "lut_size" => {
                if let Some(value) = self.number_at(line, 1, "the number of LUT inputs") {
                    device.lut_size = value;
                }
            }
            "ff" => self.ff(line, device),
            "bel" => {
                if let Some(bel) = self.bel(line) {
                    device.bels.push(bel);
                }
            }
            "bram" => {
                if let Some(bram) = self.bram(line) {
                    device.block_rams.push(bram);
                }
            }
            "dsp" => {
                if let Some(dsp) = self.dsp(line) {
                    device.dsps.push(dsp);
                }
            }
            "io_standard" => {
                if let Some(std) = self.io_standard(line) {
                    device.io_standards.push(std);
                }
            }
            "bank" => {
                if let Some(bank) = self.bank(line) {
                    device.io_banks.push(bank);
                }
            }
            "pin" => {
                if let Some(pin) = self.pin(line) {
                    device.pins.push(pin);
                }
            }
            "global_buffers" => {
                if let Some(value) = self.number_at(line, 1, "the number of global buffers") {
                    device.clock_resources.global_buffers = value;
                }
            }
            "clock_region" => {
                if let Some(region) = self.clock_region(line) {
                    device.clock_resources.regions.push(region);
                }
            }
            "pll" => {
                if let Some(pll) = self.pll(line) {
                    device.clock_resources.plls.push(pll);
                }
            }
            "site" => {
                if let Some(site) = self.site(line) {
                    device.sites.push(site);
                }
            }
            "grid" => {
                let width = self.number_at(line, 1, "the grid width");
                let height = self.number_at(line, 2, "the grid height");
                if let (Some(width), Some(height)) = (width, height) {
                    device.tile_grid = Some(Grid { width, height });
                }
            }
            other => {
                let message = format!("unknown device directive `{other}`");
                self.unknown(line.tokens[0].span, message);
            }
        }
    }

    fn ff(&mut self, line: &'a Line, device: &mut Device) {
        let mut ff = FfFeatures::default();
        let mut index = 1;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "enable" => ff.has_enable = true,
                "sync_reset" => ff.has_sync_reset = true,
                "async_reset" => ff.has_async_reset = true,
                "shared_reset" => ff.reset_is_shared = true,
                "init" => {
                    let Some(value) = line.get(index) else {
                        self.error(line.span, "expected `0` or `1` after `init`");
                        break;
                    };
                    index += 1;
                    match value.as_str() {
                        "0" => ff.init_value = Some(false),
                        "1" => ff.init_value = Some(true),
                        "any" => ff.init_value = None,
                        other => {
                            let span = value.span;
                            self.error(
                                span,
                                format!("expected `0`, `1` or `any`, found `{other}`"),
                            );
                        }
                    }
                }
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown flip-flop feature `{other}`"));
                }
            }
        }
        device.ff = ff;
    }

    /// Reads `role=name` pairs starting at `index`, stopping at the first
    /// token without an `=`.
    fn pairs(&mut self, line: &'a Line, index: &mut usize) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        while let Some(token) = line.get(*index) {
            let Some((key, value)) = token.pair() else {
                break;
            };
            *index += 1;
            pairs.push((key.to_owned(), value.to_owned()));
        }
        if pairs.is_empty() {
            self.error(line.span, "expected at least one `role=name` pair");
        }
        pairs
    }

    fn bel(&mut self, line: &'a Line) -> Option<BelKind> {
        let name = self.word(line, 1, "a primitive name")?.to_owned();
        let Some(role_token) = line.get(2) else {
            self.error(line.span, "expected a BEL role");
            return None;
        };
        let Some(role) = BelRole::from_keyword(role_token.as_str()) else {
            let span = role_token.span;
            let text = role_token.as_str().to_owned();
            self.unknown(span, format!("unknown BEL role `{text}`"));
            return None;
        };
        let mut bel = BelKind::new(name, role);
        let mut index = 3;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "count" => {
                    let count = self.number_at(line, index, "a count")?;
                    index += 1;
                    bel.count = Some(count);
                }
                "port" => bel.ports.extend(self.pairs(line, &mut index)),
                keyword if keyword == "param" || keyword.starts_with("param_") => {
                    let keyword = keyword.to_owned();
                    let params: Vec<(String, AttrValue)> = self
                        .pairs(line, &mut index)
                        .into_iter()
                        .map(|(key, value)| (key, parse_value(&value)))
                        .collect();
                    match keyword.strip_prefix("param_") {
                        Some(condition) => bel.cond_params.push((condition.to_owned(), params)),
                        None => bel.params.extend(params),
                    }
                }
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown `bel` option `{other}`"));
                }
            }
        }
        Some(bel)
    }

    fn bram(&mut self, header: &'a Line) -> Option<BramShape> {
        let name = self.word(header, 1, "a block RAM name")?.to_owned();
        let mut bram = BramShape {
            name,
            width_modes: Vec::new(),
            mode_params: Vec::new(),
            ports: 1,
            has_byte_enable: false,
            dual_port: false,
            init_supported: false,
            port_map: Vec::new(),
        };
        loop {
            let Some(line) = self.peek() else {
                self.error(header.span, "unterminated `bram` block");
                break;
            };
            if line.keyword() == "end" {
                self.bump();
                break;
            }
            if line.keyword() == "bram" || line.keyword() == "device" {
                self.error(header.span, "unterminated `bram` block");
                break;
            }
            self.bump();
            match line.keyword() {
                "ports" => {
                    if let Some(value) = self.number_at(line, 1, "the number of ports") {
                        bram.ports = value;
                    }
                }
                "mode" => {
                    let width = self.number_at(line, 1, "a data width");
                    let depth = self.number_at(line, 2, "a depth");
                    let mut index = 3;
                    let mut params = Vec::new();
                    if line.get(index).is_some_and(|t| t.is("param")) {
                        index += 1;
                        params = self
                            .pairs(line, &mut index)
                            .into_iter()
                            .map(|(key, value)| (key, parse_value(&value)))
                            .collect();
                    }
                    if let (Some(width), Some(depth)) = (width, depth) {
                        bram.width_modes.push((width, depth));
                        bram.mode_params.push(params);
                    }
                }
                "flags" => {
                    let mut index = 1;
                    while let Some(token) = line.get(index) {
                        index += 1;
                        match token.as_str() {
                            "byte_enable" => bram.has_byte_enable = true,
                            "dual_port" => bram.dual_port = true,
                            "init" => bram.init_supported = true,
                            other => {
                                let span = token.span;
                                self.unknown(span, format!("unknown block RAM flag `{other}`"));
                            }
                        }
                    }
                }
                "port" => {
                    let Some(kind) = line.get(1) else {
                        self.error(line.span, "expected `read`, `write` or `rw`");
                        continue;
                    };
                    let Some(role) = BramPortRole::from_keyword(kind.as_str()) else {
                        let span = kind.span;
                        let text = kind.as_str().to_owned();
                        self.unknown(span, format!("unknown block RAM port role `{text}`"));
                        continue;
                    };
                    let mut index = 2;
                    let signals = self.pairs(line, &mut index);
                    bram.port_map.push(BramPort { role, signals });
                }
                other => {
                    let span = line.tokens[0].span;
                    self.unknown(span, format!("unknown `bram` directive `{other}`"));
                }
            }
        }
        Some(bram)
    }

    fn dsp(&mut self, line: &'a Line) -> Option<DspShape> {
        let name = self.word(line, 1, "a DSP name")?.to_owned();
        let mut dsp = DspShape {
            name,
            a_width: 0,
            b_width: 0,
            p_width: 0,
            has_preadder: false,
            has_accumulator: false,
            pipeline_stages: 0,
            ports: Vec::new(),
        };
        let mut index = 2;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "a" | "b" | "p" | "stages" => {
                    let key = token.as_str().to_owned();
                    let Some(value) = self.number_at(line, index, "a width") else {
                        break;
                    };
                    index += 1;
                    match key.as_str() {
                        "a" => dsp.a_width = value,
                        "b" => dsp.b_width = value,
                        "p" => dsp.p_width = value,
                        _ => dsp.pipeline_stages = value,
                    }
                }
                "preadder" => dsp.has_preadder = true,
                "accumulator" => dsp.has_accumulator = true,
                "port" => dsp.ports.extend(self.pairs(line, &mut index)),
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown `dsp` option `{other}`"));
                }
            }
        }
        Some(dsp)
    }

    fn io_standard(&mut self, line: &'a Line) -> Option<IoStandard> {
        let name = self.word(line, 1, "an IO standard name")?.to_owned();
        let mut std = IoStandard::new(name);
        let mut index = 2;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "vccio" => {
                    let value = self.word(line, index, "a supply voltage")?.to_owned();
                    index += 1;
                    std.vccio = Some(value);
                }
                "drive" => {
                    let Some(list) = line.get(index) else {
                        self.error(line.span, "expected a list of drive strengths");
                        break;
                    };
                    index += 1;
                    for item in list.as_str().split(',') {
                        match item.parse::<u32>() {
                            Ok(v) => std.drive_strengths.push(v),
                            Err(_) => {
                                let span = list.span;
                                self.error(span, format!("expected a number, found `{item}`"));
                            }
                        }
                    }
                }
                "slew" => {
                    let list = self.word(line, index, "a list of slew rates")?;
                    index += 1;
                    std.slew_rates
                        .extend(list.split(',').map(str::to_ascii_lowercase));
                }
                "differential" => std.differential = true,
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown `io_standard` option `{other}`"));
                }
            }
        }
        Some(std)
    }

    fn bank(&mut self, line: &'a Line) -> Option<IoBank> {
        let name = self.word(line, 1, "a bank name")?.to_owned();
        let mut bank = IoBank {
            name,
            pins: Vec::new(),
            vccio_options: Vec::new(),
        };
        let mut index = 2;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "vccio" => {
                    let list = self.word(line, index, "a list of supply voltages")?;
                    index += 1;
                    bank.vccio_options
                        .extend(list.split(',').map(str::to_owned));
                }
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown `bank` option `{other}`"));
                }
            }
        }
        Some(bank)
    }

    fn pin(&mut self, line: &'a Line) -> Option<Pin> {
        let name = self.word(line, 1, "a pin name")?.to_owned();
        let Some(kind_token) = line.get(2) else {
            self.error(line.span, "expected a pin kind");
            return None;
        };
        let Some(kind) = PinKind::from_keyword(kind_token.as_str()) else {
            let span = kind_token.span;
            let text = kind_token.as_str().to_owned();
            self.unknown(span, format!("unknown pin kind `{text}`"));
            return None;
        };
        let mut pin = Pin {
            name,
            site: None,
            bank: None,
            kind,
        };
        let mut index = 3;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "bank" => {
                    let value = self.word(line, index, "a bank name")?.to_owned();
                    index += 1;
                    pin.bank = Some(value);
                }
                "site" => {
                    let value = self.word(line, index, "a site name")?.to_owned();
                    index += 1;
                    pin.site = Some(value);
                }
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown `pin` option `{other}`"));
                }
            }
        }
        Some(pin)
    }

    fn clock_region(&mut self, line: &'a Line) -> Option<ClockRegion> {
        let name = self.word(line, 1, "a region name")?.to_owned();
        Some(ClockRegion {
            name,
            x0: self.number_at(line, 2, "the left edge")?,
            y0: self.number_at(line, 3, "the bottom edge")?,
            x1: self.number_at(line, 4, "the right edge")?,
            y1: self.number_at(line, 5, "the top edge")?,
        })
    }

    fn pll(&mut self, line: &'a Line) -> Option<PllShape> {
        let name = self.word(line, 1, "a PLL name")?.to_owned();
        let mut pll = PllShape {
            name,
            input_mhz: None,
            vco_mhz: None,
            outputs: 1,
        };
        let mut index = 2;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "input" | "vco" => {
                    let key = token.as_str().to_owned();
                    let lo = self.number_at(line, index, "a frequency in MHz")?;
                    let hi = self.number_at(line, index + 1, "a frequency in MHz")?;
                    index += 2;
                    if key == "input" {
                        pll.input_mhz = Some((lo, hi));
                    } else {
                        pll.vco_mhz = Some((lo, hi));
                    }
                }
                "outputs" => {
                    let value = self.number_at(line, index, "a number of outputs")?;
                    index += 1;
                    pll.outputs = value;
                }
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown `pll` option `{other}`"));
                }
            }
        }
        Some(pll)
    }

    fn site(&mut self, line: &'a Line) -> Option<Site> {
        let name = self.word(line, 1, "a site name")?.to_owned();
        let kind = self.word(line, 2, "a site kind")?.to_owned();
        Some(Site {
            name,
            kind,
            x: self.number_at(line, 3, "the site column")?,
            y: self.number_at(line, 4, "the site row")?,
        })
    }
}

/// Reads a device-file value: a sized literal, an integer, or a string.
///
/// Sized literals (`6'b011001`) become [`AttrValue::Const`] so they reach a
/// netlist as bit vectors rather than as text; a bare decimal number
/// becomes [`AttrValue::Int`]; anything else stays a string.
pub(crate) fn parse_value(text: &str) -> AttrValue {
    if text.contains('\'')
        && let Ok(c) = Const::parse_verilog(text)
    {
        return AttrValue::Const(c);
    }
    if let Ok(v) = text.parse::<i64>() {
        return AttrValue::Int(v);
    }
    AttrValue::String(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn parse(text: &str) -> (Option<Device>, String) {
        let mut map = SourceMap::new();
        let file = map.add("t.dev", text).unwrap();
        let mut diags = Diagnostics::new();
        let device = Device::parse(text, file, &mut diags);
        (device, diags.render(&map))
    }

    const SAMPLE: &str = "\
device demo-a
  family demo
  package tq144
  speed 1
  lut_size 4
  ff enable sync_reset async_reset shared_reset init 0
  bel DEMO_LUT4 lut count 100 port i=I0,I1,I2,I3 o=O param LUT_INIT=16'h0000
  bel DEMO_IO io port pad=PACKAGE_PIN din=D_IN dout=D_OUT oe=OE param IO_STANDARD=\"LVCMOS\" param_in PIN_TYPE=6'b000001 param_out PIN_TYPE=6'b011001
  bel DEMO_GB gb port i=I o=O
  bram DEMO_RAM
    ports 2
    mode 16 256 param READ_MODE=0 WRITE_MODE=0
    mode 8 512 param READ_MODE=1 WRITE_MODE=1
    flags dual_port init
    port read clk=RCLK en=RE addr=RADDR dout=RDATA
    port write clk=WCLK en=WE addr=WADDR din=WDATA
  end
  dsp DEMO_MUL a 18 b 18 p 36 preadder accumulator stages 3 port a=A b=B c=C p=P
  io_standard LVCMOS33 vccio 3.3 drive 4,8,12 slew slow,fast
  io_standard LVDS vccio 2.5 differential
  bank 0 vccio 3.3,2.5,1.8
  pin 21 clock bank 0 site X0/Y0/io0
  pin 99 io bank 0
  pin 1 gnd
  global_buffers 8
  clock_region global 0 0 13 17
  pll DEMO_PLL input 10 133 vco 533 1066 outputs 2
  site X1Y1 lc 1 1
  grid 14 18
end
";

    #[test]
    fn parses_every_directive() {
        let (device, diags) = parse(SAMPLE);
        assert_eq!(diags, "");
        let d = device.expect("device");
        assert_eq!(d.name, "demo-a");
        assert_eq!(d.family, "demo");
        assert_eq!(d.package, "tq144");
        assert_eq!(d.speed_grade, "1");
        assert_eq!(d.lut_size, 4);
        assert!(d.ff.has_enable && d.ff.has_sync_reset && d.ff.has_async_reset);
        assert!(d.ff.reset_is_shared);
        assert_eq!(d.ff.init_value, Some(false));

        let lut = d.bel(BelRole::Lut).unwrap();
        assert_eq!(lut.name, "DEMO_LUT4");
        assert_eq!(lut.count, Some(100));
        assert_eq!(lut.port("o"), Some("O"));
        assert_eq!(
            lut.params,
            vec![("LUT_INIT".to_owned(), AttrValue::Const(Const::zero(16)))]
        );
        let io = d.bel(BelRole::Io).unwrap();
        assert!(io.has_ports(&["pad", "din", "dout", "oe"]));
        assert!(!io.has_ports(&["nope"]));
        assert_eq!(
            io.params_when("out"),
            [(
                "PIN_TYPE".to_owned(),
                AttrValue::Const(Const::parse_verilog("6'b011001").unwrap())
            )]
        );
        assert!(io.params_when("inout").is_empty());
        assert_eq!(io.params[0].0, "IO_STANDARD");

        let bram = &d.block_rams[0];
        assert_eq!(bram.bits(), 4096);
        assert_eq!(bram.ports, 2);
        assert!(bram.dual_port && bram.init_supported && !bram.has_byte_enable);
        assert_eq!(bram.mode_for_width(5), Some((8, 512)));
        assert_eq!(bram.addr_width(), 9);
        assert_eq!(
            bram.params_for_mode((8, 512)),
            [
                ("READ_MODE".to_owned(), AttrValue::Int(1)),
                ("WRITE_MODE".to_owned(), AttrValue::Int(1)),
            ]
        );
        assert!(bram.params_for_mode((1, 1)).is_empty());
        assert_eq!(bram.mode_for_width(32), Some((16, 256)));
        assert_eq!(bram.read_port().unwrap().signal("dout"), Some("RDATA"));
        assert_eq!(bram.write_port().unwrap().signal("din"), Some("WDATA"));

        let dsp = &d.dsps[0];
        assert!(dsp.has_preadder && dsp.has_accumulator);
        assert_eq!(dsp.pipeline_stages, 3);
        assert!(dsp.fits_multiply(8, 8, 16));
        assert!(!dsp.fits_multiply(24, 8, 32));
        assert_eq!(dsp.port("p"), Some("P"));

        let std = d.io_standard("lvcmos33").unwrap();
        assert_eq!(std.drive_strengths, vec![4, 8, 12]);
        assert_eq!(std.slew_rates, vec!["slow", "fast"]);
        assert_eq!(std.vccio.as_deref(), Some("3.3"));
        assert!(d.io_standard("LVDS").unwrap().differential);

        assert_eq!(d.io_bank("0").unwrap().pins, vec!["21", "99"]);
        assert_eq!(d.pin("21").unwrap().kind, PinKind::Clock);
        assert_eq!(d.pin("21").unwrap().site.as_deref(), Some("X0/Y0/io0"));
        assert!(d.pin("1").unwrap().bank.is_none());
        assert!(!d.pin("1").unwrap().kind.is_io());
        assert!(d.pin("nope").is_none());

        assert_eq!(d.clock_resources.global_buffers, 8);
        assert_eq!(d.clock_resources.regions[0].x1, 13);
        assert_eq!(d.clock_resources.plls[0].input_mhz, Some((10, 133)));
        assert_eq!(d.clock_resources.plls[0].outputs, 2);
        assert_eq!(d.site("X1Y1").unwrap().kind, "lc");
        let grid = d.tile_grid.unwrap();
        assert!(grid.contains(13, 17) && !grid.contains(14, 0));
    }

    #[test]
    fn text_round_trips() {
        let (device, _) = parse(SAMPLE);
        let device = device.unwrap();
        let text = device.to_text();
        let (again, diags) = parse(&text);
        assert_eq!(diags, "");
        let again = again.unwrap();
        assert_eq!(device, again);
        assert_eq!(text, again.to_text());
    }

    #[test]
    fn database_holds_several_devices() {
        let text = format!("{SAMPLE}\ndevice demo-b\n  family demo\nend\n");
        let mut map = SourceMap::new();
        let file = map.add("t.dev", &text).unwrap();
        let mut diags = Diagnostics::new();
        let db = DeviceDb::parse(&text, file, &mut diags);
        assert_eq!(diags.render(&map), "");
        assert_eq!(db.names(), vec!["demo-a", "demo-b"]);
        assert_eq!(db.len(), 2);
        assert!(!db.is_empty());
        assert!(db.get("demo-c").is_none());
        assert_eq!(db.devices().len(), 2);
        // The database round-trips as a whole, devices separated by a
        // blank line.
        let printed = db.to_text();
        let mut map2 = SourceMap::new();
        let file2 = map2.add("t.dev", &printed).unwrap();
        let mut diags2 = Diagnostics::new();
        let db2 = DeviceDb::parse(&printed, file2, &mut diags2);
        assert_eq!(diags2.render(&map2), "");
        assert_eq!(db, db2);
        // Asking for one device out of a two-device file is an error.
        let mut diags3 = Diagnostics::new();
        assert!(Device::parse(&text, file, &mut diags3).is_some());
        assert_eq!(diags3.error_count(), 1);
    }

    #[test]
    fn reports_malformed_lines() {
        let text = "\
device bad
  lut_size four
  bel X nonsense
  pin A1 unobtainium
  frobnicate 3
  ff init 7
  grid 4
end
";
        let (device, diags) = parse(text);
        assert!(device.is_some());
        assert!(diags.contains("expected a number, found `four`"), "{diags}");
        assert!(diags.contains("unknown BEL role `nonsense`"), "{diags}");
        assert!(diags.contains("unknown pin kind `unobtainium`"), "{diags}");
        assert!(diags.contains("unknown device directive `frobnicate`"));
        assert!(diags.contains("expected `0`, `1` or `any`, found `7`"));
        assert!(diags.contains("expected the grid height"));
        assert!(diags.contains("has no `family` line"));
    }

    #[test]
    fn reports_structural_problems() {
        let text = "family demo\ndevice a\n  family demo\n";
        let (_, diags) = parse(text);
        assert!(
            diags.contains("expected `device`, found `family`"),
            "{diags}"
        );
        assert!(diags.contains("unterminated `device` block"), "{diags}");

        let dup = "device a\n  family d\nend\ndevice a\n  family d\nend\n";
        let (_, diags) = parse(dup);
        assert!(diags.contains("device `a` is defined twice"), "{diags}");

        let mut map = SourceMap::new();
        let file = map.add("t.dev", "").unwrap();
        let mut diags = Diagnostics::new();
        assert!(Device::parse("", file, &mut diags).is_none());
    }

    #[test]
    fn unterminated_inner_block_is_reported() {
        let text = "device a\n  family d\n  bram R\n    mode 8 16\nend\n";
        let (device, diags) = parse(text);
        assert!(diags.contains("unterminated `device` block"), "{diags}");
        assert_eq!(device.unwrap().block_rams[0].width_modes, vec![(8, 16)]);
    }

    #[test]
    fn values_keep_their_kind() {
        assert_eq!(parse_value("7"), AttrValue::Int(7));
        assert_eq!(parse_value("-7"), AttrValue::Int(-7));
        assert_eq!(
            parse_value("4'b0011"),
            AttrValue::Const(Const::from_u64(3, 4))
        );
        assert_eq!(parse_value("FAST"), AttrValue::String("FAST".to_owned()));
        assert_eq!(parse_value("3'nonsense").as_str(), Some("3'nonsense"));
        assert_eq!(write_value(&AttrValue::Int(3)), "3");
        assert_eq!(write_value(&AttrValue::String("a b".into())), "\"a b\"");
    }

    #[test]
    fn empty_shapes_have_sane_defaults() {
        let bram = BramShape {
            name: "R".into(),
            width_modes: Vec::new(),
            mode_params: Vec::new(),
            ports: 1,
            has_byte_enable: false,
            dual_port: false,
            init_supported: false,
            port_map: Vec::new(),
        };
        assert_eq!(bram.bits(), 0);
        assert_eq!(bram.mode_for_width(8), None);
        assert_eq!(bram.addr_width(), 0);
        assert!(bram.params_for_mode((8, 8)).is_empty());
        assert!(bram.read_port().is_none());
        let device = Device::new("x", "generic");
        assert!(device.bel(BelRole::Lut).is_none());
        assert!(device.bel_named("nope").is_none());
        assert!(device.io_bank("0").is_none());
        assert!(device.site("s").is_none());
        assert_eq!(BelRole::from_keyword("lut"), Some(BelRole::Lut));
        assert_eq!(BelRole::from_keyword("zzz"), None);
        assert_eq!(BelRole::Lut.to_string(), "lut");
        assert_eq!(PinKind::Io.to_string(), "io");
        assert!(BramPortRole::ReadWrite.reads() && BramPortRole::ReadWrite.writes());
        assert!(!BramPortRole::Read.writes());
        assert_eq!(BramPortRole::from_keyword("nope"), None);
        assert!(DeviceDb::new().is_empty());
    }
}
