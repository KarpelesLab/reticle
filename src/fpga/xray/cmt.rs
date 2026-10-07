//! The clock management tile: a `PLLE2_BASE` as a bel, and the registers
//! that configure it.
//!
//! # Why a PLL is not configured like every other bel
//!
//! A lookup table's truth table is its `INIT` parameter, bit for bit, and
//! [`ConfigEntry::Param`](crate::fpga::ConfigEntry) carries it to the
//! bitstream unchanged. A PLL's parameters are not stored that way at
//! all. What sits in the configuration frames is the content of the
//! block's **dynamic reconfiguration port** (DRP) registers — the same
//! sixteen-bit words a design could write at run time through `DADDR` /
//! `DI` — and `CLKOUT0_DIVIDE = 32` is in there as a *high time* of 16,
//! a *low time* of 16, an *edge* bit and a *no-count* bit, beside a
//! 40-bit lock-detector table and a 10-bit loop-filter setting that
//! depend on the feedback multiplier through lookup tables Xilinx
//! publishes and does not explain.
//!
//! `prjxray-db` exposes those registers field by field. Its
//! `032-cmt-pll` fuzzer lays the whole register space out bit by bit
//! (`write_pll_reg.py`: register `r`, bit `b`, alternating between frames
//! 28 and 29) and names every field after XAPP888's register map, so
//! `segbits_cmt_top_l_upper_t.db` has `PLLE2_ADV.CLKOUT0_CLKOUT1_HIGH_TIME[0..5]`,
//! `PLLE2_ADV.LKTABLE[0..39]`, `PLLE2_ADV.TABLE[0..9]` and the rest. The
//! generic feature reader already turns each of those into a
//! [`ConfigEntry::Param`](crate::fpga::ConfigEntry) named after the
//! field. What nothing supplies is the *value* of the field, and that is
//! what [`pll_registers`] computes.
//!
//! # What is computed and what is quoted
//!
//! | | Where it comes from |
//! |---|---|
//! | high time, low time, edge, no-count of every counter | **computed** here, from XAPP888's divider arithmetic for a 50 % duty cycle and no phase shift |
//! | `LKTABLE`, the lock detector's settings | **quoted**: XAPP888's table, as transcribed into f4pga-arch-defs' `xilinx/xc7/techmap/cells_map.v` (`pll_lktable_lookup`), extracted from that file by a script rather than retyped |
//! | `TABLE`, the loop filter's settings | **quoted** the same way (`pll_table_lookup`); f4pga's `OPTIMIZED` table is identical to its `HIGH` one, and so is this |
//! | `FILTREG1_RESERVED = 0x008`, `LOCKREG3_RESERVED = 1`, every other reserved field and `POWER_REG` zero | **quoted**: f4pga-arch-defs and nextpnr-xilinx both write exactly these, and neither says why |
//! | `IN_USE`, `COMPENSATION.Z_ZHOLD_OR_CLKIN_BUF` | **quoted**: the two features f4pga's `plle2_adv.pb_type.xml` writes for every PLL and nextpnr-xilinx for `COMPENSATION = INTERNAL` |
//! | `ZINV_RST` / `ZINV_PWRDWN` set for a pin tied low | **derived** below, from three sources that agree once the database's naming is read correctly |
//!
//! Two independent checks tie the quoted tables to something: for a
//! multiplier of 8, the entries this file holds are `0xB5BE8FA401` and
//! `0x3B4`, which are the very constants nextpnr-xilinx hard-codes for
//! every PLL (`write_pll` in `xilinx/fasm.cc`); and the register layout
//! the field names imply is the one XAPP888 states. **Nothing here has
//! been compared with a Vivado bitstream holding a PLL** — the four
//! harness designs in `artix7/harness/` have none — and nothing has been
//! run on a part.
//!
//! # The reset that reads as one
//!
//! An unrouted interconnect input on this fabric is not floating: every
//! `IMUX` of an `INT_L` / `INT_R` tile has a `default` pseudo-pip from
//! `VCC_WIRE` (`ppips_int_l.db`), so a site pin nothing drives **reads
//! one**. The PLL's `RST` and `PWRDWN` arrive through such an `IMUX`,
//! which means a PLL whose reset is "tied low" by leaving it unrouted is
//! held in reset and powered down, and never locks.
//!
//! The bit that rescues it is called `ZINV_RST`, and the name is
//! misleading. prjxray's fuzzer tagged it `1 ^ IS_RST_INVERTED` on PLLs
//! whose `RST` was **unconnected** — so it found the bit Vivado sets when
//! it ties the reset low by leaving the pin on its default one and
//! *inverting* it. nextpnr-xilinx writes `ZINV_RST = IS_RST_INVERTED`
//! for a reset it routes, with a comment that the name looks wrong, and
//! f4pga writes `ZINV_RST = 1` with the pin tied to `VCC` for a constant
//! zero and `ZINV_RST = 0` for a routed reset. All three say the same
//! thing: **the bit set is the inverter on**. So a reset tied low is left
//! unrouted with the bit set, and a routed one gets the bit clear.
//! `CLKINSEL` is the other way round and needs nothing: `PLLE2_BASE`
//! holds it high, which selects `CLKIN1`, which is what the default one
//! already reads.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use super::XrayFabric;
use super::sites::{BelPin, ExtraBel};
use crate::fpga::arch::{ConfigEntry, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, BitstreamError};
use crate::fpga::place::{Netlist, Placement};
use crate::ir::{AttrValue, Design, ModuleId};
use crate::logic::Bit;

