//! The inside of a site: which bels a tile type contributes, which wire
//! each of their pins reaches, and what a cell on one costs in bits.
//!
//! ```text
//! =====================================================================
//! THESE ARE NOTES. [`super`] BUILDS ITS BELS FROM THE DATABASE INSTEAD.
//! =====================================================================
//! ```
//!
//! [`super`] declares the same lookup tables and the same IO buffers that
//! [`bels_for`] describes, and it does **not** call anything here: it reads
//! the wire names out of `bits.db`'s own records and checks that the tile
//! type really owns each one before it declares a pin. That is the
//! difference that matters — a table written down from reading `libtrellis`
//! can be wrong about a wire and nothing notices, while a pin whose wire
//! the type does not declare is a pad or a LUT that [`super`] leaves out.
//!
//! What is kept here is the part that is still not derivable, plus the
//! flip-flop table this module used to be the only home of:
//!
//! - **which pin of a bel a wire is.** `bits.db` names wires and the bits
//!   that join them; it does not say that `A0_SLICE` is a lookup table's
//!   first input or that `PADDOA_PIO` is what an output pad drives from.
//!   That mapping is `libtrellis`' own `Bels.cpp` and `Chip.cpp`, and
//!   [`super`] carries the same two facts inline where it needs them. This
//!   is where they are written out with their provenance.
//! - **the flip-flop's settings**, which are no longer unexercised:
//!   [`super::FF_PINS`] is the pin list [`super`] declares a flip-flop
//!   from and [`super::TrellisFabric::configure_registers`] writes the
//!   fields, taking each one's value from the cell's parameters rather than
//!   from the table below. The two agree, and the difference that matters
//!   is that [`super`]'s version is checked: a flip-flop is not declared
//!   unless the tile type owns all five wires and its `bits.db` can express
//!   "take the data from the fabric".
//!
//! Two consequences, and both matter:
//!
//! - **Nothing in this module has been checked against a part.** The tile
//!   rules in [`super`]'s header have been, for both edges it describes,
//!   and so has the clock network; what the table below says about the
//!   flip-flop has been *superseded* rather than verified.
//! - [`top_pad_tile`] and [`top_pic_tile`] describe the same rule [`super`]
//!   implements for the top edge, and [`bels_for`]'s sentence about a
//!   `PIOT1` contributing no bel **is now the right one**: a `PIOT1` holds
//!   the *bits* of a `PIOB` whose buffer is one column west, and [`super`]
//!   puts the bel at the buffer. That contradiction is resolved; the note
//!   is kept because reconciling it is what found the distinction.
//!
//! The ECP5's slice differs from both families this project has met
//! before in one way that matters a great deal: **a flip-flop has a data
//! input from the fabric.** A Gowin flip-flop's `D` comes from the lookup
//! table beside it inside the slice and has no tile wire at all, and a
//! 7-series `AFF`'s does not either, which is why neither family can route
//! anything sequential without packing a lookup table and a flip-flop onto
//! one site. Here `M<n>_SLICE` is a mux output the interconnect drives, and
//! `SLICE<l>.REG<n>.SD = 0` selects it over the lookup table's output, so a
//! flip-flop places and routes on its own — which is what it now does:
//! [`super`] declares the `M<n>` wires and their pips, and the clock that
//! reaches the `CLK<n>` ones comes off [`super::ClockNetwork`].

/// One bel a tile type contributes, at the position that tile sits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BelSpec {
    /// The bel's name, which becomes the tail of its site name.
    pub name: String,
    /// The role keyword a device file's primitives are matched against.
    pub kind: &'static str,
    /// Pin role to the wire it reaches, in the tile's own spelling.
    pub pins: Vec<(&'static str, String)>,
    /// The `.config` word holding a lookup table's truth table, for a
    /// `lut` bel.
    pub lut_init: Option<String>,
    /// The `.config_enum` settings a cell on this bel needs, as
    /// `(field, value)`. Empty for a bel whose configuration depends on
    /// the design rather than only on being occupied.
    pub enums: Vec<(String, String)>,
}

/// The letter Project Trellis names slice `index` with.
fn slice_letter(index: usize) -> char {
    ['A', 'B', 'C', 'D'][index & 3]
}

