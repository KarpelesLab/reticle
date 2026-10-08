//! The inside of a 7-series site: which tile wire each bel pin reaches,
//! which site a feature's prefix means, and what a fixed path *through*
//! a site costs in bits.
//!
//! # Why any of this is written by hand
//!
//! `prjxray-db` ships three kinds of file that between them almost say
//! everything: `segbits_<type>.db` (which bits make a feature true),
//! `ppips_<type>.db` (which connections inside a tile are not
//! programmable) and `tileconn.json` (which wire of a tile is the same
//! metal as which wire of its neighbour). What none of them says is the
//! **name of a site pin** — `A1`, `O6`, `I`, `O` — or which tile wire
//! carries it. prjxray keeps that in `tile_type_*.json`, which is
//! generated from Vivado and is not in the repository.
//!
//! So three things are supplied here, and each is labelled with where it
//! came from:
//!
//! 1. **Pin names**, from Xilinx's public user guides: UG474 *7 Series
//!    FPGAs Configurable Logic Block* for `SLICEL`'s `A1`..`A6` and `A`,
//!    and UG471 *7 Series FPGAs SelectIO Resources* for the IO buffer's
//!    `I` / `O` and the `ILOGICE3` / `OLOGICE3` data path.
//! 2. **The tile wire each pin sits on**, *read off* `ppips_<type>.db`:
//!    the line `CLBLL_L.CLBLL_L_A1.CLBLL_IMUX6 always` says the wire
//!    `CLBLL_L_A1` is fed, with no configuration, from `CLBLL_IMUX6`,
//!    and `CLBLL_L_A1` is therefore the metal at a slice's `A1` pin.
//!    The wire *names* are the database's; only the rule that maps a
//!    pin name onto one is written here.
//! 3. **Which site of a tile a feature prefix means**, derived and then
//!    measured — see [`site_of_prefix`].
//!
//! Nothing here invents a bit position. Every bit still comes from
//! `segbits_<type>.db`; this module only says which feature to look up.
//!
//! # What has been tried on a part
//!
//! The IO recipe below is a transcription of what Vivado itself did for
//! this exact board — see [`IO_STANDARDS`] — not a reading of a
//! datasheet. On 2026-09-24 it lit an LED: a lookup table and three
//! LVCMOS33 pins on a Basys 3, watched working.
//!
//! That is one IO standard, on one package, driving one output. Every
//! other entry here is still a transcription nothing has tested.
//!
//! The clock tables — [`horizontal_clock_buffers`], [`extra_bels`]'s
//! `BUFGCTRL`, [`wire_enable_features`] and [`global_clock_enable`] —
//! carried a clock from a pad to twenty-six flip-flops in a bitstream the
//! same board accepted with `DONE` high on the same day, and every
//! feature they put on the pin, the backbone hop and the global buffer is
//! the one Vivado put there for the same pin. A person watched that
//! design's LED blink at the rate it was written for, so on that one pin
//! and that one buffer these tables are an effect, not only a match.

use std::collections::BTreeMap;

use super::XrayTile;

// ---------------------------------------------------------------------------
// Which site of a tile a feature prefix names
// ---------------------------------------------------------------------------

/// The axis a site prefix's index counts along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    /// `SLICEL_X0` — counts left to right, the same way a site's own `X`
    /// coordinate does.
    X,
    /// `IOB_Y0` — counts *down the tile grid*, which is the **opposite**
    /// way from a site's own `Y` coordinate.
    Y,
}

/// Splits `SLICEL_X0` into `("SLICEL", Axis::X, 0)`.
///
/// A prefix with no such suffix — `LIOB33`'s `DIFF`, which describes the
/// differential pair rather than a site — names no site and gives
/// `None`.
fn split_prefix(prefix: &str) -> Option<(&str, Axis, u32)> {
    let cut = prefix.rfind(['X', 'Y'])?;
    let (base, tail) = prefix.split_at(cut);
    let base = base.strip_suffix('_')?;
    let axis = match &tail[..1] {
        "X" => Axis::X,
        _ => Axis::Y,
    };
    let index: u32 = tail[1..].parse().ok()?;
    if base.is_empty() {
        return None;
    }
    Some((base, axis, index))
}

/// The index a site prefix carries: `IOB_Y1` is 1.
pub(super) fn prefix_index(prefix: &str) -> Option<u32> {
    split_prefix(prefix).map(|(_, _, index)| index)
}

/// The `(x, y)` a site name ends in: `SLICE_X10Y0` is `(10, 0)`.
fn site_coordinate(name: &str) -> Option<(u32, u32)> {
    let (_, tail) = name.rsplit_once('_')?;
    let tail = tail.strip_prefix('X')?;
    let (x, y) = tail.split_once('Y')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// The part of a site name before its coordinates: `SLICE_X10Y0` is
/// `SLICE`, `IOB_X0Y11` is `IOB`.
fn site_family(name: &str) -> &str {
    name.rsplit_once('_').map_or(name, |(head, _)| head)
}

/// Which of a tile's sites the feature prefix `prefix` describes.
///
/// # The rule, and how it was arrived at
///
/// A prefix is `<BASE>_<AXIS><index>`, and the index is the site's
/// position **within its tile**, counted over every site of the same
/// family. The family is the part of a site's name before its
/// coordinates: `SLICE`, `IOB`, `ILOGIC`. `BASE` is the site's *type*,
/// which is not always the family — a `CLBLM_L` holds `SLICE_X10Y0` of
/// type `SLICEM` and `SLICE_X11Y0` of type `SLICEL`, and its prefixes
/// are `SLICEM_X0` and `SLICEL_X1`: one family, two types, positions 0
/// and 1.
///
/// So the family is found from the type where the tile has one of that
/// type (`SLICEM` → `SLICE`) and taken to be `BASE` itself where it does
/// not (`IOB_Y0`, whose sites are typed `IOB33S` and `IOB33M`). Every
/// site of that family is then ranked along the axis and `index` picks
/// one; where the type was known it is checked, so a tile that breaks
/// the assumption gives nothing rather than the wrong site.
///
/// **`X` ranks ascending and `Y` ranks descending.** The `X` direction
/// follows from `CLBLM_L`, where the type settles which slice is which:
/// its `SLICEM_X0` is `SLICE_X10Y0`, the *lower* `X`. The `Y` direction
/// is the surprising one and is **measured**: over the four Vivado
/// harness designs in `artix7/harness/`, ten IO tiles configure exactly
/// one of their two halves, and in all ten the half named `_Y0` is the
/// one serving the *higher* site `Y` — prjxray numbers a bel by its
/// position down the tile as the grid draws it, and the grid's rows run
/// opposite to a site's `Y`. `tests/fpga_xray.rs` re-measures it against
/// the database rather than trusting this paragraph.
pub(super) fn site_of_prefix<'a>(tile: &'a XrayTile, prefix: &str) -> Option<&'a (String, String)> {
    let (base, axis, index) = split_prefix(prefix)?;

    let typed = tile.sites.iter().find(|(_, kind)| kind == base);
    let family = typed.map_or(base, |(name, _)| site_family(name));
    let mut candidates: Vec<&(String, String)> = tile
        .sites
        .iter()
        .filter(|(name, _)| site_family(name) == family)
        .collect();
    if candidates.is_empty() {
        return None;
    }

    // A block RAM tile's two 18 kbit halves are named by the **parity**
    // of the site's `Y`, not by rank. See [`block_ram_half`] for why that
    // is not the same thing as the `IOB` rule below, and where it is from.
    if base == "RAMB18" {
        return block_ram_half(&candidates, index);
    }

    // A tile whose bits are aliased onto another type is named in *that*
    // type's vocabulary, so a `LIOB33_SING` holding one IOB can be
    // described by `IOB_Y1` — rank 1 of a tile that has one site. The
    // index cannot be a rank there, and there is nothing to choose
    // between: the one site of the family is the one meant. Which half
    // it is is settled by the alias and not by this function; see
    // `super::TileAlias`.
    if tile.bits_alias.is_some() && candidates.len() == 1 {
        return candidates.first().copied();
    }

    match axis {
        Axis::X => candidates.sort_by_key(|(name, _)| site_coordinate(name).map(|c| c.0)),
        Axis::Y => candidates
            .sort_by_key(|(name, _)| site_coordinate(name).map(|c| std::cmp::Reverse(c.1))),
    }
    let picked = candidates.get(index as usize).copied()?;
    if typed.is_some() && picked.1 != base {
        // The tile does hold a site of this type, but not at this
        // position: the numbering assumption above does not describe
        // this tile, and naming the wrong site would be worse than
        // naming none.
        return None;
    }
    Some(picked)
}

/// Which of a `BRAM_L` / `BRAM_R` tile's two `RAMB18_X<x>Y<y>` sites the
/// prefix `RAMB18_Y<index>` names: the one whose `Y` is **even** for
/// `RAMB18_Y0` and **odd** for `RAMB18_Y1`.
///
/// # Where this is from, and why the `IOB` rule would have been wrong
///
/// Quoted, not measured here: prjxray's `prjxray/segmaker.py` names the
/// tag of a `RAMB18` site with `name_bram18`, which is `RAMB18_Y0` when the
/// site name ends in an even digit and `RAMB18_Y1` when it ends in an odd
/// one. Its `IOB`, `ILOGIC` and `OLOGIC` sites go through `name_y0y1`, which
/// is the same parity rule — and in an IO tile the even site is the
/// *higher* one, which is what [`site_of_prefix`]'s descending rank
/// measured against Vivado's harness bitstreams. So the rank rule is the
/// parity rule in disguise for an IO tile, and the disguise falls off in a
/// block RAM tile, where the even site (`RAMB18_X0Y0`, typed `FIFO18E1` in
/// `tilegrid.json`) is the **lower** one. Ranking descending would have
/// put `RAMB18_Y0`'s configuration on the upper half.
///
/// Corroborated by the database rather than only by the script: the
/// contents bits of `RAMB18_Y0` in `segbits_bram_l.block_ram.db` occupy
/// the low words of the tile's frame window (bits 0..175) and `RAMB18_Y1`
/// the high ones (176..319), and in every other tile type checked so far a
/// higher word is a higher site `Y` — `IOB_Y0`, the higher IO site, has
/// its bits at 64..127 against `IOB_Y1`'s 32..35. Nothing on a part has
/// confirmed it.
fn block_ram_half<'a>(
    candidates: &[&'a (String, String)],
    index: u32,
) -> Option<&'a (String, String)> {
    if index > 1 {
        return None;
    }
    let mut matching = candidates
        .iter()
        .filter(|(name, _)| site_coordinate(name).is_some_and(|(_, y)| y % 2 == index));
    let picked = matching.next().copied()?;
    // Two sites of one parity would mean the tile is not the shape this
    // rule describes; naming either would be a guess.
    if matching.next().is_some() {
        return None;
    }
    Some(picked)
}

// ---------------------------------------------------------------------------
// Bel pins
// ---------------------------------------------------------------------------