/// The bel a clock management tile's PLL becomes, for the tile types that
/// hold one.
///
/// Only the `UPPER_T` quarter of a CMT has a `PLLE2_ADV` site, and both
/// orientations call the PLL's own wires `CMT_TOP_R_UPPER_T_PLLE2_*` —
/// in the `L` tile too, which is prjxray's spelling and not a typo here.
/// Every wire below is read off `ppips_cmt_top_{l,r}_upper_t.db` (the
/// outputs and the control inputs are `always` pseudo-pips from those
/// wires) or `segbits_cmt_top_{l,r}_upper_t.db` (the two clock inputs are
/// the destinations of the input muxes). The pin *names* are UG472's.
///
/// The bel's features come from the feature reader, which already files
/// every `PLLE2_ADV.*` feature under a bel of this name; this only adds
/// the kind and the pins, and the register values are
/// [`XrayFabric::configure_clock_managers`]'s.
pub(super) fn clock_manager_bels(tile_type: &str) -> Vec<ExtraBel> {
    if !matches!(tile_type, "CMT_TOP_L_UPPER_T" | "CMT_TOP_R_UPPER_T") {
        return Vec::new();
    }
    let wire = |pin: &str| format!("CMT_TOP_R_UPPER_T_PLLE2_{pin}");
    let pins = [
        ("ref", "CLKIN1"),
        ("fb", "CLKFBIN"),
        ("out", "CLKOUT0"),
        ("fbout", "CLKFBOUT"),
        ("lock", "LOCKED"),
        ("rst", "RST"),
        ("pwrdwn", "PWRDWN"),
    ];
    vec![ExtraBel {
        name: PLL_BEL.to_owned(),
        kind: "pll",
        pins: pins
            .iter()
            .map(|&(role, pin)| BelPin {
                role,
                wire: wire(pin),
            })
            .collect(),
        cells: Vec::new(),
    }]
}

/// The name the feature reader gives a CMT's PLL bel: its feature prefix.
const PLL_BEL: &str = "PLLE2_ADV";

/// The one primitive this module configures.
const PLL_PRIMITIVE: &str = "PLLE2_BASE";

/// The settings of one counter of the PLL, as XAPP888's divider registers
/// hold them for a 50 % duty cycle and no phase shift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counter {
    /// VCO cycles high, modulo 64 (a 0 means 64).
    pub high: u64,
    /// VCO cycles low, modulo 64.
    pub low: u64,
    /// Half a cycle more high time, which is how an odd division keeps
    /// its duty cycle at one half.
    pub edge: bool,
    /// The counter is bypassed, which is how a division by one is made.
    pub no_count: bool,
}

