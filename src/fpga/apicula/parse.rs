//! The rules the Apicula database implies but does not state, and the
//! narrow views over its MessagePack that the loader reads.
//!
//! Everything here is a *derivation*, and every one of them is a place
//! this could be wrong, so each says where it came from and each has a
//! test that re-derives it. The database itself is data; these are the
//! four things a reader has to work out for itself.

use std::collections::{BTreeMap, BTreeSet};

use crate::msgpack::Msgpack;

/// One bit of a tile's bitmap as the database writes it: `(row, col)`.
pub type Coord = (u32, u32);

/// One row of a `shortval`, `longval` or `longfuses` table: the attribute
/// codes that select it, and the bits it sets.
pub type CodeRow = (Vec<i64>, Vec<Coord>);

/// One pip as the database states it: destination wire, source wire, and
/// the bits that switch it on.
pub(super) type PipRow<'a> = (&'a str, &'a str, Vec<Coord>);

/// One of Gowin's inter-tile wires, taken apart.
///
/// The naming rule is `{direction}{length}{number}{segment}`, stated in
/// Project Apicula's `doc/filestructure.md`: "W270 is a westward two-hop
/// wire, number 7, segment 0 (the root). W272 (segment 2) would be the
/// same wire, two tiles to the west." So four characters, and the same
/// piece of metal has a different name in each tile it crosses — which is
/// the same situation the 7 series is in, except that here the mapping is
/// arithmetic rather than a table.
///
/// The reader that names these is `apycula/chipdb.py::wire2global`, and
/// [`InterTileWire::parse`] is that function's regular expression:
/// `([NESW])([128]\d)(\d)`. Over the whole GW2A-18 database 168 of the
/// 3918 wire names match it, in 64 families, and the segments a family
/// uses are exactly `0..=length` for the one- and two-hop wires and
/// `{0, 4, 8}` for the eight-hop ones — so the length digit **is** the
/// wire's reach, which is what [`InterTileWire::reach`] relies on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterTileWire {
    /// `N`, `E`, `S` or `W`.
    pub direction: u8,
    /// How many tiles the wire reaches: 1, 2 or 8.
    pub length: u32,
    /// Which wire of that direction and length, `0..=9`.
    pub number: u32,
    /// Which tile along the wire this name belongs to, `0` being the
    /// root — the tile the wire starts in.
    pub segment: u32,
}

impl InterTileWire {
    /// Takes a wire name apart, or `None` when it is not one of these.
    pub fn parse(name: &str) -> Option<InterTileWire> {
        let bytes = name.as_bytes();
        if bytes.len() != 4 {
            return None;
        }
        let direction = bytes[0];
        if !matches!(direction, b'N' | b'E' | b'S' | b'W') {
            return None;
        }
        let length = match bytes[1] {
            b'1' => 1,
            b'2' => 2,
            b'8' => 8,
            _ => return None,
        };
        let number = u32::from(bytes[2].checked_sub(b'0')?);
        let segment = u32::from(bytes[3].checked_sub(b'0')?);
        if number > 9 || segment > 9 {
            return None;
        }
        Some(InterTileWire {
            direction,
            length,
            number,
            segment,
        })
    }

    /// The name of this wire's segment zero, which is the name the loader
    /// declares the node under: `E212` is `E21`.
    pub fn root_name(&self) -> String {
        format!(
            "{}{}{}",
            char::from(self.direction),
            self.length,
            self.number
        )
    }

    /// How far, in `(x, y)` tiles, the root tile is from the tile that
    /// calls the wire by this name.
    ///
    /// `x` is the grid column and `y` the grid row, which is how this
    /// loader lays a Gowin grid onto an [`Arch`](super::super::arch::Arch)
    /// — so `y` increases downward, and "north" in Gowin's own names means
    /// increasing `y`. Nothing in the model cares which way is up; it
    /// cares that adjacency is right.
    ///
    /// The step per segment is `chipdb.py`'s `dirlut`: `N` is `(+1, 0)` in
    /// `(row, col)`, `E` is `(0, -1)`, `S` is `(-1, 0)` and `W` is
    /// `(0, +1)`. So an `E` wire's root is at a *lower* column and a `N`
    /// wire's at a *higher* row.
    pub fn root_offset(&self) -> (i32, i32) {
        let segment = i32::try_from(self.segment).unwrap_or(0);
        match self.direction {
            b'N' => (0, segment),
            b'S' => (0, -segment),
            b'E' => (-segment, 0),
            // b'W'
            _ => (segment, 0),
        }
    }

