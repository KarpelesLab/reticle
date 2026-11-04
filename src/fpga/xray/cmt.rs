//! The clock management tile: a PLL and an MMCM as bels, and the registers
//! that configure them.
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
//! | high time, low time, edge, no-count of every counter | **computed** here, from XAPP888's divider arithmetic; **checked** on Vivado's sweep, below |
//! | `LKTABLE`, the lock detector's settings | **quoted**: XAPP888's table, as transcribed into f4pga-arch-defs' `xilinx/xc7/techmap/cells_map.v` (`pll_lktable_lookup`), extracted from that file by a script rather than retyped; **checked** below |
//! | `TABLE`, the loop filter's settings | **quoted** the same way (`pll_table_lookup`); f4pga's `OPTIMIZED` table is identical to its `HIGH` one, and so is this, and so is Vivado's; **checked** below |
//! | `FILTREG1_RESERVED = 0x008`, `LOCKREG3_RESERVED = 1`, every other reserved field and `POWER_REG` zero | **quoted**: f4pga-arch-defs and nextpnr-xilinx both write exactly these, and neither says why |
//! | `IN_USE`, `COMPENSATION.Z_ZHOLD_OR_CLKIN_BUF` | **quoted**: the two features f4pga's `plle2_adv.pb_type.xml` writes for every PLL and nextpnr-xilinx for `COMPENSATION = INTERNAL` |
//! | `ZINV_RST` / `ZINV_PWRDWN` set for a pin tied low | **derived** below, from three sources that agree once the database's naming is read correctly |
//!
//! Two independent checks tie the quoted tables to something: for a
//! multiplier of 8, the entries this file holds are `0xB5BE8FA401` and
//! `0x3B4`, which are the very constants nextpnr-xilinx hard-codes for
//! every PLL (`write_pll` in `xilinx/fasm.cc`); and the register layout
//! the field names imply is the one XAPP888 states. Until 2026-10-16
//! **nothing here had been compared with a Vivado bitstream holding a
//! PLL** — the four harness designs in `artix7/harness/` have none. NewAE's
//! ChipWhisperer-Husky image has one `PLLE2_ADV` and three `MMCME2_ADV`s,
//! and `tests/fpga_xray_cmt.rs` finds every register field of the same four
//! instances built here equal to Vivado's.
//!
//! # The MMCM
//!
//! An `MMCME2_ADV` sits on the `LOWER_B` quarter and has the same register
//! layout, with its own filter table and a power register;
//! [`mmcm_registers`] says where each difference came from. What is refused by
//! name is spread spectrum, a cascade with another output's phase, and whatever
//! Vivado refuses ([`check_manager`]).
//!
//! # What Vivado wrote for each parameter
//!
//! **Measured**, and the fixture says each run: `testdata/fpga/vivado/clocks/`,
//! `registers.txt`, from `tools/vivado-parity/cmt_sweep.py`, one parameter at a
//! time on a routed `MMCME2_ADV` and a routed `PLLE2_ADV`. What the table above
//! calls quoted has since been held to it: `LOCK_TABLE`, `FILTER_LOW`, `FILTER_HIGH`
//! and the MMCM's two are right at every multiplier under each `BANDWIDTH`, and
//! the rule of [`counter`], of the phases and of the fractional counters
//! ([`shaped_counter`], [`fractional_counter`]) on every run there.
//!
//! - A duty cycle is rounded to half a VCO cycle, up on a tie, and an odd half sets
//!   `EDGE`. A high time of 0 becomes 1 and one of the whole division one less.
//! - A phase is rounded to an eighth of a VCO cycle: its fraction is `PHASE_MUX`
//!   and its cycles `DELAY_TIME`, and a negative one is that plus 360.
//! - **A fractional counter's second half is in another output's registers**: the
//!   falling edge of `CLKOUT0_DIVIDE_F` in `CLKOUT5`'s `FRACTIONAL` fields, and of
//!   `CLKFBOUT_MULT_F` in `CLKOUT6`'s. So a `PHASE_MUX` on `CLKOUT0` or `CLKFBOUT`
//!   is there too, fractional or not.
//! - A table is indexed by the multiplier's whole part.
//! - `POWER_REG` is `0x0100`, with `0x9800` for a fractional counter or a phase
//!   that is an odd number of eighths or of quarters of a VCO cycle (a half needs
//!   nothing), `0x0001` for a fine phase shift, and `0x0099` if `PSCLK` is a signal.
//! - `CLKOUT4_CASCADE` is `CLKOUT4`'s `MX` of 2.
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
use crate::fpga::arch::{ConfigBit, ConfigEntry, RoutingGraph};
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
    match tile_type {
        "CMT_TOP_L_UPPER_T" | "CMT_TOP_R_UPPER_T" => vec![pll_bel()],
        "CMT_TOP_L_LOWER_B" | "CMT_TOP_R_LOWER_B" => vec![mmcm_bel()],
        _ => Vec::new(),
    }
}

/// The DRP's indexed pins, as roles: the same on a PLL and an MMCM. Written
/// out because a [`BelPin`]'s role is a `&'static str`.
const DADDR: [&str; 7] = [
    "daddr0", "daddr1", "daddr2", "daddr3", "daddr4", "daddr5", "daddr6",
];
const DI: [&str; 16] = [
    "di0", "di1", "di2", "di3", "di4", "di5", "di6", "di7", "di8", "di9", "di10", "di11", "di12",
    "di13", "di14", "di15",
];
const DO: [&str; 16] = [
    "do0", "do1", "do2", "do3", "do4", "do5", "do6", "do7", "do8", "do9", "do10", "do11", "do12",
    "do13", "do14", "do15",
];
const OUT: [&str; 7] = ["out0", "out1", "out2", "out3", "out4", "out5", "out6"];
const OUTB: [&str; 4] = ["out0b", "out1b", "out2b", "out3b"];

/// The DRP's pins, as roles and pin names.
fn drp_pins() -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = vec![
        ("dclk", "DCLK".to_owned()),
        ("den", "DEN".to_owned()),
        ("dwe", "DWE".to_owned()),
        ("drdy", "DRDY".to_owned()),
    ];
    out.extend(
        DADDR
            .iter()
            .enumerate()
            .map(|(i, r)| (*r, format!("DADDR{i}"))),
    );
    out.extend(DI.iter().enumerate().map(|(i, r)| (*r, format!("DI{i}"))));
    out.extend(DO.iter().enumerate().map(|(i, r)| (*r, format!("DO{i}"))));
    out
}

