//! A `DSP48E1` on the 7-series fabric: its two sites in a `DSP_L` or `DSP_R`
//! tile, its pins, the bits of each parameter, and what each input that no
//! signal drives is made to read.
//!
//! # What this file used to say, and why it was wrong
//!
//! Until 2026-11-04 this module was a refusal. It said the block could not be
//! built because:
//!
//! > `segbits_dsp_l.db` [has] no `USE_MULT` feature [...]. Whether a block with
//! > none of those bits set multiplies is exactly what a wrong guess would get
//! > silently wrong. [...] Every other tile this backend configures was compared
//! > with what Vivado wrote for the same cell; the Vivado bitstream `prjxray-db`
//! > ships is switches to LEDs and holds no DSP.
//!
//! **Each sentence was true of the database, and the conclusion was wrong**,
//! because it was reached with no Vivado on the machine to ask. Vivado 2026.1
//! is here now. `tools/vivado-parity/gen_dsp_sweep.py` varies one parameter at a
//! time on one block, and `testdata/fpga/vivado/dsp/sweep.txt` is what each did
//! to the tile. `USE_MULT=MULTIPLY` and `DYNAMIC` set **nothing**, against a
//! base of every register on; `NONE` sets one bit the database has no name for.
//! So no `USE_MULT` feature survived into the database because the multiplying
//! mode is the all-clear one, and a block with none of those bits does multiply.
//!
//! It also said "the router has no constant source at all" for `CARRYINSEL`,
//! and that a zero on the block's inputs had no good answer. The loader has
//! tied a block RAM's inputs through `GND_WIRE` since then, and that is what
//! Vivado does for these pins too.
//!
//! The other reason, one site with no pins, was the loader's own: it names a
//! tile's bels by the head of their features, and every feature here is
//! `DSP48.DSP_0.*` or `DSP48.DSP_1.*`. [`bels`] now declares the two.
//!
//! # What was measured, and what is read
//!
//! | what | from | checked or quoted |
//! |---|---|---|
//! | which block a `DSP_k` feature means | `DSP_0` is the even site `Y` | **checked**: `sweep.txt` sets `DSP_0` at `X2Y50` and `DSP_1` at `X2Y51`, on a `DSP_L` and a `DSP_R` |
//! | the bit of each parameter ([`features`]) | `segbits_dsp_l.db` for the names, the sweep for when each is set | **checked** against Vivado, a setting at a time, and whole blocks in `tests/fpga_dsp_vivado.rs` |
//! | the four bits the database does not name ([`derived_features`]) | the sweep | **checked** on both blocks and both tile types |
//! | which wire is which pin ([`bel_pins`]) | `ppips_dsp_l.db`, and the pin names in `tile_type_DSP_L.json` | database; re-read by a test |
//! | the inputs behind a three-way mux ([`LOCAL_MUX`]) | `tile_type_DSP_L.json`, whose `DSP_<k>_D0` has three pips: `DSP_BYP7_0`, `DSP_GND_L`, `DSP_VCC_L`. `segbits` has bits for the last two and `ppips` nothing for the first | database; re-read by a test. That the first needs no bits is **checked**: the sweep's `base` routes all of these and sets nothing in the tile |
//! | what an input that no signal drives reads ([`input_drive`]) | the sweep's `tie_zero`, `tie_one`, `tie_zero_inv`, `tie_one_inv` and `unconnected` | **checked** |
//!
//! **Nothing here has been on a board.** That every bit means what Vivado
//! writes it for is all this can say.
//!
//! # A constant on an input
//!
//! An unrouted interconnect input reads one on this family (`bram` has the
//! account). Vivado's answers, each from the sweep, are of three shapes:
//!
//! - **The 39 pins with a three-way mux of their own** (`D`, `INMODE`,
//!   `ALUMODE[3:2]`, `OPMODE[6]`, `CARRYINSEL[2]`, `CEAD`, `CED`, `CEALUMODE`,
//!   `CEINMODE`, `RSTD`) take `DSP_GND_<side>` or `DSP_VCC_<side>` at the pin,
//!   which is no interconnect at all. An inverter on the pin is left off and
//!   the mux given the value after it.
//! - **The inverting pins without one** (`ALUMODE[1:0]`, `OPMODE[5:0]`,
//!   `CARRYIN`) read the fabric's one, and a zero is **the inverter**: the
//!   `ZIS_*` bit left clear. Nothing is routed.
//! - **Everything else** (`A`, `B`, `C`, `CARRYINSEL[1:0]`, the other enables
//!   and resets) is a route: a zero is `GND_WIRE -> GFAN<n> -> IMUX` as a block
//!   RAM's is, and a one is the `IMUX`'s default, or on a reset, whose `CTRL`
//!   wire has no default, a hop from `BYP_BOUNCE`.
//!
//! This is what makes a zero on `CARRYINSEL` cost a route and on `OPMODE`
//! nothing, and why the block ties `C` to all ones: the multiply does not read
//! it, and a one is free.

use std::collections::{BTreeMap, HashMap};

use super::bram::{BlockRamError, TiePolicy, Want};
use super::{Feature, XrayFabric};
use crate::fpga::arch::{ArchSite, ConfigBit};
use crate::fpga::bitstream::Bitstream;
use crate::fpga::place::{Instance, Netlist};
use crate::ir::AttrValue;
use crate::logic::Bit;

/// The primitive this backend places here.
pub const DSP_PRIMITIVE: &str = "DSP48E1";

