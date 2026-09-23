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
//!   bel SB_DFFESR ff mode posedge,enable,sync_reset port clk=C d=D q=Q en=E rst=R
//!   bel SB_IO io port pad=PACKAGE_PIN din=D_IN_0 dout=D_OUT_0 oe=OUTPUT_ENABLE
//!   bram SB_RAM40_4K
//!     ports 2
//!     mode 16 256 init high 0-15
//!     mode 8 512 data 0,2,4,6,8,10,12,14 init high 0,2,4,6,8,10,12,14 1,3,5,7,9,11,13,15
//!     flags dual_port init
//!     init_params INIT_ count 16 digits 1 rows 16 slot 16
//!     port read clk=RCLK en=RE addr=RADDR dout=RDATA
//!     port write clk=WCLK en=WE addr=WADDR din=WDATA
//!   end
//!   dsp MULT18X18D a 18 b 18 p 36 accumulator stages 3 port a=A b=B p=P
//!   io_standard LVCMOS33 vccio 3.3 drive 4,8,12 slew slow,fast
//!   bank io vccio 3.3,2.5,1.8
//!   pins partial
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
//! `oe`, `i`, `o`, `ci`, `co`, `clk`, `d`, `q`, `en`, `rst`) onto the
//! primitive's port names, and a
//! [`BramShape`] and [`DspShape`] do the same for their ports. Primitive
//! mapping ([`super::primitives`]) is written against the roles, so it
//! works for any family whose file fills them in, and it declines to map
//! (with a note) when a role it needs is missing.
//!
//! A flip-flop carries one more piece of data: its `mode` clause, an
//! [`FfVariant`] saying exactly which flip-flop the primitive is (clock
//! edge, clock enable, kind and polarity of set or reset). A family
//! declares one `bel ... ff` line per combination it offers, whether
//! those are different primitives (iCE40) or one primitive with different
//! parameters (ECP5), and [`super::techcells`] maps an inferred flip-flop
//! onto the line that matches it exactly.
//!
//! # Block RAM layouts
//!
//! A block RAM's `mode` line may say more than its width and depth, in
//! [`BramModeLayout`]: which data pins carry the mode's bits (`data`,
//! for a family that spreads the bits of a narrow mode), where the word
//! address starts on the address pins and what the pins below it are
//! tied to (`addr`, `pad`, for a family that addresses in units of its
//! narrowest mode), and where each word sits in the contents (`init
//! low|high` followed by the row bits of each word of a row, see
//! [`BramInitLayout`]). The `init_params` line says which parameters
//! carry those rows ([`BramInitParams`]). A mode without an `init`
//! clause, or a block without `init_params`, is one whose contents
//! layout the database does not know, and block RAM mapping then says
//! that a memory's initial contents are lost rather than guessing.
//!
//! A device whose pin list is known to be incomplete says `pins
//! partial`, which turns a constraint naming an unlisted pin from an
//! error into a warning.
//!
//! # IO buffers, one per direction
//!
//! Most families have one IO buffer configured by a parameter, and one
//! `bel … io` line describes it. Xilinx has a *different primitive* per
//! direction — `IBUF`, `OBUF`, `IOBUF` — which do not even agree on the
//! name of the pad pin (`I`, `O`, `IO`). Such a family declares one line
//! each with a `for` clause:
//!
//! ```text
//! bel IBUF  io for in    port pad=I  din=O
//! bel OBUF  io for out   port pad=O  dout=I
//! bel IOBUF io for inout port pad=IO din=O dout=I oe=T
//! ```
//!
//! and [`Device::io_bel`] picks between them. A line with no `for`
//! serves every direction, which is what the one-buffer families say.
//!
//! # Carry elements, one bit or several
//!
//! Most families expose a one-bit carry: two operand bits and a carry in,
//! a carry out, and the sum XORed outside. That is four roles and no
//! more:
//!
//! ```text
//! bel SB_CARRY carry port ci=CI i0=I0 i1=I1 co=CO
//! ```
//!
//! Xilinx's `CARRY4` is the other shape: four bits per instance, a
//! *propagate* (`a ^ b`, from a LUT) rather than two operands, a
//! generate source it falls back on, and sum pins of its own. A family
//! with that shape writes a `width` and six roles:
//!
//! ```text
//! bel CARRY4 carry width 4 port ci=CI cyinit=CYINIT p=S di=DI s=O co=CO
//! ```
//!
//! `width` is how many bits one instance covers; `p` is the propagate
//! input and `di` the generate source, both that many bits wide; `s` is
//! the sum output and `co` the carry out of every bit, also that wide,
//! with `co`'s top bit feeding the next instance's `ci`. `ci` and
//! `cyinit` are the two one-bit ways in — the chain below and the
//! fabric — and a family needs at least one of them; where both exist
//! the chain goes on `ci` and the constant on `cyinit`. See
//! [`WideCarry`] for the model mapping assumes, and
//! [`BelKind::wide_carry`] for how the two shapes are told apart: a
//! `carry` line with neither form is "declared, unmapped", which is what
//! the ECP5's `CCU2C` still is.
//!
//! # Wide block RAM pins
//!
//! A block RAM signal whose pin is several bits wide and takes the same
//! value on every one of them writes the count after the name:
//! `we=WEA*2`. The 7-series byte write enables (`WEA[1:0]`,
//! `WEBWE[3:0]`) are what this is for — a memory written a whole word at
//! a time drives all of them alike, and leaving the upper bits
//! unconnected would write one byte and drop the rest.

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

/// What the set/reset input of one flip-flop primitive does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfReset {
    /// True when the input takes effect immediately, false when it takes
    /// effect on the clock edge.
    pub asynchronous: bool,
    /// True when the input forces a one (a *set*), false when it forces a
    /// zero (a *reset*).
    pub sets: bool,
    /// True when the input is active high.
    pub active_high: bool,
}

impl FfReset {
    /// The `.dev` keyword for the kind of input: `sync_reset`,
    /// `async_reset`, `sync_set` or `async_set`.
    pub fn keyword(self) -> &'static str {
        match (self.asynchronous, self.sets) {
            (false, false) => "sync_reset",
            (true, false) => "async_reset",
            (false, true) => "sync_set",
            (true, true) => "async_set",
        }
    }

    /// The reset named by a `.dev` keyword, active high.
    pub fn from_keyword(word: &str) -> Option<FfReset> {
        let (asynchronous, sets) = match word {
            "sync_reset" => (false, false),
            "async_reset" => (true, false),
            "sync_set" => (false, true),
            "async_set" => (true, true),
            _ => return None,
        };
        Some(FfReset {
            asynchronous,
            sets,
            active_high: true,
        })
    }
}

/// Exactly what one flip-flop primitive does.
///
/// A family offers one flip-flop per combination of clock edge, clock
/// enable and set/reset, either as separate primitives (iCE40's `SB_DFF`,
/// `SB_DFFE`, `SB_DFFESR`, ...) or as one primitive configured by
/// parameters (ECP5's `TRELLIS_FF` with its `CLKMUX`, `CEMUX`, `LSRMUX`,
/// `SRMODE` and `REGSET`). The database declares one `bel ... ff` line per
/// combination either way, and [`super::techcells`] picks the line whose
/// variant equals the inferred flip-flop exactly. Nothing is approximated:
/// a combination the device does not declare is reported rather than
/// mapped onto a near miss.
///
/// In the `.dev` text this is the `mode` clause, a comma-separated list of
/// `posedge` / `negedge`, `enable` or `enable_low`, one of `sync_reset` /
/// `async_reset` / `sync_set` / `async_set`, and `active_low`:
///
/// ```text
/// bel SB_DFFER ff mode negedge,enable,async_reset port clk=C d=D q=Q en=E rst=R
/// ```
///
/// The set or reset pin plays the `rst` port role whether it sets or
/// resets, since which of the two it does is what the variant states.
///
/// A polarity the family does not declare is not a refusal: [`super::techcells`]
/// inverts the net feeding the pin, once per net, and counts the
/// inverters it added. Only the *clock edge* is matched strictly, since
/// inverting a clock would create a second clock network.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfVariant {
    /// True when the flip-flop captures on the rising clock edge.
    pub clk_pos: bool,
    /// True when the primitive has a clock enable.
    pub has_enable: bool,
    /// True when that clock enable is active high. Meaningless, and held
    /// at `true`, when `has_enable` is false, so that two variants that
    /// behave the same compare equal.
    pub enable_active_high: bool,
    /// The set/reset input, or `None` when the primitive has none.
    pub reset: Option<FfReset>,
}

