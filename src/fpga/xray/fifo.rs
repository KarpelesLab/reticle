//! A `FIFO18E1` or `FIFO36E1` named in a design: the block RAM features of
//! the half or halves it takes, with the tile's FIFO features beside them, its
//! pins on the block RAM site's own, and the parameters Vivado refuses
//! refused by the same rule.
//!
//! Only the Vivado model of block RAM (`--memories vivado`, the default) has a
//! bel for either; a legacy build refuses a FIFO as a cell no site takes.
//!
//! # What Vivado wrote, and what holds Reticle to it
//!
//! Every statement here was read off Vivado 2026.1's own builds of the designs
//! of `testdata/fpga/vivado/fifo/` (`tools/vivado-parity/gen_fifo.py`), each
//! decoded through this repository's database; `tests/fpga_xray_fifo_vivado.rs`
//! holds the result to them, tile by tile and net by net. **Checked**, on those
//! designs and nothing more:
//!
//! | Piece | Vivado's answer |
//! |---|---|
//! | the half | a `FIFO18E1` is the lower half's site (`RAMB18_Y0`, typed `FIFO18E1`), and only that one; a `FIFO36E1` both halves, as a `RAMB36E1` |
//! | the block RAM features ([`features`]) | read on port A and written on port B at the width, `SDP` with all four widths 18 when it is 36 (72 for a `FIFO36E1`) wide, port A's `READ_WIDTH_A_18` then too; both write modes `NO_CHANGE`; `DO_REG` sets both output registers; a `FIFO36E1` each half as wide, all four `RAMB36.BRAM36_*_1` when it is 9 wide, and its upper half's enables and reset held through their inverters |
//! | `FIFO_MODE`, `EN_SYN`, `FIRST_WORD_FALL_THROUGH`, `EN_ECC_READ`, `EN_ECC_WRITE` | a feature of the half (`RAMB18_Y0.FIFO_MODE`), of the tile and of `RAMB36.` |
//! | the flag offsets ([`offset_features`]) | `ZALMOST_EMPTY_OFFSET` is `ALMOST_EMPTY_OFFSET`, less one when the first word falls through, and `ZALMOST_FULL_OFFSET` is `ALMOST_FULL_OFFSET`, more one when the clocks are apart; each bit written inverted |
//! | the width ([`derived_features`]) | three bits the database does not name, frame 27 bits 172 to 174 of a `BRAM_L` or `BRAM_R` tile, which this module calls `FIFO_WIDTH[0..2]`: 2, 4, 9, 18 and 36 bits a half are 0 to 4 |
//! | the initial and reset values ([`register_values`]) | solved for, bit by bit, from five builds of random values per width (`values.txt`) |
//! | the pins ([`fifo18_pins`], [`fifo36_pins`], [`input_roles`]) | `get_site_pins` of every pin of the cell, and of every pin of its site |
//! | what Vivado refuses ([`check`]) | its `AVAL` design rules by name, and every flag offset's range at every width and mode (`ranges.txt`) |
//!
//! What a FIFO does on a part is UG473's word and the vendor's simulation
//! model's, **quoted**: nothing here has run on one.

use super::Feature;
use crate::fpga::arch::ConfigBit;
use crate::ir::{AttrValue, Const};
use crate::logic::Bit;

/// The bel of a tile's `FIFO18E1`, on its lower half.
pub(super) const FIFO18_BEL: &str = "FIFO18";
/// The kind of site a `FIFO18E1` takes: not `bram`, so that nothing else is put
/// on it and it is put on nothing else.
pub(super) const FIFO18_KIND: &str = "fifo18";
/// The bel of a tile's `FIFO36E1`, both halves.
pub(super) const FIFO36_BEL: &str = "FIFO36";
/// The kind of site a `FIFO36E1` takes.
pub(super) const FIFO36_KIND: &str = "fifo36";

/// Whether `primitive` is a FIFO this module configures.
pub(super) fn is_fifo(primitive: &str) -> bool {
    matches!(primitive, "FIFO18E1" | "FIFO36E1")
}

/// The roles of a FIFO's data inputs as the cell has them, which
/// [`input_roles`] moves onto the site's: `fifo_di<i>` and `fifo_dip<i>`. A
/// role with this prefix is never an input to tie.
pub(super) const CELL_ONLY: &str = "fifo_d";

/// Whether a FIFO bel's `role` is not an input of the site: an output
/// (`fifo_empty`, `p0_dout3`) or a cell-only data role ([`CELL_ONLY`]).
pub(super) fn is_not_an_input(role: &str) -> bool {
    role.starts_with("fifo_")
}

/// The three bits of a block RAM tile that hold a FIFO's width, which the
/// database does not name: `FIFO_WIDTH[0]` to `FIFO_WIDTH[2]`, frame 27 bits
/// 172 to 174. **Checked**: Vivado's builds of every width of both cells on
/// `BRAM_R` tiles, and of three on `BRAM_L` tiles (`f18_left`, `f18_left_w36`,
/// `f36_left`), set these bits to the half's width code ([`width_code`]) and
/// nothing else unexplained in the tile.
pub(super) fn derived_features(tile_type: &str) -> Vec<Feature> {
    if tile_type != "BRAM_L" && tile_type != "BRAM_R" {
        return Vec::new();
    }
    (0..3u32)
        .map(|bit| Feature {
            name: format!("FIFO_WIDTH[{bit}]"),
            ones: vec![ConfigBit::new(27, 172 + bit)],
            zeros: Vec::new(),
        })
        .collect()
}