/// Every port of a `DSP48E1`, with its width and whether it is an output.
/// Pin names are the vendor's own; a port wider than a bit has its index
/// appended to name its wires (`DSP_0_A7`). The role a netlist knows a pin by
/// is the same name lowercased.
pub(super) const PORTS: [(&str, u32, bool); 49] = [
    ("A", 30, false),
    ("ACIN", 30, false),
    ("ACOUT", 30, true),
    ("ALUMODE", 4, false),
    ("B", 18, false),
    ("BCIN", 18, false),
    ("BCOUT", 18, true),
    ("C", 48, false),
    ("CARRYCASCIN", 1, false),
    ("CARRYCASCOUT", 1, true),
    ("CARRYIN", 1, false),
    ("CARRYINSEL", 3, false),
    ("CARRYOUT", 4, true),
    ("CEA1", 1, false),
    ("CEA2", 1, false),
    ("CEAD", 1, false),
    ("CEALUMODE", 1, false),
    ("CEB1", 1, false),
    ("CEB2", 1, false),
    ("CEC", 1, false),
    ("CECARRYIN", 1, false),
    ("CECTRL", 1, false),
    ("CED", 1, false),
    ("CEINMODE", 1, false),
    ("CEM", 1, false),
    ("CEP", 1, false),
    ("CLK", 1, false),
    ("D", 25, false),
    ("INMODE", 5, false),
    ("MULTSIGNIN", 1, false),
    ("MULTSIGNOUT", 1, true),
    ("OPMODE", 7, false),
    ("OVERFLOW", 1, true),
    ("P", 48, true),
    ("PATTERNBDETECT", 1, true),
    ("PATTERNDETECT", 1, true),
    ("PCIN", 48, false),
    ("PCOUT", 48, true),
    ("RSTA", 1, false),
    ("RSTALLCARRYIN", 1, false),
    ("RSTALUMODE", 1, false),
    ("RSTB", 1, false),
    ("RSTC", 1, false),
    ("RSTCTRL", 1, false),
    ("RSTD", 1, false),
    ("RSTINMODE", 1, false),
    ("RSTM", 1, false),
    ("RSTP", 1, false),
    ("UNDERFLOW", 1, true),
];

/// Every port, in one iterator.
pub(super) fn all_ports() -> impl Iterator<Item = (&'static str, u32, bool)> {
    PORTS.iter().copied()
}

/// The ports that come from the block below over dedicated wires.
const CASCADE_INPUTS: [&str; 5] = ["ACIN", "BCIN", "PCIN", "CARRYCASCIN", "MULTSIGNIN"];

/// For each block, the pin behind a three-way mux and the interconnect wire
/// that is the mux's third input: `tile_type_DSP_L.json`'s and
/// `tile_type_DSP_R.json`'s, which are the same.
///
/// `ppips_dsp_l.db` has no line for these, since they are programmable, and
/// `segbits_dsp_l.db` has none either, since the choice of the interconnect is
/// the mux with no bit set. Without this table the pin has no way in, and `D`
/// could never be driven.
pub(super) const LOCAL_MUX: [&[(&str, &str)]; 2] = [
    &[
        ("ALUMODE2", "FAN0_0"),
        ("ALUMODE3", "BYP0_0"),
        ("CARRYINSEL2", "BYP2_2"),
        ("CEAD", "BYP0_1"),
        ("CEALUMODE", "BYP4_0"),
        ("CED", "FAN0_1"),
        ("CEINMODE", "FAN2_0"),
        ("D0", "BYP7_0"),
        ("D1", "BYP5_0"),
        ("D2", "BYP3_0"),
        ("D3", "BYP1_0"),
        ("D4", "BYP7_1"),
        ("D5", "BYP5_1"),
        ("D6", "BYP3_1"),
        ("D7", "BYP1_1"),
        ("D8", "BYP7_2"),
        ("D9", "BYP5_2"),
        ("D10", "BYP3_2"),
        ("D11", "BYP1_2"),
        ("D12", "BYP7_3"),
        ("D13", "BYP5_3"),
        ("D14", "BYP3_3"),
        ("D15", "BYP1_3"),
        ("D16", "BYP7_4"),
        ("D17", "BYP5_4"),
        ("D18", "BYP3_4"),
        ("D19", "BYP1_4"),
        ("D20", "BYP6_0"),
        ("D21", "BYP6_1"),
        ("D22", "BYP0_2"),
        ("D23", "BYP2_3"),
        ("D24", "BYP2_0"),
        ("INMODE0", "BYP6_3"),
        ("INMODE1", "FAN7_1"),
        ("INMODE2", "FAN1_2"),
        ("INMODE3", "BYP6_2"),
        ("INMODE4", "FAN1_3"),
        ("OPMODE6", "FAN7_2"),
        ("RSTD", "BYP4_1"),
    ],
    &[
        ("ALUMODE2", "FAN0_4"),
        ("ALUMODE3", "BYP0_4"),
        ("CARRYINSEL2", "BYP2_4"),
        ("CEAD", "BYP0_3"),
        ("CEALUMODE", "BYP4_3"),
        ("CED", "FAN0_3"),
        ("CEINMODE", "FAN2_3"),
        ("D0", "FAN3_0"),
        ("D1", "FAN6_0"),
        ("D2", "FAN5_0"),
        ("D3", "FAN4_0"),
        ("D4", "FAN3_1"),
        ("D5", "FAN6_1"),
        ("D6", "FAN5_1"),
        ("D7", "FAN4_1"),
        ("D8", "FAN3_2"),
        ("D9", "FAN6_2"),
        ("D10", "FAN5_2"),
        ("D11", "FAN4_2"),
        ("D12", "FAN3_3"),
        ("D13", "FAN6_3"),
        ("D14", "FAN5_3"),
        ("D15", "FAN4_3"),
        ("D16", "FAN3_4"),
        ("D17", "FAN6_4"),
        ("D18", "FAN5_4"),
        ("D19", "FAN4_4"),
        ("D20", "FAN1_0"),
        ("D21", "FAN1_1"),
        ("D22", "FAN0_2"),
        ("D23", "FAN7_3"),
        ("D24", "FAN7_0"),
        ("INMODE0", "BYP4_2"),
        ("INMODE1", "BYP2_1"),
        ("INMODE2", "FAN7_4"),
        ("INMODE3", "BYP4_4"),
        ("INMODE4", "FAN2_2"),
        ("OPMODE6", "FAN2_4"),
        ("RSTD", "BYP6_4"),
    ],
];