impl FfVariant {
    /// A plain positive-edge flip-flop with no enable and no reset.
    pub fn plain() -> FfVariant {
        FfVariant {
            clk_pos: true,
            has_enable: false,
            enable_active_high: true,
            reset: None,
        }
    }

    /// The `mode` clause describing this variant, in canonical order, so
    /// that parsing it back yields the same value.
    pub fn flags(self) -> String {
        let mut out = String::from(if self.clk_pos { "posedge" } else { "negedge" });
        if self.has_enable {
            out.push_str(if self.enable_active_high {
                ",enable"
            } else {
                ",enable_low"
            });
        }
        if let Some(reset) = self.reset {
            out.push(',');
            out.push_str(reset.keyword());
            if !reset.active_high {
                out.push_str(",active_low");
            }
        }
        out
    }

    /// Parses a `mode` clause, returning `None` on an unknown flag.
    ///
    /// ```
    /// use reticle::fpga::FfVariant;
    /// let v = FfVariant::parse("negedge,enable,async_reset").unwrap();
    /// assert!(!v.clk_pos && v.has_enable && v.enable_active_high);
    /// assert!(v.reset.unwrap().asynchronous);
    /// assert_eq!(v.flags(), "negedge,enable,async_reset");
    /// assert!(!FfVariant::parse("posedge,enable_low").unwrap().enable_active_high);
    /// assert!(FfVariant::parse("posedge,nonsense").is_none());
    /// ```
    pub fn parse(text: &str) -> Option<FfVariant> {
        let mut variant = FfVariant::plain();
        let mut active_low = false;
        for flag in text.split(',').filter(|f| !f.is_empty()) {
            match flag {
                "posedge" => variant.clk_pos = true,
                "negedge" => variant.clk_pos = false,
                "enable" => variant.has_enable = true,
                "enable_low" => {
                    variant.has_enable = true;
                    variant.enable_active_high = false;
                }
                "active_low" => active_low = true,
                other => variant.reset = Some(FfReset::from_keyword(other)?),
            }
        }
        if active_low {
            variant.reset.as_mut()?.active_high = false;
        }
        Some(variant)
    }

    /// The same variant with the set/reset input's polarity flipped, or
    /// `None` when it has no set/reset at all.
    ///
    /// This is what [`super::techcells`] asks for before giving up on a
    /// flip-flop: the other polarity plus an inverter on the net is the
    /// same circuit.
    pub fn with_flipped_reset(self) -> Option<FfVariant> {
        let mut reset = self.reset?;
        reset.active_high = !reset.active_high;
        Some(FfVariant {
            reset: Some(reset),
            ..self
        })
    }

    /// The same variant with the clock enable's polarity flipped, or
    /// `None` when it has no clock enable.
    pub fn with_flipped_enable(self) -> Option<FfVariant> {
        if !self.has_enable {
            return None;
        }
        Some(FfVariant {
            enable_active_high: !self.enable_active_high,
            ..self
        })
    }

    /// A phrase naming the variant, for diagnostics.
    pub fn describe(self) -> String {
        let mut out = String::from(if self.clk_pos {
            "a rising clock edge"
        } else {
            "a falling clock edge"
        });
        if self.has_enable {
            out.push_str(if self.enable_active_high {
                ", a clock enable"
            } else {
                ", an active-low clock enable"
            });
        }
        match self.reset {
            Some(reset) => {
                out.push_str(", an active-");
                out.push_str(if reset.active_high { "high " } else { "low " });
                out.push_str(if reset.asynchronous {
                    "asynchronous "
                } else {
                    "synchronous "
                });
                out.push_str(if reset.sets { "set" } else { "reset" });
            }
            None => out.push_str(" and no set or reset"),
        }
        out
    }
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
    /// A double-data-rate input register beside the IO buffer: `d` from
    /// the pad side, `clk`, and `q0` / `q1` for the bits captured on the
    /// rising and the falling edge (ECP5 `IDDRX1F`). A family whose IO
    /// buffer registers both edges itself (iCE40 `SB_IO`) declares none
    /// and says so on the buffer instead; see [`super::primitives`].
    DdrIn,
    /// A double-data-rate output register: `d0` / `d1` for the bits
    /// launched on the rising and the falling edge, `clk`, and `q` to the
    /// pad side (ECP5 `ODDRX1F`).
    DdrOut,
    /// A programmable delay between an IO buffer and the fabric: `i`,
    /// `o`, and the parameter carrying the delay declared under the
    /// condition `value` with its largest setting (ECP5 `DELAYG`).
    IoDelay,
    /// Anything else the family wants recorded.
    Other,
}

impl BelRole {
    /// Every role, in a fixed order.
    pub const ALL: [BelRole; 13] = [
        BelRole::Lut,
        BelRole::Ff,
        BelRole::Carry,
        BelRole::Io,
        BelRole::GlobalBuffer,
        BelRole::LutRam,
        BelRole::Bram,
        BelRole::Dsp,
        BelRole::Pll,
        BelRole::DdrIn,
        BelRole::DdrOut,
        BelRole::IoDelay,
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
            BelRole::DdrIn => "ddr_in",
            BelRole::DdrOut => "ddr_out",
            BelRole::IoDelay => "iodelay",
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
    /// For a primitive with role [`BelRole::Carry`], how many bits of
    /// carry one instance covers, from its `width` clause; `None` for the
    /// one-bit form every other family uses. See [`BelKind::wide_carry`].
    pub carry_width: Option<u32>,
    /// For a primitive with role [`BelRole::Ff`], exactly which flip-flop
    /// it is; `None` for every other role, and for a flip-flop whose file
    /// does not say (which makes flip-flop mapping decline for it).
    pub ff: Option<FfVariant>,
    /// For a primitive with role [`BelRole::Io`], the port directions it
    /// serves: `in`, `out`, `inout`. Empty means "every direction",
    /// which is what a family with one configurable buffer (iCE40's
    /// `SB_IO`, ECP5's `TRELLIS_IO`) says. A family with a *different
    /// primitive per direction* — Xilinx's `IBUF`, `OBUF` and `IOBUF`,
    /// which do not even agree on what the pad pin is called — declares
    /// one `bel` line each with a `for` clause, and
    /// [`Device::io_bel`] picks between them.
    pub io_dirs: Vec<String>,
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
            carry_width: None,
            ff: None,
            io_dirs: Vec::new(),
        }
    }

    /// The port name playing `role`, if the database records one.
    pub fn port(&self, role: &str) -> Option<&str> {
        self.ports
            .iter()
            .find(|(r, _)| r == role)
            .map(|(_, n)| n.as_str())
    }

    /// The port names playing `role`, splitting the comma-separated form a
    /// role with several ports uses: a LUT's `i=I0,I1,I2,I3` is four
    /// ports, in that order, input 0 first.
    ///
    /// ```
    /// let device = reticle::fpga::target("ice40-hx1k-tq144").unwrap();
    /// let lut = device.bel(reticle::fpga::BelRole::Lut).unwrap();
    /// assert_eq!(lut.port_names("i"), ["I0", "I1", "I2", "I3"]);
    /// assert_eq!(lut.port_names("nope"), Vec::<&str>::new());
    /// ```
    pub fn port_names(&self, role: &str) -> Vec<&str> {
        self.port(role)
            .map(|name| name.split(',').collect())
            .unwrap_or_default()
    }

    /// Every port name the primitive has, in file order, with the
    /// comma-separated entries split out.
    pub fn all_port_names(&self) -> Vec<&str> {
        self.ports
            .iter()
            .flat_map(|(_, names)| names.split(','))
            .collect()
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

    /// The wide-carry shape this primitive describes, or `None` when it
    /// is not one.
    ///
    /// A carry element is *wide* when its line carries a `width` and the
    /// roles a [`WideCarry`] needs; anything else — including a `carry`
    /// line with no ports at all — answers `None`, which leaves the
    /// one-bit `(ci, i0, i1) -> co` path and the "declined with a note"
    /// path exactly as they were.
    ///
    /// ```
    /// use reticle::fpga::BelRole;
    /// let device = reticle::fpga::target("xc7a35t-cpg236").unwrap();
    /// let carry = device.bel(BelRole::Carry).unwrap();
    /// let wide = carry.wide_carry().unwrap();
    /// assert_eq!((wide.width, wide.propagate, wide.sum), (4, "S", "O"));
    /// // The iCE40's carry is the one-bit kind, so it is not one.
    /// let ice40 = reticle::fpga::target("ice40-hx1k-tq144").unwrap();
    /// assert!(ice40.bel(BelRole::Carry).unwrap().wide_carry().is_none());
    /// ```
    pub fn wide_carry(&self) -> Option<WideCarry<'_>> {
        if self.role != BelRole::Carry {
            return None;
        }
        let width = self.carry_width?;
        if width == 0 {
            return None;
        }
        let carry_in = self.port("ci");
        let init = self.port("cyinit");
        if carry_in.is_none() && init.is_none() {
            return None;
        }
        Some(WideCarry {
            width,
            propagate: self.port("p")?,
            data: self.port("di")?,
            sum: self.port("s")?,
            carry_out: self.port("co")?,
            carry_in,
            init,
        })
    }
}