/// The counter settings for a division by `divide`, 50 % duty cycle.
///
/// **Computed**, from XAPP888's `mmcm_pll_divider`: one is a bypassed
/// counter (high and low both 1, `NO_COUNT` set); otherwise the high time
/// is half the division rounded down, the low time the rest, and an odd
/// division sets `EDGE` so that the extra half cycle evens the two out.
/// The fields are six bits wide and a division of 128 stores 64 as 0,
/// which is what the hardware means by 0.
///
/// This is the arithmetic f4pga's `pll_divider_regs` performs for a duty
/// cycle of 50 000 and nextpnr-xilinx's `write_pll_clkout` performs
/// unconditionally; the three agree for every integer division.
pub fn counter(divide: u64) -> Counter {
    if divide <= 1 {
        return Counter {
            high: 1,
            low: 1,
            edge: false,
            no_count: true,
        };
    }
    let high = divide / 2;
    Counter {
        high: high & 0x3f,
        low: (divide - high) & 0x3f,
        edge: divide % 2 == 1,
        no_count: false,
    }
}

/// The lock detector's settings for a feedback multiplier, `{LockRefDly,
/// LockFBDly, LockCnt, LockSatHigh, UnlockCnt}` packed as 5 + 5 + 10 + 10
/// + 10 bits. Indexed by multiplier minus one.
///
/// **Quoted**: XAPP888's table as f4pga-arch-defs transcribes it in
/// `pll_lktable_lookup`, extracted from that file by script.
const LOCK_TABLE: [u64; 64] = [
    0x31be8fa401, // 1
    0x31be8fa401, // 2
    0x423e8fa401, // 3
    0x5afe8fa401, // 4
    0x73be8fa401, // 5
    0x8c7e8fa401, // 6
    0x9cfe8fa401, // 7
    0xb5be8fa401, // 8
    0xce7e8fa401, // 9
    0xe73e8fa401, // 10
    0xfff84fa401, // 11
    0xfff39fa401, // 12
    0xffeeefa401, // 13
    0xffebcfa401, // 14
    0xffe8afa401, // 15
    0xffe71fa401, // 16
    0xffe3ffa401, // 17
    0xffe26fa401, // 18
    0xffe0dfa401, // 19
    0xffdf4fa401, // 20
    0xffddbfa401, // 21
    0xffdc2fa401, // 22
    0xffda9fa401, // 23
    0xffd90fa401, // 24
    0xffd90fa401, // 25
    0xffd77fa401, // 26
    0xffd5efa401, // 27
    0xffd5efa401, // 28
    0xffd45fa401, // 29
    0xffd45fa401, // 30
    0xffd2cfa401, // 31
    0xffd2cfa401, // 32
    0xffd2cfa401, // 33
    0xffd13fa401, // 34
    0xffd13fa401, // 35
    0xffd13fa401, // 36
    0xffcfafa401, // 37
    0xffcfafa401, // 38
    0xffcfafa401, // 39
    0xffcfafa401, // 40
    0xffcfafa401, // 41
    0xffcfafa401, // 42
    0xffcfafa401, // 43
    0xffcfafa401, // 44
    0xffcfafa401, // 45
    0xffcfafa401, // 46
    0xffcfafa401, // 47
    0xffcfafa401, // 48
    0xffcfafa401, // 49
    0xffcfafa401, // 50
    0xffcfafa401, // 51
    0xffcfafa401, // 52
    0xffcfafa401, // 53
    0xffcfafa401, // 54
    0xffcfafa401, // 55
    0xffcfafa401, // 56
    0xffcfafa401, // 57
    0xffcfafa401, // 58
    0xffcfafa401, // 59
    0xffcfafa401, // 60
    0xffcfafa401, // 61
    0xffcfafa401, // 62
    0xffcfafa401, // 63
    0xffcfafa401, // 64
];