/// The wire prefix a tile type gives the slice at `index`.
///
/// A `CLBLM` settles this by itself: its `M` wires belong to the
/// `SLICEM`, which is the tile's `X`-index-0 site. A `CLBLL` holds two
/// `SLICEL`s and the letters cannot tell them apart, so the
/// interconnect settles it — `CLBLM_M_A1` and `CLBLL_LL_A1` are both fed
/// from interconnect index 7 and both drive `LOGIC_OUTS12`, while
/// `CLBLM_L_A1` and `CLBLL_L_A1` are both index 6 and `LOGIC_OUTS8`.
/// The same silicon position carries the same interconnect index, so
/// `CLBLL_LL` is the `X`-index-0 slice. `tests/fpga_xray.rs` re-derives
/// that from `ppips_*.db` so it cannot silently drift.
pub(super) fn slice_wire_prefix(tile_type: &str, index: u32) -> Option<&'static str> {
    let pair: [&'static str; 2] = match tile_type {
        "CLBLL_L" | "CLBLL_R" => ["CLBLL_LL", "CLBLL_L"],
        "CLBLM_L" | "CLBLM_R" => ["CLBLM_M", "CLBLM_L"],
        _ => return None,
    };
    pair.get(index as usize).copied()
}

/// The feature prefix the slice at `index` of a CLB tile type uses, which
/// is the other half of [`slice_wire_prefix`]: the wire prefix names the
/// metal, this names the bits.
///
/// A `CLBLM` holds a `SLICEM` at `X`-index 0 and a `SLICEL` at 1, so its
/// prefixes differ in the type rather than only in the index.
fn slice_feature_prefix(tile_type: &str, index: u32) -> Option<&'static str> {
    let pair: [&'static str; 2] = match tile_type {
        "CLBLL_L" | "CLBLL_R" => ["SLICEL_X0", "SLICEL_X1"],
        "CLBLM_L" | "CLBLM_R" => ["SLICEM_X0", "SLICEL_X1"],
        _ => return None,
    };
    pair.get(index as usize).copied()
}

/// The prefix a CLB tile type gives the wires it shares between its two
/// slices: `CLBLL_L` and `CLBLL_R` both call them `CLBLL_*`.
fn clb_shared_prefix(tile_type: &str) -> Option<&'static str> {
    match tile_type {
        "CLBLL_L" | "CLBLL_R" => Some("CLBLL"),
        "CLBLM_L" | "CLBLM_R" => Some("CLBLM"),
        _ => None,
    }
}

/// The four storage positions of a slice, in UG474's own order. Each one
/// has a lookup table (`ALUT`), a flip-flop (`AFF`), a bypass input
/// (`AX`), a direct output (`A`), a muxed output (`AMUX`) and a
/// flip-flop output (`AQ`).
const SLICE_LETTERS: [char; 4] = ['A', 'B', 'C', 'D'];

/// The name this module gives the wire inside a slice that carries the
/// output of the `<letter>FFMUX` — the mux that chooses what a
/// flip-flop's `D` reads.
///
/// **No such wire is in the database.** prjxray names tile wires, and
/// this net never leaves the site, so it has no name to take. It is
/// invented here so that the choice the mux makes can be a *pip* with
/// the mux's own bits on it, which is what lets a router pick between
/// the lookup table beside the flip-flop and the slice's bypass input
/// instead of the loader deciding for it. The `_MUX_` infix is not a
/// prefix any database wire uses, so it cannot collide with one.
fn ff_data_wire(wires: &str, letter: char) -> String {
    format!("{wires}_MUX_{letter}FF_D")
}

/// The slice position a flip-flop sub-element names: `AFF` is `A`.
///
/// The five-input flip-flops (`A5FF`) are deliberately **not** here.
/// They exist, but their data comes from a mux this module does not
/// describe and their output reaches the fabric only through the same
/// `AMUX` the main flip-flop and the carry chain want, so declaring them
/// would offer a placer eight sites per slice of which only four can be
/// wired. Four is the honest number.
fn slice_ff_letter(sub: &str) -> Option<char> {
    let letter = sub.strip_suffix("FF")?;
    let mut chars = letter.chars();
    let first = chars.next()?;
    if chars.next().is_some() || !SLICE_LETTERS.contains(&first) {
        return None;
    }
    Some(first)
}

/// The interconnect-side wires that feed the slice at `index` with its
/// clock, its clock enable and its set/reset, read off `ppips_<type>.db`:
/// `CLBLL_L.CLBLL_LL_CLK.CLBLL_CLK1 always` says the `X`-index-0 slice's
/// clock pin is fed from `CLBLL_CLK1`.
///
/// The pattern is the same in all four CLB tile types: the `X`-index-0
/// slice takes `CLK1` / `FAN7` / `CTRL1`, and the `X`-index-1 slice takes
/// `CLK0` / `FAN6` / `CTRL0`.
fn shared_control_sources(shared: &str, index: u32) -> (String, String, String) {
    let (clk, fan, ctrl) = if index == 0 { (1, 7, 1) } else { (0, 6, 0) };
    (
        format!("{shared}_CLK{clk}"),
        format!("{shared}_FAN{fan}"),
        format!("{shared}_CTRL{ctrl}"),
    )
}

/// One pin of a bel: the role the device database knows it by, and the
/// tile wire it reaches.
pub(super) struct BelPin {
    /// The role, as [`Device`](crate::fpga::Device)'s `port` lines spell
    /// it: `i0`, `o`, `din`, `dout`.
    pub role: &'static str,
    /// The tile wire, already resolved.
    pub wire: String,
}

/// The pins of the bel `<prefix>_<sub>` (or of `<prefix>` itself when
/// `sub` is empty) in a tile of type `tile_type`.
///
/// Returns nothing for a bel this module has no table for, which is most
/// of them: a flip-flop's `D` has no tile wire of its own (it is fed
/// from inside the slice), a `BUFGCTRL` needs the clock tree, and an
/// `IDELAYE2` is not in the milestone's path. A bel with no pins is
/// placeable and unroutable, which is what the loader did for every bel
/// before this module existed.
pub(super) fn bel_pins(tile_type: &str, prefix: &str, sub: &str) -> Vec<BelPin> {
    let Some((base, _, index)) = split_prefix(prefix) else {
        return Vec::new();
    };

    // A slice's lookup table. UG474 names the inputs A1..A6 and the
    // direct output A (B, C, D for the other three); Xilinx's INIT is
    // indexed by {A6..A1}, so Reticle's `i0` is A1 and `i5` is A6.
    if base == "SLICEL" || base == "SLICEM" {
        let Some(wires) = slice_wire_prefix(tile_type, index) else {
            return Vec::new();
        };
        if let Some(letter) = sub.strip_suffix("LUT") {
            if letter.len() != 1 {
                return Vec::new();
            }
            let mut pins = Vec::with_capacity(7);
            for i in 0..6u32 {
                pins.push(BelPin {
                    role: LUT_INPUT_ROLES[i as usize],
                    wire: format!("{wires}_{letter}{}", i + 1),
                });
            }
            pins.push(BelPin {
                role: "o",
                wire: format!("{wires}_{letter}"),
            });
            return pins;
        }
        // A slice's flip-flop. UG474 calls its pins D, CE, C(lock), SR
        // and Q; `xc7.dev` calls the same five `d`, `en`, `clk`, `rst`
        // and `q`. Four of the five sit on tile wires the database
        // names, and the clock, the clock enable and the set/reset are
        // **shared by all eight** storage elements of the slice, which
        // is why every flip-flop of a slice names the same three wires.
        // `D` is the exception: it is the output of the `<letter>FFMUX`
        // and never leaves the site, so it sits on the wire
        // [`ff_data_wire`] invents and [`pass_throughs`] feeds.
        if let Some(letter) = slice_ff_letter(sub) {
            return vec![
                BelPin {
                    role: "clk",
                    wire: format!("{wires}_CLK"),
                },
                BelPin {
                    role: "d",
                    wire: ff_data_wire(wires, letter),
                },
                BelPin {
                    role: "q",
                    wire: format!("{wires}_{letter}Q"),
                },
                BelPin {
                    role: "en",
                    wire: format!("{wires}_CE"),
                },
                BelPin {
                    role: "rst",
                    wire: format!("{wires}_SR"),
                },
            ];
        }
        if sub == "CARRY4" {
            return carry_pins(wires);
        }
        return Vec::new();
    }

    // An IO buffer. UG471 calls the fabric-side pins `O` (what the input
    // buffer drives) and `I` (what the output buffer drives the pad
    // from); `xc7.dev` calls the same two roles `din` and `dout`. The
    // pad itself is deliberately absent: it is a package ball, not a
    // wire the router can reach, and a design's port net ends there.
    //
    // The third pin is the tristate, UG471's `T`: what `xc7.dev` declares
    // as `oen=T` and `fpga::place` hands the graph as `oe`, because the
    // graph knows wires and not which way round a pin's logic is. Its
    // metal is `IOB_T<n>`, which `tileconn.json` joins to `LIOI_T<n>` /
    // `RIOI_T<n>` in the IO-logic tile beside it — the end of the `TQ`
    // path [`tristate_through`] declares. An `IBUF` or `OBUF` on the same
    // site leaves the pin unrouted, as Vivado leaves it.
    // A `_SING` tile holds one IOB where a `LIOB33` holds two, and its
    // wires are numbered **0** whichever half of the two-IOB tile the
    // database describes its bits with: `ppips_lioi3_sing.db` and
    // `tileconn.json`'s `LIOB33_SING`/`LIOI3_SING` pairs name
    // `IOB_IBUF0`, `IOB_O0` and `LIOI_IBUF0` and nothing with a 1 in it.
    // So the *feature* prefix may be `IOB_Y1` (see `super::TileAlias`)
    // while the *wire* is still `IOB_IBUF0`, and the index is not taken
    // from the prefix here.
    if base == "IOB" && matches!(tile_type, "LIOB33" | "RIOB33") {
        return iob_pins(index);
    }
    if base == "IOB" && matches!(tile_type, "LIOB33_SING" | "RIOB33_SING") {
        return iob_pins(0);
    }

    Vec::new()
}

/// The three fabric-side pins of the IO buffer whose wires carry
/// `index`. See [`bel_pins`] for what the pad's absence means.
fn iob_pins(index: u32) -> Vec<BelPin> {
    vec![
        BelPin {
            role: "din",
            wire: format!("IOB_IBUF{index}"),
        },
        BelPin {
            role: "dout",
            wire: format!("IOB_O{index}"),
        },
        BelPin {
            role: "oe",
            wire: format!("IOB_T{index}"),
        },
    ]
}

/// `i0`..`i5`, the roles `xc7.dev`'s `port i=I0,I1,I2,I3,I4,I5` produces.
const LUT_INPUT_ROLES: [&str; 6] = ["i0", "i1", "i2", "i3", "i4", "i5"];

// ---------------------------------------------------------------------------
// Fixed paths through a site
// ---------------------------------------------------------------------------