/// A carry element several bits wide that takes a *propagate* and
/// computes its own sums: the Xilinx 7-series `CARRY4` and its like.
///
/// The one-bit element [`super::primitives`] maps onto by default takes
/// two operand bits and a carry in and answers a carry out, with the sum
/// XORed outside it. This is the other shape the fabrics use, and it
/// differs in three ways at once: it covers [`width`](Self::width) bits
/// per instance, it takes the propagate `a ^ b` (a LUT computes it)
/// rather than the two operands, and its `sum` pins are the adder's
/// result, so no XOR follows it.
///
/// Its model, which `CARRY4` states and which mapping relies on:
///
/// ```text
/// sum[i]     = propagate[i] ^ carry[i]
/// carry[0]   = the instance's carry in
/// carry[i+1] = carry_out[i] = propagate[i] ? carry[i] : data[i]
/// ```
///
/// so `data` is the bit the element falls back on where the propagate is
/// zero, which for `a + b` is `a` itself (`a == b` there, so either
/// operand would do).
///
/// The two carry inputs are both optional and at least one must be
/// present: `carry_in` is the pin fed from the instance below, `init`
/// the pin fed from the fabric or a constant. A family with only one of
/// them names only that one; a family with both — `CI` and `CYINIT` on
/// the 7-series, where the silicon ORs them and only one may be driven —
/// gets the chain on `carry_in` and the constant on `init`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WideCarry<'a> {
    /// How many bits of the adder one instance covers.
    pub width: u32,
    /// The propagate input, `width` bits wide (`S`).
    pub propagate: &'a str,
    /// The generate source, `width` bits wide (`DI`).
    pub data: &'a str,
    /// The sum output, `width` bits wide (`O`).
    pub sum: &'a str,
    /// The carry output of each bit, `width` bits wide (`CO`); its top
    /// bit is the next instance's carry in.
    pub carry_out: &'a str,
    /// The one-bit carry input from the instance below (`CI`).
    pub carry_in: Option<&'a str>,
    /// The one-bit carry input from the fabric or a constant (`CYINIT`).
    pub init: Option<&'a str>,
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
    /// Signals whose pin is several bits wide and takes the same value
    /// on every one of them, as `(role, bits)`, for the roles where that
    /// is so; a role with no entry is one bit. The write enable of a
    /// family with **byte** write enables is the case this exists for:
    /// a 7-series `RAMB18E1` has `WEA[1:0]` and `WEBWE[3:0]`, and a
    /// memory written a whole word at a time drives every one of them
    /// alike. Written `we=WEA*2` in the text format.
    pub widths: Vec<(String, u32)>,
}

impl BramPort {
    /// How many bits wide the pin playing `role` is; 1 unless the
    /// database says otherwise.
    pub fn width(&self, role: &str) -> u32 {
        self.widths
            .iter()
            .find(|(r, _)| r == role)
            .map_or(1, |(_, bits)| *bits)
    }

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
    /// How each mode places a word on the pins and in the contents,
    /// parallel to `width_modes` like `mode_params`. Use
    /// [`BramShape::layout_for_mode`] rather than indexing it.
    pub mode_layouts: Vec<BramModeLayout>,
    /// The parameters that carry the initial contents, when the database
    /// describes them (`init_params` in the text format).
    pub init_params: Option<BramInitParams>,
    /// The physical ports and their pin names, in file order.
    pub port_map: Vec<BramPort>,
}

/// How one width mode of a block RAM places a word: on the data and
/// address pins, and in the initial contents.
///
/// The default is the plain case — data bit `j` on data pin `j`, the word
/// address from address bit 0 up, and no statement about the contents.
/// A family that differs says so on the `mode` line of its `.dev` file
/// (`data`, `addr`, `pad` and `init`), so nothing about a particular
/// primitive is written in Rust.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BramModeLayout {
    /// The data-port bit that carries data bit `j` of the mode, for both
    /// the write data and the read data; empty means bit `j` on pin `j`.
    /// The iCE40's narrow modes spread their bits (`1,5,9,13` for 1024x4).
    pub data_bits: Vec<u32>,
    /// The lowest address-port bit carrying the word address. A block
    /// that addresses its narrowest unit with every pin and ignores the
    /// low pins in wide modes (the ECP5's DP16KD: `ADA[13:4]` in 18-bit
    /// mode) sets it; the bits below are tied to `addr_pad`.
    pub addr_low: u32,
    /// What the address bits below `addr_low` are tied to, `addr_low`
    /// bits wide; `None` ties them to zero.
    pub addr_pad: Option<Const>,
    /// Where the bits of each word sit in the initial contents, or `None`
    /// when the database does not say, in which case a memory with
    /// initial contents cannot be put in this mode with them.
    pub init: Option<BramInitLayout>,
}

impl BramModeLayout {
    /// The data-port bit that carries data bit `bit` of the mode.
    pub fn data_bit(&self, bit: u32) -> u32 {
        usize::try_from(bit)
            .ok()
            .and_then(|i| self.data_bits.get(i))
            .copied()
            .unwrap_or(bit)
    }
}

/// Which address bits choose among the words that share one row of a
/// block RAM's contents in a narrow mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WordSelect {
    /// The low address bits: consecutive addresses share a row (ECP5).
    Low,
    /// The address bits above the row address: a row holds words a whole
    /// block's worth of rows apart (iCE40, where address bits 7..0 are
    /// always the row).
    High,
}

impl WordSelect {
    /// The keyword used in the text format.
    pub fn keyword(self) -> &'static str {
        match self {
            WordSelect::Low => "low",
            WordSelect::High => "high",
        }
    }
}

/// Where the words of one width mode sit in the rows of a block RAM's
/// initial contents.
///
/// The contents are rows of [`BramInitParams::slot`] bits. In a mode,
/// `words.len()` words share a row, and word `k` of a row keeps its data
/// bit `j` in row bit `words[k][j]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BramInitLayout {
    /// Which address bits pick the word within a row.
    pub select: WordSelect,
    /// For each word of a row, the row bit of each of its data bits.
    pub words: Vec<Vec<u32>>,
}

impl BramInitLayout {
    /// The row and the row bit holding bit `bit` of the word at `address`
    /// (counted from the block's first word), for a block of `rows` rows;
    /// `None` when the layout does not cover it.
    pub fn locate(&self, address: u64, bit: u32, rows: u64) -> Option<(u64, u32)> {
        let per_row = u64::try_from(self.words.len()).ok()?;
        if per_row == 0 || rows == 0 {
            return None;
        }
        let (row, word) = match self.select {
            WordSelect::Low => (address / per_row, address % per_row),
            WordSelect::High => (address % rows, address / rows),
        };
        let word = self.words.get(usize::try_from(word).ok()?)?;
        let at = *word.get(usize::try_from(bit).ok()?)?;
        (row < rows).then_some((row, at))
    }
}

/// The parameters that carry a block RAM's initial contents.
///
/// There are `count` of them, named `prefix` followed by the index in
/// upper-case hexadecimal of `digits` digits (`INIT_0`..`INIT_F`,
/// `INITVAL_00`..`INITVAL_3F`), each holding `rows` rows of `slot` bits
/// with its first row in its lowest bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BramInitParams {
    /// The parameter name before the index.
    pub prefix: String,
    /// How many parameters there are.
    pub count: u32,
    /// How many hexadecimal digits the index is written with.
    pub digits: u32,
    /// Rows of the contents per parameter.
    pub rows: u32,
    /// Bits per row in a parameter; a row narrower than its slot fills
    /// the low bits and the rest are zero (the ECP5 keeps 18-bit rows in
    /// 20-bit slots).
    pub slot: u32,
}

impl BramInitParams {
    /// The name of parameter `index`.
    pub fn name(&self, index: u32) -> String {
        let digits = usize::try_from(self.digits).unwrap_or(1);
        format!("{}{index:0digits$X}", self.prefix)
    }

    /// The number of rows the parameters hold between them.
    pub fn total_rows(&self) -> u64 {
        u64::from(self.count) * u64::from(self.rows)
    }

    /// The width of one parameter.
    pub fn param_width(&self) -> u32 {
        self.rows.saturating_mul(self.slot)
    }
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