/// The loop filter's `{CP, RES, LFHF}` for `BANDWIDTH = "LOW"`, indexed
/// by multiplier minus one. **Quoted**, as [`LOCK_TABLE`] is.
const FILTER_LOW: [u16; 64] = [
    0x0bc, 0x0bc, 0x09c, 0x0b4, 0x094, 0x094, 0x0a4, 0x0b8, // 1..8
    0x0b8, 0x084, 0x084, 0x098, 0x098, 0x098, 0x098, 0x0a8, // 9..16
    0x0a8, 0x0a8, 0x0a8, 0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, // 17..24
    0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x088, 0x088, // 25..32
    0x088, 0x088, 0x088, 0x088, 0x088, 0x088, 0x088, 0x088, // 33..40
    0x0f0, 0x0f0, 0x0f0, 0x0f0, 0x0f0, 0x0f0, 0x0f0, 0x090, // 41..48
    0x090, 0x090, 0x090, 0x090, 0x090, 0x090, 0x090, 0x090, // 49..56
    0x090, 0x090, 0x090, 0x090, 0x090, 0x090, 0x090, 0x090, // 57..64
];

/// The loop filter for `BANDWIDTH = "HIGH"` and `"OPTIMIZED"`, which
/// f4pga's transcription gives identical tables. **Quoted**.
const FILTER_HIGH: [u16; 64] = [
    0x0dc, 0x0dc, 0x17c, 0x1fc, 0x1ec, 0x35c, 0x3ac, 0x3b4, // 1..8
    0x3f4, 0x3dc, 0x3ec, 0x3f4, 0x3cc, 0x394, 0x3d4, 0x3d4, // 9..16
    0x3d4, 0x3d4, 0x1d8, 0x1d8, 0x1d8, 0x1d8, 0x170, 0x170, // 17..24
    0x170, 0x304, 0x304, 0x304, 0x304, 0x304, 0x304, 0x304, // 25..32
    0x304, 0x108, 0x108, 0x108, 0x0a0, 0x0a0, 0x0a0, 0x0d0, // 33..40
    0x0a0, 0x0a0, 0x0a0, 0x0a0, 0x0a0, 0x0a0, 0x0a0, 0x0a0, // 41..48
    0x0a0, 0x0a0, 0x0a0, 0x0a0, 0x0a0, 0x130, 0x130, 0x130, // 49..56
    0x130, 0x130, 0x130, 0x130, 0x090, 0x090, 0x090, 0x090, // 57..64
];

/// What a `PLLE2_BASE` is asked to do, in its own parameter names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PllSettings {
    /// `DIVCLK_DIVIDE`, 1..=56.
    pub divclk_divide: u64,
    /// `CLKFBOUT_MULT`, 2..=64.
    pub clkfbout_mult: u64,
    /// `CLKOUT0_DIVIDE`, 1..=128.
    pub clkout0_divide: u64,
    /// `BANDWIDTH`: `LOW`, `HIGH` or `OPTIMIZED`.
    pub bandwidth: String,
    /// `STARTUP_WAIT = "TRUE"`.
    pub startup_wait: bool,
    /// Whether `CLKOUT0` drives anything; its output enable follows.
    pub clkout0_used: bool,
    /// Whether `CLKFBOUT` drives anything.
    pub clkfbout_used: bool,
    /// Whether the reset pin must be inverted: true when it is tied low
    /// and left unrouted, so that the one it reads becomes a zero.
    pub invert_rst: bool,
    /// The same for `PWRDWN`.
    pub invert_pwrdwn: bool,
}

/// Why a PLL could not be configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClockManagerError {
    /// A cell of another primitive sits on a PLL site. An `MMCME2_BASE`
    /// is a different block with a different register map; configuring
    /// it with a PLL's would give a part that computes nothing.
    Unsupported {
        /// The instance.
        instance: String,
        /// Its primitive.
        primitive: String,
    },
    /// A parameter is missing, not an integer, or outside its range.
    BadParameter {
        /// The instance.
        instance: String,
        /// The parameter.
        name: String,
        /// What is wrong with it.
        message: String,
    },
    /// The database has no feature for a register field this needs, so
    /// the database and this module have drifted apart.
    MissingField {
        /// The field.
        field: String,
    },
    /// A bit fell outside its tile.
    Bitstream(BitstreamError),
}