/// The code `FIFO_WIDTH` holds for a half `width` bits wide: 2 is 0, 4 is 1,
/// 9 is 2, 18 is 3 and 36 is 4. A `FIFO18E1` 4 wide is 1, and a `FIFO36E1` 4
/// wide, whose halves are 2 wide, is 0.
fn width_code(half: i64) -> Option<u32> {
    match half {
        2 => Some(0),
        4 => Some(1),
        9 => Some(2),
        18 => Some(3),
        36 => Some(4),
        _ => None,
    }
}

/// A FIFO's shape, from its parameters: see [`check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// A `FIFO36E1`, both halves.
    pub wide: bool,
    /// `DATA_WIDTH`.
    pub width: i64,
    /// The width of each half: the width of a `FIFO18E1`, half that (or 2 for
    /// 4) of a `FIFO36E1`.
    pub half: i64,
    /// Whether each half is in simple dual port, 36 bits wide.
    pub sdp: bool,
    /// `EN_SYN`: both ports on one clock.
    pub sync: bool,
    /// `FIRST_WORD_FALL_THROUGH`.
    pub fwft: bool,
    /// `DO_REG`.
    pub registered: bool,
}

/// A `"TRUE"`/`"FALSE"` parameter, `default` when absent.
fn flag(
    params: &dyn Fn(&str) -> Option<AttrValue>,
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match params(name) {
        None => Ok(default),
        Some(AttrValue::String(s)) if s.eq_ignore_ascii_case("TRUE") => Ok(true),
        Some(AttrValue::String(s)) if s.eq_ignore_ascii_case("FALSE") => Ok(false),
        Some(other) => Err(format!(
            "`{name}` is {other:?}; \"TRUE\" or \"FALSE\" is asked"
        )),
    }
}

/// An integer parameter, `default` when absent.
fn int(
    params: &dyn Fn(&str) -> Option<AttrValue>,
    name: &str,
    default: i64,
) -> Result<i64, String> {
    match params(name) {
        None => Ok(default),
        Some(value) => value
            .as_int()
            .ok_or_else(|| format!("`{name}` is {value:?}, which is not a number")),
    }
}

/// The depth Vivado's design rules count a FIFO `width` bits wide as having,
/// whichever cell it is: 8192 words of 4, 4096 of 9, 2048 of 18, 1024 of 36
/// and 512 of 72. **A `FIFO18E1` is counted as deep as a `FIFO36E1`**: its
/// own depth is half that (UG473, quoted), but the ranges of `ranges.txt`,
/// Vivado's refusal of each width in each mode, are these for both cells.
fn rule_depth(width: i64) -> i64 {
    match width {
        4 => 8192,
        9 => 4096,
        18 => 2048,
        36 => 1024,
        _ => 512,
    }
}

/// The flag offsets Vivado accepts for a FIFO of this shape, as
/// `(empty, full)`, each `(least, most)`. **Checked** against Vivado's own
/// refusal (`FIFO_AlmostRange_empty`, `FIFO_AlmostRange_full`) of every width
/// of both cells on one clock, on two, and on two with the first word falling
/// through: `testdata/fpga/vivado/fifo/ranges.txt`.
#[must_use]
pub fn offset_ranges(width: i64, sync: bool, fwft: bool) -> ((i64, i64), (i64, i64)) {
    let depth = rule_depth(width);
    if sync {
        return ((1, depth - 2), (1, depth - 2));
    }
    let empty = if fwft { (6, depth - 5) } else { (5, depth - 6) };
    (empty, (4, depth - 7))
}