    /// How `mode` places a word; the plain default when the database
    /// says nothing.
    pub fn layout_for_mode(&self, mode: (u32, u32)) -> BramModeLayout {
        self.width_modes
            .iter()
            .position(|m| *m == mode)
            .and_then(|index| self.mode_layouts.get(index))
            .cloned()
            .unwrap_or_default()
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
///
/// Everything [`super::pll::solve`] needs to configure one is here as
/// data: the frequency ranges, the three divider fields with the
/// parameter each is written to and how the written value relates to the
/// division it performs, where the feedback is taken from, and any
/// parameter that follows from the solution (a phase that must track the
/// output divider, a loop-filter setting chosen by the phase detector's
/// frequency). A shape without dividers or without an `out` port is
/// recorded but cannot be instantiated, and a design asking for one is
/// told so.
///
/// In the `.dev` text all of it is one `pll` line:
///
/// ```text
/// pll SB_PLL40_CORE input 10 133 pfd 10 133 vco 533 1066 outputs 1 count 1
///     feedback vco divide ref DIVR 0 15 offset 1 divide feedback DIVF 0 63 offset 1
///     divide out DIVQ 1 6 pow2 band FILTER_RANGE 17=1 26=2 44=3 66=4 101=5 134=6
///     port ref=REFERENCECLK out=PLLOUTCORE lock=LOCK tie RESETB=1 BYPASS=0
///     param FEEDBACK_PATH="SIMPLE"
/// ```
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
    /// How many of these blocks the device has, when known. Several
    /// `pll` lines may describe one physical block used different ways
    /// (`SB_PLL40_CORE` and `SB_PLL40_PAD`), in which case they carry the
    /// same count and the device has that many in all, not that many of
    /// each.
    pub count: Option<u32>,
    /// The phase-detector frequency range in MHz — the reference after
    /// the reference divider — when known.
    pub pfd_mhz: Option<(u32, u32)>,
    /// Where the feedback divider takes its clock from.
    pub feedback: PllFeedback,
    /// The divider fields, in file order.
    pub dividers: Vec<PllDivider>,
    /// Parameters computed from a divider's written value: `(parameter,
    /// divider role, offset)`, so the ECP5's `CLKOP_CPHASE`, which must
    /// be one less than `CLKOP_DIV` for no phase shift, is
    /// `("CLKOP_CPHASE", Output, -1)`.
    pub derived: Vec<(String, PllDividerRole, i64)>,
    /// Parameters chosen by the phase-detector frequency: for each, the
    /// bands as `(upper bound in MHz, exclusive; value)`, lowest first.
    /// iCE40's `FILTER_RANGE` is one.
    pub bands: Vec<(String, Vec<(u32, i64)>)>,
    /// Abstract role to port name: `ref` (the reference clock in), `out`
    /// (the generated clock), `fb` (the feedback input, wired to `out`
    /// when the feedback is taken from the output), `fbout` (the
    /// feedback *output* of a block whose loop is closed outside it, as
    /// the 7-series `PLLE2_BASE` closes `CLKFBOUT` onto `CLKFBIN`; when
    /// both are named, mapping runs a net between them and `fb` is not
    /// taken from `out`), `lock`.
    pub ports: Vec<(String, String)>,
    /// Input ports tied to a constant so the block runs: an active-low
    /// reset held high, a bypass held low.
    pub ties: Vec<(String, bool)>,
    /// Parameters every instance carries, in file order.
    pub params: Vec<(String, AttrValue)>,
}

impl PllShape {
    /// A PLL with a name and nothing else, which cannot be configured.
    pub fn new(name: impl Into<String>) -> Self {
        PllShape {
            name: name.into(),
            input_mhz: None,
            vco_mhz: None,
            outputs: 1,
            count: None,
            pfd_mhz: None,
            feedback: PllFeedback::Vco,
            dividers: Vec::new(),
            derived: Vec::new(),
            bands: Vec::new(),
            ports: Vec::new(),
            ties: Vec::new(),
            params: Vec::new(),
        }
    }

    /// The port name playing `role`, if the database records one.
    pub fn port(&self, role: &str) -> Option<&str> {
        self.ports
            .iter()
            .find(|(r, _)| r == role)
            .map(|(_, n)| n.as_str())
    }

    /// The divider playing `role`, if the database records one.
    pub fn divider(&self, role: PllDividerRole) -> Option<&PllDivider> {
        self.dividers.iter().find(|d| d.role == role)
    }

    /// True when the database says enough to configure and connect the
    /// block: all three dividers, a VCO range, and `ref` and `out` ports.
    pub fn is_configurable(&self) -> bool {
        PllDividerRole::ALL
            .iter()
            .all(|role| self.divider(*role).is_some())
            && self.vco_mhz.is_some()
            && self.port("ref").is_some()
            && self.port("out").is_some()
    }
}

/// Where a PLL's feedback divider takes its clock from, which decides
/// how the three dividers combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PllFeedback {
    /// From the VCO: `vco = in / ref * feedback`, `out = vco / out`
    /// (iCE40 `SIMPLE` feedback).
    Vco,
    /// From the output: `out = in / ref * feedback`, `vco = out * out`
    /// (ECP5 `CLKOP` feedback).
    Output,
}

impl PllFeedback {
    /// The keyword used in the text format: `vco` or `out`.
    pub fn keyword(self) -> &'static str {
        match self {
            PllFeedback::Vco => "vco",
            PllFeedback::Output => "out",
        }
    }
}

/// Which of the three dividers of a PLL one field is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PllDividerRole {
    /// Divides the reference before the phase detector.
    Reference,
    /// Divides the clock fed back to the phase detector, and so
    /// multiplies.
    Feedback,
    /// Divides the VCO down to the output.
    Output,
}

impl PllDividerRole {
    /// Every role, in a fixed order.
    pub const ALL: [PllDividerRole; 3] = [
        PllDividerRole::Reference,
        PllDividerRole::Feedback,
        PllDividerRole::Output,
    ];

    /// The keyword used in the text format: `ref`, `feedback` or `out`.
    pub fn keyword(self) -> &'static str {
        match self {
            PllDividerRole::Reference => "ref",
            PllDividerRole::Feedback => "feedback",
            PllDividerRole::Output => "out",
        }
    }

    /// The role with the given keyword.
    pub fn from_keyword(word: &str) -> Option<PllDividerRole> {
        PllDividerRole::ALL
            .into_iter()
            .find(|r| r.keyword() == word)
    }
}

/// One divider field of a PLL: the parameter it is written to, the range
/// of values that parameter takes, and what division a value means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PllDivider {
    /// Which of the three dividers this is.
    pub role: PllDividerRole,
    /// The parameter carrying it (`DIVR`, `CLKI_DIV`).
    pub param: String,
    /// The smallest value the parameter may hold.
    pub min: u32,
    /// The largest value the parameter may hold.
    pub max: u32,
    /// Added to the parameter's value to give the division, for a field
    /// that stores `divisor - 1` (iCE40's `DIVR` and `DIVF`).
    pub offset: u32,
    /// True when the division is two to the power of the value (iCE40's
    /// `DIVQ`); `offset` is then ignored.
    pub power_of_two: bool,
}