/// Which bels a tile of type `ty` contributes.
///
/// The top and bottom edges put two IO bels on a position and the left and
/// right edges put four, which is why the count is a property of the tile
/// type.
///
/// It says a `PIOT1` contributes no bel, on the grounds that it only holds
/// the *bits* of a `PIOB` whose buffer is one column west, and that is
/// right: [`super`] puts the bel at the buffer's position, which is a
/// `PIOT0`, because that is the tile whose namespace owns `PADDOB_PIO`.
/// While nothing routed [`super`] put it at the bits instead and this
/// module's header recorded the contradiction; the contradiction is gone.
pub fn bels_for(ty: &str) -> Vec<BelSpec> {
    let mut out = Vec::new();
    if ty == "PLC2" {
        // Eight lookup tables and eight flip-flops, two of each in each of
        // the four slices. This is libtrellis' "split slice" shape
        // (`add_logic_comb` and `add_ff`), which is the one that lets a
        // lookup table and a flip-flop be placed independently.
        for z in 0..8usize {
            let letter = slice_letter(z / 2);
            let half = z % 2;
            out.push(BelSpec {
                name: format!("SLICE{letter}.K{half}"),
                kind: "lut",
                pins: vec![
                    ("i0", format!("A{z}_SLICE")),
                    ("i1", format!("B{z}_SLICE")),
                    ("i2", format!("C{z}_SLICE")),
                    ("i3", format!("D{z}_SLICE")),
                    ("o", format!("F{z}_SLICE")),
                ],
                lut_init: Some(format!("SLICE{letter}.K{half}.INIT")),
                // `MODE = LOGIC` and `GSR = ENABLED` are the defaults and
                // cost no bits; nothing else a lookup table needs does
                // either, so a LUT is its truth table and nothing more.
                enums: Vec::new(),
            });
        }
        for z in 0..8usize {
            let letter = slice_letter(z / 2);
            let half = z % 2;
            let control = z / 2;
            out.push(BelSpec {
                name: format!("SLICE{letter}.FF{half}"),
                kind: "ff",
                pins: vec![
                    // The fabric data input, not the lookup table's.
                    ("d", format!("M{z}_SLICE")),
                    ("clk", format!("CLK{control}_SLICE")),
                    ("rst", format!("LSR{control}_SLICE")),
                    ("en", format!("CE{control}_SLICE")),
                    ("q", format!("Q{z}_SLICE")),
                ],
                lut_init: None,
                enums: vec![
                    // Read the data from the fabric (`M`) rather than from
                    // the lookup table beside it.
                    (format!("SLICE{letter}.REG{half}.SD"), "0".to_owned()),
                    // Reset rather than set, which is what a Reticle
                    // flip-flop primitive means by its reset.
                    (
                        format!("SLICE{letter}.REG{half}.REGSET"),
                        "RESET".to_owned(),
                    ),
                    (format!("SLICE{letter}.REG{half}.LSRMODE"), "LSR".to_owned()),
                    (format!("SLICE{letter}.GSR"), "ENABLED".to_owned()),
                    // No clock enable: the enable mux passes a constant
                    // one. A design that wants one needs this to be `CE`,
                    // which this flow does not yet choose between.
                    (format!("SLICE{letter}.CEMUX"), "1".to_owned()),
                    (format!("CLK{control}.CLKMUX"), "CLK".to_owned()),
                    (format!("LSR{control}.LSRMUX"), "LSR".to_owned()),
                    (format!("LSR{control}.SRMODE"), "LSR_OVER_CE".to_owned()),
                ],
            });
        }
        return out;
    }
    let letters: &[usize] = if ty == "PIOT0" || is_bottom_a(ty) {
        &[0, 1]
    } else if ty == "SPICB0" {
        &[0]
    } else if ty.starts_with("PICL0") || ty.starts_with("PICR0") {
        &[0, 1, 2, 3]
    } else {
        &[]
    };
    for z in letters {
        let letter = slice_letter(*z);
        out.push(BelSpec {
            name: format!("PIO{letter}"),
            kind: "io",
            // There is no `pad` pin. A package ball is not a wire a router
            // can reach, which is what `super::super::xray` and
            // `super::super::apicula` both say about their IO too.
            pins: vec![
                ("dout", format!("PADDO{letter}_PIO")),
                ("oe", format!("PADDT{letter}_PIO")),
                ("din", format!("JPADDI{letter}_PIO")),
            ],
            lut_init: None,
            // An IO buffer's bits are spread over four tiles, two of which
            // are not the bel's own, and depend on the standard the board
            // supplies. They are `super::Periphery`'s and not a bel's.
            enums: Vec::new(),
        });
    }
    out
}

/// The bottom-edge tile types that carry a `PIOA` and a `PIOB`.
fn is_bottom_a(ty: &str) -> bool {
    matches!(ty, "PICB0" | "EFB0_PICB0" | "EFB2_PICB0")
}

/// The tile types that hold the interconnect's constant drivers, which is
/// where an output pad's data comes from when a design ties it.
///
/// `libtrellis` calls these the CIB tiles and nextpnr's ECP5 writer uses
/// exactly this set to find the `CIB.<wire>MUX` field that ties a wire.
pub const CIB_TILE_TYPES: [&str; 5] = ["CIB", "CIB_LR", "CIB_LR_S", "CIB_EFB0", "CIB_EFB1"];