/// A bel from roles and the pin names its wires are spelt with.
fn manager_bel(
    name: &str,
    kind: &'static str,
    wire: &dyn Fn(&str) -> String,
    pins: Vec<(&'static str, String)>,
) -> ExtraBel {
    ExtraBel {
        name: name.to_owned(),
        kind,
        pins: pins
            .into_iter()
            .map(|(role, pin)| BelPin {
                role,
                wire: wire(&pin),
            })
            .collect(),
        cells: Vec::new(),
    }
}

/// The PLL of an `UPPER_T` quarter, with every pin a `PLLE2_ADV` has.
fn pll_bel() -> ExtraBel {
    let mut pins: Vec<(&'static str, String)> = [
        ("ref", "CLKIN1"),
        ("clkin2", "CLKIN2"),
        ("clkinsel", "CLKINSEL"),
        ("fb", "CLKFBIN"),
        ("out", "CLKOUT0"),
        ("fbout", "CLKFBOUT"),
        ("lock", "LOCKED"),
        ("rst", "RST"),
        ("pwrdwn", "PWRDWN"),
    ]
    .iter()
    .map(|(r, p)| (*r, (*p).to_owned()))
    .collect();
    pins.extend((1..6).map(|i| (OUT[i], format!("CLKOUT{i}"))));
    pins.extend(drp_pins());
    manager_bel(
        PLL_BEL,
        "pll",
        &|pin| format!("CMT_TOP_R_UPPER_T_PLLE2_{pin}"),
        pins,
    )
}

/// The MMCM of a `LOWER_B` quarter. Its wires are `CMT_LR_LOWER_B_MMCM_*`
/// in both orientations (`ppips_cmt_top_{l,r}_lower_b.db`).
fn mmcm_bel() -> ExtraBel {
    let mut pins: Vec<(&'static str, String)> = [
        ("ref", "CLKIN1"),
        ("clkin2", "CLKIN2"),
        ("clkinsel", "CLKINSEL"),
        ("fb", "CLKFBIN"),
        ("fbout", "CLKFBOUT"),
        ("fboutb", "CLKFBOUTB"),
        ("lock", "LOCKED"),
        ("clkinstopped", "CLKINSTOPPED"),
        ("clkfbstopped", "CLKFBSTOPPED"),
        ("rst", "RST"),
        ("pwrdwn", "PWRDWN"),
        ("psclk", "PSCLK"),
        ("psen", "PSEN"),
        ("psincdec", "PSINCDEC"),
        ("psdone", "PSDONE"),
    ]
    .iter()
    .map(|(r, p)| (*r, (*p).to_owned()))
    .collect();
    pins.extend((0..7).map(|i| (OUT[i], format!("CLKOUT{i}"))));
    pins.extend((0..4).map(|i| (OUTB[i], format!("CLKOUT{i}B"))));
    pins.extend(drp_pins());
    manager_bel(
        MMCM_BEL,
        "mmcm",
        &|pin| format!("CMT_LR_LOWER_B_MMCM_{pin}"),
        pins,
    )
}

/// The name the feature reader gives a CMT's MMCM bel.
const MMCM_BEL: &str = "MMCME2_ADV";

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

/// A counter beyond its division: its phase and its duty cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterShape {
    /// `<name>_DUTY_CYCLE`, in millionths.
    pub duty: u64,
    /// `<name>_PHASE`, in thousandths of a degree.
    pub phase: i64,
}

impl Default for CounterShape {
    fn default() -> Self {
        CounterShape {
            duty: 500_000,
            phase: 0,
        }
    }
}

/// A division of eighths of a cycle rounded to half of one: what a duty cycle or
/// a phase is held at, a half up. `scale` is the unit of `value`.
fn round_to(value: u128, scale: u128, steps: u128) -> u64 {
    u64::try_from((2 * value * steps + scale) / (2 * scale)).unwrap_or(0)
}

/// What [`counter`] gives for a duty cycle other than a half.
///
/// **Computed**, from XAPP888's `mmcm_pll_divider`, and held to Vivado's sweep.
pub fn shaped_counter(divide: u64, duty_millionths: u64) -> Counter {
    if divide <= 1 {
        return counter(divide);
    }
    let halves = round_to(u128::from(divide * duty_millionths), 1_000_000, 2);
    let (mut high, mut edge) = (halves / 2, halves % 2 == 1);
    if high == 0 {
        high = 1;
        edge = false;
    }
    if high >= divide {
        high = divide - 1;
        edge = true;
    }
    Counter {
        high: high & 0x3f,
        low: (divide - high) & 0x3f,
        edge,
        no_count: false,
    }
}

/// A phase as `(cycles, eighths)` of a counter of `eighths_per_cycle` eighths of
/// a VCO cycle, which is `8 * divide`: `DELAY_TIME` and `PHASE_MUX`.
fn phase_of(eighths_per_cycle: u64, phase: i64) -> (u64, u64) {
    let phase = u128::try_from(phase.rem_euclid(360_000)).unwrap_or(0);
    let eighths = round_to(phase * u128::from(eighths_per_cycle), 360_000, 1);
    (eighths / 8, eighths % 8)
}

/// A real number to the nearest integer, or `None` when it is not one an `i64`
/// holds: the one float-to-integer conversion of this module, with its argument
/// checked.
#[allow(clippy::cast_possible_truncation)]
fn to_units(value: f64) -> Option<i64> {
    let rounded = value.round();
    (rounded.is_finite() && rounded.abs() < 9.0e15).then_some(rounded as i64)
}

/// How a clock manager's feedback is compensated, once what Vivado makes of its
/// `COMPENSATION` and its wiring is known: see [`check_manager`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Compensation {
    /// `INTERNAL`, `BUF_IN` or `EXTERNAL`, all one to these registers.
    #[default]
    NotZhold,
    /// `ZHOLD`, which sets its own bits and takes the feedback's way in by
    /// another code.
    Zhold,
}

/// What an `MMCME2_ADV` or a `PLLE2_ADV` is asked beyond a plain division.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ManagerShapes {
    /// `CLKFBOUT`'s.
    pub feedback: CounterShape,
    /// `CLKOUT<n>`'s.
    pub outputs: [CounterShape; 7],
    /// `CLKFBOUT_MULT_F`'s fraction, in eighths; an `MMCME2_ADV`'s alone.
    pub feedback_eighths: u64,
    /// `CLKOUT0_DIVIDE_F`'s fraction, in eighths.
    pub clkout0_eighths: u64,
    /// `CLKOUT4_CASCADE`.
    pub cascade: bool,
    /// Whether `PSCLK` is a signal: `PSEN` or `PSINCDEC` alone change nothing here.
    pub phase_shift_routed: bool,
    /// See [`Compensation`].
    pub compensation: Compensation,
}

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
    /// `CLKOUT1_DIVIDE` to `CLKOUT5_DIVIDE` and whether each output drives
    /// anything: a `PLLE2_ADV`'s other five outputs. A `PLLE2_BASE` as
    /// mapping makes one leaves them at a division of one, unused.
    pub more_outputs: [(u64, bool); 5],
    /// Whether `CLKINSEL` must be inverted: tied low and left unrouted, it
    /// would read the one that selects `CLKIN1`.
    pub invert_clkinsel: bool,
    /// Phases, duty cycles and compensation.
    pub shapes: ManagerShapes,
}