/// The FIFO's shape, or the reason it is not built.
///
/// The refusals Vivado makes are its own, named by the design rule it gives
/// (`AVAL`, at `write_bitstream`): two clocks with no output register
/// (`enum_EN_SYN_enum_DO_REG_1`), a first word falling through on one clock
/// (`en_syn_and_fwft`), error correction narrower than 72
/// (`enum_EN_ECC_READ_TRUE_enum_DATA_WIDTH_72` and its `WRITE` twin), and a
/// flag offset out of [`offset_ranges`] (`FIFO_AlmostRange_empty`,
/// `FIFO_AlmostRange_full`).
///
/// One refusal is Reticle's and not Vivado's: a `DATA_WIDTH` its `FIFO_MODE`
/// does not have (36 in `FIFO18`, 72 in `FIFO36`, or 72 for a `FIFO18E1`).
/// Vivado builds each of those without a word, and what it builds is neither
/// mode: `fx_f18_w36_mode` is read and written 18 wide with the width bits
/// of 36, `fx_f18_w72` 4 wide with the bits of 4, and `fx_f36_w72_mode` has
/// `SDP_*_WIDTH_36` without the widths `SDP` has everywhere else. UG473 gives
/// each mode its widths, quoted.
///
/// # Errors
///
/// The reason, as a sentence.
pub fn check(primitive: &str, params: &dyn Fn(&str) -> Option<AttrValue>) -> Result<Shape, String> {
    let wide = primitive == "FIFO36E1";
    let width = int(params, "DATA_WIDTH", 4)?;
    let mode = match params("FIFO_MODE") {
        None => if wide { "FIFO36" } else { "FIFO18" }.to_owned(),
        Some(AttrValue::String(s)) => s,
        Some(other) => return Err(format!("`FIFO_MODE` is {other:?}, which is not a string")),
    };
    let widths: &[i64] = match (wide, mode.as_str()) {
        (false, "FIFO18") => &[4, 9, 18],
        (false, "FIFO18_36") => &[36],
        (true, "FIFO36") => &[4, 9, 18, 36],
        (true, "FIFO36_72") => &[72],
        _ => {
            return Err(format!(
                "`FIFO_MODE` is \"{mode}\", which a {primitive} has not got: {}",
                if wide {
                    "\"FIFO36\" or \"FIFO36_72\""
                } else {
                    "\"FIFO18\" or \"FIFO18_36\""
                }
            ));
        }
    };
    if !widths.contains(&width) {
        return Err(format!(
            "`DATA_WIDTH` is {width} in `FIFO_MODE` \"{mode}\", which has {}. Vivado builds this \
             without a word, as neither mode (docs/fpga-vivado-parity.md); Reticle refuses it",
            widths
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(" and ")
        ));
    }
    let sync = flag(params, "EN_SYN", false)?;
    let fwft = flag(params, "FIRST_WORD_FALL_THROUGH", false)?;
    let registered = match int(params, "DO_REG", 1)? {
        0 => false,
        1 => true,
        other => return Err(format!("`DO_REG` is {other}; 0 or 1 is asked")),
    };
    if !sync && !registered {
        return Err(
            "Vivado's design rule `enum_EN_SYN_enum_DO_REG_1`: `EN_SYN` set to FALSE requires \
             `DO_REG` is 1 for FIFO operation"
                .to_owned(),
        );
    }
    if sync && fwft {
        return Err(
            "Vivado's design rule `en_syn_and_fwft`: `FIRST_WORD_FALL_THROUGH` may not be TRUE \
             if `EN_SYN` is TRUE"
                .to_owned(),
        );
    }
    for name in ["EN_ECC_READ", "EN_ECC_WRITE"] {
        if !wide && params(name).is_some() {
            return Err(format!(
                "`{name}` is a FIFO36E1's; a FIFO18E1 has no error correction"
            ));
        }
        if flag(params, name, false)? && width != 72 {
            return Err(format!(
                "Vivado's design rule `enum_{name}_TRUE_enum_DATA_WIDTH_72`: built-in error \
                 correction (`{name}` set to TRUE) is supported only for the FIFO `DATA_WIDTH` of 72"
            ));
        }
    }
    let (empty, full) = offset_ranges(width, sync, fwft);
    for (name, rule, (least, most)) in [
        ("ALMOST_EMPTY_OFFSET", "FIFO_AlmostRange_empty", empty),
        ("ALMOST_FULL_OFFSET", "FIFO_AlmostRange_full", full),
    ] {
        let value = int(params, name, 0x80)?;
        if value < least || value > most {
            return Err(format!(
                "Vivado's design rule `{rule}`: `{name}` value of {value} is out of range {least} \
                 to {most}"
            ));
        }
    }
    let half = if wide {
        match width {
            4 => 2,
            other => other / 2,
        }
    } else {
        width
    };
    Ok(Shape {
        wide,
        width,
        half,
        sdp: half == 36,
        sync,
        fwft,
        registered,
    })
}

/// The tile's flag offsets, as Vivado writes them: see the module docs.
fn offset_features(
    params: &dyn Fn(&str) -> Option<AttrValue>,
    shape: Shape,
) -> Result<Vec<String>, String> {
    let empty = int(params, "ALMOST_EMPTY_OFFSET", 0x80)? - i64::from(shape.fwft);
    let full = int(params, "ALMOST_FULL_OFFSET", 0x80)? + i64::from(!shape.sync);
    let mut out = Vec::new();
    for (name, value) in [
        ("ZALMOST_EMPTY_OFFSET", empty),
        ("ZALMOST_FULL_OFFSET", full),
    ] {
        for bit in 0..13 {
            if (value >> bit) & 1 == 0 {
                out.push(format!("{name}[{bit}]"));
            }
        }
    }
    Ok(out)
}

/// What one register bit of a half's `INIT_A`, `INIT_B`, `SRVAL_A` or
/// `SRVAL_B` holds: bit `k` of the cell's `INIT` (or `SRVAL`), or a constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Bit(u32),
    Zero,
}