/// The tile type that holds a PIO's **pad** configuration, for a PIO on
/// the top edge.
///
/// `PIOA`'s pad bits are in the `PIOT0` at its own position and `PIOB`'s
/// are in the `PIOT1` one column east. That is not a guess: it is what
/// nextpnr's `get_pio_tile` does, and it is the reason a decode of a real
/// bitstream shows an output on ball A15 — `PIOB` of column 67 — configured
/// in the tile at column 68. **This much of the module is the rule
/// [`super`] implements and `docs/fpga-trellis.md` measured**, for all six
/// of a Cynthion's LEDs against the board's own gateware.
pub fn top_pad_tile(letter: char) -> Option<(&'static str, i32)> {
    match letter {
        'A' => Some(("PIOT0", 0)),
        'B' => Some(("PIOT1", 1)),
        _ => None,
    }
}

/// The tile type that holds a PIO's **fabric-side** configuration, for a
/// PIO on the top edge: the IO logic, the output data mux, and a second
/// copy of `BASE_TYPE`. One row south of the pad tile.
pub fn top_pic_tile(letter: char) -> Option<(&'static str, i32)> {
    match letter {
        'A' => Some(("PICT0", 0)),
        'B' => Some(("PICT1", 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logic_tile_offers_eight_lookup_tables_and_eight_flip_flops() {
        let bels = bels_for("PLC2");
        assert_eq!(bels.len(), 16);
        let luts: Vec<&BelSpec> = bels.iter().filter(|b| b.kind == "lut").collect();
        let ffs: Vec<&BelSpec> = bels.iter().filter(|b| b.kind == "ff").collect();
        assert_eq!(luts.len(), 8);
        assert_eq!(ffs.len(), 8);
        assert_eq!(luts[0].name, "SLICEA.K0");
        assert_eq!(luts[7].name, "SLICED.K1");
        assert_eq!(luts[0].lut_init.as_deref(), Some("SLICEA.K0.INIT"));
        assert_eq!(luts[5].lut_init.as_deref(), Some("SLICEC.K1.INIT"));
        assert_eq!(
            luts[3].pins,
            vec![
                ("i0", "A3_SLICE".to_owned()),
                ("i1", "B3_SLICE".to_owned()),
                ("i2", "C3_SLICE".to_owned()),
                ("i3", "D3_SLICE".to_owned()),
                ("o", "F3_SLICE".to_owned()),
            ]
        );
        // The flip-flop's data input is a fabric wire, which is the whole
        // reason a clocked design can route on this family.
        assert_eq!(ffs[0].name, "SLICEA.FF0");
        assert_eq!(ffs[0].pins[0], ("d", "M0_SLICE".to_owned()));
        assert_eq!(ffs[7].pins[0], ("d", "M7_SLICE".to_owned()));
        // Two flip-flops share one control set, which is what the `/2` is.
        assert_eq!(ffs[0].pins[1], ("clk", "CLK0_SLICE".to_owned()));
        assert_eq!(ffs[1].pins[1], ("clk", "CLK0_SLICE".to_owned()));
        assert_eq!(ffs[2].pins[1], ("clk", "CLK1_SLICE".to_owned()));
        assert!(
            ffs[0]
                .enums
                .contains(&("SLICEA.REG0.SD".to_owned(), "0".to_owned()))
        );
    }

    #[test]
    fn the_top_edge_has_two_io_bels_a_position_and_the_sides_have_four() {
        assert_eq!(bels_for("PIOT0").len(), 2);
        assert_eq!(bels_for("PIOT0")[0].name, "PIOA");
        assert_eq!(bels_for("PIOT0")[1].name, "PIOB");
        assert_eq!(bels_for("PIOT0")[0].kind, "io");
        // A PIOT1 holds the bits of a PIOB that belongs one column west,
        // and no bel of its own.
        assert!(bels_for("PIOT1").is_empty());
        assert_eq!(bels_for("PICL0").len(), 4);
        assert_eq!(bels_for("PICR0").len(), 4);
        assert_eq!(bels_for("PICL0")[3].name, "PIOD");
        assert_eq!(bels_for("PICB0").len(), 2);
        assert_eq!(bels_for("SPICB0").len(), 1);
        assert!(bels_for("CIB").is_empty());
        assert!(bels_for("TAP_DRIVE").is_empty());
        // No `pad` pin anywhere: a ball is not a wire.
        for spec in bels_for("PIOT0") {
            assert!(spec.pins.iter().all(|(role, _)| *role != "pad"));
        }
    }

    #[test]
    fn a_top_edge_pio_bs_bits_are_one_column_east_of_its_bel() {
        assert_eq!(top_pad_tile('A'), Some(("PIOT0", 0)));
        assert_eq!(top_pad_tile('B'), Some(("PIOT1", 1)));
        assert_eq!(top_pic_tile('A'), Some(("PICT0", 0)));
        assert_eq!(top_pic_tile('B'), Some(("PICT1", 1)));
        assert_eq!(top_pad_tile('C'), None);
        assert_eq!(top_pic_tile('D'), None);
        assert!(CIB_TILE_TYPES.contains(&"CIB"));
    }
}