impl fmt::Display for ClockManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClockManagerError::Unsupported {
                instance,
                primitive,
            } => write!(
                f,
                "`{instance}` is a `{primitive}` on a PLL site; this flow configures only \
                 `{PLL_PRIMITIVE}`, and another block's registers would configure nothing"
            ),
            ClockManagerError::BadParameter {
                instance,
                name,
                message,
            } => write!(f, "`{instance}`: `{name}` {message}"),
            ClockManagerError::MissingField { field } => write!(
                f,
                "the chip database has no `PLLE2_ADV.{field}` feature, which a PLL needs; \
                 the database is not the one this flow was written against"
            ),
            ClockManagerError::Bitstream(e) => write!(f, "{e}"),
        }
    }
}

impl Error for ClockManagerError {}

impl From<BitstreamError> for ClockManagerError {
    fn from(e: BitstreamError) -> Self {
        ClockManagerError::Bitstream(e)
    }
}

/// Every register field of a configured PLL, in the database's own
/// feature names (without the `PLLE2_ADV.` prefix), with its value.
///
/// A multi-bit field is one entry, read bit by bit against the
/// `name[index]` features; a one-bit feature with no index is an entry
/// whose value is 0 or 1. Fields written as zero are listed too, so that
/// a database missing one of them is caught rather than ignored.
pub fn pll_registers(settings: &PllSettings) -> BTreeMap<String, u64> {
    let mut out: BTreeMap<String, u64> = BTreeMap::new();
    let mut put = |name: String, value: u64| {
        out.insert(name, value);
    };

    put("IN_USE".to_owned(), 1);
    // Feedback through the tile's own `CLKFBOUT2IN` path, with no global
    // buffer on it: compensation `INTERNAL`, which is this one bit and
    // the input mux the router sets.
    put("COMPENSATION.Z_ZHOLD_OR_CLKIN_BUF".to_owned(), 1);
    put("COMPENSATION.ZHOLD_NO_CLKIN_BUF".to_owned(), 0);
    put("COMPENSATION.ZHOLD_NO_CLKIN_BUF_NO_TOP".to_owned(), 0);
    put("COMP.ZHOLD_NO_CLKIN_BUF_TOP".to_owned(), 0);
    put("STARTUP_WAIT".to_owned(), u64::from(settings.startup_wait));
    put("ZINV_RST".to_owned(), u64::from(settings.invert_rst));
    put("ZINV_PWRDWN".to_owned(), u64::from(settings.invert_pwrdwn));
    put("INV_CLKINSEL".to_owned(), 0);

    let divclk = counter(settings.divclk_divide);
    put("DIVCLK_DIVCLK_HIGH_TIME".to_owned(), divclk.high);
    put("DIVCLK_DIVCLK_LOW_TIME".to_owned(), divclk.low);
    put("DIVCLK_DIVCLK_EDGE".to_owned(), u64::from(divclk.edge));
    put(
        "DIVCLK_DIVCLK_NO_COUNT".to_owned(),
        u64::from(divclk.no_count),
    );
    put("DIVCLK_DIVCLK_RESERVED".to_owned(), 0);

    // Every output counter is written, used or not: an unused one keeps
    // the library default of a division by one, as f4pga writes it, and
    // its output enable stays off.
    for (name, divide, used) in [
        ("CLKFBOUT", settings.clkfbout_mult, settings.clkfbout_used),
        ("CLKOUT0", settings.clkout0_divide, settings.clkout0_used),
        ("CLKOUT1", 1, false),
        ("CLKOUT2", 1, false),
        ("CLKOUT3", 1, false),
        ("CLKOUT4", 1, false),
        ("CLKOUT5", 1, false),
    ] {
        let c = counter(divide);
        put(format!("{name}_CLKOUT1_HIGH_TIME"), c.high);
        put(format!("{name}_CLKOUT1_LOW_TIME"), c.low);
        put(format!("{name}_CLKOUT1_OUTPUT_ENABLE"), u64::from(used));
        put(format!("{name}_CLKOUT1_PHASE_MUX"), 0);
        put(format!("{name}_CLKOUT2_DELAY_TIME"), 0);
        put(format!("{name}_CLKOUT2_EDGE"), u64::from(c.edge));
        put(format!("{name}_CLKOUT2_NO_COUNT"), u64::from(c.no_count));
        put(format!("{name}_CLKOUT2_MX"), 0);
        put(format!("{name}_CLKOUT2_FRAC"), 0);
        put(format!("{name}_CLKOUT2_FRAC_EN"), 0);
        put(format!("{name}_CLKOUT2_FRAC_WF_R"), 0);
        put(format!("{name}_CLKOUT2_RESERVED"), 0);
    }

    let index = usize::try_from(settings.clkfbout_mult.clamp(1, 64) - 1).unwrap_or(0);
    put("LKTABLE".to_owned(), LOCK_TABLE[index]);
    let filter = if settings.bandwidth == "LOW" {
        FILTER_LOW[index]
    } else {
        FILTER_HIGH[index]
    };
    put("TABLE".to_owned(), u64::from(filter));
    put("FILTREG1_RESERVED".to_owned(), 0x008);
    put("FILTREG2_RESERVED".to_owned(), 0);
    put("LOCKREG1_RESERVED".to_owned(), 0);
    put("LOCKREG2_RESERVED".to_owned(), 0);
    put("LOCKREG3_RESERVED".to_owned(), 1);
    put("POWER_REG_POWER_REG_POWER_REG".to_owned(), 0);
    out
}

