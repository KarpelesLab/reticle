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
//! # NOTHING PRODUCED FROM THIS HAS BEEN LOADED INTO A PART
//!
//! The IO recipe below is a transcription of what Vivado itself did for
//! this exact board — see [`IO_STANDARDS`] — not a reading of a
//! datasheet. That makes it a measurement rather than a guess, and it
//! still does not make it a lit LED.

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
fn slice_wire_prefix(tile_type: &str, index: u32) -> Option<&'static str> {
    let pair: [&'static str; 2] = match tile_type {
        "CLBLL_L" | "CLBLL_R" => ["CLBLL_LL", "CLBLL_L"],
        "CLBLM_L" | "CLBLM_R" => ["CLBLM_M", "CLBLM_L"],
        _ => return None,
    };
    pair.get(index as usize).copied()
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
        return Vec::new();
    }

    // An IO buffer. UG471 calls the fabric-side pins `O` (what the input
    // buffer drives) and `I` (what the output buffer drives the pad
    // from); `xc7.dev` calls the same two roles `din` and `dout`. The
    // pad itself is deliberately absent: it is a package ball, not a
    // wire the router can reach, and a design's port net ends there.
    if base == "IOB" && (tile_type == "LIOB33" || tile_type == "RIOB33") {
        return vec![
            BelPin {
                role: "din",
                wire: format!("IOB_IBUF{index}"),
            },
            BelPin {
                role: "dout",
                wire: format!("IOB_O{index}"),
            },
        ];
    }

    Vec::new()
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
    let side = match tile_type {
        "LIOI3" | "LIOI3_TBYTESRC" | "LIOI3_TBYTETERM" => "LIOI",
        "RIOI3" | "RIOI3_TBYTESRC" | "RIOI3_TBYTETERM" => "RIOI",
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for n in 0..2u32 {
        // Pad -> fabric. `ZINV_D` is the ILOGIC's D input inverter held
        // off; it is the only bit Vivado sets on an otherwise default
        // input path, and it appears once per *used* input half.
        out.push(PassThrough {
            to: format!("IOI_ILOGIC{n}_O"),
            from: format!("{side}_ILOGIC{n}_D"),
            features: vec![format!("ILOGIC_Y{n}.ZINV_D")],
        });
        // Fabric -> pad. The OLOGIC's output mux takes D1, the OQ output
        // is in use, and the tristate path is a plain buffer.
        out.push(PassThrough {
            to: format!("{side}_OLOGIC{n}_OQ"),
            from: format!("IOI_OLOGIC{n}_D1"),
            features: vec![
                format!("OLOGIC_Y{n}.OMUX.D1"),
                format!("OLOGIC_Y{n}.OQUSED"),
                format!("OLOGIC_Y{n}.OSERDES.DATA_RATE_TQ.BUF"),
            ],
        });
    }
    out
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

/// Which primitive of `xc7.dev` each direction of an IO buffer is, so a
/// [`ConfigEntry::Cell`](crate::fpga::ConfigEntry) can select it.
///
/// `IBUF` drives the fabric from the pad and `OBUF` the pad from the
/// fabric; `OBUFT` and `IOBUF` also exist and are **not** here, because
/// their tristate path needs `OLOGIC` `T` features this module has not
/// measured.
pub(super) const IO_PRIMITIVES: [(&str, bool); 2] = [("IBUF", true), ("OBUF", false)];

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
        assert_eq!(names, vec![("din", "IOB_IBUF1"), ("dout", "IOB_O1")]);
        assert!(!pins.iter().any(|p| p.role == "pad"));
    }

    #[test]
    fn an_ioi3_declares_both_halves_of_both_directions() {
        let p = pass_throughs("LIOI3");
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].to, "IOI_ILOGIC0_O");
        assert_eq!(p[0].from, "LIOI_ILOGIC0_D");
        assert_eq!(p[0].features, vec!["ILOGIC_Y0.ZINV_D"]);
        assert_eq!(p[1].to, "LIOI_OLOGIC0_OQ");
        assert_eq!(p[1].from, "IOI_OLOGIC0_D1");
        assert_eq!(pass_throughs("RIOI3")[0].from, "RIOI_ILOGIC0_D");
        assert!(pass_throughs("CLBLL_L").is_empty());
    }

    #[test]
    fn only_lvcmos33_is_offered() {
        assert_eq!(io_standards(), vec!["LVCMOS33"]);
        assert!(io_standard("LVCMOS18").is_none());
        let s = io_standard("LVCMOS33").unwrap();
        assert_eq!(with_index(s.input[4], 1), "IOB_Y1.PULLTYPE.NONE");
    }
}