    /// How far the wire reaches from its root, in `(x, y)` tiles, which is
    /// the span the root's [`WireDecl`](super::super::arch::WireDecl)
    /// carries.
    ///
    /// The reach is the *opposite* of a segment step: a wire named `E` has
    /// its root at a lower column and extends towards higher ones.
    pub fn reach(&self) -> (i32, i32) {
        let length = i32::try_from(self.length).unwrap_or(0);
        match self.direction {
            b'N' => (0, -length),
            b'S' => (0, length),
            b'E' => (length, 0),
            // b'W'
            _ => (-length, 0),
        }
    }
}

/// The `IOLOC` name of a tile on the die's IO ring, or `None` for a tile
/// that is not on it.
///
/// This is `chipdb.py::rc2tbrl_0`: the edge letter comes from which side
/// the tile is on, the index is the one-based column for the top and
/// bottom edges and the one-based row for the left and right ones, and the
/// four corner tiles are told which letter to use by the database's own
/// `corner_tiles_io` because a corner is on two edges at once. On a
/// GW2A-18 that map says `(0, 0)` and `(0, 55)` are top and `(54, 0)` and
/// `(54, 55)` are bottom, so the die has no `IOL1` or `IOR1` at all.
///
/// Getting this backwards would put a design's output on the wrong ball,
/// which is the sort of error only a board finds, so the loader builds the
/// map in this direction — over every tile of the ring — and inverts it,
/// rather than trying to read a name and guess where it points.
pub fn ioloc(
    rows: u32,
    cols: u32,
    corners: &BTreeMap<(u32, u32), String>,
    row: u32,
    col: u32,
) -> Option<String> {
    let edge = match corners.get(&(row, col)) {
        Some(e) => e.as_str(),
        None if row == 0 => "T",
        None if rows > 0 && row == rows - 1 => "B",
        None if col == 0 => "L",
        None if cols > 0 && col == cols - 1 => "R",
        None => return None,
    };
    let index = if edge == "T" || edge == "B" {
        col + 1
    } else {
        row + 1
    };
    Some(format!("IO{edge}{index}"))
}

/// The bits a `shortval`, `longval` or `longfuses` table sets for a set of
/// attribute codes.
///
/// This is `chipdb.py::get_table_fuses`, and it is the one piece of the
/// database's encoding that is a rule rather than a lookup. A row's key is
/// a tuple of *codes* — two for a `shortval` table, sixteen for a
/// `longval` one, one for `longfuses` — and the row's bits apply when
/// every code of the key is satisfied:
///
/// | code | meaning |
/// |---|---|
/// | `0` | the key ends here; the codes after it are padding |
/// | positive | this attribute value must be among `codes` |
/// | negative | this attribute value must **not** be among `codes` |
///
/// The negative form is what makes a Gowin bitstream not a blank sheet: a
/// row whose whole key is negative applies when *nothing* is set, so those
/// bits are on by default and a design turns them off. The reference
/// GW2A-18 stream `gowin_pack` produces for a design with nothing in it
/// has 862 bits set, and that is where most of them come from.
///
/// A code is looked up through the `logicinfo` table, which maps
/// `(attribute id, value id)` to the code a row's key holds. **The names
/// of those ids are not in the database** — they are Python dictionaries
/// in `apycula/attrids.py` — which is why this function takes codes and
/// not names, and why the loader configures no bel but the lookup table.
/// See `docs/fpga-gowin.md`.
pub fn fuses_for(table: &[CodeRow], codes: &BTreeSet<i64>) -> BTreeSet<Coord> {
    let mut bits = BTreeSet::new();
    for (key, row_bits) in table {
        let mut matched = true;
        for code in key {
            if *code == 0 {
                break;
            }
            let present = codes.contains(&code.abs());
            if (*code > 0 && !present) || (*code < 0 && present) {
                matched = false;
                break;
            }
        }
        if matched {
            bits.extend(row_bits.iter().copied());
        }
    }
    bits
}