/// Where bit `j` (0 to 17) of a half's port-`port` register comes from, for
/// this shape: **Vivado's**, solved bit by bit from its builds of random values
/// (`testdata/fpga/vivado/fifo/values.txt`, which
/// `the_initial_values_are_vivados` holds this to, every bit):
///
/// - **36 or 72 wide** (`SDP`): a half's 36 bits are port A's 18 and then port
///   B's, `INIT[17:0]` and `INIT[35:18]` of a `FIFO18E1`, and of a `FIFO36E1`
///   the even bits of `INIT` for the lower half and the odd for the upper.
/// - **Narrower**, port B holds `INIT[0]` in its bit 0 and nothing else, on
///   both halves.
/// - A narrower `FIFO18E1`'s port A is `INIT[17:0]`, but 9 wide its bits 8 and
///   16 are the other way round: `INIT[8]` is the word's parity bit, and the
///   register's parity bit is its bit 16.
/// - A narrower `FIFO36E1`'s halves are its word's pins: data bit `i` is bit
///   `i / 2` of the half `i` is even or odd for, parity bit `k` the half's
///   parity bit `k / 2` (bit `16 + k / 2`), and the bits of `INIT` past the
///   word fill each half's other bits in order, the even ones the lower
///   half's.
fn register_source(shape: Shape, upper: bool, port: usize, j: u32) -> Source {
    if shape.sdp {
        let at = j + 18 * u32::try_from(port).unwrap_or(0);
        return Source::Bit(if shape.wide {
            2 * at + u32::from(upper)
        } else {
            at
        });
    }
    if port == 1 {
        return if j == 0 { Source::Bit(0) } else { Source::Zero };
    }
    if !shape.wide {
        return Source::Bit(match (shape.width, j) {
            (9, 8) => 16,
            (9, 16) => 8,
            _ => j,
        });
    }
    // The word's pins first, then what is past it, half by half.
    let width = u32::try_from(shape.width).unwrap_or(0);
    let data = match width {
        9 => 8,
        18 => 16,
        36 => 32,
        other => other,
    };
    let mut register: [Option<u32>; 18] = [None; 18];
    let half = u32::from(upper);
    for i in (0..data).filter(|i| i % 2 == half) {
        register[(i / 2) as usize] = Some(i);
    }
    for k in (0..width - data).filter(|k| k % 2 == half) {
        register[(16 + k / 2) as usize] = Some(data + k);
    }
    let mut past = (width..72).filter(|i| i % 2 == half);
    for slot in register.iter_mut().filter(|s| s.is_none()) {
        *slot = past.next();
    }
    register[j as usize].map_or(Source::Zero, Source::Bit)
}

/// Which bit of `INIT` (and of `SRVAL`) bit `j` of port `port`'s register of
/// a half is, the lower half or the `upper`, at this shape: `Some(k)` for bit
/// `k`, `None` for a bit held at zero, as `values.txt` has it.
#[must_use]
pub fn register_bit(shape: Shape, upper: bool, port: usize, j: u32) -> Option<u32> {
    match register_source(shape, upper, port, j) {
        Source::Bit(k) => Some(k),
        Source::Zero => None,
    }
}

/// `INIT_A`, `INIT_B`, `SRVAL_A` and `SRVAL_B` of one half, as the block RAM
/// features want them, from the cell's `INIT` and `SRVAL`.
fn register_values(
    params: &dyn Fn(&str) -> Option<AttrValue>,
    shape: Shape,
    upper: bool,
) -> Vec<(String, AttrValue)> {
    let mut out = Vec::new();
    for param in ["INIT", "SRVAL"] {
        let value = params(param);
        for (port, suffix) in ["_A", "_B"].into_iter().enumerate() {
            let bits: Vec<Bit> = (0..18)
                .map(|j| match register_source(shape, upper, port, j) {
                    Source::Bit(k) if crate::fpga::bitstream::param_bit(value.as_ref(), k) => {
                        Bit::One
                    }
                    _ => Bit::Zero,
                })
                .collect();
            out.push((
                format!("{param}{suffix}"),
                AttrValue::Const(Const::from_bits(&bits)),
            ));
        }
    }
    out
}

/// The cell's inverter parameter behind a block RAM site pin's `ZINV`.
fn inverter_of(pin: &str) -> Option<&'static str> {
    Some(match pin {
        "CLKARDCLK" | "REGCLKARDRCLK" => "IS_RDCLK_INVERTED",
        "CLKBWRCLK" => "IS_WRCLK_INVERTED",
        "ENARDEN" => "IS_RDEN_INVERTED",
        "ENBWREN" => "IS_WREN_INVERTED",
        "RSTRAMARSTRAM" => "IS_RST_INVERTED",
        "RSTREGARSTREG" => "IS_RSTREG_INVERTED",
        _ => return None,
    })
}