/// True for the two tile types that hold a pair of blocks.
pub(super) fn is_dsp_tile(tile_type: &str) -> bool {
    matches!(tile_type, "DSP_L" | "DSP_R")
}

/// The bel of block `block`: the name its features are filed under.
pub(super) fn bel_name(block: u32) -> String {
    format!("DSP_{block}")
}

/// The role and tile wire of every pin of one block.
pub(super) fn bel_pins(block: u32) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (port, width, _) in all_ports() {
        let role = port.to_lowercase();
        if width == 1 {
            out.push((role, format!("DSP_{block}_{port}")));
        } else {
            for bit in 0..width {
                out.push((format!("{role}{bit}"), format!("DSP_{block}_{port}{bit}")));
            }
        }
    }
    out
}

/// The tile wire of every pin of block `block`.
pub fn dsp_pin_wires(block: u32) -> Vec<String> {
    bel_pins(block).into_iter().map(|(_, wire)| wire).collect()
}

/// Every three-way mux's pin wire and interconnect wire, as `(to, from)`.
pub fn dsp_local_muxes() -> Vec<(String, String)> {
    local_mux_pips("DSP_L")
}

/// The bels of a tile type, for `parse::bels_of`: both blocks, or none.
pub(super) fn bels(tile_type: &str) -> Vec<(String, Vec<(String, String)>)> {
    if !is_dsp_tile(tile_type) {
        return Vec::new();
    }
    (0..2).map(|b| (bel_name(b), bel_pins(b))).collect()
}

/// The connections from the interconnect to the pins of [`LOCAL_MUX`], as
/// `(to, from)` local wires: selected by no bit, so not a feature of the
/// tile, and there.
pub(super) fn local_mux_pips(tile_type: &str) -> Vec<(String, String)> {
    if !is_dsp_tile(tile_type) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (block, table) in LOCAL_MUX.iter().enumerate() {
        for (pin, wire) in *table {
            out.push((format!("DSP_{block}_{pin}"), format!("DSP_{wire}")));
        }
    }
    out
}

/// How far `DSP_1`'s configuration is from `DSP_0`'s in the tile's words,
/// which is every feature's in `segbits_dsp_l.db`. That it also holds of the
/// bits the database has no name for is measured in the sweep's `*_odd`
/// settings.
const BLOCK_STRIDE: u32 = 160;

/// The features of a tile type the database does not name, which the sweep
/// measured:
///
/// - `USE_MULT_NONE`, `26_85`: `USE_MULT="NONE"`, and any `USE_SIMD` that
///   requires it. **This is the database's missing `USE_MULT`**: multiplying
///   is the all-clear setting.
/// - `SEL_PATTERN_C`, `26_80`: `SEL_PATTERN="C"`.
/// - `A1_UNUSED`, `27_111`, and `B1_UNUSED`, `27_38`: a block with one `A`
///   register whose `INMODE[0]` is a constant zero (`INMODE[4]` and `B` for
///   the other), so that the path through `A1` can never be taken. The database
///   has this bit only folded into `AREG_0` and `BREG_0`, which is why a
///   block with no register, whose bypass is always taken, shows it anyway. A
///   block with its `INMODE` routed does not set it, even with one register.
pub(super) fn derived_features(tile_type: &str) -> Vec<Feature> {
    if !is_dsp_tile(tile_type) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for block in 0..2 {
        for (name, frame, bit) in [
            ("USE_MULT_NONE", 26, 85),
            ("SEL_PATTERN_C", 26, 80),
            ("A1_UNUSED", 27, 111),
            ("B1_UNUSED", 27, 38),
        ] {
            out.push(Feature {
                name: format!("DSP48.DSP_{block}.{name}"),
                ones: vec![ConfigBit::new(frame, bit + block * BLOCK_STRIDE)],
                zeros: Vec::new(),
            });
        }
    }
    out
}

/// A parameter of the instance, by name, and the vendor's default.
struct Params<'a>(&'a dyn Fn(&str) -> Option<AttrValue>);

impl Params<'_> {
    fn int(&self, name: &str, default: i64) -> Result<i64, String> {
        match (self.0)(name) {
            None => Ok(default),
            Some(value) => value
                .as_int()
                .ok_or_else(|| format!("`{name}` is {value:?}, which is not a number")),
        }
    }

    fn text(&self, name: &str, default: &str) -> Result<String, String> {
        match (self.0)(name) {
            None => Ok(default.to_owned()),
            Some(AttrValue::String(text)) => Ok(text),
            Some(other) => Err(format!("`{name}` is {other:?}, which is not a string")),
        }
    }

    /// A parameter one bit per position, as a mask with the low bit first.
    fn bits(&self, name: &str, default: u64) -> Result<u64, String> {
        match (self.0)(name) {
            None => Ok(default),
            Some(AttrValue::Int(value)) => u64::try_from(value)
                .map_err(|_| format!("`{name}` is {value}, which is not a bit pattern")),
            Some(AttrValue::Const(value)) => value
                .to_u64()
                .ok_or_else(|| format!("`{name}` holds a bit that is neither 0 nor 1")),
            Some(other) => Err(format!("`{name}` is {other:?}, which is not a bit pattern")),
        }
    }

    /// One of a fixed set of values, refused by name otherwise.
    fn choice(&self, name: &str, default: &str, allowed: &[&str]) -> Result<String, String> {
        let value = self.text(name, default)?;
        if allowed.contains(&value.as_str()) {
            Ok(value)
        } else {
            Err(format!(
                "`{name}` is \"{value}\", and a `{DSP_PRIMITIVE}` takes {}",
                allowed.join(", ")
            ))
        }
    }

    /// A register count, refused if the block has no such setting.
    fn register(&self, name: &str, allowed: &[i64]) -> Result<i64, String> {
        let value = self.int(name, 1)?;
        if allowed.contains(&value) {
            Ok(value)
        } else {
            Err(format!("`{name}` is {value}, and it may be {allowed:?}"))
        }
    }
}