/// Sets the bits `registers` asks for on a bel whose configuration the
/// feature reader built, and says how many.
///
/// # Errors
///
/// [`ClockManagerError::MissingField`] when a field has no entry in
/// `config` at all, and [`ClockManagerError::Bitstream`] when a bit falls
/// outside the tile.
pub(super) fn apply_registers(
    config: &[ConfigEntry],
    registers: &BTreeMap<String, u64>,
    tile: (u32, u32),
    bits: &mut Bitstream,
) -> Result<usize, ClockManagerError> {
    for field in registers.keys() {
        let known = config.iter().any(|entry| match entry {
            ConfigEntry::Param { name, .. } => name == field,
            ConfigEntry::Cell { primitive, .. } => primitive == field,
            // A constant on a pin is applied with the rest of the site's
            // configuration, never as a register field.
            ConfigEntry::ParamZero { .. } | ConfigEntry::Tied { .. } => false,
        });
        if !known {
            return Err(ClockManagerError::MissingField {
                field: field.clone(),
            });
        }
    }
    let mut count = 0usize;
    for entry in config {
        match entry {
            ConfigEntry::Param { name, index, at } => {
                let Some(value) = registers.get(name) else {
                    continue;
                };
                if *index < 64 && (value >> index) & 1 == 1 {
                    bits.set(tile, *at)?;
                    count += 1;
                }
            }
            ConfigEntry::Cell {
                primitive,
                bits: ones,
            } => {
                if registers.get(primitive) == Some(&1) {
                    for bit in ones {
                        bits.set(tile, *bit)?;
                        count += 1;
                    }
                }
            }
            ConfigEntry::ParamZero { .. } | ConfigEntry::Tied { .. } => {}
        }
    }
    Ok(count)
}