/// The `RAMB18E1` parameters one half of a FIFO is, which
/// [`super::bram::mode_features_in`] turns into the half's features: the
/// widths, both output registers on `DO_REG`, both write modes `NO_CHANGE`,
/// the inverters by the FIFO's own names, and the registers' values.
pub(super) fn half_params(
    params: &dyn Fn(&str) -> Option<AttrValue>,
    shape: Shape,
    upper: bool,
    name: &str,
) -> Option<AttrValue> {
    let width = |w: i64| Some(AttrValue::Int(w));
    match name {
        "RAM_MODE" => Some(AttrValue::String(
            if shape.sdp { "SDP" } else { "TDP" }.to_owned(),
        )),
        "READ_WIDTH_A" => width(shape.half),
        "WRITE_WIDTH_B" => width(shape.half),
        "READ_WIDTH_B" | "WRITE_WIDTH_A" => None,
        "DOA_REG" | "DOB_REG" => Some(AttrValue::Int(i64::from(shape.registered))),
        "WRITE_MODE_A" | "WRITE_MODE_B" => Some(AttrValue::String("NO_CHANGE".to_owned())),
        "INIT_A" | "INIT_B" | "SRVAL_A" | "SRVAL_B" => register_values(params, shape, upper)
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v),
        _ => {
            let pin = name.strip_prefix("IS_")?.strip_suffix("_INVERTED")?;
            params(inverter_of(pin)?)
        }
    }
}

/// The site pins whose `ZINV` a FIFO never sets: port B's resets and output
/// register's clock, which a FIFO's site does not use.
const NOT_A_FIFOS: [&str; 3] = ["ZINV_REGCLKB", "ZINV_RSTRAMB", "ZINV_RSTREGB"];

/// The features a FIFO with these parameters turns on, each with its prefix
/// (`RAMB18_Y0.FIFO_MODE`, `RAMB36.EN_ECC_READ`) or none for the tile's own
/// (`EN_SYN`, `ZALMOST_FULL_OFFSET[3]`, `FIFO_WIDTH[1]`). `held` is
/// `xray::bram`'s list of the lower half's pins held low through their
/// inverter; the upper half of a `FIFO36E1` holds its enables and its reset so
/// too, always.
///
/// # Errors
///
/// [`check`]'s refusals.
pub fn features(
    primitive: &str,
    params: &dyn Fn(&str) -> Option<AttrValue>,
    held: &[&str],
) -> Result<Vec<String>, String> {
    let shape = check(primitive, params)?;
    let mut out = Vec::new();
    let halves: &[(&str, bool)] = if shape.wide {
        &[("RAMB18_Y0", false), ("RAMB18_Y1", true)]
    } else {
        &[("RAMB18_Y0", false)]
    };
    for (prefix, upper) in halves {
        let view = |name: &str| half_params(params, shape, *upper, name);
        let mut held_here: Vec<&str> = held.to_vec();
        if *upper {
            held_here.extend(["ENARDEN", "ENBWREN", "RSTRAMARSTRAM"]);
        }
        let mut here =
            super::bram::mode_features_in(&view, super::LutRamModel::Vivado, &held_here)?;
        here.retain(|f| !NOT_A_FIFOS.contains(&f.as_str()));
        here.push("FIFO_MODE".to_owned());
        if shape.sdp {
            here.push("READ_WIDTH_A_18".to_owned());
        }
        out.extend(here.into_iter().map(|f| format!("{prefix}.{f}")));
    }
    if shape.wide && shape.width == 9 {
        for name in [
            "READ_WIDTH_A",
            "READ_WIDTH_B",
            "WRITE_WIDTH_A",
            "WRITE_WIDTH_B",
        ] {
            out.push(format!("RAMB36.BRAM36_{name}_1"));
        }
    }
    for name in ["EN_ECC_READ", "EN_ECC_WRITE"] {
        if shape.wide && flag(params, name, false)? {
            out.push(format!("RAMB36.{name}"));
        }
    }
    if shape.sync {
        out.push("EN_SYN".to_owned());
    }
    if shape.fwft {
        out.push("FIRST_WORD_FALL_THROUGH".to_owned());
    }
    out.extend(offset_features(params, shape)?);
    let code = width_code(shape.half).unwrap_or(0);
    for bit in 0..3 {
        if (code >> bit) & 1 == 1 {
            out.push(format!("FIFO_WIDTH[{bit}]"));
        }
    }
    Ok(out)
}

/// The site roles a FIFO's data input role reaches at this shape, as Vivado
/// wires them; empty for a bit no site pin takes. `fifo_di<i>` and
/// `fifo_dip<i>` are the cell's `DI[i]` and `DIP[i]`.
///
/// **Vivado's**, every bit of every width (`get_site_pins` of each cell pin):
///
/// | cell, width | `DI` | `DIP` |
/// |---|---|---|
/// | `FIFO18E1`, 18 or less | `[15:0]` on `DIBDI`, the rest nothing | `[1:0]` on `DIPBDIP`, the rest nothing |
/// | `FIFO18E1`, 36 | `[15:0]` on `DIADI`, `[31:16]` on `DIBDI` | `[1:0]` on `DIPADIP`, `[3:2]` on `DIPBDIP` |
/// | `FIFO36E1`, 36 or less | `[31:0]` on `DIBDI` | `[3:0]` on `DIPBDIP`; 9 wide, `DIP[0]` on both halves' first and `DIP[1]` nothing |
/// | `FIFO36E1`, 72 | `[31:0]` on `DIADI`, `[63:32]` on `DIBDI` | `[3:0]` on `DIPADIP`, `[7:4]` on `DIPBDIP` |
///
/// A `FIFO36E1`'s `DIBDI<j>` is the even half's for an even `j`, as a
/// `RAMB36E1`'s is (`bram::ramb36_pins`).
#[must_use]
pub fn input_roles(shape: Shape, role: &str) -> Vec<String> {
    let (parity, bit) = match role.strip_prefix("fifo_dip") {
        Some(bit) => (true, bit),
        None => match role.strip_prefix("fifo_di") {
            Some(bit) => (false, bit),
            None => return vec![role.to_owned()],
        },
    };
    let Ok(bit) = bit.parse::<u32>() else {
        return Vec::new();
    };
    let (per_port, kind) = match (shape.wide, parity) {
        (false, false) => (16, "din"),
        (false, true) => (2, "dinp"),
        (true, false) => (32, "din"),
        (true, true) => (4, "dinp"),
    };
    if shape.sdp {
        // Port A first, then port B.
        return match bit.checked_sub(per_port) {
            None => vec![format!("p0_{kind}{bit}")],
            Some(b) if b < per_port => vec![format!("p1_{kind}{b}")],
            Some(_) => Vec::new(),
        };
    }
    if bit >= per_port {
        return Vec::new();
    }
    if shape.wide && parity && shape.width == 9 {
        return match bit {
            0 => vec!["p1_dinp0".to_owned(), "p1_dinp1".to_owned()],
            1 => Vec::new(),
            b => vec![format!("p1_dinp{b}")],
        };
    }
    vec![format!("p1_{kind}{bit}")]
}