/// What drives one input, once the netlist and the parameters have had
/// their say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drive {
    /// A routed signal.
    Signal,
    /// A constant at the pin, before its inverter.
    Constant(Bit),
    /// Nothing that matters: a cascade input, or a clock with nothing on it.
    Nothing,
}

/// What a pin the instance leaves unconnected reads, the sweep's
/// `unconnected` and `comb_open`: `CEP`, `CEAD` and `CED` read one where the
/// other enables read zero, as do `A`, `B` and `C`. What the vendor's library
/// declares for an unconnected port, as Vivado compiled it, not a rule.
fn unconnected_value(port: &str) -> Bit {
    match port {
        "A" | "B" | "C" | "CEP" | "CEAD" | "CED" => Bit::One,
        _ => Bit::Zero,
    }
}

/// The port and bit a role names, from [`PORTS`].
fn port_of_role(role: &str) -> Option<(&'static str, u32)> {
    let head = role.trim_end_matches(|c: char| c.is_ascii_digit());
    let mut found = None;
    for (port, width, _) in all_ports() {
        if width == 1 && port.eq_ignore_ascii_case(role) {
            return Some((port, 0));
        }
        if width > 1
            && port.eq_ignore_ascii_case(head)
            && let Ok(bit) = role[head.len()..].parse::<u32>()
        {
            found = Some((port, bit));
        }
    }
    found
}

/// The inverter parameter a port has, if it has one.
fn inverter_of(port: &str) -> Option<(&'static str, &'static str)> {
    match port {
        "ALUMODE" => Some(("IS_ALUMODE_INVERTED", "ZIS_ALUMODE_INVERTED")),
        "INMODE" => Some(("IS_INMODE_INVERTED", "ZIS_INMODE_INVERTED")),
        "OPMODE" => Some(("IS_OPMODE_INVERTED", "ZIS_OPMODE_INVERTED")),
        "CARRYIN" => Some(("IS_CARRYIN_INVERTED", "ZIS_CARRYIN_INVERTED")),
        _ => None,
    }
}

/// True when the pin has its own mux with a ground and a power input. The
/// table is the same for both blocks.
fn has_local_mux(role: &str) -> bool {
    LOCAL_MUX[0]
        .iter()
        .any(|(pin, _)| pin.eq_ignore_ascii_case(role))
}

/// What each input role of the instance is, keyed on role.
fn input_drives(
    instance: &Instance,
    netlist: &Netlist,
    is_routed: &dyn Fn(usize) -> bool,
) -> Result<BTreeMap<String, Drive>, String> {
    let mut given: HashMap<&str, Drive> = HashMap::new();
    for pin in &instance.pins {
        let pin = &netlist.pins[*pin];
        if pin.output {
            continue;
        }
        let drive = match (pin.signal, pin.constant) {
            (Some(signal), _) if is_routed(signal) => Drive::Signal,
            (Some(signal), _) => {
                // A signal with no driver reads as a pin nothing connects;
                // one with a driver the router failed on is the router's
                // failure, reported beside this.
                let undriven = netlist.signals[signal].driver.is_none();
                if !undriven {
                    return Err(format!(
                        "`{}` carries a signal the router did not route",
                        pin.role
                    ));
                }
                continue;
            }
            (None, Some(Bit::Zero)) => Drive::Constant(Bit::Zero),
            (None, Some(Bit::One)) => Drive::Constant(Bit::One),
            (None, Some(_)) | (None, None) => continue,
        };
        given.insert(pin.role.as_str(), drive);
    }
    let mut out = BTreeMap::new();
    for (port, width, output) in all_ports() {
        if output {
            continue;
        }
        for bit in 0..width {
            let role = if width == 1 {
                port.to_lowercase()
            } else {
                format!("{}{bit}", port.to_lowercase())
            };
            let drive = match given.get(role.as_str()) {
                Some(drive) => *drive,
                None if CASCADE_INPUTS.contains(&port) || port == "CLK" => Drive::Nothing,
                None => Drive::Constant(unconnected_value(port)),
            };
            // A constant on a cascade input or the clock reaches nothing:
            // `A_INPUT` chooses, and the sweep's `comb_open` has no register
            // for a clock to matter to.
            let drive = match drive {
                Drive::Constant(_) if CASCADE_INPUTS.contains(&port) || port == "CLK" => {
                    Drive::Nothing
                }
                other => other,
            };
            out.insert(role, drive);
        }
    }
    Ok(out)
}

/// The value after a pin's inverter of a constant at the pin, as a bit of
/// the instance's parameter.
fn after_inverter(params: &Params<'_>, port: &str, bit: u32, value: Bit) -> Result<Bit, String> {
    let Some((name, _)) = inverter_of(port) else {
        return Ok(value);
    };
    let inverted = (params.bits(name, 0)? >> bit) & 1 == 1;
    Ok(match (value, inverted) {
        (Bit::Zero, false) | (Bit::One, true) => Bit::Zero,
        _ => Bit::One,
    })
}