impl XrayFabric {
    /// Configures every PLL the placement put on a clock management tile,
    /// and says how many bits that set.
    ///
    /// # Why this is a pass of its own
    ///
    /// For the same reason as [`XrayFabric::enable_global_clocks`]: what
    /// a PLL's bits are depends on more than its bel. The register values
    /// are computed from the cell's parameters rather than copied from
    /// them ([`pll_registers`]), and the reset's inverter depends on
    /// whether the reset was *routed* — a pin left unrouted reads one on
    /// this fabric; see the module documentation. So it runs after
    /// [`crate::fpga::bitstream::generate`], on the same bitstream.
    ///
    /// # Errors
    ///
    /// [`ClockManagerError`]: a primitive other than `PLLE2_BASE` on a PLL
    /// site, a divider out of its range, or a database missing a field.
    pub fn configure_clock_managers(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
        bits: &mut Bitstream,
    ) -> Result<usize, ClockManagerError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(0);
        };
        let mut count = 0usize;
        for (index, instance) in netlist.instances.iter().enumerate() {
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(bel) = self.arch.tile_types[site.tile_type].bel(&site.bel) else {
                continue;
            };
            if bel.name != PLL_BEL {
                continue;
            }
            if instance.primitive != PLL_PRIMITIVE {
                return Err(ClockManagerError::Unsupported {
                    instance: instance.name.clone(),
                    primitive: instance.primitive.clone(),
                });
            }
            let params = m.cells.get(instance.cell).map(|cell| &cell.params);
            let integer = |name: &str, default: i64, lo: u64, hi: u64| {
                let value = params
                    .and_then(|p| p.get(name))
                    .map_or(Some(default), AttrValue::as_int);
                let bad = |message: String| ClockManagerError::BadParameter {
                    instance: instance.name.clone(),
                    name: name.to_owned(),
                    message,
                };
                let value = value.ok_or_else(|| bad("is not an integer".to_owned()))?;
                let value =
                    u64::try_from(value).map_err(|_| bad(format!("is {value}, below {lo}")))?;
                if value < lo || value > hi {
                    return Err(bad(format!("is {value}, outside {lo}..={hi}")));
                }
                Ok(value)
            };
            let text = |name: &str, default: &str| -> String {
                params
                    .and_then(|p| p.get(name))
                    .and_then(AttrValue::as_str)
                    .unwrap_or(default)
                    .to_ascii_uppercase()
            };
            // The library defaults for the ones a design may leave out.
            let divclk_divide = integer("DIVCLK_DIVIDE", 1, 1, 56)?;
            let clkfbout_mult = integer("CLKFBOUT_MULT", 5, 2, 64)?;
            let clkout0_divide = integer("CLKOUT0_DIVIDE", 1, 1, 128)?;
            let bandwidth = text("BANDWIDTH", "OPTIMIZED");
            if !matches!(bandwidth.as_str(), "LOW" | "HIGH" | "OPTIMIZED") {
                return Err(ClockManagerError::BadParameter {
                    instance: instance.name.clone(),
                    name: "BANDWIDTH".to_owned(),
                    message: format!("is `{bandwidth}`, not LOW, HIGH or OPTIMIZED"),
                });
            }

            let pin = |role: &str| {
                instance
                    .pins
                    .iter()
                    .map(|p| &netlist.pins[*p])
                    .find(|p| p.role == role)
            };
            let used = |role: &str| pin(role).is_some_and(|p| p.signal.is_some());
            // A control pin with no signal is left unrouted and reads one;
            // it has to be inverted to mean the zero a `PLLE2_BASE`
            // ties it to. One tied *high* reads what it asks for.
            let invert = |role: &str| match pin(role) {
                Some(p) if p.signal.is_some() => false,
                Some(p) => p.constant != Some(Bit::One),
                None => true,
            };
            let settings = PllSettings {
                divclk_divide,
                clkfbout_mult,
                clkout0_divide,
                bandwidth,
                startup_wait: text("STARTUP_WAIT", "FALSE") == "TRUE",
                clkout0_used: used("out"),
                clkfbout_used: used("fbout"),
                invert_rst: invert("rst"),
                invert_pwrdwn: invert("pwrdwn"),
            };
            count += self.write_pll(site.tile, &settings, bits)?;
        }
        Ok(count)
    }

    /// Writes one PLL's registers into the clock management tile at
    /// `tile`, and says how many bits that set.
    ///
    /// This is the half of [`XrayFabric::configure_clock_managers`] that
    /// needs no netlist, public so that a test can put two different
    /// settings into the same tile and read them back through
    /// [`super::XrayDatabase::decode`].
    ///
    /// # Errors
    ///
    /// [`ClockManagerError::MissingField`] when the tile at `tile` has no
    /// PLL, or its database lacks a field; a bit outside the tile is
    /// [`ClockManagerError::Bitstream`].
    pub fn write_pll(
        &self,
        tile: (u32, u32),
        settings: &PllSettings,
        bits: &mut Bitstream,
    ) -> Result<usize, ClockManagerError> {
        let bel = self
            .arch
            .tile_index_at(tile.0, tile.1)
            .and_then(|index| self.arch.tile_types[index].bel(PLL_BEL))
            .ok_or_else(|| ClockManagerError::MissingField {
                field: "IN_USE".to_owned(),
            })?;
        apply_registers(&bel.config, &pll_registers(settings), tile, bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The divider arithmetic, worked by hand from XAPP888 rather than
    /// re-derived by the same code. Would catch an `EDGE` on the wrong
    /// parity or high and low swapped for an odd division (both of which
    /// give the right frequency at a wrong duty cycle on silicon, or the
    /// wrong frequency when `EDGE` is lost); would not catch a wrong
    /// reading of XAPP888 itself.
    #[test]
    fn a_counter_splits_a_division_into_high_and_low_time() {
        let c = |high, low, edge, no_count| Counter {
            high,
            low,
            edge,
            no_count,
        };
        assert_eq!(counter(1), c(1, 1, false, true));
        assert_eq!(counter(2), c(1, 1, false, false));
        assert_eq!(counter(8), c(4, 4, false, false));
        assert_eq!(counter(5), c(2, 3, true, false));
        assert_eq!(counter(32), c(16, 16, false, false));
        assert_eq!(counter(33), c(16, 17, true, false));
        // 128 is 64 + 64, and 64 does not fit six bits: it is stored as 0.
        assert_eq!(counter(128), c(0, 0, false, false));
    }

    /// The two quoted tables at the one point a second, independent tool
    /// pins them: nextpnr-xilinx writes `LKTABLE = 0xB5BE8FA401` and
    /// `TABLE = 0x3B4` for every PLL, and those are f4pga's entries for a
    /// multiplier of 8 under `OPTIMIZED`. Would catch an off-by-one in the
    /// indexing (multiplier 7 or 9 differ in both); would not catch a
    /// wrong entry anywhere else in either table.
    #[test]
    fn the_quoted_tables_agree_with_nextpnr_at_a_multiplier_of_eight() {
        let settings = PllSettings {
            divclk_divide: 1,
            clkfbout_mult: 8,
            clkout0_divide: 32,
            bandwidth: "OPTIMIZED".to_owned(),
            startup_wait: false,
            clkout0_used: true,
            clkfbout_used: true,
            invert_rst: true,
            invert_pwrdwn: true,
        };
        let r = pll_registers(&settings);
        assert_eq!(r["LKTABLE"], 0xB5_BE8F_A401);
        assert_eq!(r["TABLE"], 0x3B4);
        assert_eq!(r["CLKFBOUT_CLKOUT1_HIGH_TIME"], 4);
        assert_eq!(r["CLKOUT0_CLKOUT1_LOW_TIME"], 16);
        assert_eq!(r["CLKOUT1_CLKOUT2_NO_COUNT"], 1);
        assert_eq!(r["CLKOUT1_CLKOUT1_OUTPUT_ENABLE"], 0);
        assert_eq!(r["DIVCLK_DIVCLK_NO_COUNT"], 1);
        // The neighbours really are different, so the test has teeth.
        assert_ne!(LOCK_TABLE[6], LOCK_TABLE[7]);
        assert_ne!(LOCK_TABLE[8], LOCK_TABLE[7]);
        assert_ne!(FILTER_HIGH[6], FILTER_HIGH[7]);
        assert_ne!(FILTER_HIGH[8], FILTER_HIGH[7]);
    }

    /// The `{LockRefDly, LockFBDly, LockCnt, LockSatHigh, UnlockCnt}`
    /// packing: from multiplier 11 on, both delays saturate at 31 and the
    /// count falls, which is the shape XAPP888's table has. Would catch the
    /// table having been extracted in reverse order.
    #[test]
    fn the_lock_table_runs_from_short_delays_to_saturated_ones() {
        let field = |v: u64, shift: u32, width: u32| (v >> shift) & ((1 << width) - 1);
        assert_eq!(field(LOCK_TABLE[0], 35, 5), 6);
        assert_eq!(field(LOCK_TABLE[9], 35, 5), 28);
        assert_eq!(field(LOCK_TABLE[10], 35, 5), 31);
        assert_eq!(field(LOCK_TABLE[63], 30, 5), 31);
        assert_eq!(field(LOCK_TABLE[0], 20, 10), 1000);
        assert_eq!(field(LOCK_TABLE[63], 20, 10), 250);
        for entry in LOCK_TABLE {
            assert_eq!(field(entry, 10, 10), 1001, "LockSatHigh is constant");
            assert_eq!(field(entry, 0, 10), 1, "UnlockCnt is constant");
        }
    }
}