/// What an `MMCME2_ADV` is asked to do, in its own parameter names. A fractional
/// `CLKFBOUT_MULT_F` or `CLKOUT0_DIVIDE_F` is its whole part here and its eighths
/// in [`ManagerShapes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MmcmSettings {
    /// `DIVCLK_DIVIDE`, 1..=106.
    pub divclk_divide: u64,
    /// `CLKFBOUT_MULT_F`, 2..=64.
    pub clkfbout_mult: u64,
    /// `CLKFBOUT_USE_FINE_PS`.
    pub clkfbout_fine_ps: bool,
    /// Whether `CLKFBOUT` or `CLKFBOUTB` drives anything.
    pub clkfbout_used: bool,
    /// `CLKOUT0_DIVIDE_F` .. `CLKOUT6_DIVIDE`, whether the output (or its
    /// inverse) drives anything, and `CLKOUT<n>_USE_FINE_PS`.
    pub outputs: [(u64, bool, bool); 7],
    /// `BANDWIDTH`: `LOW`, `HIGH` or `OPTIMIZED`.
    pub bandwidth: String,
    /// `STARTUP_WAIT = "TRUE"`.
    pub startup_wait: bool,
    /// Which control pins read the wrong value unrouted and must be
    /// inverted: `RST`, `PWRDWN`, `PSEN`, `PSINCDEC`, as for the PLL.
    pub invert: [bool; 4],
    /// Whether `CLKINSEL` must be inverted.
    pub invert_clkinsel: bool,
    /// Phases, duty cycles, fractions and compensation.
    pub shapes: ManagerShapes,
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
                "`{instance}` is a `{primitive}` on a clock management site; this flow \
                 configures `{PLL_PRIMITIVE}` and `PLLE2_ADV` on a PLL and `MMCME2_ADV` on an \
                 MMCM, and another block's registers would configure nothing"
            ),
            ClockManagerError::BadParameter {
                instance,
                name,
                message,
            } => write!(f, "`{instance}`: `{name}` {message}"),
            ClockManagerError::MissingField { field } => write!(
                f,
                "the chip database has no `{field}` feature for a clock manager; the \
                 database is not the one this flow was written against"
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
    // the input mux the router sets. Vivado writes the same for `BUF_IN` and
    // `EXTERNAL`, and these three in its place for `ZHOLD`.
    let zhold = settings.shapes.compensation == Compensation::Zhold;
    put("ZHOLD_CLKFBIN".to_owned(), u64::from(zhold));
    put(
        "COMPENSATION.Z_ZHOLD_OR_CLKIN_BUF".to_owned(),
        u64::from(!zhold),
    );
    put(
        "COMPENSATION.ZHOLD_NO_CLKIN_BUF".to_owned(),
        u64::from(zhold),
    );
    put(
        "COMPENSATION.ZHOLD_NO_CLKIN_BUF_NO_TOP".to_owned(),
        u64::from(zhold),
    );
    put("COMP.ZHOLD_NO_CLKIN_BUF_TOP".to_owned(), 0);
    put("STARTUP_WAIT".to_owned(), u64::from(settings.startup_wait));
    put("ZINV_RST".to_owned(), u64::from(settings.invert_rst));
    put("ZINV_PWRDWN".to_owned(), u64::from(settings.invert_pwrdwn));
    put(
        "INV_CLKINSEL".to_owned(),
        u64::from(settings.invert_clkinsel),
    );

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
    let more = settings.more_outputs;
    let shapes = &settings.shapes;
    for (name, divide, used, shape) in [
        (
            "CLKFBOUT",
            settings.clkfbout_mult,
            settings.clkfbout_used,
            shapes.feedback,
        ),
        (
            "CLKOUT0",
            settings.clkout0_divide,
            settings.clkout0_used,
            shapes.outputs[0],
        ),
        ("CLKOUT1", more[0].0, more[0].1, shapes.outputs[1]),
        ("CLKOUT2", more[1].0, more[1].1, shapes.outputs[2]),
        ("CLKOUT3", more[2].0, more[2].1, shapes.outputs[3]),
        ("CLKOUT4", more[3].0, more[3].1, shapes.outputs[4]),
        ("CLKOUT5", more[4].0, more[4].1, shapes.outputs[5]),
    ] {
        divider_fields(&mut put, name, divide, used, 0, false, shape);
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

/// The fields of one divider of a PLL or MMCM, written into `put`:
/// XAPP888's `ClkReg1` (high time, low time, output enable, phase mux) and
/// `ClkReg2` (delay, edge, no-count, `MX`, and the fractional fields, all
/// zero for an integer division with no phase).
///
/// An MMCM's `CLKOUT5` and `CLKOUT6` spell their `ClkReg2` differently —
/// prjxray calls it `CLKOUT2_FRACTIONAL_*`, because on those two it holds
/// the fractional phase of `CLKOUT0` and `CLKFBOUT` — and `fractional`
/// selects that spelling.
fn divider_fields(
    put: &mut dyn FnMut(String, u64),
    name: &str,
    divide: u64,
    used: bool,
    mx: u64,
    fractional: bool,
    shape: CounterShape,
) {
    let c = shaped_counter(divide, shape.duty);
    let (delay, mux) = phase_of(8 * divide, shape.phase);
    put(format!("{name}_CLKOUT1_HIGH_TIME"), c.high);
    put(format!("{name}_CLKOUT1_LOW_TIME"), c.low);
    put(format!("{name}_CLKOUT1_OUTPUT_ENABLE"), u64::from(used));
    put(format!("{name}_CLKOUT1_PHASE_MUX"), mux);
    if fractional {
        put(
            format!("{name}_CLKOUT2_FRACTIONAL_DELAY_TIME"),
            delay & 0x3f,
        );
        put(format!("{name}_CLKOUT2_FRACTIONAL_EDGE"), u64::from(c.edge));
        put(
            format!("{name}_CLKOUT2_FRACTIONAL_NO_COUNT"),
            u64::from(c.no_count),
        );
        put(format!("{name}_CLKOUT2_FRACTIONAL_MX"), mx);
        put(format!("{name}_CLKOUT2_FRACTIONAL_FRAC_WF_F"), 0);
        put(format!("{name}_CLKOUT2_FRACTIONAL_PHASE_MUX_F"), 0);
        put(format!("{name}_CLKOUT2_FRACTIONAL_RESERVED"), 0);
    } else {
        put(format!("{name}_CLKOUT2_DELAY_TIME"), delay & 0x3f);
        put(format!("{name}_CLKOUT2_EDGE"), u64::from(c.edge));
        put(format!("{name}_CLKOUT2_NO_COUNT"), u64::from(c.no_count));
        put(format!("{name}_CLKOUT2_MX"), mx);
        put(format!("{name}_CLKOUT2_FRAC"), 0);
        put(format!("{name}_CLKOUT2_FRAC_EN"), 0);
        put(format!("{name}_CLKOUT2_FRAC_WF_R"), 0);
        put(format!("{name}_CLKOUT2_RESERVED"), 0);
    }
}

/// The fields of a fractional counter of `whole` and `eighths` eighths, as
/// XAPP888's `mmcm_frac_count_calc` works them out and Vivado's sweep holds them:
/// `name`'s own, and in `fall`'s `FRACTIONAL` fields its falling edge. `CLKOUT0`'s
/// falling edge is in `CLKOUT5`, and `CLKFBOUT`'s in `CLKOUT6`.
fn fractional_counter(
    put: &mut dyn FnMut(String, u64),
    name: &str,
    fall: &str,
    whole: u64,
    eighths: u64,
    used: bool,
    phase: i64,
) {
    let even = whole / 2;
    let odd = whole - 2 * even;
    let odd_and_frac = 8 * odd + eighths;
    let low = even - u64::from(odd_and_frac <= 9);
    let high = even - u64::from(odd_and_frac <= 8);
    let rise = (1..=8).contains(&odd_and_frac);
    let falling = (2..=9).contains(&odd_and_frac) || (eighths == 1 && whole == 2);
    let (delay, mux) = phase_of(8 * whole + eighths, phase);
    put(format!("{name}_CLKOUT1_HIGH_TIME"), high & 0x3f);
    put(format!("{name}_CLKOUT1_LOW_TIME"), low & 0x3f);
    put(format!("{name}_CLKOUT1_OUTPUT_ENABLE"), u64::from(used));
    put(format!("{name}_CLKOUT1_PHASE_MUX"), mux);
    put(format!("{name}_CLKOUT2_DELAY_TIME"), delay & 0x3f);
    put(format!("{name}_CLKOUT2_EDGE"), odd);
    put(format!("{name}_CLKOUT2_NO_COUNT"), 0);
    put(format!("{name}_CLKOUT2_FRAC"), eighths);
    put(format!("{name}_CLKOUT2_FRAC_EN"), 1);
    put(format!("{name}_CLKOUT2_FRAC_WF_R"), u64::from(rise));
    put(
        format!("{fall}_CLKOUT2_FRACTIONAL_PHASE_MUX_F"),
        (odd * 4 + eighths / 2 + mux) % 8,
    );
    put(
        format!("{fall}_CLKOUT2_FRACTIONAL_FRAC_WF_F"),
        u64::from(falling),
    );
}

/// An MMCM's loop filter `{CP, RES, LFHF}` for `BANDWIDTH = "LOW"`, by
/// multiplier minus one. **Quoted**: XAPP888's table as f4pga-arch-defs
/// transcribes it (`mmcm_table_lookup` in `xilinx/xc7/techmap/cells_map.v`),
/// extracted by script. It is not the PLL's.
const MMCM_FILTER_LOW: [u16; 64] = [
    0x0bc, 0x0bc, 0x0bc, 0x0bc, 0x09c, 0x0ac, 0x0b4, 0x08c, // 1..8
    0x094, 0x094, 0x0a4, 0x0b8, 0x0b8, 0x0b8, 0x0b8, 0x084, // 9..16
    0x084, 0x084, 0x098, 0x098, 0x098, 0x098, 0x098, 0x098, // 17..24
    0x098, 0x0a8, 0x0a8, 0x0a8, 0x0a8, 0x0a8, 0x0b0, 0x0b0, // 25..32
    0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, // 33..40
    0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x0b0, 0x088, // 41..48
    0x088, 0x088, 0x088, 0x088, 0x088, 0x088, 0x088, 0x088, // 49..56
    0x088, 0x088, 0x088, 0x088, 0x088, 0x088, 0x088, 0x088, // 57..64
];

/// The MMCM's filter for `HIGH` and `OPTIMIZED`, which f4pga's
/// transcription gives identical tables. **Quoted** the same way, and
/// **checked**: Vivado wrote `0x3AC` for a multiplier of 6 and `0x0D0` for
/// 32 into NewAE's Husky image, which are entries 6 and 32 here.
const MMCM_FILTER_HIGH: [u16; 64] = [
    0x0bc, 0x13c, 0x16c, 0x1dc, 0x35c, 0x3ac, 0x3b4, 0x3cc, // 1..8
    0x394, 0x3d4, 0x3e4, 0x344, 0x3e4, 0x3e4, 0x3e4, 0x3e4, // 9..16
    0x3d4, 0x3d4, 0x304, 0x304, 0x304, 0x170, 0x170, 0x170, // 17..24
    0x170, 0x0d0, 0x0d0, 0x0d0, 0x0d0, 0x0d0, 0x0d0, 0x0d0, // 25..32
    0x0d0, 0x0d0, 0x0d0, 0x0d0, 0x0d0, 0x0d0, 0x0d0, 0x0d0, // 33..40
    0x0d0, 0x0a0, 0x0a0, 0x0a0, 0x0a0, 0x0a0, 0x1c4, 0x1c4, // 41..48
    0x130, 0x130, 0x130, 0x130, 0x184, 0x184, 0x158, 0x158, // 49..56
    0x158, 0x090, 0x090, 0x090, 0x090, 0x128, 0x0f0, 0x0f0, // 57..64
];

/// Every register field of a configured MMCM, in the database's own names
/// (without `MMCME2_ADV.`), with its value.
///
/// The arithmetic is [`pll_registers`]'s; what differs, and where each
/// difference came from:
///
/// | Field | Value | Source |
/// |---|---|---|
/// | `TABLE` | `MMCM_FILTER_LOW` / `MMCM_FILTER_HIGH` in this module | quoted from f4pga, two entries checked against Vivado |
/// | `LKTABLE` | the PLL's table | f4pga's `mmcm_lktable_lookup` *is* `pll_lktable_lookup`; Vivado's 6 and 32 match |
/// | `POWER_REG` | see the module docs | **Vivado**, four MMCMs in NewAE's image, and the sweep. f4pga writes `0x9900` for a fine phase shift and `0x0100`: `0x9800` is a different bit of Vivado's, for a fractional counter or an odd phase, which no Husky disagrees with only because none uses one |
/// | `<out>_CLKOUT2_MX` | 1 on an output with `USE_FINE_PS`, else 0 | **Vivado**, the same four |
/// | `COMP.Z_ZHOLD` | set, for anything but `ZHOLD`; `COMP.ZHOLD` for that | **Vivado**, the same four, and the sweep |
/// | `ZINV_RST`, `ZINV_PWRDWN`, `ZINV_PSEN`, `ZINV_PSINCDEC` | set when the pin is tied low and left unrouted | the PLL's reading of `ZINV_RST`; **Vivado** set `ZINV_PSEN`/`ZINV_PSINCDEC` on the one MMCM whose phase-shift pins are unconnected and on none of the three that route them |
pub fn mmcm_registers(settings: &MmcmSettings) -> BTreeMap<String, u64> {
    let mut out: BTreeMap<String, u64> = BTreeMap::new();
    let mut put = |name: String, value: u64| {
        out.insert(name, value);
    };
    let shapes = &settings.shapes;
    let zhold = shapes.compensation == Compensation::Zhold;
    put("IN_USE".to_owned(), 1);
    put("ZHOLD_CLKFBIN".to_owned(), u64::from(zhold));
    put("COMP.Z_ZHOLD".to_owned(), u64::from(!zhold));
    put("COMP.ZHOLD".to_owned(), u64::from(zhold));
    put("STARTUP_WAIT".to_owned(), u64::from(settings.startup_wait));
    put("SS_EN".to_owned(), 0);
    for (name, invert) in ["ZINV_RST", "ZINV_PWRDWN", "ZINV_PSEN", "ZINV_PSINCDEC"]
        .iter()
        .zip(settings.invert)
    {
        put((*name).to_owned(), u64::from(invert));
    }
    put(
        "INV_CLKINSEL".to_owned(),
        u64::from(settings.invert_clkinsel),
    );

    let divclk = counter(settings.divclk_divide);
    put("DIVCLK_DIVCLK_HIGH_TIME".to_owned(), divclk.high);
    put("DIVCLK_DIVCLK_LOW_TIME".to_owned(), divclk.low);
    put("DIVCLK_DIVCLK_EDGE".to_owned(), u64::from(divclk.edge));
    put(
        "DIVCLK_DIVCLK_NO_COUNT".to_owned(),
        u64::from(divclk.no_count),
    );
    put("DIVCLK_DIVCLK_RESERVED".to_owned(), 0);

    divider_fields(
        &mut put,
        "CLKFBOUT",
        settings.clkfbout_mult,
        settings.clkfbout_used,
        u64::from(settings.clkfbout_fine_ps),
        false,
        shapes.feedback,
    );
    for (index, (divide, used, fine)) in settings.outputs.iter().enumerate() {
        let mx = if index == 4 && shapes.cascade {
            2
        } else {
            u64::from(*fine)
        };
        divider_fields(
            &mut put,
            &format!("CLKOUT{index}"),
            *divide,
            *used,
            mx,
            index >= 5,
            shapes.outputs[index],
        );
    }
    // The other half of a counter on two registers, once the counters each
    // has written are there to be written over: see the module docs.
    let eighth_of = |divide: u64, shape: CounterShape| phase_of(8 * divide, shape.phase).1;
    put(
        "CLKOUT5_CLKOUT2_FRACTIONAL_PHASE_MUX_F".to_owned(),
        eighth_of(settings.outputs[0].0, shapes.outputs[0]),
    );
    put(
        "CLKOUT6_CLKOUT2_FRACTIONAL_PHASE_MUX_F".to_owned(),
        eighth_of(settings.clkfbout_mult, shapes.feedback),
    );
    if shapes.clkout0_eighths != 0 {
        fractional_counter(
            &mut put,
            "CLKOUT0",
            "CLKOUT5",
            settings.outputs[0].0,
            shapes.clkout0_eighths,
            settings.outputs[0].1,
            shapes.outputs[0].phase,
        );
    }
    if shapes.feedback_eighths != 0 {
        fractional_counter(
            &mut put,
            "CLKFBOUT",
            "CLKOUT6",
            settings.clkfbout_mult,
            shapes.feedback_eighths,
            settings.clkfbout_used,
            shapes.feedback.phase,
        );
    }

    let index = usize::try_from(settings.clkfbout_mult.clamp(1, 64) - 1).unwrap_or(0);
    put("LKTABLE".to_owned(), LOCK_TABLE[index]);
    let filter = if settings.bandwidth == "LOW" {
        MMCM_FILTER_LOW[index]
    } else {
        MMCM_FILTER_HIGH[index]
    };
    put("TABLE".to_owned(), u64::from(filter));
    put("FILTREG1_RESERVED".to_owned(), 0x008);
    put("FILTREG2_RESERVED".to_owned(), 0);
    put("LOCKREG1_RESERVED".to_owned(), 0);
    put("LOCKREG2_RESERVED".to_owned(), 0);
    put("LOCKREG3_RESERVED".to_owned(), 1);
    let fine = settings.clkfbout_fine_ps || settings.outputs.iter().any(|o| o.2);
    let quarter =
        |divide: u64, shape: CounterShape| !phase_of(8 * divide, shape.phase).1.is_multiple_of(4);
    let split = shapes.clkout0_eighths != 0
        || shapes.feedback_eighths != 0
        || quarter(settings.clkfbout_mult, shapes.feedback)
        || settings
            .outputs
            .iter()
            .zip(&shapes.outputs)
            .any(|(o, shape)| quarter(o.0, *shape));
    let fine_bits = match (fine, shapes.phase_shift_routed) {
        (false, _) => 0,
        (true, false) => 0x0001,
        (true, true) => 0x0099,
    };
    put(
        "POWER_REG_POWER_REG_POWER_REG".to_owned(),
        0x0100 | if split { 0x9800 } else { 0 } | fine_bits,
    );
    out
}

/// What Vivado refuses of a clock manager's frequencies and phases, by the rules
/// its own DRC gave on `registers.txt`, as `(parameter, message)`.
///
/// `period` is `CLKIN1_PERIOD` in thousandths of a nanosecond, zero when none is
/// given, which is when nothing here can be worked out of the frequencies. The
/// windows are `.dev`'s, DS181's at grade 1, **quoted**, and what the run confirms
/// is that a value outside them is refused and each inside taken.
///
/// # Errors
///
/// The parameter at fault and what is wrong with it.
pub fn check_manager(
    mmcm: bool,
    period: u64,
    divclk: u64,
    mult_eighths: u64,
    phases: &[(&str, u64, i64, bool)],
) -> Result<(), (String, String)> {
    let (input, vco) = if mmcm {
        ((10.0, 800.0), (600.0, 1200.0))
    } else {
        ((19.0, 800.0), (800.0, 1600.0))
    };
    let (check, phase_check, kind) = if mmcm {
        ("PDRC-29, PDRC-34", "AVAL-139", "an MMCM")
    } else {
        ("PDRC-38, PDRC-43", "AVAL-167", "a PLL")
    };
    if period != 0 {
        let megahertz = 1.0e6 / f64::from(u32::try_from(period).unwrap_or(u32::MAX));
        let within =
            |value: f64, (lo, hi): (f64, f64)| value >= lo - 0.0005 && value <= hi + 0.0005;
        let scale = f64::from(u32::try_from(mult_eighths).unwrap_or(u32::MAX)) / 8.0;
        let divided = megahertz / f64::from(u32::try_from(divclk).unwrap_or(1));
        // No rule of its says anything of the phase detector, in the runs here:
        // 600 MHz of it went through, and the window is DS181's and not its.
        if !within(megahertz, input) {
            return Err((
                "CLKIN1_PERIOD".to_owned(),
                format!("makes {megahertz:.3} MHz, outside {kind}'s input range ({check})"),
            ));
        }
        if !within(divided * scale, vco) {
            return Err((
                "CLKFBOUT_MULT".to_owned(),
                format!(
                    "makes {:.3} MHz at the VCO, outside {kind}'s range ({check})",
                    divided * scale
                ),
            ));
        }
    }
    for (name, divide_eighths, phase, fine) in phases {
        if *fine || *phase == 0 {
            continue;
        }
        // A phase is a multiple of 45 / divide, to what Vivado prints of it.
        let steps = i128::from(phase.rem_euclid(360_000)) * i128::from(*divide_eighths);
        let unit = 45_000i128 * 8;
        let off = (steps + unit / 2).rem_euclid(unit) - unit / 2;
        // Three decimal places of a degree, over a step of up to 128 divisions.
        let allowed = i128::from(*divide_eighths) * 8;
        if off.abs() > allowed {
            return Err((
                format!("{name}_PHASE"),
                format!(
                    "is {:.3}, not a multiple of 45 divided by its counter's division ({phase_check})",
                    *phase as f64 / 1000.0
                ),
            ));
        }
    }
    Ok(())
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

/// The pin of a role on an instance.
fn pins_of_role<'a>(
    instance: &crate::fpga::place::Instance,
    netlist: &'a Netlist,
    role: &str,
) -> Option<&'a crate::fpga::place::NetPin> {
    instance
        .pins
        .iter()
        .map(|p| &netlist.pins[*p])
        .find(|p| p.role == role)
}

/// The bits a `ZHOLD` feedback takes its way in by, in place of the pip's, and
/// the feature that names them, by the tile type: **measured**, on `mmcm_fb_bufg`
/// and `pll_fb_bufg` and on the right of the die. This feedback's mux has one
/// code for `CLK_IN3_HCLK` that Project X-Ray's fuzzers never saw, since they never
/// routed a `ZHOLD`: on the MMCM `28_1012 28_1017`, where every other code on that
/// input has `29_979`; on the PLL `29_43`, where they have `28_76`.
pub(super) fn zhold_features(tile_type: &str) -> Vec<super::Feature> {
    let feature = |name: &str, ones: Vec<ConfigBit>, zeros: Vec<ConfigBit>| super::Feature {
        name: name.to_owned(),
        ones,
        zeros,
    };
    match tile_type {
        "CMT_TOP_L_LOWER_B" | "CMT_TOP_R_LOWER_B" => vec![feature(
            ZHOLD_MMCM,
            vec![ConfigBit::new(28, 1012), ConfigBit::new(28, 1017)],
            vec![ConfigBit::new(29, 979)],
        )],
        "CMT_TOP_L_UPPER_T" | "CMT_TOP_R_UPPER_T" => vec![feature(
            ZHOLD_PLL,
            vec![ConfigBit::new(29, 43)],
            vec![ConfigBit::new(28, 76)],
        )],
        _ => Vec::new(),
    }
}

/// See [`zhold_features`].
pub(super) const ZHOLD_MMCM: &str = "MMCME2_ADV.ZHOLD_CLKFBIN";
/// See [`zhold_features`].
pub(super) const ZHOLD_PLL: &str = "PLLE2_ADV.ZHOLD_CLKFBIN";

/// The arc a `ZHOLD` feedback's code selects: see [`zhold_features`].
pub(super) fn zhold_arc(tile_type: &str) -> Option<&'static str> {
    match tile_type {
        "CMT_TOP_L_LOWER_B" => Some("CMT_LR_LOWER_B_MMCM_CLKFBIN.CMT_L_LOWER_B_CLK_IN3_HCLK"),
        "CMT_TOP_R_LOWER_B" => Some("CMT_LR_LOWER_B_MMCM_CLKFBIN.CMT_R_LOWER_B_CLK_IN3_HCLK"),
        "CMT_TOP_L_UPPER_T" => Some("CMT_TOP_R_UPPER_T_PLLE2_CLKFBIN.CMT_TOP_L_UPPER_T_CLKFBIN"),
        "CMT_TOP_R_UPPER_T" => Some("CMT_TOP_R_UPPER_T_PLLE2_CLKFBIN.CMT_TOP_R_UPPER_T_CLKFBIN"),
        _ => None,
    }
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
    ///
    /// `clock_periods` is each `create_clock` by its net, in nanoseconds: Vivado
    /// checks a manager's frequencies against the clock it times its input by,
    /// over its `CLKIN1_PERIOD` (`mmcm_period_clocked`, `pll_period_clocked`), and
    /// this checks them only for an input a pad's clock reaches through buffers.
    #[allow(clippy::too_many_arguments)]
    pub fn configure_clock_managers(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
        clock_periods: &BTreeMap<String, f64>,
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
            let mmcm = match (bel.name.as_str(), instance.primitive.as_str()) {
                (PLL_BEL, PLL_PRIMITIVE | "PLLE2_ADV") => false,
                (MMCM_BEL, "MMCME2_ADV") => true,
                (PLL_BEL | MMCM_BEL, _) => {
                    return Err(ClockManagerError::Unsupported {
                        instance: instance.name.clone(),
                        primitive: instance.primitive.clone(),
                    });
                }
                _ => continue,
            };
            let params = m.cells.get(instance.cell).map(|cell| &cell.params);
            let bad = |name: &str, message: String| ClockManagerError::BadParameter {
                instance: instance.name.clone(),
                name: name.to_owned(),
                message,
            };
            // An integer parameter, or a real one holding an integer: an
            // MMCM's `CLKFBOUT_MULT_F` arrives as `6.0`, which a black box
            // keeps as the text `6`.
            let integer = |name: &str, default: i64, lo: u64, hi: u64| {
                let value = match params.and_then(|p| p.get(name)) {
                    None => Some(default),
                    Some(v) => v.as_int().or_else(|| {
                        // `6`, or `6.0` written out: an integer either way.
                        let t = v.as_str()?.trim();
                        match t.split_once('.') {
                            Some((whole, frac)) if frac.chars().all(|c| c == '0') => {
                                whole.parse::<i64>().ok()
                            }
                            Some(_) => None,
                            None => t.parse::<i64>().ok(),
                        }
                    }),
                };
                let value = value.ok_or_else(|| {
                    bad(
                        name,
                        "is not an integer; a fractional divider is not one this flow \
                         configures"
                            .to_owned(),
                    )
                })?;
                let value = u64::try_from(value)
                    .map_err(|_| bad(name, format!("is {value}, below {lo}")))?;
                if value < lo || value > hi {
                    return Err(bad(name, format!("is {value}, outside {lo}..={hi}")));
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
            // A real parameter in units of `1 / scale`, to the nearest.
            let real = |name: &str, scale: f64| -> Option<i64> {
                params.and_then(|p| p.get(name)).and_then(|v| {
                    let value = v
                        .as_int()
                        .map(|i| i as f64)
                        .or_else(|| v.as_str().and_then(|t| t.trim().parse::<f64>().ok()))?;
                    to_units(value * scale)
                })
            };
            // A divider in eighths, whole or fractional: `CLKOUT0_DIVIDE_F` and
            // `CLKFBOUT_MULT_F`, which Vivado takes in steps of an eighth.
            let eighths = |name: &str, default: u64, lo: u64, hi: u64| {
                let Some(raw) = real(name, 8_000.0) else {
                    return Ok(default * 8);
                };
                let value = u64::try_from(raw)
                    .map_err(|_| bad(name, format!("is {}, below {lo}", raw as f64 / 8_000.0)))?;
                if value % 1_000 != 0 {
                    return Err(bad(
                        name,
                        format!(
                            "is {}, which is not in eighths, the steps Vivado takes",
                            raw as f64 / 8_000.0
                        ),
                    ));
                }
                let value = value / 1_000;
                if value < lo * 8 || value > hi * 8 {
                    return Err(bad(
                        name,
                        format!("is {}, outside {lo}..={hi}", value as f64 / 8.0),
                    ));
                }
                Ok(value)
            };
            let shape_of = |n: &str| -> Result<CounterShape, ClockManagerError> {
                let duty = real(&format!("{n}_DUTY_CYCLE"), 1_000_000.0).unwrap_or(500_000);
                Ok(CounterShape {
                    duty: u64::try_from(duty).map_err(|_| {
                        bad(&format!("{n}_DUTY_CYCLE"), "is not a duty cycle".to_owned())
                    })?,
                    phase: real(&format!("{n}_PHASE"), 1_000.0).unwrap_or(0),
                })
            };
            if text("SS_EN", "FALSE") == "TRUE" {
                return Err(bad(
                    "SS_EN",
                    "is TRUE: spread spectrum is built by Vivado and not by this flow yet"
                        .to_owned(),
                ));
            }
            // What the feedback goes through says more than `COMPENSATION` does:
            // Vivado takes a direct feedback as `INTERNAL`, whichever is asked
            // for. These are its answers, on `registers.txt`'s designs.
            let fb_driver = pins_of_role(instance, netlist, "fb")
                .and_then(|p| p.signal)
                .and_then(|s| netlist.signals[s].driver)
                .map(|d| &netlist.pins[d]);
            let direct = fb_driver.is_none_or(|d| {
                d.instance == index && matches!(d.role.as_str(), "fbout" | "fboutb")
            });
            let through_a_pad =
                fb_driver.is_some_and(|d| netlist.instances[d.instance].kind == "io");
            let asked = text("COMPENSATION", "ZHOLD");
            let compensation = match (asked.as_str(), direct, through_a_pad) {
                ("ZHOLD" | "INTERNAL", true, _) => Compensation::NotZhold,
                ("BUF_IN", true, _) => {
                    return Err(bad(
                        "COMPENSATION",
                        "is BUF_IN, with the feedback direct: Vivado wants it through a BUFG, \
                         BUFH or BUFR (REQP-1575, REQP-1684)"
                            .to_owned(),
                    ));
                }
                ("EXTERNAL", true, _) => {
                    return Err(bad(
                        "COMPENSATION",
                        "is EXTERNAL, with the feedback direct: Vivado wants it off the part and \
                         back through a pad (REQP-1575, REQP-1684)"
                            .to_owned(),
                    ));
                }
                ("INTERNAL", false, _) => {
                    return Err(bad(
                        "COMPENSATION",
                        "is INTERNAL, with the feedback not direct (REQP-1575, REQP-1684)"
                            .to_owned(),
                    ));
                }
                ("EXTERNAL", false, false) => {
                    return Err(bad(
                        "COMPENSATION",
                        "is EXTERNAL, with the feedback inside the part (Timing 38-290)".to_owned(),
                    ));
                }
                ("ZHOLD", false, false) => Compensation::Zhold,
                ("BUF_IN" | "EXTERNAL" | "ZHOLD", false, _) => Compensation::NotZhold,
                _ => {
                    return Err(bad(
                        "COMPENSATION",
                        format!("is `{asked}`, not ZHOLD, BUF_IN, EXTERNAL or INTERNAL"),
                    ));
                }
            };
            let bandwidth = text("BANDWIDTH", "OPTIMIZED");
            if !matches!(bandwidth.as_str(), "LOW" | "HIGH" | "OPTIMIZED") {
                return Err(bad(
                    "BANDWIDTH",
                    format!("is `{bandwidth}`, not LOW, HIGH or OPTIMIZED"),
                ));
            }

            let pins_of = |role: &str| -> Vec<&crate::fpga::place::NetPin> {
                instance
                    .pins
                    .iter()
                    .map(|p| &netlist.pins[*p])
                    .filter(|p| p.role == role)
                    .collect()
            };
            // An output in use has a reader: `PLLE2_BASE`'s are wires in the
            // library cell whether a design reads them or not.
            let used = |role: &str| {
                pins_of(role).iter().any(|p| {
                    p.signal
                        .is_some_and(|s| !p.output || !netlist.signals[s].sinks.is_empty())
                })
            };
            let inverted = |name: &str| {
                params
                    .and_then(|p| p.get(name))
                    .and_then(AttrValue::as_int)
                    .unwrap_or(0)
                    != 0
            };
            // A control pin with no signal is left unrouted and reads one;
            // it has to be inverted to mean a zero. One tied *high* reads
            // what it asks for. `IS_<pin>_INVERTED` turns either round.
            let invert = |role: &str, name: &str| {
                inverted(name)
                    != match pins_of(role).first() {
                        Some(p) if p.signal.is_some() => false,
                        Some(p) => p.constant != Some(Bit::One),
                        None => true,
                    }
            };
            // `CLKINSEL` is the other way round: left out, it is high and
            // selects `CLKIN1`, which is what a primitive without the pin
            // means.
            let invert_clkinsel = inverted("IS_CLKINSEL_INVERTED")
                != pins_of("clkinsel")
                    .first()
                    .is_some_and(|p| p.signal.is_none() && p.constant != Some(Bit::One));
            let startup_wait = text("STARTUP_WAIT", "FALSE") == "TRUE";
            // The two choices of input: what Vivado refuses of one that is a
            // signal.
            if used("clkinsel") && !used("rst") {
                return Err(bad(
                    "CLKINSEL",
                    "is a signal, with no signal on RST: Vivado wants one (REQP-164)".to_owned(),
                ));
            }
            // On an MMCM, the two inputs are the same kind of thing. A pad that
            // also clocks the design is on its buffer, where Vivado's is, since
            // that is what it reads: a pad and a buffer are two kinds.
            if mmcm && used("clkin2") {
                let source = |role: &str| {
                    let signal = pins_of_role(instance, netlist, role).and_then(|p| p.signal)?;
                    let buffered = netlist.signals[signal].sinks.iter().any(|sink| {
                        let pin = &netlist.pins[*sink];
                        netlist.instances[pin.instance].kind == "gb" && pin.role == "i"
                    });
                    let driver = netlist.signals[signal].driver?;
                    Some(if buffered {
                        "gb"
                    } else {
                        netlist.instances[netlist.pins[driver].instance]
                            .kind
                            .as_str()
                    })
                };
                if source("ref") != source("clkin2") {
                    return Err(bad(
                        "CLKIN2",
                        "comes from a different kind of source from CLKIN1: one compensation \
                         cannot suit both (REQP-119)"
                            .to_owned(),
                    ));
                }
            }
            // The input's clock, when a `create_clock` names it: see above. With
            // none, Vivado checks the attribute and this does not, since a
            // manager behind another manager or a multiplexer has a period only
            // Vivado's timer knows, and refusing on the attribute alone refused
            // the ChipWhisperer-Husky's glitch MMCM, which Vivado builds.
            let mut input = pins_of_role(instance, netlist, "ref").and_then(|p| p.signal);
            let mut period = 0u64;
            for _ in 0..4 {
                let Some(signal) = input else { break };
                if let Some(ns) = clock_periods.get(&netlist.signals[signal].name) {
                    period = u64::try_from(to_units(ns * 1_000.0).unwrap_or(0)).unwrap_or(0);
                    break;
                }
                input = netlist.signals[signal]
                    .driver
                    .map(|d| netlist.pins[d].instance)
                    .filter(|d| netlist.instances[*d].kind == "gb")
                    .and_then(|d| pins_of_role(&netlist.instances[d], netlist, "i"))
                    .and_then(|p| p.signal);
            }
            let mut shapes = ManagerShapes {
                compensation,
                ..ManagerShapes::default()
            };
            shapes.feedback = shape_of("CLKFBOUT")?;
            for (n, shape) in shapes.outputs.iter_mut().enumerate() {
                *shape = shape_of(&format!("CLKOUT{n}"))?;
            }

            if mmcm {
                let fine = |name: &str| text(&format!("{name}_USE_FINE_PS"), "FALSE") == "TRUE";
                shapes.cascade = text("CLKOUT4_CASCADE", "FALSE") == "TRUE";
                shapes.phase_shift_routed = used("psclk");
                let clkout0 = eighths("CLKOUT0_DIVIDE_F", 1, 1, 128)?;
                let feedback = eighths("CLKFBOUT_MULT_F", 5, 2, 64)?;
                shapes.clkout0_eighths = clkout0 % 8;
                shapes.feedback_eighths = feedback % 8;
                let mut outputs = [(1u64, false, false); 7];
                let mut phases = vec![(
                    "CLKFBOUT".to_owned(),
                    feedback,
                    shapes.feedback.phase,
                    fine("CLKFBOUT"),
                )];
                for (n, out) in outputs.iter_mut().enumerate() {
                    let divide = if n == 0 {
                        clkout0 / 8
                    } else {
                        integer(&format!("CLKOUT{n}_DIVIDE"), 1, 1, 128)?
                    };
                    // A cascade runs `CLKOUT6`'s counter for `CLKOUT4`.
                    let on = used(OUT[n])
                        || OUTB.get(n).is_some_and(|b| used(b))
                        || (n == 6 && shapes.cascade);
                    let name = format!("CLKOUT{n}");
                    *out = (divide, on, fine(&name));
                    let eighths = if n == 0 { clkout0 } else { divide * 8 };
                    phases.push((name.clone(), eighths, shapes.outputs[n].phase, fine(&name)));
                }
                // A fine phase shift starting off a whole VCO cycle: Vivado writes
                // bits for it that this database has no name for (`28_91` to
                // `29_93`, in `registers.txt`), so it has nothing to write here.
                for (name, eighths, phase, is_fine) in &phases {
                    if *is_fine && phase_of(*eighths, *phase).1 != 0 {
                        return Err(bad(
                            &format!("{name}_PHASE"),
                            "is not a whole number of VCO cycles, with a fine phase shift: \
                             this flow does not configure that yet"
                                .to_owned(),
                        ));
                    }
                }
                let divclk_divide = integer("DIVCLK_DIVIDE", 1, 1, 106)?;
                let listed: Vec<(&str, u64, i64, bool)> = phases
                    .iter()
                    .map(|(n, e, p, f)| (n.as_str(), *e, *p, *f))
                    .collect();
                check_manager(true, period, divclk_divide, feedback, &listed)
                    .map_err(|(name, message)| bad(&name, message))?;
                let settings = MmcmSettings {
                    divclk_divide,
                    clkfbout_mult: feedback / 8,
                    clkfbout_fine_ps: fine("CLKFBOUT"),
                    clkfbout_used: used("fbout") || used("fboutb"),
                    outputs,
                    bandwidth,
                    startup_wait,
                    invert: [
                        invert("rst", "IS_RST_INVERTED"),
                        invert("pwrdwn", "IS_PWRDWN_INVERTED"),
                        invert("psen", "IS_PSEN_INVERTED"),
                        invert("psincdec", "IS_PSINCDEC_INVERTED"),
                    ],
                    invert_clkinsel,
                    shapes,
                };
                count += self.write_mmcm(site.tile, &settings, bits)?;
                if compensation == Compensation::Zhold {
                    bits.clear(site.tile, ConfigBit::new(29, 979))?;
                }
                continue;
            }
            let mut more_outputs = [(1u64, false); 5];
            for (n, out) in more_outputs.iter_mut().enumerate() {
                *out = (
                    integer(&format!("CLKOUT{}_DIVIDE", n + 1), 1, 1, 128)?,
                    used(OUT[n + 1]),
                );
            }
            let divclk_divide = integer("DIVCLK_DIVIDE", 1, 1, 56)?;
            let clkfbout_mult = integer("CLKFBOUT_MULT", 5, 2, 64)?;
            let clkout0_divide = integer("CLKOUT0_DIVIDE", 1, 1, 128)?;
            let names = ["CLKOUT1", "CLKOUT2", "CLKOUT3", "CLKOUT4", "CLKOUT5"];
            let mut listed = vec![
                ("CLKFBOUT", clkfbout_mult * 8, shapes.feedback.phase, false),
                (
                    "CLKOUT0",
                    clkout0_divide * 8,
                    shapes.outputs[0].phase,
                    false,
                ),
            ];
            for (n, name) in names.iter().enumerate() {
                listed.push((
                    *name,
                    more_outputs[n].0 * 8,
                    shapes.outputs[n + 1].phase,
                    false,
                ));
            }
            check_manager(false, period, divclk_divide, clkfbout_mult * 8, &listed)
                .map_err(|(name, message)| bad(&name, message))?;
            let settings = PllSettings {
                divclk_divide,
                clkfbout_mult,
                clkout0_divide,
                bandwidth,
                startup_wait,
                clkout0_used: used("out"),
                clkfbout_used: used("fbout"),
                invert_rst: invert("rst", "IS_RST_INVERTED"),
                invert_pwrdwn: invert("pwrdwn", "IS_PWRDWN_INVERTED"),
                more_outputs,
                invert_clkinsel,
                shapes,
            };
            count += self.write_pll(site.tile, &settings, bits)?;
            if compensation == Compensation::Zhold {
                bits.clear(site.tile, ConfigBit::new(28, 76))?;
            }
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

    /// Writes one MMCM's registers into the clock management tile at
    /// `tile`, as [`XrayFabric::write_pll`] does a PLL's.
    ///
    /// # Errors
    ///
    /// Those of [`XrayFabric::write_pll`].
    pub fn write_mmcm(
        &self,
        tile: (u32, u32),
        settings: &MmcmSettings,
        bits: &mut Bitstream,
    ) -> Result<usize, ClockManagerError> {
        let bel = self
            .arch
            .tile_index_at(tile.0, tile.1)
            .and_then(|index| self.arch.tile_types[index].bel(MMCM_BEL))
            .ok_or_else(|| ClockManagerError::MissingField {
                field: "IN_USE".to_owned(),
            })?;
        apply_registers(&bel.config, &mmcm_registers(settings), tile, bits)
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
            more_outputs: [(1, false); 5],
            invert_clkinsel: false,
            shapes: ManagerShapes::default(),
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