impl PllDivider {
    /// The division the parameter value `value` performs.
    pub fn divisor(&self, value: u32) -> u64 {
        if self.power_of_two {
            1u64 << value.min(62)
        } else {
            u64::from(value) + u64::from(self.offset)
        }
    }
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
    /// The JTAG IDCODE of the die, when the device file states one.
    ///
    /// A bitstream writes it into the configuration engine's `IDCODE`
    /// register and the part refuses a stream that names a different
    /// one, so it is also what lets a flow notice that the chip database
    /// it loaded is for another die before it emits anything. `None`
    /// means the file does not say, which is not the same as zero.
    pub idcode: Option<u32>,
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
    /// The package has pins that `pins` does not list (`pins partial` in
    /// the text format). A constraint naming an unlisted pin is then a
    /// warning that Reticle cannot check it, rather than an error, and
    /// the place-and-route tool, which knows the package, decides.
    pub pins_partial: bool,
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
            idcode: None,
            lut_size: 4,
            ff: FfFeatures::default(),
            bels: Vec::new(),
            block_rams: Vec::new(),
            dsps: Vec::new(),
            io_standards: Vec::new(),
            io_banks: Vec::new(),
            pins: Vec::new(),
            pins_partial: false,
            clock_resources: ClockResources::default(),
            sites: Vec::new(),
            tile_grid: None,
        }
    }

    /// The first primitive with the given role.
    pub fn bel(&self, role: BelRole) -> Option<&BelKind> {
        self.bels.iter().find(|b| b.role == role)
    }

    /// The IO buffer to use for a port of direction `dir` (`in`, `out`
    /// or `inout`).
    ///
    /// A family with one configurable buffer declares it without a `for`
    /// clause and gets it back whatever the direction; a family with one
    /// primitive per direction (Xilinx's `IBUF`, `OBUF`, `IOBUF`)
    /// declares a line each and gets the matching one. `None` means the
    /// device describes no buffer that can serve that direction, which
    /// [`super::primitives`] reports rather than working around.
    ///
    /// ```
    /// use reticle::fpga::target;
    /// // One buffer for everything: the same primitive every time.
    /// let ice40 = target("ice40-hx1k-tq144").unwrap();
    /// assert_eq!(ice40.io_bel("in").unwrap().name, "SB_IO");
    /// assert_eq!(ice40.io_bel("inout").unwrap().name, "SB_IO");
    /// ```
    pub fn io_bel(&self, dir: &str) -> Option<&BelKind> {
        let io = || self.bels.iter().filter(|b| b.role == BelRole::Io);
        io().find(|b| b.io_dirs.iter().any(|d| d == dir))
            .or_else(|| io().find(|b| b.io_dirs.is_empty()))
    }

    /// The primitive with the given name.
    pub fn bel_named(&self, name: &str) -> Option<&BelKind> {
        self.bels.iter().find(|b| b.name == name)
    }

    /// The flip-flop primitive that behaves exactly like `variant`.
    ///
    /// A family whose flip-flops are one primitive configured by
    /// parameters declares one `bel` line per variant under the same name;
    /// the first matching line wins, so the file's order decides.
    ///
    /// ```
    /// use reticle::fpga::{FfReset, FfVariant, target};
    /// let device = target("ice40-hx1k-tq144").unwrap();
    /// let variant = FfVariant {
    ///     clk_pos: true,
    ///     has_enable: true,
    ///     enable_active_high: true,
    ///     reset: Some(FfReset { asynchronous: false, sets: false, active_high: true }),
    /// };
    /// assert_eq!(device.ff_variant(variant).unwrap().name, "SB_DFFESR");
    /// ```
    pub fn ff_variant(&self, variant: FfVariant) -> Option<&BelKind> {
        self.bels
            .iter()
            .find(|b| b.role == BelRole::Ff && b.ff == Some(variant))
    }

    /// Every flip-flop variant the device declares, in file order.
    pub fn ff_variants(&self) -> impl Iterator<Item = (&BelKind, FfVariant)> {
        self.bels
            .iter()
            .filter(|b| b.role == BelRole::Ff)
            .filter_map(|b| b.ff.map(|v| (b, v)))
    }

    /// The port names of the primitive `name`, or `None` when the device
    /// declares no primitive by that name.
    ///
    /// Every kind of primitive is covered: the [`BelKind`]s (several of
    /// which may share a name, as the flip-flop variants of a family with
    /// parameter-configured flops do, in which case the union of their
    /// ports is returned), the block RAMs, the DSP blocks and the PLLs.
    /// The result is what a netlist may connect, which is what
    /// [`super::flow::check_nextpnr_json`] checks each cell against.
    /// A primitive the database records without a port map yields an
    /// empty list, meaning "declared, ports unknown".
    pub fn primitive_ports(&self, name: &str) -> Option<Vec<String>> {
        let mut found = false;
        let mut ports: Vec<String> = Vec::new();
        let mut push = |port: &str| {
            if !ports.iter().any(|p| p == port) {
                ports.push(port.to_owned());
            }
        };
        for bel in self.bels.iter().filter(|b| b.name == name) {
            found = true;
            for port in bel.all_port_names() {
                push(port);
            }
        }
        for bram in self.block_rams.iter().filter(|b| b.name == name) {
            found = true;
            for port in &bram.port_map {
                for (_, signal) in &port.signals {
                    push(signal);
                }
            }
        }
        for dsp in self.dsps.iter().filter(|d| d.name == name) {
            found = true;
            for (_, port) in &dsp.ports {
                push(port);
            }
        }
        for pll in self.clock_resources.plls.iter().filter(|p| p.name == name) {
            found = true;
            for (_, port) in &pll.ports {
                push(port);
            }
            for (port, _) in &pll.ties {
                push(port);
            }
        }
        found.then_some(ports)
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
        if let Some(idcode) = self.idcode {
            out.push_str(&format!("  idcode {idcode:#010x}\n"));
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
        if self.pins_partial {
            out.push_str("  pins partial\n");
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
            if let Some(count) = pll.count {
                line.push_str(&format!(" count {count}"));
            }
            if let Some((lo, hi)) = pll.pfd_mhz {
                line.push_str(&format!(" pfd {lo} {hi}"));
            }
            if !pll.dividers.is_empty() {
                line.push_str(&format!(" feedback {}", pll.feedback.keyword()));
            }
            for divider in &pll.dividers {
                line.push_str(&format!(
                    " divide {} {} {} {}",
                    divider.role.keyword(),
                    quote(&divider.param),
                    divider.min,
                    divider.max
                ));
                if divider.power_of_two {
                    line.push_str(" pow2");
                } else if divider.offset != 0 {
                    line.push_str(&format!(" offset {}", divider.offset));
                }
            }
            for (param, role, offset) in &pll.derived {
                line.push_str(&format!(
                    " derive {} {} {offset}",
                    quote(param),
                    role.keyword()
                ));
            }
            for (param, bands) in &pll.bands {
                line.push_str(&format!(" band {}", quote(param)));
                for (upper, value) in bands {
                    line.push_str(&format!(" {upper}={value}"));
                }
            }
            if !pll.ports.is_empty() {
                line.push_str(" port");
                for (role, name) in &pll.ports {
                    line.push_str(&format!(" {role}={name}"));
                }
            }
            if !pll.ties.is_empty() {
                line.push_str(" tie");
                for (port, level) in &pll.ties {
                    line.push_str(&format!(" {port}={}", u8::from(*level)));
                }
            }
            write_params(&mut line, "param", &pll.params);
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
    if let Some(width) = bel.carry_width {
        line.push_str(&format!(" width {width}"));
    }
    if let Some(ff) = bel.ff {
        line.push_str(&format!(" mode {}", ff.flags()));
    }
    if !bel.io_dirs.is_empty() {
        line.push_str(&format!(" for {}", join(&bel.io_dirs)));
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
        if let Some(layout) = bram.mode_layouts.get(index) {
            write_mode_layout(&mut line, layout);
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
    if let Some(init) = &bram.init_params {
        out.push_str(&format!(
            "    init_params {} count {} digits {} rows {} slot {}\n",
            quote(&init.prefix),
            init.count,
            init.digits,
            init.rows,
            init.slot
        ));
    }
    for port in &bram.port_map {
        let mut line = format!("    port {}", port.role.keyword());
        for (role, name) in &port.signals {
            line.push_str(&format!(" {}={}", quote(role), quote(name)));
            let bits = port.width(role);
            if bits > 1 {
                line.push_str(&format!("*{bits}"));
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("  end\n");
}

/// Appends the `data`, `addr`, `pad` and `init` clauses of a `mode` line.
fn write_mode_layout(line: &mut String, layout: &BramModeLayout) {
    if !layout.data_bits.is_empty() {
        line.push_str(&format!(" data {}", write_bit_list(&layout.data_bits)));
    }
    if layout.addr_low > 0 {
        line.push_str(&format!(" addr {}", layout.addr_low));
    }
    if let Some(pad) = &layout.addr_pad {
        line.push_str(&format!(" pad {}", pad.to_verilog_literal()));
    }
    if let Some(init) = &layout.init {
        line.push_str(&format!(" init {}", init.select.keyword()));
        for word in &init.words {
            line.push(' ');
            line.push_str(&write_bit_list(word));
        }
    }
}

/// Writes bit positions as a comma-separated list, with a run of three
/// or more consecutive positions as `first-last`.
fn write_bit_list(bits: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bits.len() {
        let mut j = i;
        while j + 1 < bits.len() && bits[j].checked_add(1) == Some(bits[j + 1]) {
            j += 1;
        }
        if j >= i + 2 {
            parts.push(format!("{}-{}", bits[i], bits[j]));
            i = j + 1;
        } else {
            parts.push(bits[i].to_string());
            i += 1;
        }
    }
    parts.join(",")
}

/// Reads a list written by [`write_bit_list`]; `None` when it is not one.
fn parse_bit_list(text: &str) -> Option<Vec<u32>> {
    let mut bits = Vec::new();
    for part in text.split(',') {
        match part.split_once('-') {
            Some((first, last)) => {
                let first: u32 = first.parse().ok()?;
                let last: u32 = last.parse().ok()?;
                if last < first {
                    return None;
                }
                bits.extend(first..=last);
            }
            None => bits.push(part.parse().ok()?),
        }
    }
    Some(bits)
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
            if line.keyword() == "arch" {
                // A routing architecture ([`super::arch`]) may share a
                // file with the devices it serves. Its grammar is flat,
                // so skipping it is skipping to its `end`.
                parser.bump();
                while let Some(line) = parser.bump() {
                    if line.keyword() == "end" {
                        break;
                    }
                }
            } else if line.keyword() == "device" {
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
            "idcode" => {
                if let Some(word) = self.word(line, 1, "a 32-bit IDCODE") {
                    let body = word.trim_start_matches("0x").trim_start_matches("0X");
                    match u32::from_str_radix(body, 16) {
                        Ok(value) => device.idcode = Some(value),
                        Err(_) => self.error(
                            line.span,
                            format!("expected a 32-bit IDCODE in hexadecimal, found `{word}`"),
                        ),
                    }
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
            "pins" => match line.get(1) {
                Some(token) if token.is("partial") => device.pins_partial = true,
                Some(token) => {
                    let span = token.span;
                    let text = token.as_str().to_owned();
                    self.unknown(span, format!("unknown `pins` option `{text}`"));
                }
                None => self.error(line.span, "expected `partial` after `pins`"),
            },
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
                "width" => {
                    let width = self.number_at(line, index, "a carry width")?;
                    index += 1;
                    bel.carry_width = Some(width);
                }
                "mode" => {
                    let Some(token) = line.get(index) else {
                        self.error(line.span, "expected flip-flop mode flags after `mode`");
                        break;
                    };
                    index += 1;
                    match FfVariant::parse(token.as_str()) {
                        Some(variant) => bel.ff = Some(variant),
                        None => {
                            let (span, text) = (token.span, token.as_str().to_owned());
                            self.unknown(span, format!("unknown flip-flop mode `{text}`"));
                        }
                    }
                }
                "for" => {
                    let Some(token) = line.get(index) else {
                        self.error(line.span, "expected port directions after `for`");
                        break;
                    };
                    index += 1;
                    for word in token.as_str().split(',').filter(|w| !w.is_empty()) {
                        if matches!(word, "in" | "out" | "inout") {
                            bel.io_dirs.push(word.to_owned());
                        } else {
                            let span = token.span;
                            self.unknown(
                                span,
                                format!("unknown port direction `{word}`, expected `in`, `out` or `inout`"),
                            );
                        }
                    }
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
            mode_layouts: Vec::new(),
            init_params: None,
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
                    let layout = self.mode_layout(line, index, width);
                    if let (Some(width), Some(depth)) = (width, depth) {
                        bram.width_modes.push((width, depth));
                        bram.mode_params.push(params);
                        bram.mode_layouts.push(layout);
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
                "init_params" => {
                    bram.init_params = self.init_params(line);
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
                    let mut signals = Vec::new();
                    let mut widths = Vec::new();
                    for (signal, name) in self.pairs(line, &mut index) {
                        // `we=WEA*2`: a pin several bits wide that takes
                        // the same value on each. Only a count of two or
                        // more is recorded, so that the text round-trips.
                        match name.split_once('*') {
                            Some((base, count)) => match count.parse::<u32>() {
                                Ok(bits) if bits >= 2 => {
                                    widths.push((signal.clone(), bits));
                                    signals.push((signal, base.to_owned()));
                                }
                                Ok(_) => signals.push((signal, base.to_owned())),
                                Err(_) => {
                                    self.error(
                                        line.span,
                                        format!("expected a bit count after `*`, found `{count}`"),
                                    );
                                    signals.push((signal, base.to_owned()));
                                }
                            },
                            None => signals.push((signal, name)),
                        }
                    }
                    bram.port_map.push(BramPort {
                        role,
                        signals,
                        widths,
                    });
                }
                other => {
                    let span = line.tokens[0].span;
                    self.unknown(span, format!("unknown `bram` directive `{other}`"));
                }
            }
        }
        Some(bram)
    }

    /// A list of bit positions, `expected` long when that is known.
    fn bit_list(&mut self, token: &Token, what: &str, expected: Option<u32>) -> Option<Vec<u32>> {
        let Some(bits) = parse_bit_list(token.as_str()) else {
            self.error(
                token.span,
                format!("expected {what} as a list of bit positions, found `{token}`"),
            );
            return None;
        };
        if let Some(width) = expected
            && u32::try_from(bits.len()).ok() != Some(width)
        {
            self.error(
                token.span,
                format!(
                    "{what} lists {} bit(s) for a mode {width} bits wide",
                    bits.len()
                ),
            );
            return None;
        }
        Some(bits)
    }

    /// The `data`, `addr`, `pad` and `init` clauses of a `mode` line,
    /// from token `index` on.
    fn mode_layout(
        &mut self,
        line: &'a Line,
        mut index: usize,
        width: Option<u32>,
    ) -> BramModeLayout {
        let mut layout = BramModeLayout::default();
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "data" => {
                    let Some(value) = line.get(index) else {
                        self.error(line.span, "expected the data pins after `data`");
                        break;
                    };
                    index += 1;
                    if let Some(bits) = self.bit_list(value, "`data`", width) {
                        layout.data_bits = bits;
                    }
                }
                "addr" => {
                    if let Some(value) = self.number_at(line, index, "the lowest address bit") {
                        layout.addr_low = value;
                    }
                    index += 1;
                }
                "pad" => {
                    let Some(value) = line.get(index) else {
                        self.error(line.span, "expected a constant after `pad`");
                        break;
                    };
                    index += 1;
                    match parse_value(value.as_str()) {
                        AttrValue::Const(c) => layout.addr_pad = Some(c),
                        _ => self.error(
                            value.span,
                            format!("expected a sized constant such as `4'b0011`, found `{value}`"),
                        ),
                    }
                }
                "init" => {
                    let select = match line.get(index).map(Token::as_str) {
                        Some("low") => WordSelect::Low,
                        Some("high") => WordSelect::High,
                        _ => {
                            self.error(line.span, "expected `low` or `high` after `init`");
                            break;
                        }
                    };
                    index += 1;
                    let mut words = Vec::new();
                    while let Some(value) = line.get(index) {
                        if !value.as_str().starts_with(|c: char| c.is_ascii_digit()) {
                            break;
                        }
                        index += 1;
                        if let Some(bits) = self.bit_list(value, "a word of `init`", width) {
                            words.push(bits);
                        }
                    }
                    if words.is_empty() {
                        self.error(line.span, "expected the row bits of each word after `init`");
                    } else {
                        layout.init = Some(BramInitLayout { select, words });
                    }
                }
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown `mode` option `{other}`"));
                }
            }
        }
        layout
    }

    /// An `init_params <prefix> count <n> digits <n> rows <n> slot <n>`
    /// line.
    fn init_params(&mut self, line: &'a Line) -> Option<BramInitParams> {
        let prefix = self.word(line, 1, "the parameter name prefix")?.to_owned();
        let mut params = BramInitParams {
            prefix,
            count: 0,
            digits: 1,
            rows: 0,
            slot: 0,
        };
        let mut index = 2;
        while let Some(token) = line.get(index) {
            index += 1;
            let value = match token.as_str() {
                "count" | "digits" | "rows" | "slot" => {
                    let value = self.number_at(line, index, "a number")?;
                    index += 1;
                    value
                }
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown `init_params` option `{other}`"));
                    continue;
                }
            };
            match token.as_str() {
                "count" => params.count = value,
                "digits" => params.digits = value,
                "rows" => params.rows = value,
                _ => params.slot = value,
            }
        }
        if params.count == 0 || params.rows == 0 || params.slot == 0 || params.digits == 0 {
            self.error(
                line.span,
                "`init_params` needs a non-zero `count`, `digits`, `rows` and `slot`",
            );
            return None;
        }
        Some(params)
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
        let mut pll = PllShape::new(name);
        let mut index = 2;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "input" | "vco" | "pfd" => {
                    let key = token.as_str().to_owned();
                    let lo = self.number_at(line, index, "a frequency in MHz")?;
                    let hi = self.number_at(line, index + 1, "a frequency in MHz")?;
                    index += 2;
                    match key.as_str() {
                        "input" => pll.input_mhz = Some((lo, hi)),
                        "vco" => pll.vco_mhz = Some((lo, hi)),
                        _ => pll.pfd_mhz = Some((lo, hi)),
                    }
                }
                "outputs" => {
                    let value = self.number_at(line, index, "a number of outputs")?;
                    index += 1;
                    pll.outputs = value;
                }
                "count" => {
                    let value = self.number_at(line, index, "a count")?;
                    index += 1;
                    pll.count = Some(value);
                }
                "feedback" => {
                    let word = self.word(line, index, "`vco` or `out` after `feedback`")?;
                    index += 1;
                    pll.feedback = match word {
                        "vco" => PllFeedback::Vco,
                        "out" => PllFeedback::Output,
                        other => {
                            let span = line.tokens[index - 1].span;
                            self.unknown(span, format!("unknown PLL feedback `{other}`"));
                            return None;
                        }
                    };
                }
                "divide" => {
                    let role = self.pll_role(line, index)?;
                    let param = self
                        .word(line, index + 1, "a divider parameter")?
                        .to_owned();
                    let min = self.number_at(line, index + 2, "the smallest divider value")?;
                    let max = self.number_at(line, index + 3, "the largest divider value")?;
                    index += 4;
                    let mut divider = PllDivider {
                        role,
                        param,
                        min,
                        max,
                        offset: 0,
                        power_of_two: false,
                    };
                    match line.get(index).map(Token::as_str) {
                        Some("pow2") => {
                            divider.power_of_two = true;
                            index += 1;
                        }
                        Some("offset") => {
                            divider.offset = self.number_at(line, index + 1, "an offset")?;
                            index += 2;
                        }
                        _ => {}
                    }
                    if min > max {
                        self.error(
                            line.span,
                            format!("divider `{}` runs from {min} down to {max}", divider.param),
                        );
                    }
                    pll.dividers.push(divider);
                }
                "derive" => {
                    let param = self.word(line, index, "a derived parameter")?.to_owned();
                    let role = self.pll_role(line, index + 1)?;
                    let offset = self.signed_at(line, index + 2)?;
                    index += 3;
                    pll.derived.push((param, role, offset));
                }
                "band" => {
                    let param = self.word(line, index, "a banded parameter")?.to_owned();
                    index += 1;
                    let mut bands = Vec::new();
                    for (upper, value) in self.pairs(line, &mut index) {
                        match (upper.parse::<u32>(), value.parse::<i64>()) {
                            (Ok(upper), Ok(value)) => bands.push((upper, value)),
                            _ => self.error(
                                line.span,
                                format!("expected `<MHz>=<value>` in band `{param}`"),
                            ),
                        }
                    }
                    pll.bands.push((param, bands));
                }
                "port" => pll.ports.extend(self.pairs(line, &mut index)),
                "tie" => {
                    for (port, level) in self.pairs(line, &mut index) {
                        match level.as_str() {
                            "0" => pll.ties.push((port, false)),
                            "1" => pll.ties.push((port, true)),
                            _ => self.error(
                                line.span,
                                format!("port `{port}` can be tied to 0 or 1, not `{level}`"),
                            ),
                        }
                    }
                }
                "param" => {
                    let params: Vec<(String, AttrValue)> = self
                        .pairs(line, &mut index)
                        .into_iter()
                        .map(|(key, value)| (key, parse_value(&value)))
                        .collect();
                    pll.params.extend(params);
                }
                other => {
                    let span = token.span;
                    self.unknown(span, format!("unknown `pll` option `{other}`"));
                }
            }
        }
        Some(pll)
    }

    /// The divider role named at `index`.
    fn pll_role(&mut self, line: &'a Line, index: usize) -> Option<PllDividerRole> {
        let word = self.word(line, index, "`ref`, `feedback` or `out`")?;
        match PllDividerRole::from_keyword(word) {
            Some(role) => Some(role),
            None => {
                let span = line.tokens[index].span;
                self.unknown(span, format!("unknown PLL divider `{word}`"));
                None
            }
        }
    }

    /// A signed integer at `index`.
    fn signed_at(&mut self, line: &'a Line, index: usize) -> Option<i64> {
        let token = match line.get(index) {
            Some(token) => token,
            None => {
                self.error(line.span, "expected a signed number");
                return None;
            }
        };
        match token.as_str().parse::<i64>() {
            Ok(value) => Some(value),
            Err(_) => {
                self.error(
                    token.span,
                    format!("expected a signed number, found `{token}`"),
                );
                None
            }
        }
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
    fn block_ram_layouts_and_partial_pins_round_trip() {
        let text = "\
device demo-l
  family demo
  bram R
    ports 2
    mode 16 256 init high 0-15
    mode 8 512 data 0,2,4,6,8,10,12,14 init high 0,2,4,6,8,10,12,14 1,3,5,7,9,11,13,15
    mode 18 1024 addr 4 pad 4'b0011 init low 0-17
    flags init
    init_params INIT_ count 16 digits 1 rows 16 slot 16
  end
  pins partial
  pin A1 io
end
";
        let (device, diags) = parse(text);
        assert_eq!(diags, "");
        let device = device.unwrap();
        assert!(device.pins_partial);
        let bram = &device.block_rams[0];
        let narrow = bram.layout_for_mode((8, 512));
        assert_eq!(narrow.data_bit(3), 6);
        let init = narrow.init.as_ref().unwrap();
        assert_eq!(init.select, WordSelect::High);
        // Word 300 is row 44, the odd bits: data bit 2 in row bit 5.
        assert_eq!(init.locate(300, 2, 256), Some((44, 5)));
        let wide = bram.layout_for_mode((18, 1024));
        assert_eq!(wide.addr_low, 4);
        assert_eq!(wide.addr_pad, Some(Const::from_u64(3, 4)));
        assert_eq!(wide.data_bit(17), 17);
        let params = bram.init_params.as_ref().unwrap();
        assert_eq!(
            (params.name(15), params.total_rows()),
            ("INIT_F".to_owned(), 256)
        );
        assert_eq!(params.param_width(), 256);
        let again = device.to_text();
        assert!(
            again.contains("    mode 8 512 data 0,2,4,6,8,10,12,14 init high"),
            "{again}"
        );
        assert!(again.contains("  pins partial\n"), "{again}");
        let (reparsed, diags) = parse(&again);
        assert_eq!(diags, "");
        assert_eq!(reparsed.unwrap(), device);

        // Low selection: consecutive words share a row.
        let low = BramInitLayout {
            select: WordSelect::Low,
            words: vec![vec![0, 1], vec![2, 3]],
        };
        assert_eq!(low.locate(5, 1, 8), Some((2, 3)));
        assert_eq!(low.locate(16, 0, 8), None);
        let hex = BramInitParams {
            prefix: "INITVAL_".into(),
            count: 64,
            digits: 2,
            rows: 16,
            slot: 20,
        };
        assert_eq!(hex.name(10), "INITVAL_0A");
        assert_eq!(write_bit_list(&[0, 1, 2, 4, 5, 7, 8, 9]), "0-2,4,5,7-9");
        assert_eq!(parse_bit_list("3-1"), None);
    }

    #[test]
    fn bad_block_ram_layouts_are_reported() {
        let text = "\
device demo-bad
  family demo
  bram R
    mode 8 512 data 0,2 init sideways 0-7
    mode 4 1024 init low 0-3 x pad 12 addr
    init_params INIT_ count 0 rows 16 slot 16
    init_params INIT_ count 16 bogus 3
  end
  pins everywhere
end
";
        let (_, diags) = parse(text);
        assert!(
            diags.contains("`data` lists 2 bit(s) for a mode 8 bits wide"),
            "{diags}"
        );
        assert!(
            diags.contains("expected `low` or `high` after `init`"),
            "{diags}"
        );
        assert!(diags.contains("unknown `mode` option `x`"), "{diags}");
        assert!(diags.contains("expected a sized constant"), "{diags}");
        assert!(diags.contains("expected the lowest address bit"), "{diags}");
        assert!(diags.contains("needs a non-zero `count`"), "{diags}");
        assert!(
            diags.contains("unknown `init_params` option `bogus`"),
            "{diags}"
        );
        assert!(
            diags.contains("unknown `pins` option `everywhere`"),
            "{diags}"
        );
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
            mode_layouts: Vec::new(),
            init_params: None,
            port_map: Vec::new(),
        };
        assert_eq!(bram.bits(), 0);
        assert_eq!(bram.layout_for_mode((8, 8)), BramModeLayout::default());
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

    #[test]
    fn flip_flop_modes_round_trip() {
        // Every combination the model can express writes and parses back
        // to itself, which is what keeps a `.dev` file diffable.
        for clk_pos in [true, false] {
            for (has_enable, enable_active_high) in [(false, true), (true, true), (true, false)] {
                for reset in [
                    None,
                    Some("sync_reset"),
                    Some("async_reset"),
                    Some("sync_set"),
                    Some("async_set"),
                ] {
                    for active_high in [true, false] {
                        if reset.is_none() && !active_high {
                            continue;
                        }
                        let variant = FfVariant {
                            clk_pos,
                            has_enable,
                            enable_active_high,
                            reset: reset.map(|word| FfReset {
                                active_high,
                                ..FfReset::from_keyword(word).unwrap()
                            }),
                        };
                        let flags = variant.flags();
                        assert_eq!(FfVariant::parse(&flags), Some(variant), "{flags}");
                        assert!(!variant.describe().is_empty());
                        // Flipping a polarity twice is the identity, and
                        // flipping one the variant has not is refused.
                        assert_eq!(
                            variant
                                .with_flipped_reset()
                                .and_then(FfVariant::with_flipped_reset),
                            reset.map(|_| variant)
                        );
                        assert_eq!(
                            variant
                                .with_flipped_enable()
                                .and_then(FfVariant::with_flipped_enable),
                            has_enable.then_some(variant)
                        );
                    }
                }
            }
        }
        assert_eq!(FfVariant::parse("posedge"), Some(FfVariant::plain()));
        // `active_low` without a set or reset says nothing, so it is not
        // a mode at all.
        assert_eq!(FfVariant::parse("posedge,active_low"), None);
        assert_eq!(FfVariant::parse("sideways"), None);
        assert_eq!(FfReset::from_keyword("nope"), None);
    }

    #[test]
    fn a_pll_line_carries_what_the_solver_needs() {
        let text = concat!(
            "device a\n",
            "  family f\n",
            "  pll P input 10 133 vco 533 1066 outputs 1 count 2 pfd 10 133 feedback out ",
            "divide ref R 0 15 offset 1 divide feedback F 1 64 divide out Q 1 6 pow2 ",
            "derive PH out -1 band FR 17=1 134=2 port ref=REF out=OUT fb=FB ",
            "tie RSTN=1 BYP=0 param MODE=\"SIMPLE\"\n",
            "  pll BAD divide sideways R 0 1 tie X=2\n",
            "end\n"
        );
        let mut map = SourceMap::new();
        let file = map.add("a.dev", text).unwrap();
        let mut diags = Diagnostics::new();
        let db = DeviceDb::parse(text, file, &mut diags);
        let rendered = diags.render(&map);
        assert!(
            rendered.contains("unknown PLL divider `sideways`"),
            "{rendered}"
        );
        let device = db.get("a").unwrap();
        let pll = &device.clock_resources.plls[0];
        assert!(pll.is_configurable());
        assert_eq!(pll.count, Some(2));
        assert_eq!(pll.feedback, PllFeedback::Output);
        assert_eq!(
            pll.divider(PllDividerRole::Reference).unwrap().divisor(0),
            1
        );
        assert_eq!(pll.divider(PllDividerRole::Output).unwrap().divisor(3), 8);
        assert_eq!(pll.derived, [("PH".to_owned(), PllDividerRole::Output, -1)]);
        assert_eq!(pll.bands[0].1, [(17, 1), (134, 2)]);
        assert_eq!(pll.port("fb"), Some("FB"));
        assert_eq!(
            pll.ties,
            [("RSTN".to_owned(), true), ("BYP".to_owned(), false)]
        );
        // And it writes back to itself.
        let again = Device::parse(&device.to_text(), file, &mut Diagnostics::new()).unwrap();
        assert_eq!(again.clock_resources.plls[0], *pll);
    }

    #[test]
    fn a_mode_clause_is_parsed_and_written() {
        let text = concat!(
            "device a\n",
            "  family f\n",
            "  bel FF ff mode negedge,enable,async_set,active_low port clk=C d=D q=Q en=E rst=S\n",
            "  bel FF2 ff mode sideways port clk=C\n",
            "  bel FF3 ff mode\n",
            "end\n"
        );
        let (device, diags) = parse(text);
        let device = device.unwrap();
        assert!(
            diags.contains("unknown flip-flop mode `sideways`"),
            "{diags}"
        );
        assert!(diags.contains("expected flip-flop mode flags"), "{diags}");
        let ff = device.bel_named("FF").unwrap();
        let variant = ff.ff.unwrap();
        assert!(!variant.clk_pos && variant.has_enable);
        let reset = variant.reset.unwrap();
        assert!(reset.asynchronous && reset.sets && !reset.active_high);
        assert_eq!(device.ff_variant(variant).unwrap().name, "FF");
        assert_eq!(device.ff_variant(FfVariant::plain()), None);
        assert_eq!(device.ff_variants().count(), 1);
        assert!(
            device
                .to_text()
                .contains("mode negedge,enable,async_set,active_low")
        );
        // A `ff` bel with no mode is not a variant mapping can choose.
        assert!(device.bel_named("FF3").is_some_and(|b| b.ff.is_none()));
    }

    #[test]
    fn a_width_clause_makes_a_carry_element_wide() {
        let text = concat!(
            "device a\n",
            "  family f\n",
            "  bel WIDE carry width 4 port ci=CI cyinit=CYINIT p=S di=DI s=O co=CO\n",
            // Only one way in, which is allowed: a family with no
            // fabric pin names only its chain pin.
            "  bel CHAIN carry width 2 port ci=CIN p=P di=D s=SUM co=COUT\n",
            // A width with a role missing is not a wide carry; nor is a
            // width of zero, nor a width with no way in at all.
            "  bel SHORT carry width 4 port ci=CI p=S s=O co=CO\n",
            "  bel ZERO carry width 0 port ci=CI p=S di=DI s=O co=CO\n",
            "  bel NOWAY carry width 4 port p=S di=DI s=O co=CO\n",
            // And the one-bit form is untouched by all of this.
            "  bel NARROW carry port ci=CI i0=I0 i1=I1 co=CO\n",
            "end\n"
        );
        let (device, diags) = parse(text);
        let device = device.unwrap();
        assert_eq!(diags, "", "the grammar complained");

        let wide = device.bel_named("WIDE").unwrap().wide_carry().unwrap();
        assert_eq!(wide.width, 4);
        assert_eq!(wide.propagate, "S");
        assert_eq!(wide.data, "DI");
        assert_eq!(wide.sum, "O");
        assert_eq!(wide.carry_out, "CO");
        assert_eq!((wide.carry_in, wide.init), (Some("CI"), Some("CYINIT")));

        let chain = device.bel_named("CHAIN").unwrap().wide_carry().unwrap();
        assert_eq!(
            (chain.width, chain.carry_in, chain.init),
            (2, Some("CIN"), None)
        );

        for name in ["SHORT", "ZERO", "NOWAY", "NARROW"] {
            assert!(
                device.bel_named(name).unwrap().wide_carry().is_none(),
                "`{name}` was read as a wide carry"
            );
        }
        // A role list alone is not a carry element either: a LUT with a
        // `width` is still a LUT.
        assert!(device.bel_named("NARROW").unwrap().has_ports(&["i0"]));

        // The clause survives a round trip through the text format.
        let again = parse(&device.to_text()).0.unwrap();
        assert_eq!(again, device);
        assert!(
            device
                .to_text()
                .contains("bel WIDE carry width 4 port ci=CI cyinit=CYINIT"),
            "{}",
            device.to_text()
        );
    }

    #[test]
    fn primitive_ports_cover_every_kind_of_primitive() {
        let device = super::super::target("ice40-hx1k-tq144").unwrap();
        assert!(device.primitive_ports("SB_NONESUCH").is_none());
        let lut = device.primitive_ports("SB_LUT4").unwrap();
        assert_eq!(lut, ["I0", "I1", "I2", "I3", "O"]);
        // The flip-flop variants share one name on some families and not
        // on others; either way the union of their pins is the answer.
        let ff = device.primitive_ports("SB_DFFESR").unwrap();
        assert_eq!(ff, ["C", "D", "Q", "E", "R"]);
        let ram = device.primitive_ports("SB_RAM40_4K").unwrap();
        assert!(ram.contains(&"RDATA".to_owned()) && ram.contains(&"WCLKE".to_owned()));
        let ecp5 = super::super::target("ecp5-45f-CABGA381").unwrap();
        assert_eq!(
            ecp5.primitive_ports("TRELLIS_FF").unwrap(),
            ["CLK", "DI", "Q", "CE", "LSR"]
        );
        assert!(
            ecp5.primitive_ports("MULT18X18D")
                .unwrap()
                .contains(&"P".to_owned())
        );
        // A PLL's ports are its port map and the pins it ties.
        let pll = ecp5.primitive_ports("EHXPLLL").unwrap();
        for port in ["CLKI", "CLKOP", "CLKFB", "RST", "STDBY"] {
            assert!(pll.contains(&port.to_owned()), "{port}");
        }
        // A carry unit without a port map is "declared, ports unknown":
        // a `Some(empty)`, not a `None`.
        assert_eq!(ecp5.primitive_ports("CCU2C"), Some(Vec::new()));
    }
}