/// The pins of a tile's `FIFO18E1` bel, as `(role, tile wire)`, or `None` for
/// a tile that is not a block RAM's: the lower half's site pins
/// (`BRAM_FIFO18_*`), named as a `RAMB18E1`'s roles are where a FIFO uses the
/// same pin (`p0_clk` is `RDCLK`, which is the site's `CLKARDCLK`), with the
/// FIFO's own beside them.
///
/// Read port A, write port B: `RDCLK`, `RDEN`, `RST`, `RSTREG`, `REGCE` and the
/// output register's clock on port A's pins, `WRCLK` and `WREN` on port B's;
/// `DO[15:0]` on `DOADO` and `DO[31:16]` on `DOBDO`, the parity likewise; the
/// flags and counts on pins of their own (`EMPTY`, `RDCOUNT0`). The cell's data
/// inputs are `fifo_di<i>` and `fifo_dip<i>` on the site's port B, which
/// [`input_roles`] moves to where the width puts them before anything routes.
pub(super) fn fifo18_pins(tile_type: &str) -> Option<Vec<(String, String)>> {
    if tile_type != "BRAM_L" && tile_type != "BRAM_R" {
        return None;
    }
    let mut out = Vec::new();
    let mut pin =
        |role: String, site_pin: &str| out.push((role, format!("BRAM_FIFO18_{site_pin}")));
    for (role, site_pin) in [
        ("p0_clk", "CLKARDCLK"),
        ("p1_clk", "CLKBWRCLK"),
        ("p0_regclk", "REGCLKARDRCLK"),
        ("p0_en", "ENARDEN"),
        ("p1_en", "ENBWREN"),
        ("p0_rst", "RSTRAMARSTRAM"),
        ("p0_rstreg", "RSTREGARSTREG"),
        ("p0_regce", "REGCEAREGCE"),
        ("fifo_almostempty", "ALMOSTEMPTY"),
        ("fifo_almostfull", "ALMOSTFULL"),
        ("fifo_empty", "EMPTY"),
        ("fifo_full", "FULL"),
        ("fifo_rderr", "RDERR"),
        ("fifo_wrerr", "WRERR"),
    ] {
        pin(role.to_owned(), site_pin);
    }
    for i in 0..16 {
        pin(format!("p0_din{i}"), &format!("DIADI{i}"));
        pin(format!("p1_din{i}"), &format!("DIBDI{i}"));
        pin(format!("p0_dout{i}"), &format!("DOADO{i}"));
        pin(format!("p0_dout{}", i + 16), &format!("DOBDO{i}"));
        pin(format!("fifo_di{i}"), &format!("DIBDI{i}"));
        pin(format!("fifo_di{}", i + 16), &format!("DIADI{i}"));
    }
    for i in 0..2 {
        pin(format!("p0_dinp{i}"), &format!("DIPADIP{i}"));
        pin(format!("p1_dinp{i}"), &format!("DIPBDIP{i}"));
        pin(format!("p0_doutp{i}"), &format!("DOPADOP{i}"));
        pin(format!("p0_doutp{}", i + 2), &format!("DOPBDOP{i}"));
        pin(format!("fifo_dip{i}"), &format!("DIPBDIP{i}"));
        pin(format!("fifo_dip{}", i + 2), &format!("DIPADIP{i}"));
    }
    for i in 0..12 {
        pin(format!("fifo_rdcount{i}"), &format!("RDCOUNT{i}"));
        pin(format!("fifo_wrcount{i}"), &format!("WRCOUNT{i}"));
    }
    Some(out)
}