/// Reads a `{dst: {src: [[row, col], ..]}}` map — the shape both `pips`
/// and `clock_pips` have — as `(dst, src, bits)` triples.
pub(super) fn pip_table(value: &Msgpack) -> Vec<PipRow<'_>> {
    let mut out = Vec::new();
    for (dst, sources) in value.pairs() {
        let Some(dst) = dst.as_str() else { continue };
        for (src, bits) in sources.pairs() {
            let Some(src) = src.as_str() else { continue };
            out.push((dst, src, coords(bits)));
        }
    }
    out
}

/// Reads a set of `[row, col]` pairs.
pub(super) fn coords(value: &Msgpack) -> Vec<Coord> {
    value
        .array()
        .iter()
        .filter_map(|pair| {
            let items = pair.array();
            Some((items.first()?.as_u32()?, items.get(1)?.as_u32()?))
        })
        .collect()
}

/// Reads a table whose keys are code tuples and whose values are bit sets,
/// which is what `shortval`, `longval` and `longfuses` hold.
pub(super) fn code_table(value: &Msgpack) -> Vec<CodeRow> {
    value
        .pairs()
        .iter()
        .map(|(key, bits)| {
            let key = key.array().iter().filter_map(Msgpack::as_i64).collect();
            (key, coords(bits))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inter_tile_names_are_the_ones_apicula_matches() {
        // Every name in `doc/filestructure.md`'s example, plus the shapes
        // that must *not* match: tile-local wires, site wires and the
        // clock spines all live in the same namespace.
        let w = InterTileWire::parse("W270").unwrap();
        assert_eq!(w.direction, b'W');
        assert_eq!(w.length, 2);
        assert_eq!(w.number, 7);
        assert_eq!(w.segment, 0);
        assert_eq!(w.root_name(), "W27");
        let w = InterTileWire::parse("W272").unwrap();
        assert_eq!(w.segment, 2);
        assert_eq!(w.root_name(), "W27");
        for name in [
            "A0",
            "F6",
            "Q2",
            "X08",
            "LB11",
            "VCC",
            "VSS",
            "SPINE0",
            "GT00",
            "LT02",
            "LWSPINETL1",
            "CLK0",
            "SN10",
            "N10",
            "N10000",
            "",
            "E9A0",
            "e210",
        ] {
            assert!(
                InterTileWire::parse(name).is_none(),
                "{name} should not be an inter-tile wire"
            );
        }
        // And the three lengths the database uses, and no others.
        assert_eq!(InterTileWire::parse("E100").unwrap().length, 1);
        assert_eq!(InterTileWire::parse("E200").unwrap().length, 2);
        assert_eq!(InterTileWire::parse("E800").unwrap().length, 8);
        assert!(InterTileWire::parse("E300").is_none());
    }

    #[test]
    fn a_wires_root_is_where_apiculas_dirlut_puts_it() {
        // `dirlut` in (row, col): N (+1, 0), E (0, -1), S (-1, 0),
        // W (0, +1); this loader reports (x = col, y = row).
        let at = |name: &str| InterTileWire::parse(name).unwrap().root_offset();
        assert_eq!(at("N210"), (0, 0));
        assert_eq!(at("N212"), (0, 2));
        assert_eq!(at("S212"), (0, -2));
        assert_eq!(at("E212"), (-2, 0));
        assert_eq!(at("W212"), (2, 0));
        // An eight-hop wire is tapped at segments 0, 4 and 8.
        assert_eq!(at("E814"), (-4, 0));
        assert_eq!(at("E818"), (-8, 0));
    }

    #[test]
    fn a_root_wires_span_is_the_length_digit_pointing_the_other_way() {
        // The root is behind the segments, so the wire reaches forward.
        let reach = |name: &str| InterTileWire::parse(name).unwrap().reach();
        assert_eq!(reach("E210"), (2, 0));
        assert_eq!(reach("W210"), (-2, 0));
        assert_eq!(reach("N210"), (0, -2));
        assert_eq!(reach("S210"), (0, 2));
        assert_eq!(reach("E810"), (8, 0));
        assert_eq!(reach("E110"), (1, 0));
        // The reach does not depend on which segment named it.
        assert_eq!(reach("E212"), reach("E210"));
    }

    #[test]
    fn the_io_ring_is_named_the_way_the_corner_map_says() {
        // A GW2A-18's corner map: both top corners are `T` and both
        // bottom ones `B`, so there is no `IOL1` or `IOR1`.
        let mut corners = BTreeMap::new();
        corners.insert((0, 0), "T".to_owned());
        corners.insert((0, 55), "T".to_owned());
        corners.insert((54, 0), "B".to_owned());
        corners.insert((54, 55), "B".to_owned());
        let name = |r, c| ioloc(55, 56, &corners, r, c);
        assert_eq!(name(0, 0).as_deref(), Some("IOT1"));
        assert_eq!(name(0, 1).as_deref(), Some("IOT2"));
        assert_eq!(name(0, 55).as_deref(), Some("IOT56"));
        assert_eq!(name(54, 0).as_deref(), Some("IOB1"));
        assert_eq!(name(54, 55).as_deref(), Some("IOB56"));
        assert_eq!(name(9, 0).as_deref(), Some("IOL10"));
        assert_eq!(name(9, 55).as_deref(), Some("IOR10"));
        // Nothing in the interior is on the ring.
        assert_eq!(name(10, 10), None);
    }

    #[test]
    fn a_tables_negative_codes_are_what_makes_a_bit_default_to_on() {
        // A `shortval` row as the GW2A-18's `CLS0` table has them: the
        // key `(-7, 0)` sets `(20, 3)` when code 7 is *absent*, and
        // `(3, 10)` sets `(22, 3)` when both 3 and 10 are present.
        let table = vec![
            (vec![-7, 0], vec![(20, 3)]),
            (vec![3, 10], vec![(22, 3)]),
            (vec![1, 0], vec![(22, 7), (20, 7)]),
        ];
        // Nothing set: only the negative row applies. This is the case
        // that makes an "empty" Gowin bitstream not all zeros.
        let none = fuses_for(&table, &BTreeSet::new());
        assert_eq!(none, BTreeSet::from([(20, 3)]));
        // Code 7 present: the negative row stops applying.
        let seven = fuses_for(&table, &BTreeSet::from([7]));
        assert!(seven.is_empty());
        // Both halves of a two-code key are needed.
        let half = fuses_for(&table, &BTreeSet::from([3]));
        assert_eq!(half, BTreeSet::from([(20, 3)]));
        let both = fuses_for(&table, &BTreeSet::from([3, 10]));
        assert_eq!(both, BTreeSet::from([(20, 3), (22, 3)]));
        // A zero ends the key, so `(1, 0)` needs only code 1.
        let one = fuses_for(&table, &BTreeSet::from([1]));
        assert_eq!(one, BTreeSet::from([(20, 3), (22, 7), (20, 7)]));
    }

    #[test]
    fn the_msgpack_views_read_the_shapes_the_database_has() {
        // {"D1": {"F0": [[16, 32], [19, 31]]}}
        let inner = vec![(
            Msgpack::Str("F0".into()),
            Msgpack::Array(vec![
                Msgpack::Array(vec![Msgpack::Uint(16), Msgpack::Uint(32)]),
                Msgpack::Array(vec![Msgpack::Uint(19), Msgpack::Uint(31)]),
            ]),
        )];
        let table = Msgpack::Map(vec![(Msgpack::Str("D1".into()), Msgpack::Map(inner))]);
        assert_eq!(
            pip_table(&table),
            vec![("D1", "F0", vec![(16, 32), (19, 31)])]
        );
        // {(-7, 0): [[20, 3]]}
        let codes = Msgpack::Map(vec![(
            Msgpack::Array(vec![Msgpack::Int(-7), Msgpack::Uint(0)]),
            Msgpack::Array(vec![Msgpack::Array(vec![
                Msgpack::Uint(20),
                Msgpack::Uint(3),
            ])]),
        )]);
        assert_eq!(code_table(&codes), vec![(vec![-7, 0], vec![(20, 3)])]);
    }
}