/// A connection straight through a site, and what it costs.
///
/// A pad does not reach the interconnect directly: it goes through the
/// `ILOGICE3` or `OLOGICE3` beside it, and those are sites, not wires.
/// prjxray records *some* of those hops in `ppips_<type>.db` as
/// unconditional connections and leaves others out, but either way the
/// hop is not free — using it turns bits on inside the site. So the hop
/// is declared here as a pip that carries the features it needs, and the
/// `ppips` loader is told to leave the same pair alone so the router
/// cannot take a free copy of it.
#[derive(Debug)]
pub(super) struct PassThrough {
    /// The wire the hop drives.
    pub to: String,
    /// The wire it reads.
    pub from: String,
    /// The features, in the tile type's own vocabulary, whose bits it
    /// costs.
    pub features: Vec<String>,
}

/// Every fixed path through a site that a tile of this type has.
///
/// Only the IO tiles have any today, and only the combinational data
/// path through them: an `ILOGICE3` passing its pad input to the
/// interconnect, and an `OLOGICE3` passing interconnect data to the pad.
/// The features are the ones Vivado's own bitstream for this board sets
/// on exactly those two paths and nothing else — see [`IO_STANDARDS`]
/// for how that was read.
pub(super) fn pass_throughs(tile_type: &str) -> Vec<PassThrough> {
    if clb_shared_prefix(tile_type).is_some() {
        return slice_pass_throughs(tile_type);
    }
    if matches!(tile_type, "CLK_HROW_BOT_R" | "CLK_HROW_TOP_R") {
        return horizontal_clock_buffers();
    }
    let (side, single) = match tile_type {
        "LIOI3" | "LIOI3_TBYTESRC" | "LIOI3_TBYTETERM" => ("LIOI", false),
        "RIOI3" | "RIOI3_TBYTESRC" | "RIOI3_TBYTETERM" => ("RIOI", false),
        "LIOI3_SING" => ("LIOI", true),
        "RIOI3_SING" => ("RIOI", true),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for n in 0..2u32 {
        // A `_SING` tile has one `ILOGICE3` and one `OLOGICE3` where a
        // `LIOI3` has two, and its wires are numbered 0 whichever half
        // of the two-row tile the database describes its bits with:
        // `ppips_lioi3_sing.db` names `IOI_OLOGIC0_D1` and
        // `LIOI_OLOGIC0_OQ` and nothing with a 1 in it. So both halves'
        // *features* are offered on the *same* wire pair and the one the
        // tile actually has survives — `build_arch` drops a hop whose
        // features its tile type does not hold, and `super::TileAlias`
        // has already cut the aliased feature set down to the one half
        // this tile is. The other hop is counted in
        // `pass_throughs_unresolved`, which is how a table that has
        // drifted from the database says so.
        let w = if single { 0 } else { n };
        // Pad -> fabric. `ZINV_D` is the ILOGIC's D input inverter held
        // off; it is the only bit Vivado sets on an otherwise default
        // input path, and it appears once per *used* input half.
        out.push(PassThrough {
            to: format!("IOI_ILOGIC{w}_O"),
            from: format!("{side}_ILOGIC{w}_D"),
            features: vec![format!("ILOGIC_Y{n}.ZINV_D")],
        });
        // Fabric -> pad. The OLOGIC's output mux takes D1, the OQ output
        // is in use, and the tristate path is a plain buffer.
        out.push(PassThrough {
            to: format!("{side}_OLOGIC{w}_OQ"),
            from: format!("IOI_OLOGIC{w}_D1"),
            features: vec![
                format!("OLOGIC_Y{n}.OMUX.D1"),
                format!("OLOGIC_Y{n}.OQUSED"),
                format!("OLOGIC_Y{n}.OSERDES.DATA_RATE_TQ.BUF"),
            ],
        });
        out.push(tristate_through(side, w, n));
    }
    out
}

/// The tristate's path through an `OLOGICE3`: fabric `T1` in, `TQ` out,
/// on to the pad's `T`. See [`pass_throughs`] for why a hop through a
/// site is declared here rather than taken from `ppips`.
///
/// # The two features, and what each one is for
///
/// `ppips_lioi3.db` records `LIOI_OLOGIC<n>_TQ` ← `IOI_OLOGIC<n>_T1` as
/// `always`, and it is no more free than the data hop beside it:
///
/// - **`OLOGIC_Y<n>.OSERDES.DATA_RATE_TQ.BUF`** chooses what `TQ` is: `T1`
///   passed straight through, rather than the `SDR` or `DDR` tristate
///   register. Its three values are one-hot in `segbits_lioi3.db`
///   (`32_66 !32_70 !33_69` is `BUF` on `Y0`), so a blank tile is *none*
///   of them. The data hop already sets it, because Vivado sets it on
///   every plain output; it is repeated here so that a tristate whose
///   data is not routed through the same site still gets its buffer.
///   Setting a bit twice sets it once.
/// - **`OLOGIC_Y<n>.ZINV_T1`** is the polarity, and the polarity is the
///   bug that matters. prjxray's `Z` prefix marks an inverted field:
///   `fuzzers/036-iob-ologic/generate.py` tags it as
///   `ZINV_T1 = 1 ^ IS_T1_INVERTED`, so the bit **set** is `T1` **not**
///   inverted and a blank tile inverts it. nextpnr-xilinx's
///   `xilinx/fasm.cc` puts exactly this one feature on exactly this
///   pseudo-pip (`IOI_OLOGIC<n>_T1` → `…_OLOGIC<n>_TQ`). With it set, a
///   one the fabric drives reaches the pad's `T` as a one, and a `T` of
///   one releases the pad — UG471's `OBUFT` truth table, and the sense
///   `xc7.dev`'s `oen=T` declares.
///
/// **Both are quoted, not measured.** No Vivado bitstream in
/// `artix7/harness/` has a tristate pin — every `OLOGIC` they configure
/// is a plain output — so no oracle here sets `ZINV_T1` at all. And the
/// one thing a board has said is compatible with either reading: an
/// `OBUF` leaves `ZINV_T1` clear and `T1` unrouted, and `sw_led`'s LED
/// lit, which fits "inverted, and an unrouted `T1` reads one" exactly as
/// well as "not inverted, and it reads zero". `examples/basys3/pmod_bidir.v`
/// is the design that tells the two apart on a part.
fn tristate_through(side: &str, wire: u32, n: u32) -> PassThrough {
    PassThrough {
        to: format!("{side}_OLOGIC{wire}_TQ"),
        from: format!("IOI_OLOGIC{wire}_T1"),
        features: vec![
            format!("OLOGIC_Y{n}.OSERDES.DATA_RATE_TQ.BUF"),
            format!("OLOGIC_Y{n}.ZINV_T1"),
        ],
    }
}

/// The paths through a slice that a clocked design needs.
///
/// Three kinds, and each is a mux whose bits the database names:
///
/// 1. **`<letter>FFMUX`** — what a flip-flop's `D` reads. Two of its six
///    sources are offered here: `O6`, the lookup table sitting beside
///    the flip-flop in the same slice, and `<letter>X`, the slice's
///    bypass input, which comes from the interconnect and therefore
///    works wherever the driver was placed. The router picks; `O6` costs
///    no interconnect at all when the placer happened to put the pair
///    together, and `<letter>X` always works. The other four sources
///    (`O5`, `XOR`, `CY`, `F7`/`F8`) read things inside the slice this
///    module does not yet describe.
/// 2. **`CEUSEDMUX`** and **`SRUSEDMUX`** — whether the slice's clock
///    enable and set/reset come from the fabric at all. A cleared bit
///    ties `CE` to one and `SR` to zero, which is what a flip-flop with
///    no enable and no reset wants, so the cost belongs on the hop that
///    brings the signal in rather than on the flip-flop. `ppips` records
///    both hops as unconditional and they are **not**: taking one
///    without its bit routes a reset to a pin that is tied off.
///
/// The clock hop (`CLBLL_LL_CLK` from `CLBLL_CLK1`) really is free and
/// is left to `ppips`.
///
/// **A slice's `FFSYNC`, `CLKINV` and `LATCH` are one field for all eight
/// of its storage elements**, and this model gives each flip-flop its own
/// bel, so two flip-flops of different kinds may be placed in one slice
/// and their mode bits will be OR-ed together rather than refused. The
/// device file's `shared_reset` says the same thing about the control
/// set. Until the placer packs a slice, a design that mixes `FDRE` with
/// `FDCE` can be built and will be wrong; `docs/fpga-xray.md` says so.
fn slice_pass_throughs(tile_type: &str) -> Vec<PassThrough> {
    let Some(shared) = clb_shared_prefix(tile_type) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for index in 0..2u32 {
        let (Some(wires), Some(prefix)) = (
            slice_wire_prefix(tile_type, index),
            slice_feature_prefix(tile_type, index),
        ) else {
            continue;
        };
        let (_, fan, ctrl) = shared_control_sources(shared, index);
        out.push(PassThrough {
            to: format!("{wires}_CE"),
            from: fan,
            features: vec![format!("{prefix}.CEUSEDMUX")],
        });
        out.push(PassThrough {
            to: format!("{wires}_SR"),
            from: ctrl,
            features: vec![format!("{prefix}.SRUSEDMUX")],
        });
        for letter in SLICE_LETTERS {
            let data = ff_data_wire(wires, letter);
            out.push(PassThrough {
                to: data.clone(),
                from: format!("{wires}_{letter}"),
                features: vec![format!("{prefix}.{letter}FFMUX.O6")],
            });
            out.push(PassThrough {
                to: data,
                from: format!("{wires}_{letter}X"),
                features: vec![format!("{prefix}.{letter}FFMUX.{letter}X")],
            });
        }
        out.extend(carry_pass_throughs(wires, prefix));
    }
    out
}

// ---------------------------------------------------------------------------
// The carry chain
// ---------------------------------------------------------------------------

/// The name this module gives a wire **inside** the carry chain of the
/// slice whose wires are prefixed `wires`: `CLBLL_LL_CARRY_XOR_A`.
///
/// Like [`ff_data_wire`], these are invented: the database names tile
/// wires, and the `CARRY4`'s sum, its per-bit carry out and the inputs
/// of its `PRECYINIT` mux never leave the slice. No
/// wire anywhere in `artix7/` contains `_CARRY_` (checked with a `grep`
/// over every `segbits`, `ppips` and `tileconn` file of the pinned
/// commit), so the names cannot collide with one.
fn carry_wire(wires: &str, what: &str) -> String {
    format!("{wires}_CARRY_{what}")
}

/// The pins of a slice's `CARRY4`, in the roles `xc7.dev`'s
/// `port ci=CI cyinit=CYINIT p=S di=DI s=O co=CO` produces.
///
/// # Where each one is
///
/// - **`p<n>` (`S`) is the lookup table's own output.** UG474 draws the
///   propagate input of each bit as the `O6` of the lookup table at the
///   same position, with no mux between them, and the database agrees:
///   `ppips_clbll_l.db` has no wire for `S` at all, only
///   `CLBLL_L_A.CLBLL_L_A1 hint` — the lookup table as a wire. So the pin
///   is declared on the wire `CLBLL_LL_A` itself, the same node as the
///   lookup table's `o`. That is the whole of the packing rule, stated as
///   metal: the router can reach that node from nowhere, so a propagate
///   signal arrives only when the cell driving it is the lookup table at
///   that position of that slice, and the placer reads the same fact off
///   the graph (see `place`'s dedicated connections).
/// - **`di<n>` (`DI`)** sits on the slice's bypass input `<L>X` itself.
///   The generate mux's cleared state (`CARRY4.<L>CY0` clear) *is* that
///   input, so a routed generate costs nothing and takes the wire, and
///   nothing else can use `<L>X` while it does. A tied generate is not
///   routed and comes from the lookup table's `O5` instead — see
///   [`carry_tied`].
/// - **`s<n>` (`O`) and `co<n>` (`CO`)** sit on invented wires that the
///   output mux and the flip-flop's data mux can select
///   ([`carry_pass_throughs`]); `co3` is the exception and sits on the
///   database's own `COUT`, because that is the wire `tileconn` carries up
///   to the next slice's `CIN`.
/// - **`ci` (`CI`) and `cyinit` (`CYINIT`)** are the two inputs of the
///   `PRECYINIT` mux, on invented wires fed from `CIN` and from `AX`.
fn carry_pins(wires: &str) -> Vec<BelPin> {
    const P: [&str; 4] = ["p0", "p1", "p2", "p3"];
    const DI: [&str; 4] = ["di0", "di1", "di2", "di3"];
    const S: [&str; 4] = ["s0", "s1", "s2", "s3"];
    const CO: [&str; 4] = ["co0", "co1", "co2", "co3"];
    let mut pins = Vec::with_capacity(18);
    for (n, letter) in SLICE_LETTERS.iter().enumerate() {
        pins.push(BelPin {
            role: P[n],
            wire: format!("{wires}_{letter}"),
        });
        pins.push(BelPin {
            role: DI[n],
            wire: format!("{wires}_{letter}X"),
        });
        pins.push(BelPin {
            role: S[n],
            wire: carry_wire(wires, &format!("XOR_{letter}")),
        });
        pins.push(BelPin {
            role: CO[n],
            wire: carry_out_wire(wires, *letter),
        });
    }
    pins.push(BelPin {
        role: "ci",
        wire: carry_wire(wires, "CI"),
    });
    pins.push(BelPin {
        role: "cyinit",
        wire: carry_wire(wires, "CYINIT"),
    });
    pins
}

/// The wire bit `letter`'s carry out sits on: an invented one for `A` to
/// `C`, and the database's `COUT` for `D`, which is the one that leaves
/// the slice.
fn carry_out_wire(wires: &str, letter: char) -> String {
    if letter == 'D' {
        format!("{wires}_COUT")
    } else {
        carry_wire(wires, &format!("CY_{letter}"))
    }
}

/// The paths into and out of a slice's carry chain, each with the mux
/// setting that selects it.
///
/// | Hop | Feature | Meaning |
/// |---|---|---|
/// | `CARRY_CI` ← `CIN` | `PRECYINIT.CIN` | the chain continues from the slice below |
/// | `CARRY_CYINIT` ← `<A>X` | `PRECYINIT.AX` | the chain starts from a fabric signal |
/// | `<L>MUX` ← `CARRY_XOR_<L>` | `<L>OUTMUX.XOR` | the sum out of the slice |
/// | `<L>MUX` ← carry out | `<L>OUTMUX.CY` | the carry out of the slice |
/// | flip-flop `D` ← `CARRY_XOR_<L>` | `<L>FFMUX.XOR` | the sum into the flip-flop beside it |
/// | flip-flop `D` ← carry out | `<L>FFMUX.CY` | the carry into it |
///
/// The meanings are prjxray's fuzzers', **quoted, not measured**:
/// `017-clb-precyinit` documents `PRECYINIT` as `C0` logic 0, `C1` logic 1,
/// `AX` the bypass input and `CIN` the carry from the slice below;
/// `013-clb-ncy0` documents `<L>CY0` as **clear** selecting the bypass
/// input `<L>X` and **set** selecting `O5`, which is why the generate pin
/// sits on `<L>X` with no hop at all; and `016-clb-noutmux` drives its `XOR`
/// variant from `CARRY4.O[n]` and its `CY` variant from `CARRY4.CO[n]`.
/// The `CIN` hop's feature is the one nextpnr-xilinx writes whenever a
/// `CARRY4`'s `CIN` is connected.
fn carry_pass_throughs(wires: &str, prefix: &str) -> Vec<PassThrough> {
    let mut out = vec![
        PassThrough {
            to: carry_wire(wires, "CI"),
            from: format!("{wires}_CIN"),
            features: vec![format!("{prefix}.PRECYINIT.CIN")],
        },
        PassThrough {
            to: carry_wire(wires, "CYINIT"),
            from: format!("{wires}_AX"),
            features: vec![format!("{prefix}.PRECYINIT.AX")],
        },
    ];
    for letter in SLICE_LETTERS {
        let sum = carry_wire(wires, &format!("XOR_{letter}"));
        let carry = carry_out_wire(wires, letter);
        for (from, what) in [(&sum, "XOR"), (&carry, "CY")] {
            out.push(PassThrough {
                to: format!("{wires}_{letter}MUX"),
                from: from.clone(),
                features: vec![format!("{prefix}.{letter}OUTMUX.{what}")],
            });
            out.push(PassThrough {
                to: ff_data_wire(wires, letter),
                from: from.clone(),
                features: vec![format!("{prefix}.{letter}FFMUX.{what}")],
            });
        }
    }
    out
}

/// What a `CARRY4` costs when one of its inputs is a constant rather
/// than a signal, which is the one thing about it routing cannot say.
///
/// - **The carry in.** `PRECYINIT` has a constant zero (`C0`, which is
///   every bit clear and so costs nothing) and a constant one (`C1`). The
///   primitive's model is `CI | CYINIT`, so either pin tied to one asks
///   for `C1`. A pin tied to zero asks for nothing: if the other is a
///   signal its own hop selects it, and if not, `C0` is what a blank
///   slice already is.
/// - **A generate input.** `DI<n>` tied to either constant takes it from
///   the lookup table's `O5` (`CARRY4.<L>CY0` set), because no other
///   source the slice offers is a constant. What `O5` then says is the
///   lookup table's business: `xray::carry` gives that position a table
///   whose lower half is the constant, and only a lane whose outputs are
///   read is given one.
fn carry_tied(prefix: &str) -> Vec<(String, bool, Vec<String>)> {
    let c1 = vec![format!("{prefix}.PRECYINIT.C1")];
    let mut out = vec![
        ("cyinit".to_owned(), true, c1.clone()),
        ("ci".to_owned(), true, c1),
    ];
    for (n, letter) in SLICE_LETTERS.iter().enumerate() {
        let cy0 = vec![format!("{prefix}.CARRY4.{letter}CY0")];
        out.push((format!("di{n}"), false, cy0.clone()));
        out.push((format!("di{n}"), true, cy0));
    }
    out
}

/// The twenty-four `BUFHCE` sites of a clock row, as the hop each one
/// makes from the clock row's input mux to the leaf network.
///
/// `ppips_clk_hrow_bot_r.db` records
/// `CLK_HROW_CK_HCLK_OUT_L0.CLK_HROW_CK_MUX_OUT_L0 always`, which is the
/// buffer seen as metal; it is not free, and `IN_USE` is what turns it
/// on. `ZINV_CE` is the enable held un-inverted, so an unrouted `CE`
/// leaves the buffer running — which is what both Vivado's harness
/// bitstream and nextpnr-xilinx's blinky do for a plain `BUFG` clock.
///
/// **The side-to-`X` and index-to-`Y` mapping is corroborated at one
/// point only.** Both of those designs pair `CLK_HROW_CK_MUX_OUT_L0`
/// with `BUFHCE_X0Y0`; that the twelve `L` wires are `X0Y0`..`X0Y11` and
/// the twelve `R` wires `X1Y0`..`X1Y11` follows from the counts and from
/// nothing else. A route that takes any other index is configuring a
/// buffer on the word of this comment.
fn horizontal_clock_buffers() -> Vec<PassThrough> {
    let mut out = Vec::new();
    for (side, x) in [("L", 0u32), ("R", 1u32)] {
        for n in 0..12u32 {
            out.push(PassThrough {
                to: format!("CLK_HROW_CK_HCLK_OUT_{side}{n}"),
                from: format!("CLK_HROW_CK_MUX_OUT_{side}{n}"),
                features: vec![
                    format!("BUFHCE.BUFHCE_X{x}Y{n}.IN_USE"),
                    format!("BUFHCE.BUFHCE_X{x}Y{n}.ZINV_CE"),
                ],
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Bels the prefix machinery cannot name
// ---------------------------------------------------------------------------

/// A bel this module declares outright, because the tile type's feature
/// names do not describe it as one.
///
/// [`super::parse::bels_of`] builds a tile type's bels from the site
/// prefixes its features use, and that works wherever prjxray named a
/// prefix after a site. The global clock buffers break it: all sixteen
/// `BUFGCTRL` sites of a `CLK_BUFG_BOT_R` share the single prefix
/// `BUFGCTRL` and are told apart by the *second* component
/// (`BUFGCTRL.BUFGCTRL_X0Y3.IN_USE`), so the machinery sees one bel
/// where there are sixteen.
pub(super) struct ExtraBel {
    /// The bel's name inside its tile type.
    pub name: String,
    /// Its kind, in [`BelRole`](crate::fpga::BelRole)'s vocabulary.
    pub kind: &'static str,
    /// Its pins.
    pub pins: Vec<BelPin>,
    /// The primitive that selects a feature list, and the features.
    pub cells: Vec<(&'static str, Vec<String>)>,
}

/// Every bel a tile type has that [`super::parse::bels_of`] cannot find.
///
/// # The global clock buffer
///
/// A 7-series `BUFG` is a `BUFGCTRL` with its mux held still. Both
/// Vivado (in `artix7/harness/basys3/swbut/design.json`) and
/// nextpnr-xilinx (in its own Basys 3 blinky) configure a plain clock
/// buffer with exactly four features and no others:
///
/// ```text
/// BUFGCTRL.BUFGCTRL_X0Y0.IN_USE
/// BUFGCTRL.BUFGCTRL_X0Y0.IS_IGNORE1_INVERTED
/// BUFGCTRL.BUFGCTRL_X0Y0.ZINV_CE0
/// BUFGCTRL.BUFGCTRL_X0Y0.ZINV_S0
/// ```
///
/// which is the `I0` input selected, the `S0`/`CE0` controls held
/// un-inverted so that leaving them unrouted means *on*, and the unused
/// half's `IGNORE1` inverted. That list is transcribed, not derived:
/// what `IS_IGNORE1_INVERTED` does to the silicon is not established
/// here, only that two independent tools set it and one of them has run
/// on this board.
///
/// The wires come from `ppips_clk_bufg_bot_r.db` and
/// `segbits_clk_bufg_bot_r.db`: `CLK_BUFG_BUFGCTRL<n>_I0` is the input
/// the harness routes into and `CLK_BUFG_BUFGCTRL<n>_O` the output it
/// takes `CLK_BUFG_CK_GCLK<n>` from. That the wire index `<n>` is the
/// same number as the site's `Y` is the one thing here nothing
/// corroborates past `n = 0`.
///
/// # The PLL of a clock management tile
///
/// Its features do have a prefix of their own, `PLLE2_ADV`, but no
/// `_X<n>`/`_Y<n>` suffix says which site it is, so the prefix machinery
/// makes it a bel of kind `other` with no pins. `super::cmt` supplies the
/// kind and the pins, and the reader keeps the features it already filed
/// under that name.
pub(super) fn extra_bels(tile_type: &str) -> Vec<ExtraBel> {
    let managers = super::cmt::clock_manager_bels(tile_type);
    if !managers.is_empty() {
        return managers;
    }
    if !matches!(tile_type, "CLK_BUFG_BOT_R" | "CLK_BUFG_TOP_R") {
        return Vec::new();
    }
    (0..16u32)
        .map(|n| ExtraBel {
            name: format!("BUFGCTRL_X0Y{n}"),
            kind: "gb",
            pins: vec![
                BelPin {
                    role: "i",
                    wire: format!("CLK_BUFG_BUFGCTRL{n}_I0"),
                },
                BelPin {
                    role: "o",
                    wire: format!("CLK_BUFG_BUFGCTRL{n}_O"),
                },
            ],
            cells: vec![(
                "BUFG",
                vec![
                    format!("BUFGCTRL.BUFGCTRL_X0Y{n}.IN_USE"),
                    format!("BUFGCTRL.BUFGCTRL_X0Y{n}.IS_IGNORE1_INVERTED"),
                    format!("BUFGCTRL.BUFGCTRL_X0Y{n}.ZINV_CE0"),
                    format!("BUFGCTRL.BUFGCTRL_X0Y{n}.ZINV_S0"),
                ],
            )],
        })
        .collect()
}

// ---------------------------------------------------------------------------
// What a cell on a bel costs
// ---------------------------------------------------------------------------

/// How a cell sitting on a bel is configured, in feature names rather
/// than in bits, so that the bits stay the database's.
#[derive(Default)]
pub(super) struct BelConfig {
    /// A primitive and the features a cell of that primitive needs: one
    /// [`ConfigEntry::Cell`](crate::fpga::ConfigEntry) each.
    pub cells: Vec<(&'static str, Vec<String>)>,
    /// A parameter, one of its bits, and the feature that carries that
    /// bit **inverted**: one
    /// [`ConfigEntry::ParamZero`](crate::fpga::ConfigEntry) each.
    pub inverted: Vec<(&'static str, u32, String)>,
    /// A pin role, a constant, and the features set when the cell's pin of
    /// that role is tied to that constant: one
    /// [`ConfigEntry::Tied`](crate::fpga::ConfigEntry) each.
    pub tied: Vec<(String, bool, Vec<String>)>,
}

/// What a cell on the bel `<prefix>_<sub>` of `tile_type` costs.
///
/// # A 7-series flip-flop
///
/// The four features that decide what a storage element *is* are spread
/// over two scopes, and the split matters:
///
/// | Feature | Scope | Set when |
/// |---|---|---|
/// | `<letter>FF.ZINI` | the one flip-flop | its `INIT` is **0** |
/// | `<letter>FF.ZRST` | the one flip-flop | its set/reset drives it to **0** (`FDRE`, `FDCE`) |
/// | `FFSYNC` | the whole slice | the set/reset is synchronous (`FDRE`, `FDSE`) |
/// | `CLKINV` | the whole slice | the clock is inverted (the `_1` primitives) |
///
/// The `Z` prefix is prjxray's mark for an inverted field, and it is the
/// trap in this table: a blank bitstream has `ZINI` clear, which means
/// every flip-flop on the part powers up holding **one**. An `FDRE` with
/// the default `INIT=1'b0` therefore has to *set* a bit to get zero,
/// which is why `ZINI` is a
/// [`ConfigEntry::ParamZero`](crate::fpga::ConfigEntry) over `INIT`
/// rather than a plain `Param`.
///
/// `LATCH` stays clear: this flow never places a latch, and the `.dev`
/// file declares none.
///
/// These four are what prjxray's own `011-clb-ffconfig` fuzzer tags, and
/// what nextpnr-xilinx writes for the same primitives. **None of them is
/// corroborated by the Vivado harness bitstream**, which places no logic
/// at all — see `docs/fpga-xray.md`.
pub(super) fn bel_config(tile_type: &str, prefix: &str, sub: &str) -> BelConfig {
    let mut out = BelConfig::default();
    let Some((base, _, index)) = split_prefix(prefix) else {
        return out;
    };
    if base != "SLICEL" && base != "SLICEM" {
        return out;
    }
    if sub == "CARRY4" && slice_feature_prefix(tile_type, index).is_some() {
        out.tied = carry_tied(prefix);
        return out;
    }
    let (Some(_), Some(letter)) = (slice_feature_prefix(tile_type, index), slice_ff_letter(sub))
    else {
        return out;
    };
    // The prefix the caller already knows; the features below are named
    // relative to it, exactly as `segbits_<type>.db` spells them.
    let ff = format!("{prefix}.{letter}FF");
    let sync = format!("{prefix}.FFSYNC");
    let inv = format!("{prefix}.CLKINV");
    let zrst = format!("{ff}.ZRST");
    for (primitive, negative_edge, synchronous, resets_to_zero) in [
        ("FDRE", false, true, true),
        ("FDSE", false, true, false),
        ("FDCE", false, false, true),
        ("FDPE", false, false, false),
        ("FDRE_1", true, true, true),
        ("FDSE_1", true, true, false),
        ("FDCE_1", true, false, true),
        ("FDPE_1", true, false, false),
    ] {
        let mut features = Vec::new();
        if resets_to_zero {
            features.push(zrst.clone());
        }
        if synchronous {
            features.push(sync.clone());
        }
        if negative_edge {
            features.push(inv.clone());
        }
        out.cells.push((primitive, features));
    }
    out.inverted.push(("INIT", 0, format!("{ff}.ZINI")));
    out
}

// ---------------------------------------------------------------------------
// A wire that costs bits merely to be used
// ---------------------------------------------------------------------------

/// The features a tile type may charge for *touching* the wire `wire`,
/// whichever pip touches it.
///
/// The clock tiles have a kind of feature nothing else does: a one-
/// component name like `HCLK_CMT_CCIO0_ACTIVE`, `HCLK_CMT_CCIO0_USED` or
/// `ENABLE_BUFFER.HCLK_CK_BUFHCLK0`, which is neither a pip (it names no
/// pair of wires) nor a bel feature (it has no site prefix). What it is
/// is a buffer sitting *on* a wire, and it has to be switched on before
/// the wire carries anything.
///
/// Rather than a table, this is a rule the database answers: for every
/// pip of a tile type, look up these names for each of its local wires
/// and take the bits of whichever the `segbits` file has. Three of the
/// four forms are the wire's own name with a suffix or a prefix; the
/// fourth is [`global_clock_enable`], which is the one place where the
/// feature is not named after the wire at all. Between them they cover
/// **every** one-component feature in `artix7/` that names a clock
/// resource — the complete list of shapes is `*_ACTIVE`, `*_USED`,
/// `GCLK<n>_ENABLE_ABOVE` and `GCLK<n>_ENABLE_BELOW` — and outside the
/// clock tiles no tile type has one at all, so the rule costs nothing
/// where it does not apply.
///
/// Checked against two working bitstreams: for the Basys 3 clock route
/// from pin W5 this produces exactly the `HCLK_CMT_CCIO0_ACTIVE`,
/// `HCLK_CMT_CCIO0_USED`, `CLK_HROW_CK_IN_R0_ACTIVE`,
/// `CLK_HROW_R_CK_GCLK0_ACTIVE` and `ENABLE_BUFFER.HCLK_CK_BUFHCLK0`
/// that Vivado and nextpnr-xilinx set on the same route.
pub(super) fn wire_enable_features(wire: &str) -> Vec<String> {
    let mut out = vec![
        format!("{wire}_ACTIVE"),
        format!("{wire}_USED"),
        format!("ENABLE_BUFFER.{wire}"),
    ];
    out.extend(global_clock_enable(wire));
    out
}

/// The `CLK_BUFG_REBUF` buffer enable that the global-clock wire `wire`
/// needs before it carries anything, if it is one.
///
/// # What a rebuffer is, and why this is not a suffix rule
///
/// A global clock leaves its `BUFGCTRL` onto one of the 32 vertical
/// `GCLK` tracks, and that track is **cut** at every `CLK_BUFG_REBUF`
/// tile of the column — on the `xc7a50t` at `Y13`, `Y38`, `Y65`, `Y90`,
/// `Y117` and `Y142`, with the two global buffer tiles at `Y48` and
/// `Y53` and the three clock rows at `Y26`, `Y78` and `Y130` between
/// them. Each cut is a pair of wires, `…_CK_GCLK<n>_TOP` above and
/// `…_CK_GCLK<n>_BOT` below, a pip each way between them, and a buffer
/// on each side whose enable is a one-component feature:
/// `GCLK<n>_ENABLE_ABOVE` and `GCLK<n>_ENABLE_BELOW`. **The enable is
/// named after the track, not after the wire**, so the suffix rule in
/// [`wire_enable_features`] cannot find it, and a clock that crosses a
/// rebuffer with the enable clear reaches nothing.
///
/// # Which enable belongs to which wire, and how that was settled
///
/// Measured, and the names read backwards from the wires' own: the
/// **`_TOP` wire is enabled by `GCLK<n>_ENABLE_BELOW`** and the `_BOT`
/// wire by `GCLK<n>_ENABLE_ABOVE`.
///
/// All four Vivado designs in `artix7/harness/` route a clock up this
/// column, and each one sets exactly eleven `CLK_BUFG_REBUF` features:
/// the `TOP` ← `BOT` pip at `Y65`, `Y90` and `Y117` with **both**
/// enables at each, `GCLK<n>_ENABLE_BELOW` alone at `Y38`, and
/// `GCLK<n>_ENABLE_ABOVE` alone at `Y142`. Two of the four put the
/// buffer in `CLK_BUFG_BOT_R_X60Y48` and two in
/// `CLK_BUFG_TOP_R_X60Y53`, and one drives `GCLK16` rather than
/// `GCLK0`, so the index and the tile vary and the pattern does not.
///
/// A live track segment spans from one rebuffer's `TOP` wire to the next
/// one's `BOT` wire, and every one of those 44 features is exactly "each
/// end of each live segment", with `TOP` ↔ `BELOW`: the segment holding
/// the global buffer is `Y38`'s `TOP` and `Y65`'s `BOT`, which is
/// `Y38.ENABLE_BELOW` and `Y65.ENABLE_ABOVE`; the segment holding the
/// clock row at `Y130` is `Y117`'s `TOP` and `Y142`'s `BOT`, which is
/// `Y117.ENABLE_BELOW` and `Y142.ENABLE_ABOVE`. The opposite assignment
/// explains none of the four. `tests/fpga_xray.rs` re-derives it from
/// those files rather than trusting this paragraph.
///
/// What the two words *mean* is not established here, only which wire
/// each one switches on. Reticle's own blink route runs **down** the
/// column instead — its flip-flops are in the bottom clock region — so
/// it takes the `BOT` ← `TOP` pip that no harness design takes, and the
/// rule puts a mirror image of the harness pattern on the two rebuffers
/// below the buffer.
fn global_clock_enable(wire: &str) -> Option<String> {
    let (side, index) = global_clock_track(wire)?;
    let end = if side == "TOP" { "BELOW" } else { "ABOVE" };
    Some(format!("GCLK{index}_ENABLE_{end}"))
}

/// The global clock track the rebuffer wire `wire` is one end of: the
/// number in `CLK_BUFG_REBUF_R_CK_GCLK<n>_TOP`, with the side.
///
/// This is the same reading as [`global_clock_enable`] and exists
/// separately because [`super::XrayFabric::enable_global_clocks`] needs
/// the *number* rather than a feature name: the enables have to be
/// switched on over the whole column, and a route only ever traverses
/// the one rebuffer it crosses.
pub(super) fn global_clock_track(wire: &str) -> Option<(&str, u32)> {
    let (track, side) = wire.rsplit_once('_')?;
    if side != "TOP" && side != "BOT" {
        return None;
    }
    let (_, index) = track.rsplit_once("_CK_GCLK")?;
    Some((side, index.parse().ok()?))
}

/// The global clock track whose rebuffer enable the one-component
/// feature `feature` is, which is how a `segbits` file's
/// `GCLK<n>_ENABLE_ABOVE` is found without a table of all 32.
pub(super) fn global_clock_enable_track(feature: &str) -> Option<u32> {
    let rest = feature.strip_prefix("GCLK")?;
    let (index, end) = rest.split_once("_ENABLE_")?;
    if end != "ABOVE" && end != "BELOW" {
        return None;
    }
    index.parse().ok()
}

// ---------------------------------------------------------------------------
// IO standards
// ---------------------------------------------------------------------------

/// What an IO buffer of a given standard costs, as feature names of the
/// `LIOB33` / `RIOB33` tile type with `{}` standing for the half.
///
/// # Where this came from
///
/// Not from a datasheet: from Vivado. `artix7/harness/basys3/swbut/`
/// ships a working bitstream for this exact board together with
/// `design.json`, whose `required_features` list names every feature the
/// design occupies. Filtering it to the two `LIOB33` tiles that hold a
/// switch and an LED gives, for an input and an output respectively,
/// exactly the lists below — four names each, out of the eighty-three
/// the tile type has. `tests/fpga_xray.rs` re-derives them from that
/// file rather than trusting this table.
///
/// Features whose bits are all zeros (`IN_TERM.NONE`, an input's
/// `SLEW.FAST`) are kept in the list because they are what Vivado
/// recorded; they set nothing, and saying so is cheaper than explaining
/// their absence.
pub(super) struct IoStandard {
    /// The name a `set_io -io_standard` uses.
    pub name: &'static str,
    /// The features an input buffer of this standard needs.
    pub input: &'static [&'static str],
    /// The features an output buffer of this standard needs.
    pub output: &'static [&'static str],
    /// What a bidirectional buffer needs **on top of** [`IoStandard::output`]:
    /// the input receiver, without the input-only setting that switches
    /// the driver off. See [`io_features`].
    pub receiver: &'static [&'static str],
}

/// Every IO standard this flow can configure, which is one.
///
/// A design whose constraints ask for anything else is refused with its
/// name rather than silently given LVCMOS33 bits: an IO standard is a
/// voltage, and guessing one is how a board gets damaged.
pub(super) const IO_STANDARDS: &[IoStandard] = &[IoStandard {
    name: "LVCMOS33",
    input: &[
        "IOB_Y{}.IN_TERM.NONE",
        "IOB_Y{}.LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVDS_25_LVTTL_SSTL135_SSTL15_TMDS_33.IN_ONLY",
        "IOB_Y{}.LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVTTL.SLEW.FAST",
        "IOB_Y{}.LVCMOS25_LVCMOS33_LVTTL.IN",
        "IOB_Y{}.PULLTYPE.NONE",
    ],
    output: &[
        "IOB_Y{}.IN_TERM.NONE",
        "IOB_Y{}.LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVTTL_SSTL135_SSTL15.SLEW.SLOW",
        "IOB_Y{}.LVCMOS33_LVTTL.DRIVE.I12_I16",
        "IOB_Y{}.PULLTYPE.NONE",
    ],
    receiver: &["IOB_Y{}.LVCMOS25_LVCMOS33_LVTTL.IN"],
}];

/// The standard of that name.
pub(super) fn io_standard(name: &str) -> Option<&'static IoStandard> {
    IO_STANDARDS.iter().find(|s| s.name == name)
}

/// The names of every IO standard the flow can configure, for an error
/// message.
pub fn io_standards() -> Vec<&'static str> {
    IO_STANDARDS.iter().map(|s| s.name).collect()
}

/// Every IO primitive of `xc7.dev` that an IO buffer site configures, so
/// a [`ConfigEntry::Cell`](crate::fpga::ConfigEntry) can select each.
///
/// `IBUF` drives the fabric from the pad and `OBUF` the pad from the
/// fabric. `OBUFT` and `IOBUF` were left out until 2026-10-08 because
/// their tristate path needed `OLOGIC` `T` features nothing here had
/// read; [`tristate_through`] is that path now, and these two are what
/// the pad itself costs — see [`io_features`].
pub(super) const IO_PRIMITIVES: [&str; 4] = ["IBUF", "OBUF", "OBUFT", "IOBUF"];

/// The features, as templates over the half, that a pad of `standard`
/// costs when `primitive` sits on it.
///
/// # What a tristate or a bidirectional pad costs at the pad
///
/// The pad itself does not know whether its `T` is routed: `segbits_liob33.db`
/// has no feature that mentions a tristate at all, out of eighty-three.
/// The tristate lives entirely in the `OLOGIC` beside it
/// ([`tristate_through`]). So an `OBUFT` costs exactly what an `OBUF`
/// costs.
///
/// An `IOBUF` is an output with the **input receiver** switched on
/// as well: [`IoStandard::output`] plus [`IoStandard::receiver`], and
/// **not** `IN_ONLY`. The two input features do different things, which
/// the bits say by themselves: `LVCMOS25_LVCMOS33_LVTTL.IN` is
/// `38_86 39_85 39_87` for `Y0` and touches no drive bit, while `IN_ONLY`
/// is a *value of the drive field*: it is written over the same eighteen
/// bits as every `DRIVE.*` pattern, with `38_64` and `39_65` — which
/// every `DRIVE.*` value sets — required clear. An `IBUF` takes both; an
/// `IOBUF` must take the first and must not take the second, or its
/// drive field would hold two values at once.
///
/// That split is **quoted**: it is what nextpnr-xilinx's
/// `write_io_config` writes (`LVCMOS25_LVCMOS33_LVTTL.IN` when the pad has
/// an input buffer, `IN_ONLY` only `if (!is_output)`), and the bit
/// patterns agree with it. No Vivado bitstream in `artix7/harness/` has a
/// bidirectional pad to measure it against.
pub(super) fn io_features(standard: &IoStandard, primitive: &str) -> Option<Vec<&'static str>> {
    match primitive {
        "IBUF" => Some(standard.input.to_vec()),
        "OBUF" | "OBUFT" => Some(standard.output.to_vec()),
        "IOBUF" => Some(
            standard
                .output
                .iter()
                .chain(standard.receiver)
                .copied()
                .collect(),
        ),
        _ => None,
    }
}

/// Substitutes a bel index into a feature template.
pub(super) fn with_index(template: &str, index: u32) -> String {
    template.replace("{}", &index.to_string())
}

/// A report of what the site tables covered in one load, so a caller can
/// say how much of a fabric it actually understands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteCoverage {
    /// Bel pins resolved to a wire the tile type really has.
    pub pins: usize,
    /// Bel pins whose wire the tile type does not declare, which is a
    /// table that has drifted from the database.
    pub pins_unresolved: usize,
    /// Fixed paths through a site that were declared with their bits.
    pub pass_throughs: usize,
    /// Fixed paths whose feature the `segbits` file does not have.
    pub pass_throughs_unresolved: usize,
    /// IO buffers given a standard's bits.
    pub io_buffers: usize,
    /// Features an IO standard asked for that the `segbits` file has
    /// not got. Any at all means that buffer was left unconfigured
    /// rather than half configured.
    pub io_unresolved: usize,
    /// Cell modes given their bits: a flip-flop primitive's mode field,
    /// a global buffer's, and the inverted-parameter entries beside
    /// them.
    pub modes: usize,
    /// Cell modes whose feature the `segbits` file has not got, so the
    /// primitive is placeable and will configure nothing.
    pub modes_unresolved: usize,
    /// Pips that a wire's `_ACTIVE`, `_USED` or `ENABLE_BUFFER` feature
    /// added bits to — the buffers that sit on a clock wire.
    ///
    /// The three feature names are built by `wire_enable_features`, which
    /// is crate-private; this field is the count of pips it found one for.
    pub wire_enables: usize,
}

impl SiteCoverage {
    /// A report, one fact per line, or nothing at all when the fabric has
    /// no site this module knows.
    pub fn to_text(&self) -> String {
        if self.pins == 0 && self.pass_throughs == 0 {
            return String::new();
        }
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "  site pins: {} resolved, {} not; {} fixed path(s) through a site, {} not; \
             {} io buffer(s), {} feature(s) missing",
            self.pins,
            self.pins_unresolved,
            self.pass_throughs,
            self.pass_throughs_unresolved,
            self.io_buffers,
            self.io_unresolved
        );
        let _ = writeln!(
            out,
            "  cell modes: {} resolved, {} not; {} wire(s) that cost bits to use",
            self.modes, self.modes_unresolved, self.wire_enables
        );
        out
    }
}

/// Every `(prefix, site)` pair of a tile, worked out once.
///
/// The map is keyed on the feature prefix because that is what a bel is
/// named after.
pub(super) fn sites_by_prefix<'a>(
    tile: &'a XrayTile,
    prefixes: &[&str],
) -> BTreeMap<String, &'a (String, String)> {
    let mut out = BTreeMap::new();
    for prefix in prefixes {
        if let Some(site) = site_of_prefix(tile, prefix) {
            out.insert((*prefix).to_owned(), site);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(sites: &[(&str, &str)]) -> XrayTile {
        XrayTile {
            name: "T".to_owned(),
            tile_type: "T".to_owned(),
            grid_x: 0,
            grid_y: 0,
            sites: sites
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect(),
            bits: Vec::new(),
            bits_alias: None,
        }
    }

    /// The same tile with its bits aliased onto another type, which is
    /// what a `_SING` IO tile looks like: see `super::TileAlias`.
    fn aliased_tile(sites: &[(&str, &str)], start_offset: u32) -> XrayTile {
        XrayTile {
            bits_alias: Some(crate::fpga::xray::TileAlias {
                tile_type: "LIOB33".to_owned(),
                start_offset,
            }),
            ..tile(sites)
        }
    }

    #[test]
    fn a_prefix_splits_into_a_base_an_axis_and_an_index() {
        assert_eq!(split_prefix("SLICEL_X0"), Some(("SLICEL", Axis::X, 0)));
        assert_eq!(split_prefix("IOB_Y1"), Some(("IOB", Axis::Y, 1)));
        assert_eq!(split_prefix("ILOGIC_Y0"), Some(("ILOGIC", Axis::Y, 0)));
        // `DIFF` describes the differential pair, not a site.
        assert_eq!(split_prefix("DIFF"), None);
    }

    #[test]
    fn a_site_type_settles_a_clblm() {
        let t = tile(&[("SLICE_X10Y0", "SLICEM"), ("SLICE_X11Y0", "SLICEL")]);
        assert_eq!(
            site_of_prefix(&t, "SLICEM_X0").map(|(n, _)| n.as_str()),
            Some("SLICE_X10Y0")
        );
        assert_eq!(
            site_of_prefix(&t, "SLICEL_X1").map(|(n, _)| n.as_str()),
            Some("SLICE_X11Y0")
        );
    }

    #[test]
    fn x_counts_up_and_y_counts_down() {
        let clb = tile(&[("SLICE_X0Y0", "SLICEL"), ("SLICE_X1Y0", "SLICEL")]);
        assert_eq!(
            site_of_prefix(&clb, "SLICEL_X0").map(|(n, _)| n.as_str()),
            Some("SLICE_X0Y0")
        );
        let iob = tile(&[("IOB_X0Y11", "IOB33S"), ("IOB_X0Y12", "IOB33M")]);
        // Measured, not assumed: `_Y0` is the *higher* site Y.
        assert_eq!(
            site_of_prefix(&iob, "IOB_Y0").map(|(n, _)| n.as_str()),
            Some("IOB_X0Y12")
        );
        assert_eq!(
            site_of_prefix(&iob, "IOB_Y1").map(|(n, _)| n.as_str()),
            Some("IOB_X0Y11")
        );
    }

    #[test]
    fn an_ioi3_tile_has_three_kinds_of_site_and_each_finds_its_own() {
        let t = tile(&[
            ("IDELAY_X0Y11", "IDELAYE2"),
            ("IDELAY_X0Y12", "IDELAYE2"),
            ("ILOGIC_X0Y11", "ILOGICE3"),
            ("ILOGIC_X0Y12", "ILOGICE3"),
            ("OLOGIC_X0Y11", "OLOGICE3"),
            ("OLOGIC_X0Y12", "OLOGICE3"),
        ]);
        assert_eq!(
            site_of_prefix(&t, "ILOGIC_Y0").map(|(n, _)| n.as_str()),
            Some("ILOGIC_X0Y12")
        );
        assert_eq!(
            site_of_prefix(&t, "OLOGIC_Y1").map(|(n, _)| n.as_str()),
            Some("OLOGIC_X0Y11")
        );
    }

    #[test]
    fn a_lut_gets_six_inputs_and_an_output_on_its_own_slices_wires() {
        let pins = bel_pins("CLBLL_L", "SLICEL_X0", "ALUT");
        let names: Vec<(&str, &str)> = pins.iter().map(|p| (p.role, p.wire.as_str())).collect();
        assert_eq!(
            names,
            vec![
                ("i0", "CLBLL_LL_A1"),
                ("i1", "CLBLL_LL_A2"),
                ("i2", "CLBLL_LL_A3"),
                ("i3", "CLBLL_LL_A4"),
                ("i4", "CLBLL_LL_A5"),
                ("i5", "CLBLL_LL_A6"),
                ("o", "CLBLL_LL_A"),
            ]
        );
        // The other slice of the same tile is the `_L_` one.
        assert_eq!(
            bel_pins("CLBLL_L", "SLICEL_X1", "DLUT")[6].wire,
            "CLBLL_L_D"
        );
    }

    #[test]
    fn an_io_buffer_has_a_fabric_side_and_no_pad() {
        let pins = bel_pins("LIOB33", "IOB_Y1", "");
        let names: Vec<(&str, &str)> = pins.iter().map(|p| (p.role, p.wire.as_str())).collect();
        assert_eq!(
            names,
            vec![("din", "IOB_IBUF1"), ("dout", "IOB_O1"), ("oe", "IOB_T1")]
        );
        assert!(!pins.iter().any(|p| p.role == "pad"));
    }

    /// A `_SING` tile's lone buffer sits on the **zero** wires however
    /// the database names its bits.
    ///
    /// This is the one place the two numberings come apart: the feature
    /// prefix of the tile at the top of a bank is `IOB_Y1` (see
    /// `super::TileAlias`, and `artix7/harness/arty-a7/pmod`'s
    /// `design.json`, which is Vivado writing exactly that), while the
    /// wire is `IOB_IBUF0` because `tileconn.json`'s
    /// `LIOB33_SING`/`LIOI3_SING` pairs name no wire with a 1 in it.
    /// Taking the index from the prefix, as the two-IOB tiles do, would
    /// put the pin on a wire the tile does not have, and the bel would
    /// come out with no pins at all.
    ///
    /// What this would not catch: that `IOB_O0` is the *output* and
    /// `IOB_IBUF0` the input in a `_SING` tile. That is the same
    /// `tileconn` pairing as in a `LIOB33`, and nothing here re-measures
    /// it.
    #[test]
    fn a_single_io_tiles_buffer_is_on_the_zero_wires_whichever_half_names_it() {
        for (tile_type, prefix) in [
            ("LIOB33_SING", "IOB_Y0"),
            ("LIOB33_SING", "IOB_Y1"),
            ("RIOB33_SING", "IOB_Y0"),
            ("RIOB33_SING", "IOB_Y1"),
        ] {
            let pins = bel_pins(tile_type, prefix, "");
            let names: Vec<(&str, &str)> = pins.iter().map(|p| (p.role, p.wire.as_str())).collect();
            assert_eq!(
                names,
                vec![("din", "IOB_IBUF0"), ("dout", "IOB_O0"), ("oe", "IOB_T0")],
                "{tile_type} {prefix}"
            );
        }
    }

    /// The prefix index of an aliased tile is not a rank, so the one
    /// site of the family is the one meant.
    ///
    /// `LIOB33_SING_X0Y49` holds `IOB_X0Y49` and nothing else, and
    /// `tilegrid.json` aliases it onto `LIOB33`'s `IOB_Y1` — rank one of
    /// a one-site tile. Without this the top ball of every bank
    /// resolves to no site at all, which is the shape the bug had.
    #[test]
    fn an_aliased_tiles_one_site_answers_to_either_half() {
        let top = aliased_tile(&[("IOB_X0Y49", "IOB33")], 0);
        assert_eq!(site_of_prefix(&top, "IOB_Y1").unwrap().0, "IOB_X0Y49");
        assert_eq!(site_of_prefix(&top, "IOB_Y0").unwrap().0, "IOB_X0Y49");
        let bottom = aliased_tile(&[("IOB_X0Y0", "IOB33")], 2);
        assert_eq!(site_of_prefix(&bottom, "IOB_Y0").unwrap().0, "IOB_X0Y0");
        // And the relaxation is only for an aliased tile: an ordinary
        // one-site tile still ranks, so `_Y1` of it is nothing.
        let plain = tile(&[("IOB_X0Y49", "IOB33")]);
        assert_eq!(site_of_prefix(&plain, "IOB_Y0").unwrap().0, "IOB_X0Y49");
        assert!(site_of_prefix(&plain, "IOB_Y1").is_none());
    }

    /// A `_SING` IO logic tile offers both halves' features on the one
    /// wire pair it has, and the half the tile is keeps its hop.
    ///
    /// Which half that is, this function does not know and must not
    /// guess: `build_arch` drops the hop whose features the tile type's
    /// feature set does not hold, and that set has already been cut to
    /// one half by `super::TileAlias`. So the table's job is to offer
    /// both, and the test's job is to say that the wires do not move
    /// while the features do.
    #[test]
    fn a_single_io_logic_tile_offers_both_halves_on_the_one_wire_set() {
        for (tile_type, side) in [("LIOI3_SING", "LIOI"), ("RIOI3_SING", "RIOI")] {
            let p = pass_throughs(tile_type);
            assert_eq!(p.len(), 6, "{tile_type}");
            for (i, half) in [(0usize, 0u32), (3, 1)] {
                assert_eq!(p[i].to, "IOI_ILOGIC0_O");
                assert_eq!(p[i].from, format!("{side}_ILOGIC0_D"));
                assert_eq!(p[i].features, vec![format!("ILOGIC_Y{half}.ZINV_D")]);
                assert_eq!(p[i + 1].to, format!("{side}_OLOGIC0_OQ"));
                assert_eq!(p[i + 1].from, "IOI_OLOGIC0_D1");
                assert_eq!(
                    p[i + 1].features,
                    vec![
                        format!("OLOGIC_Y{half}.OMUX.D1"),
                        format!("OLOGIC_Y{half}.OQUSED"),
                        format!("OLOGIC_Y{half}.OSERDES.DATA_RATE_TQ.BUF"),
                    ]
                );
                assert_eq!(p[i + 2].to, format!("{side}_OLOGIC0_TQ"));
                assert_eq!(p[i + 2].from, "IOI_OLOGIC0_T1");
            }
        }
    }

    /// Both halves, the two data directions and — since the tristate path
    /// was declared — the `T` hop beside the output one. This asserted four
    /// hops while the tristate was missing; six is the same table plus
    /// the hop that makes an `IOBUF` route, and the data hops are still
    /// exactly where they were.
    #[test]
    fn an_ioi3_declares_both_halves_of_both_directions_and_the_tristate() {
        let p = pass_throughs("LIOI3");
        assert_eq!(p.len(), 6);
        assert_eq!(p[0].to, "IOI_ILOGIC0_O");
        assert_eq!(p[0].from, "LIOI_ILOGIC0_D");
        assert_eq!(p[0].features, vec!["ILOGIC_Y0.ZINV_D"]);
        assert_eq!(p[1].to, "LIOI_OLOGIC0_OQ");
        assert_eq!(p[1].from, "IOI_OLOGIC0_D1");
        assert_eq!(p[2].to, "LIOI_OLOGIC0_TQ");
        assert_eq!(p[2].from, "IOI_OLOGIC0_T1");
        assert_eq!(pass_throughs("RIOI3")[0].from, "RIOI_ILOGIC0_D");
        assert_eq!(pass_throughs("RIOI3")[5].to, "RIOI_OLOGIC1_TQ");
    }

    /// The tristate hop's polarity bit is on, and is the one for *its own*
    /// half.
    ///
    /// What this catches: the hop declared without `ZINV_T1` (a blank
    /// tile inverts `T1`, so every `IOBUF` would drive exactly when it
    /// was told to release), or with the other half's bit (one pad's
    /// enable inverted by the routing of its neighbour). What it cannot
    /// catch: that `ZINV_T1` set really means "not inverted" on silicon.
    /// That is quoted from prjxray's fuzzer and nextpnr-xilinx, and only a
    /// part can check it — `examples/basys3/pmod_bidir.v`.
    #[test]
    fn the_tristate_hop_holds_t_uninverted_and_passes_it_straight_through() {
        for n in 0..2u32 {
            let hop = tristate_through("LIOI", n, n);
            assert_eq!(hop.to, format!("LIOI_OLOGIC{n}_TQ"));
            assert_eq!(hop.from, format!("IOI_OLOGIC{n}_T1"));
            assert_eq!(
                hop.features,
                vec![
                    format!("OLOGIC_Y{n}.OSERDES.DATA_RATE_TQ.BUF"),
                    format!("OLOGIC_Y{n}.ZINV_T1"),
                ]
            );
        }
    }

    /// An `IOBUF` is an output plus the receiver, and never `IN_ONLY`; an
    /// `OBUFT` is an output and nothing more.
    ///
    /// What this catches: a bidirectional pad given the input recipe
    /// (whose `IN_ONLY` is a value of the drive field, so the pad would be
    /// configured as two drive strengths at once), or given no receiver
    /// (so it would drive and never read). It does not check that the
    /// receiver feature is the right one on silicon; that is quoted from
    /// nextpnr-xilinx, and the harness has no bidirectional pad.
    #[test]
    fn a_bidirectional_pad_is_an_output_with_its_receiver_on() {
        let s = io_standard("LVCMOS33").unwrap();
        let iobuf = io_features(s, "IOBUF").unwrap();
        assert!(iobuf.iter().all(|f| !f.ends_with(".IN_ONLY")), "{iobuf:?}");
        assert!(iobuf.contains(&"IOB_Y{}.LVCMOS25_LVCMOS33_LVTTL.IN"));
        assert!(iobuf.contains(&"IOB_Y{}.LVCMOS33_LVTTL.DRIVE.I12_I16"));
        assert_eq!(io_features(s, "OBUFT").unwrap(), s.output.to_vec());
        assert_eq!(io_features(s, "OBUF").unwrap(), s.output.to_vec());
        assert_eq!(io_features(s, "IBUF").unwrap(), s.input.to_vec());
        assert!(io_features(s, "BUFG").is_none());
        for primitive in IO_PRIMITIVES {
            assert!(io_features(s, primitive).is_some(), "{primitive}");
        }
    }

    /// A slice's pass-throughs are the ones a clocked design and a carry
    /// chain need, and nothing else.
    ///
    /// This used to assert a `CLBLL` had none at all, which was true when
    /// only an IO tile did and was the thing stopping the router treating
    /// a slice as a length of wire. A flip-flop changed that: it needs a
    /// path to its `D`, and its clock enable and reset come off the
    /// slice's shared control lines. The carry chain changed it again: it
    /// needs its carry in from `CIN` or `AX`, a generate from the bypass
    /// input, and a way out for its sums and carries. So the guard is
    /// about *which* paths exist rather than whether any do — every one is
    /// a multiplexer inside the slice, named by the feature that selects
    /// it, and none crosses the slice from one side to the other.
    #[test]
    fn a_slice_passes_through_only_where_a_flip_flop_or_a_carry_needs_it() {
        let p = pass_throughs("CLBLL_L");
        assert!(!p.is_empty(), "a flip-flop needs a path to its `D`");

        // Two slices, each with a clock enable, a set/reset, and two ways
        // into each of four flip-flops; and a carry chain with two ways
        // in, and per bit the sum and the carry each out to the output mux
        // and to the flip-flop.
        assert_eq!(p.len(), 2 * (2 + 4 * 2 + 2 + 4 * 2 * 2));

        for hop in &p {
            assert_eq!(
                hop.features.len(),
                1,
                "a slice hop is one multiplexer, not a route: {hop:?}"
            );
            let feature = &hop.features[0];
            assert!(
                feature.contains("FFMUX")
                    || feature.contains("OUTMUX")
                    || feature.contains("PRECYINIT")
                    || feature.ends_with("CEUSEDMUX")
                    || feature.ends_with("SRUSEDMUX"),
                "unexpected slice pass-through `{feature}`"
            );
        }

        // The data hops end at a flip-flop's input, never at a wire that
        // leaves the slice; the output mux hops end at the `<L>MUX` the
        // interconnect reads.
        assert!(
            p.iter()
                .filter(|h| h.features.first().is_some_and(|f| f.contains("FFMUX")))
                .all(|h| h.to.ends_with("FF_D")),
            "a data hop must end inside a flip-flop: {p:?}"
        );
        assert!(
            p.iter()
                .filter(|h| h.features.first().is_some_and(|f| f.contains("OUTMUX")))
                .all(|h| h.to.ends_with("MUX") && !h.to.contains("_MUX_")),
            "an output hop must end on the slice's output: {p:?}"
        );
    }

    /// A `CARRY4`'s propagate inputs are the lookup tables' own output
    /// wires, its top carry out is the database's `COUT`, and everything
    /// else it has sits inside the slice.
    ///
    /// Would catch: `S` put on an invented wire (the placer would then
    /// see no dedicated connection and the router no path), `CO[3]` off
    /// `COUT` (the chain would have no way up), or the two slices' pins
    /// crossed. Would not catch: the letters being in the wrong order —
    /// that is UG474's drawing, and `tests/fpga_xray_carry.rs` reads it
    /// back out of a routed bitstream.
    #[test]
    fn a_carry4_takes_its_propagate_from_the_lookup_tables_beside_it() {
        let pins = bel_pins("CLBLL_L", "SLICEL_X1", "CARRY4");
        let wire = |role: &str| {
            pins.iter()
                .find(|p| p.role == role)
                .map(|p| p.wire.as_str())
                .unwrap_or_else(|| panic!("no `{role}`"))
        };
        assert_eq!(pins.len(), 18);
        assert_eq!(wire("p0"), "CLBLL_L_A");
        assert_eq!(wire("p3"), "CLBLL_L_D");
        assert_eq!(wire("co3"), "CLBLL_L_COUT");
        assert_eq!(wire("co0"), "CLBLL_L_CARRY_CY_A");
        assert_eq!(wire("s2"), "CLBLL_L_CARRY_XOR_C");
        assert_eq!(wire("ci"), "CLBLL_L_CARRY_CI");
        assert_eq!(
            wire("di1"),
            "CLBLL_L_BX",
            "a routed generate is the bypass input"
        );
        assert_eq!(
            bel_pins("CLBLM_R", "SLICEM_X0", "CARRY4")[0].wire,
            "CLBLM_M_A",
            "a CLBLM's X0 is its SLICEM"
        );
        // And a tied carry in or generate costs what prjxray names.
        let tied = bel_config("CLBLL_L", "SLICEL_X0", "CARRY4").tied;
        assert!(tied.contains(&(
            "cyinit".to_owned(),
            true,
            vec!["SLICEL_X0.PRECYINIT.C1".to_owned()]
        )));
        assert!(tied.contains(&(
            "di2".to_owned(),
            false,
            vec!["SLICEL_X0.CARRY4.CCY0".to_owned()]
        )));
        assert!(
            !tied.iter().any(|(pin, value, _)| pin == "cyinit" && !value),
            "a carry in tied to zero is `C0`, which is no bits at all"
        );
    }

    /// The rebuffer enable is the one wire feature that is not named
    /// after its wire, and the `TOP` / `BELOW` crossing is the part that
    /// would be silently wrong if it were guessed.
    #[test]
    fn a_global_clock_track_names_the_rebuffer_enable_it_needs() {
        assert_eq!(
            global_clock_enable("CLK_BUFG_REBUF_R_CK_GCLK0_TOP").as_deref(),
            Some("GCLK0_ENABLE_BELOW")
        );
        assert_eq!(
            global_clock_enable("CLK_BUFG_REBUF_R_CK_GCLK0_BOT").as_deref(),
            Some("GCLK0_ENABLE_ABOVE")
        );
        assert_eq!(
            global_clock_enable("CLK_BUFG_REBUF_R_CK_GCLK16_BOT").as_deref(),
            Some("GCLK16_ENABLE_ABOVE")
        );
        // And it claims nothing about a wire that is not one of those.
        for wire in [
            "CLK_BUFG_REBUF_R_CK_GCLK0",
            "CLK_BUFG_REBUF_LH12_1",
            "CLK_HROW_R_CK_GCLK0",
            "HCLK_CK_BUFHCLK0",
            "CLBLL_LL_A1",
            "_TOP",
        ] {
            assert_eq!(global_clock_enable(wire), None, "{wire}");
        }
        // The suffix forms still come first, so a wire that has both is
        // charged for both.
        let names = wire_enable_features("CLK_BUFG_REBUF_R_CK_GCLK0_TOP");
        assert_eq!(names[0], "CLK_BUFG_REBUF_R_CK_GCLK0_TOP_ACTIVE");
        assert_eq!(names[3], "GCLK0_ENABLE_BELOW");
        assert_eq!(wire_enable_features("CLBLL_LL_A1").len(), 3);
    }

    /// The track number, read from a wire and from a feature, is the one
    /// thing that says a rebuffer enable belongs to *this* clock.
    #[test]
    fn a_rebuffer_wire_and_its_feature_name_the_same_track() {
        assert_eq!(
            global_clock_track("CLK_BUFG_REBUF_R_CK_GCLK16_BOT"),
            Some(("BOT", 16))
        );
        assert_eq!(global_clock_track("CLK_BUFG_REBUF_R_CK_GCLK16"), None);
        assert_eq!(global_clock_track("CLK_HROW_R_CK_GCLK0"), None);
        assert_eq!(global_clock_enable_track("GCLK16_ENABLE_ABOVE"), Some(16));
        assert_eq!(global_clock_enable_track("GCLK0_ENABLE_BELOW"), Some(0));
        for feature in [
            "GCLK0_ENABLE_SIDEWAYS",
            "GCLKX_ENABLE_ABOVE",
            "CLK_HROW_R_CK_GCLK0_ACTIVE",
            "ENABLE_BUFFER.HCLK_CK_BUFHCLK0",
        ] {
            assert_eq!(global_clock_enable_track(feature), None, "{feature}");
        }
    }

    /// A block RAM half is named by parity, so `RAMB18_Y0` is the lower,
    /// even site — the opposite of what the `IOB` ranking would give the
    /// same two names. Would catch the rank rule being applied to block
    /// RAM; would not catch prjxray's parity rule itself being wrong.
    #[test]
    fn a_block_ram_half_is_named_by_parity_not_rank() {
        let t = tile(&[
            ("RAMB18_X0Y40", "FIFO18E1"),
            ("RAMB18_X0Y41", "RAMB18E1"),
            ("RAMB36_X0Y20", "RAMBFIFO36E1"),
        ]);
        assert_eq!(site_of_prefix(&t, "RAMB18_Y0").unwrap().0, "RAMB18_X0Y40");
        assert_eq!(site_of_prefix(&t, "RAMB18_Y1").unwrap().0, "RAMB18_X0Y41");
        assert!(site_of_prefix(&t, "RAMB18_Y2").is_none());
        // And the IO rule is untouched: `IOB_Y0` is still the higher site.
        let io = tile(&[("IOB_X0Y11", "IOB33S"), ("IOB_X0Y12", "IOB33M")]);
        assert_eq!(site_of_prefix(&io, "IOB_Y0").unwrap().0, "IOB_X0Y12");
    }

    #[test]
    fn only_lvcmos33_is_offered() {
        assert_eq!(io_standards(), vec!["LVCMOS33"]);
        assert!(io_standard("LVCMOS18").is_none());
        let s = io_standard("LVCMOS33").unwrap();
        assert_eq!(with_index(s.input[4], 1), "IOB_Y1.PULLTYPE.NONE");
    }
}