/// The pins of a tile's `FIFO36E1` bel, as [`fifo18_pins`] has a `FIFO18E1`'s,
/// on the site's `BRAM_FIFO36_*` pins: a clock reaches both halves
/// (`CLKARDCLKL`, `CLKARDCLKU`), and so do `RSTREG`, `REGCE` and the output
/// register's clock; `RDEN`, `WREN` and `RST` the lower half alone
/// (`ENARDENL`, `ENBWRENL`, `RSTRAMARSTRAMLRST`), Vivado holding the upper
/// half's enables (`p0_enu`, `p1_enu`) low through their inverters and leaving
/// its reset alone; a word's even bits on the lower half and its odd bits on
/// the upper, as a `RAMB36E1`'s; and the error correction's pins.
pub(super) fn fifo36_pins(tile_type: &str) -> Option<Vec<(String, String)>> {
    let ramb36 = super::bram::ramb36_pins(tile_type)?;
    let wire_of = |role: &str| -> Vec<String> {
        ramb36
            .iter()
            .filter(|(r, _)| r == role)
            .map(|(_, w)| w.clone())
            .collect()
    };
    let mut out: Vec<(String, String)> = Vec::new();
    let mut take = |role: String, from: &str| {
        for wire in wire_of(from) {
            out.push((role.clone(), wire));
        }
    };
    for role in ["p0_clk", "p1_clk", "p0_regclk", "p0_rstreg", "p0_regce"] {
        take(role.to_owned(), role);
    }
    for i in 0..32 {
        take(format!("p0_din{i}"), &format!("p0_din{i}"));
        take(format!("p1_din{i}"), &format!("p1_din{i}"));
        take(format!("p0_dout{i}"), &format!("p0_dout{i}"));
        take(format!("p0_dout{}", i + 32), &format!("p1_dout{i}"));
        take(format!("fifo_di{i}"), &format!("p1_din{i}"));
        take(format!("fifo_di{}", i + 32), &format!("p0_din{i}"));
    }
    for i in 0..4 {
        take(format!("p0_dinp{i}"), &format!("p0_dinp{i}"));
        take(format!("p1_dinp{i}"), &format!("p1_dinp{i}"));
        take(format!("p0_doutp{i}"), &format!("p0_doutp{i}"));
        take(format!("p0_doutp{}", i + 4), &format!("p1_doutp{i}"));
        take(format!("fifo_dip{i}"), &format!("p1_dinp{i}"));
        take(format!("fifo_dip{}", i + 4), &format!("p0_dinp{i}"));
    }
    let mut pin =
        |role: String, site_pin: &str| out.push((role, format!("BRAM_FIFO36_{site_pin}")));
    for (role, site_pin) in [
        ("p0_en", "ENARDENL"),
        ("p1_en", "ENBWRENL"),
        ("p0_enu", "ENARDENU"),
        ("p1_enu", "ENBWRENU"),
        ("p0_rst", "RSTRAMARSTRAMLRST"),
        ("p0_injectdbiterr", "INJECTDBITERR"),
        ("p0_injectsbiterr", "INJECTSBITERR"),
        ("p0_dbiterr", "DBITERR"),
        ("p0_sbiterr", "SBITERR"),
        ("fifo_almostempty", "ALMOSTEMPTY"),
        ("fifo_almostfull", "ALMOSTFULL"),
        ("fifo_empty", "EMPTY"),
        ("fifo_full", "FULL"),
        ("fifo_rderr", "RDERR"),
        ("fifo_wrerr", "WRERR"),
    ] {
        pin(role.to_owned(), site_pin);
    }
    for i in 0..8 {
        pin(format!("p0_eccparity{i}"), &format!("ECCPARITY{i}"));
    }
    for i in 0..13 {
        pin(format!("fifo_rdcount{i}"), &format!("RDCOUNT{i}"));
        pin(format!("fifo_wrcount{i}"), &format!("WRCOUNT{i}"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params<'a>(list: &'a [(&'a str, AttrValue)]) -> impl Fn(&str) -> Option<AttrValue> + 'a {
        move |name: &str| {
            list.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        }
    }

    fn s(text: &str) -> AttrValue {
        AttrValue::String(text.to_owned())
    }

    fn n(value: i64) -> AttrValue {
        AttrValue::Int(value)
    }

    /// Would catch a refusal Vivado makes going missing, or one appearing for
    /// a shape it builds.
    #[test]
    fn what_vivado_refuses_is_refused() {
        let check18 = |list: &[(&str, AttrValue)]| check("FIFO18E1", &params(list));
        let refused = |list: &[(&str, AttrValue)], rule: &str| {
            let message = check18(list).unwrap_err();
            assert!(message.contains(rule), "{message}");
        };
        refused(
            &[("DATA_WIDTH", n(18)), ("DO_REG", n(0))],
            "enum_EN_SYN_enum_DO_REG_1",
        );
        refused(
            &[
                ("EN_SYN", s("TRUE")),
                ("FIRST_WORD_FALL_THROUGH", s("TRUE")),
            ],
            "en_syn_and_fwft",
        );
        refused(
            &[("ALMOST_FULL_OFFSET", n(0x1fff))],
            "FIFO_AlmostRange_full",
        );
        refused(&[("ALMOST_EMPTY_OFFSET", n(4))], "FIFO_AlmostRange_empty");
        refused(&[("DATA_WIDTH", n(36))], "`DATA_WIDTH` is 36");
        refused(&[("EN_ECC_READ", s("TRUE"))], "no error correction");
        let ecc = check(
            "FIFO36E1",
            &params(&[("DATA_WIDTH", n(36)), ("EN_ECC_WRITE", s("TRUE"))]),
        );
        assert!(
            ecc.unwrap_err()
                .contains("enum_EN_ECC_WRITE_TRUE_enum_DATA_WIDTH_72")
        );
        // The defaults build: 4 wide, two clocks, registered.
        let shape = check18(&[]).unwrap();
        assert_eq!(
            (shape.width, shape.half, shape.sync, shape.registered),
            (4, 4, false, true)
        );
        let wide = check(
            "FIFO36E1",
            &params(&[("DATA_WIDTH", n(72)), ("FIFO_MODE", s("FIFO36_72"))]),
        )
        .unwrap();
        assert_eq!((wide.half, wide.sdp), (36, true));
    }

    /// The offsets as Vivado wrote them for `f18_offsets` (asked 0x0b6 and
    /// 0x3c5, written 0x0b6 and 0x3c6) and `f18_fwft` (0x080 empty written
    /// 0x07f). Would catch the one added or taken away on the wrong side, or a
    /// bit not inverted.
    #[test]
    fn the_offsets_move_by_one_as_vivados_do() {
        let written = |list: &[(&str, AttrValue)], name: &str| -> u32 {
            let p = params(list);
            let shape = check("FIFO18E1", &p).unwrap();
            let mut value = 0x1fff;
            for f in offset_features(&p, shape).unwrap() {
                if let Some(bit) = f.strip_prefix(&format!("{name}[")) {
                    value &= !(1 << bit.trim_end_matches(']').parse::<u32>().unwrap());
                }
            }
            value
        };
        let asked = [
            ("DATA_WIDTH", n(18)),
            ("ALMOST_EMPTY_OFFSET", n(0x0b6)),
            ("ALMOST_FULL_OFFSET", n(0x3c5)),
        ];
        assert_eq!(written(&asked, "ZALMOST_EMPTY_OFFSET"), 0x0b6);
        assert_eq!(written(&asked, "ZALMOST_FULL_OFFSET"), 0x3c6);
        let fwft = [
            ("DATA_WIDTH", n(18)),
            ("FIRST_WORD_FALL_THROUGH", s("TRUE")),
        ];
        assert_eq!(written(&fwft, "ZALMOST_EMPTY_OFFSET"), 0x07f);
        let sync = [
            ("DATA_WIDTH", n(18)),
            ("EN_SYN", s("TRUE")),
            ("DO_REG", n(0)),
        ];
        assert_eq!(written(&sync, "ZALMOST_FULL_OFFSET"), 0x080);
    }

    /// Would catch a width's data on the other port, a narrow `FIFO36E1`'s
    /// parity bit on one half only, or a bit past the width reaching a pin.
    #[test]
    fn the_data_inputs_are_where_the_width_puts_them() {
        let shape = |cell: &str, width: i64, mode: &str| {
            check(
                cell,
                &params(&[("DATA_WIDTH", n(width)), ("FIFO_MODE", s(mode))]),
            )
            .unwrap()
        };
        let f18 = shape("FIFO18E1", 18, "FIFO18");
        assert_eq!(input_roles(f18, "fifo_di3"), ["p1_din3"]);
        assert!(input_roles(f18, "fifo_di16").is_empty());
        assert!(input_roles(f18, "fifo_dip2").is_empty());
        let f18w = shape("FIFO18E1", 36, "FIFO18_36");
        assert_eq!(input_roles(f18w, "fifo_di3"), ["p0_din3"]);
        assert_eq!(input_roles(f18w, "fifo_di19"), ["p1_din3"]);
        assert_eq!(input_roles(f18w, "fifo_dip3"), ["p1_dinp1"]);
        let f36 = shape("FIFO36E1", 9, "FIFO36");
        assert_eq!(input_roles(f36, "fifo_dip0"), ["p1_dinp0", "p1_dinp1"]);
        assert!(input_roles(f36, "fifo_dip1").is_empty());
        assert_eq!(input_roles(f36, "fifo_di31"), ["p1_din31"]);
        let f72 = shape("FIFO36E1", 72, "FIFO36_72");
        assert_eq!(input_roles(f72, "fifo_di40"), ["p1_din8"]);
        assert_eq!(input_roles(f72, "fifo_dip5"), ["p1_dinp1"]);
        assert_eq!(input_roles(f72, "p0_clk"), ["p0_clk"]);
    }

    /// The width code and the three bits that hold it. Would catch a code off
    /// by one, or the bits moved.
    #[test]
    fn the_width_bits_are_frame_27() {
        assert_eq!(
            [2, 4, 9, 18, 36].map(width_code),
            [Some(0), Some(1), Some(2), Some(3), Some(4)]
        );
        let bits = derived_features("BRAM_L");
        assert_eq!(bits.len(), 3);
        assert_eq!(bits[2].name, "FIFO_WIDTH[2]");
        assert_eq!(bits[2].ones, [ConfigBit::new(27, 174)]);
        assert!(derived_features("CLBLL_L").is_empty());
    }
}