/// Every feature a block needs for its parameters and its constants, in the
/// database's spelling after `DSP48.DSP_<k>.`: the table in the module
/// documentation, line by line from `sweep.txt`.
///
/// `drives` says which inputs are constants, which is what changes an
/// inverter's bit and sets [`derived_features`]' `A1_UNUSED`.
fn features(params: &Params<'_>, drives: &BTreeMap<String, Drive>) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();

    let areg = params.register("AREG", &[0, 1, 2])?;
    let breg = params.register("BREG", &[0, 1, 2])?;
    let acasc = params.register("ACASCREG", &[0, 1, 2])?;
    let bcasc = params.register("BCASCREG", &[0, 1, 2])?;
    for (name, reg, casc) in [("A", areg, acasc), ("B", breg, bcasc)] {
        // UG479's table of the two: the cascade may only be as far back as
        // the register, and never further than one short of it when two.
        let legal = matches!((reg, casc), (0, 0) | (1, 1) | (2, 1) | (2, 2));
        if !legal {
            return Err(format!(
                "`{name}REG` {reg} with `{name}CASCREG` {casc} is not a setting the block has"
            ));
        }
    }
    if areg == 0 {
        out.push("AREG_0".to_owned());
    }
    if areg == 2 {
        out.push("AREG_2".to_owned());
    }
    if breg == 0 {
        out.push("BREG_0".to_owned());
    }
    if breg == 2 {
        out.push("BREG_2".to_owned());
    }
    if !(areg == 2 && acasc == 1) {
        out.push("ZAREG_2_ACASCREG_1".to_owned());
    }
    if !(breg == 2 && bcasc == 1) {
        out.push("ZBREG_2_BCASCREG_1".to_owned());
    }

    let dport = match params.text("USE_DPORT", "FALSE")?.as_str() {
        "TRUE" => true,
        "FALSE" => false,
        other => return Err(format!("`USE_DPORT` is \"{other}\"")),
    };
    if dport {
        out.push("USE_DPORT[0]".to_owned());
    }
    // A register a block without the port has no use for is written off
    // whatever its parameter says; `dport_*` and `nodport_*` in the sweep.
    if !dport || params.register("ADREG", &[0, 1])? == 0 {
        out.push("ZADREG[0]".to_owned());
    }
    if !dport || params.register("DREG", &[0, 1])? == 0 {
        out.push("ZDREG[0]".to_owned());
    }
    for (name, feature) in [
        ("ALUMODEREG", "ZALUMODEREG[0]"),
        ("CARRYINREG", "ZCARRYINREG[0]"),
        ("CARRYINSELREG", "ZCARRYINSELREG[0]"),
        ("CREG", "ZCREG[0]"),
        ("INMODEREG", "ZINMODEREG[0]"),
        ("MREG", "ZMREG[0]"),
        ("OPMODEREG", "ZOPMODEREG[0]"),
        ("PREG", "ZPREG[0]"),
    ] {
        if params.register(name, &[0, 1])? == 0 {
            out.push(feature.to_owned());
        }
    }

    if params.choice("A_INPUT", "DIRECT", &["DIRECT", "CASCADE"])? == "CASCADE" {
        out.push("A_INPUT[0]".to_owned());
    }
    if params.choice("B_INPUT", "DIRECT", &["DIRECT", "CASCADE"])? == "CASCADE" {
        out.push("B_INPUT[0]".to_owned());
    }
    let use_mult = params.choice("USE_MULT", "MULTIPLY", &["MULTIPLY", "DYNAMIC", "NONE"])?;
    let use_simd = params.choice("USE_SIMD", "ONE48", &["ONE48", "TWO24", "FOUR12"])?;
    if use_mult == "NONE" {
        // The Vivado rule the sweep had to obey: the multiplier's register
        // with no multiplier is a design rule failure there.
        if params.register("MREG", &[0, 1])? != 0 {
            return Err("`USE_MULT` \"NONE\" needs `MREG` 0".to_owned());
        }
        out.push("USE_MULT_NONE".to_owned());
    } else if use_simd != "ONE48" {
        return Err(format!(
            "`USE_SIMD` \"{use_simd}\" needs `USE_MULT` \"NONE\""
        ));
    }
    if use_simd != "ONE48" {
        out.push("USE_SIMD_FOUR12_TWO24".to_owned());
    }
    if use_simd == "FOUR12" {
        out.push("USE_SIMD_FOUR12".to_owned());
    }

    let mask = params.bits("MASK", 0x3FFF_FFFF_FFFF)?;
    let pattern = params.bits("PATTERN", 0)?;
    for bit in 0..48 {
        if mask >> bit & 1 == 1 {
            out.push(format!("MASK[{bit}]"));
        }
        if pattern >> bit & 1 == 1 {
            out.push(format!("PATTERN[{bit}]"));
        }
    }
    match params
        .choice(
            "SEL_MASK",
            "MASK",
            &["MASK", "C", "ROUNDING_MODE1", "ROUNDING_MODE2"],
        )?
        .as_str()
    {
        "C" => out.push("SEL_MASK_C".to_owned()),
        "ROUNDING_MODE1" => out.push("SEL_MASK_ROUNDING_MODE1".to_owned()),
        "ROUNDING_MODE2" => {
            out.push("SEL_MASK_ROUNDING_MODE1".to_owned());
            out.push("SEL_MASK_ROUNDING_MODE2".to_owned());
        }
        _ => {}
    }
    if params.choice("SEL_PATTERN", "PATTERN", &["PATTERN", "C"])? == "C" {
        out.push("SEL_PATTERN_C".to_owned());
    }
    match params
        .choice(
            "AUTORESET_PATDET",
            "NO_RESET",
            &["NO_RESET", "RESET_MATCH", "RESET_NOT_MATCH"],
        )?
        .as_str()
    {
        "RESET_MATCH" => out.push("AUTORESET_PATDET_RESET".to_owned()),
        "RESET_NOT_MATCH" => {
            out.push("AUTORESET_PATDET_RESET".to_owned());
            out.push("AUTORESET_PATDET_RESET_NOT_MATCH".to_owned());
        }
        _ => {}
    }
    // Sets no bit, and is checked so that a misspelling is still an error.
    params.choice("USE_PATTERN_DETECT", "NO_PATDET", &["NO_PATDET", "PATDET"])?;

    // A block no signal clocks reads the fabric's one and inverts it, as an
    // inverting pin does a zero; the sweep's `comb_open`.
    let clocked = matches!(drives.get("clk"), Some(Drive::Signal));
    if clocked && params.bits("IS_CLK_INVERTED", 0)? & 1 == 0 {
        out.push("ZIS_CLK_INVERTED".to_owned());
    }
    for (port, width) in [("ALUMODE", 4), ("INMODE", 5), ("OPMODE", 7), ("CARRYIN", 1)] {
        let Some((param, feature)) = inverter_of(port) else {
            continue;
        };
        let parameter = params.bits(param, 0)?;
        for bit in 0..width {
            let role = if width == 1 {
                port.to_lowercase()
            } else {
                format!("{}{bit}", port.to_lowercase())
            };
            let drive = drives.get(&role).copied().unwrap_or(Drive::Nothing);
            let set = match drive {
                // A mux of its own carries the value after the inverter, and
                // the inverter stays off; without one, the pin reads the
                // fabric's one and the inverter is the zero.
                Drive::Constant(value) => {
                    has_local_mux(&role) || after_inverter(params, port, bit, value)? == Bit::One
                }
                _ => parameter >> bit & 1 == 0,
            };
            if set {
                out.push(if width == 1 {
                    feature.to_owned()
                } else {
                    format!("{feature}[{bit}]")
                });
            }
        }
    }
    // The register behind `INMODE`'s choice, on a block whose `INMODE` can
    // only ever choose the other one.
    for (bit, reg, feature) in [(0, areg, "A1_UNUSED"), (4, breg, "B1_UNUSED")] {
        let role = format!("inmode{bit}");
        if reg == 1
            && let Some(Drive::Constant(value)) = drives.get(&role)
            && after_inverter(params, "INMODE", bit, *value)? == Bit::Zero
        {
            out.push(feature.to_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// One input of a placed block that is a constant: what the [`Want`] list
/// should make of it. `None` when it costs no interconnect at all.
fn input_drive(params: &Params<'_>, role: &str, drive: Drive) -> Result<Option<TiePolicy>, String> {
    let Drive::Constant(value) = drive else {
        return Ok(None);
    };
    let Some((port, bit)) = port_of_role(role) else {
        return Ok(None);
    };
    // The value the pin must be made to read.
    let at_pin = if has_local_mux(role) {
        after_inverter(params, port, bit, value)?
    } else if inverter_of(port).is_some() {
        // An inverting pin without a mux reads the fabric's one either way.
        Bit::One
    } else {
        value
    };
    Ok(Some(if at_pin == Bit::One {
        TiePolicy::One
    } else {
        TiePolicy::Zero
    }))
}

/// Which block a site is.
fn block_of(site: &ArchSite) -> Option<u32> {
    site.bel.strip_prefix("DSP_")?.parse().ok()
}

/// What every constant input of one placed block is to be made to read,
/// appended to `wants`. See the module documentation.
pub(super) fn wants(
    params: &dyn Fn(&str) -> Option<AttrValue>,
    instance: &Instance,
    site: &ArchSite,
    netlist: &Netlist,
    is_routed: &dyn Fn(usize) -> bool,
    wants: &mut Vec<Want>,
) -> Result<(), String> {
    let params = Params(params);
    let drives = input_drives(instance, netlist, is_routed)?;
    for (role, drive) in &drives {
        let Some(want) = input_drive(&params, role, *drive)? else {
            continue;
        };
        let Some(node) = site.pin(role) else {
            return Err(format!("the site has no pin `{role}`"));
        };
        wants.push((want, instance.name.clone(), role.clone(), node));
    }
    Ok(())
}

impl XrayFabric {
    /// Sets the features of one placed block, and returns how many.
    ///
    /// # Errors
    ///
    /// [`BlockRamError`] for a setting the block does not have, a feature the
    /// database does not have, or a bit outside its tile.
    pub(super) fn configure_dsp(
        &self,
        params: &dyn Fn(&str) -> Option<AttrValue>,
        instance: &Instance,
        site: &ArchSite,
        netlist: &Netlist,
        is_routed: &dyn Fn(usize) -> bool,
        bits: &mut Bitstream,
    ) -> Result<usize, BlockRamError> {
        let fail = |message: String| BlockRamError {
            instance: instance.name.clone(),
            message,
        };
        let Some(block) = block_of(site) else {
            return Err(fail(format!(
                "it sits on `{}`, which is not one of a tile's two blocks",
                site.bel
            )));
        };
        let tile_type = self.arch.tile_types[site.tile_type].name.as_str();
        let Some(table) = self.block_ram.features.get(tile_type) else {
            return Err(fail(format!(
                "it sits in a `{tile_type}` tile, which has no features"
            )));
        };
        let params = Params(params);
        let drives = input_drives(instance, netlist, is_routed).map_err(fail)?;
        let names = features(&params, &drives).map_err(fail)?;
        for name in &names {
            let full = format!("DSP48.DSP_{block}.{name}");
            let Some(ones) = table.get(&full) else {
                return Err(fail(format!("the database has no feature `{full}`")));
            };
            for bit in ones {
                bits.set(site.tile, *bit).map_err(|e| fail(e.to_string()))?;
            }
        }
        Ok(names.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn no_params(_: &str) -> Option<AttrValue> {
        None
    }

    /// Every input a signal drives: nothing is a constant.
    fn all_routed() -> BTreeMap<String, Drive> {
        let mut out = BTreeMap::new();
        for (port, width, output) in all_ports() {
            if output {
                continue;
            }
            for bit in 0..width {
                let role = if width == 1 {
                    port.to_lowercase()
                } else {
                    format!("{}{bit}", port.to_lowercase())
                };
                out.insert(role, Drive::Signal);
            }
        }
        out
    }

    /// `testdata/fpga/vivado/dsp/sweep.txt`, line by line: each setting's
    /// difference from the base, against what [`features`] makes of the same
    /// setting. Would catch any one parameter's feature read the wrong way or
    /// named wrongly; a wrong *bit* behind the name is the database's, and
    /// `tests/fpga_dsp_vivado.rs` compares those in a real tile.
    #[test]
    fn each_parameter_sets_what_the_sweep_measured() {
        let Ok(text) = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/fpga/vivado/dsp/sweep.txt"
        )) else {
            eprintln!("skipped: `testdata/` is not in the published crate");
            return;
        };
        let settings: BTreeMap<&str, Vec<(&str, AttrValue)>> = BTreeMap::from([
            (
                "acasc",
                vec![("A_INPUT", AttrValue::String("CASCADE".into()))],
            ),
            (
                "bcasc",
                vec![("B_INPUT", AttrValue::String("CASCADE".into()))],
            ),
            ("mreg0", vec![("MREG", AttrValue::Int(0))]),
            ("preg0", vec![("PREG", AttrValue::Int(0))]),
            ("creg0", vec![("CREG", AttrValue::Int(0))]),
            ("opreg0", vec![("OPMODEREG", AttrValue::Int(0))]),
            ("alureg0", vec![("ALUMODEREG", AttrValue::Int(0))]),
            ("inreg0", vec![("INMODEREG", AttrValue::Int(0))]),
            ("ciselreg0", vec![("CARRYINSELREG", AttrValue::Int(0))]),
            ("cireg0", vec![("CARRYINREG", AttrValue::Int(0))]),
            (
                "areg0",
                vec![("AREG", AttrValue::Int(0)), ("ACASCREG", AttrValue::Int(0))],
            ),
            (
                "areg2",
                vec![("AREG", AttrValue::Int(2)), ("ACASCREG", AttrValue::Int(2))],
            ),
            (
                "areg2c1",
                vec![("AREG", AttrValue::Int(2)), ("ACASCREG", AttrValue::Int(1))],
            ),
            (
                "breg0",
                vec![("BREG", AttrValue::Int(0)), ("BCASCREG", AttrValue::Int(0))],
            ),
            (
                "breg2c1",
                vec![("BREG", AttrValue::Int(2)), ("BCASCREG", AttrValue::Int(1))],
            ),
            (
                "dport",
                vec![("USE_DPORT", AttrValue::String("TRUE".into()))],
            ),
            (
                "dport_dreg0",
                vec![
                    ("USE_DPORT", AttrValue::String("TRUE".into())),
                    ("DREG", AttrValue::Int(0)),
                ],
            ),
            (
                "dport_adreg0",
                vec![
                    ("USE_DPORT", AttrValue::String("TRUE".into())),
                    ("ADREG", AttrValue::Int(0)),
                ],
            ),
            ("nodport_dreg0", vec![("DREG", AttrValue::Int(0))]),
            (
                "mult_none",
                vec![
                    ("USE_MULT", AttrValue::String("NONE".into())),
                    ("MREG", AttrValue::Int(0)),
                ],
            ),
            (
                "mult_dyn",
                vec![("USE_MULT", AttrValue::String("DYNAMIC".into()))],
            ),
            (
                "simd2",
                vec![
                    ("USE_MULT", AttrValue::String("NONE".into())),
                    ("MREG", AttrValue::Int(0)),
                    ("USE_SIMD", AttrValue::String("TWO24".into())),
                ],
            ),
            (
                "simd4",
                vec![
                    ("USE_MULT", AttrValue::String("NONE".into())),
                    ("MREG", AttrValue::Int(0)),
                    ("USE_SIMD", AttrValue::String("FOUR12".into())),
                ],
            ),
            (
                "patdet",
                vec![("USE_PATTERN_DETECT", AttrValue::String("PATDET".into()))],
            ),
            (
                "selmask_c",
                vec![("SEL_MASK", AttrValue::String("C".into()))],
            ),
            (
                "selmask_r1",
                vec![("SEL_MASK", AttrValue::String("ROUNDING_MODE1".into()))],
            ),
            (
                "selmask_r2",
                vec![("SEL_MASK", AttrValue::String("ROUNDING_MODE2".into()))],
            ),
            (
                "selpat_c",
                vec![("SEL_PATTERN", AttrValue::String("C".into()))],
            ),
            (
                "auto_match",
                vec![("AUTORESET_PATDET", AttrValue::String("RESET_MATCH".into()))],
            ),
            (
                "auto_notmatch",
                vec![(
                    "AUTORESET_PATDET",
                    AttrValue::String("RESET_NOT_MATCH".into()),
                )],
            ),
            ("inv_clk", vec![("IS_CLK_INVERTED", AttrValue::Int(1))]),
            (
                "inv_carryin",
                vec![("IS_CARRYIN_INVERTED", AttrValue::Int(1))],
            ),
            ("inv_alu0", vec![("IS_ALUMODE_INVERTED", AttrValue::Int(1))]),
            ("inv_alu3", vec![("IS_ALUMODE_INVERTED", AttrValue::Int(8))]),
            ("inv_in0", vec![("IS_INMODE_INVERTED", AttrValue::Int(1))]),
            ("inv_in4", vec![("IS_INMODE_INVERTED", AttrValue::Int(16))]),
            ("inv_op0", vec![("IS_OPMODE_INVERTED", AttrValue::Int(1))]),
            ("inv_op6", vec![("IS_OPMODE_INVERTED", AttrValue::Int(64))]),
        ]);
        let run = |over: &[(&str, AttrValue)]| -> BTreeSet<String> {
            let lookup = |name: &str| -> Option<AttrValue> {
                over.iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.clone())
            };
            features(&Params(&lookup), &all_routed())
                .unwrap()
                .into_iter()
                .collect()
        };
        let base = run(&[]);
        let mut checked = 0;
        for line in text.lines() {
            let Some((head, rest)) = line.split_once(" add ") else {
                continue;
            };
            let name = head.split_whitespace().next().unwrap();
            let Some(over) = settings.get(name) else {
                continue;
            };
            let (add, gone) = rest.split_once(" ; gone ").unwrap();
            let clean = |list: &str| -> BTreeSet<String> {
                list.split_whitespace()
                    .filter(|w| *w != "-")
                    .filter_map(|w| w.rsplit_once('.').map_or(Some(w), |(_, tail)| Some(tail)))
                    .map(str::to_owned)
                    .collect()
            };
            let (mut add, gone) = (clean(add), clean(gone));
            // The decoder names the bit `AREG_0` is partly made of as well.
            if add.contains("AREG_0") {
                add.remove("A1_UNUSED");
            }
            if add.contains("BREG_0") {
                add.remove("B1_UNUSED");
            }
            let now = run(over);
            let added: BTreeSet<String> = now.difference(&base).cloned().collect();
            let lost: BTreeSet<String> = base.difference(&now).cloned().collect();
            assert_eq!(added, add, "{name} adds");
            assert_eq!(lost, gone, "{name} takes away");
            checked += 1;
        }
        assert_eq!(checked, settings.len(), "a setting missing from the sweep");
    }

    /// The base's own features, which is the all-registers-on default a
    /// by-name instance with no parameters gets: Vivado's `MASK` of 46 ones
    /// and no others but the `Z` of the registers it never uses.
    #[test]
    fn a_default_block_is_vivados_base() {
        let names = features(&Params(&no_params), &all_routed()).unwrap();
        let plain: Vec<&str> = names
            .iter()
            .map(String::as_str)
            .filter(|n| !n.starts_with("MASK["))
            .collect();
        assert!(plain.contains(&"ZAREG_2_ACASCREG_1"));
        assert!(plain.contains(&"ZADREG[0]"));
        assert!(!plain.contains(&"ZMREG[0]"));
        assert_eq!(names.iter().filter(|n| n.starts_with("MASK[")).count(), 46);
    }

    /// A zero on `OPMODE[1]` is the inverter, a zero on `OPMODE[6]` is the
    /// mux, a zero on `CARRYINSEL[0]` is a route. Would catch any pin put on
    /// the wrong one of the three, which `sweep.txt`'s `tie_*` lines measured.
    #[test]
    fn a_constant_takes_the_shape_its_pin_has() {
        let params = Params(&no_params);
        let zero = Drive::Constant(Bit::Zero);
        assert_eq!(
            input_drive(&params, "opmode1", zero).unwrap(),
            Some(TiePolicy::One)
        );
        assert_eq!(
            input_drive(&params, "opmode6", zero).unwrap(),
            Some(TiePolicy::Zero)
        );
        assert_eq!(
            input_drive(&params, "carryinsel0", zero).unwrap(),
            Some(TiePolicy::Zero)
        );
        assert_eq!(
            input_drive(&params, "c3", Drive::Constant(Bit::One)).unwrap(),
            Some(TiePolicy::One)
        );
        assert_eq!(input_drive(&params, "a3", Drive::Signal).unwrap(), None);
    }

    /// `AREG` 3 and the like are refused rather than written.
    #[test]
    fn a_setting_the_block_has_not_got_is_refused() {
        let three = |name: &str| (name == "AREG").then_some(AttrValue::Int(3));
        assert!(features(&Params(&three), &all_routed()).is_err());
        let bad = |name: &str| (name == "AREG").then_some(AttrValue::Int(2));
        // `AREG` 2 with the default `ACASCREG` 1 is legal.
        assert!(features(&Params(&bad), &all_routed()).is_ok());
        let casc = |name: &str| (name == "ACASCREG").then_some(AttrValue::Int(2));
        assert!(features(&Params(&casc), &all_routed()).is_err());
        let simd = |name: &str| (name == "USE_SIMD").then(|| AttrValue::String("TWO24".into()));
        assert!(features(&Params(&simd), &all_routed()).is_err());
    }

    /// The bits the sweep measured for what the database does not name: its
    /// `mult_none`, `selpat_c`, `in0_zero`, `in4_zero` and their `*_odd` counterparts.
    /// Would catch a typing error, or the second block moved by anything but what
    /// the sweep says; `sweep.txt` has both as it found them.
    #[test]
    fn the_unnamed_bits_are_where_the_sweep_found_them() {
        let find = |name: &str| {
            derived_features("DSP_R")
                .into_iter()
                .find(|f| f.name == name)
                .map(|f| f.ones[0])
                .unwrap()
        };
        for (name, frame, bit) in [
            ("DSP48.DSP_0.USE_MULT_NONE", 26, 85),
            ("DSP48.DSP_1.USE_MULT_NONE", 26, 245),
            ("DSP48.DSP_0.SEL_PATTERN_C", 26, 80),
            ("DSP48.DSP_1.SEL_PATTERN_C", 26, 240),
            ("DSP48.DSP_0.A1_UNUSED", 27, 111),
            ("DSP48.DSP_1.A1_UNUSED", 27, 271),
            ("DSP48.DSP_0.B1_UNUSED", 27, 38),
            ("DSP48.DSP_1.B1_UNUSED", 27, 198),
        ] {
            assert_eq!(find(name), ConfigBit::new(frame, bit), "{name}");
        }
        assert!(derived_features("CLBLL_L").is_empty());
    }

    #[test]
    fn a_role_names_its_port_and_bit() {
        assert_eq!(port_of_role("a7"), Some(("A", 7)));
        assert_eq!(port_of_role("cea1"), Some(("CEA1", 0)));
        assert_eq!(port_of_role("opmode6"), Some(("OPMODE", 6)));
        assert_eq!(port_of_role("clk"), Some(("CLK", 0)));
        assert_eq!(port_of_role("nothing"), None);
        assert!(has_local_mux("d24") && has_local_mux("rstd") && !has_local_mux("a0"));
    }
}
