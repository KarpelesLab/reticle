//! Naming a Lattice part from its `IDCODE`.
//!
//! **This module identifies and does not configure.** There is no ECP5
//! configuration sequence here and none is planned until there is an
//! ECP5 fabric to aim one at: `src/program/xilinx.rs` is what a device
//! backend looks like in this crate, and this file is a fifth of a page
//! of tables because reading an identifier is all it does.
//!
//! An `IDCODE` is the same shape for every vendor — IEEE 1149.1 clause
//! 12 — so the only vendor-specific part is what the device field means:
//!
//! | Bits | Field |
//! |---|---|
//! | 31..28 | version |
//! | 27..12 | part number |
//! | 11..1 | JEDEC manufacturer |
//! | 0 | always 1 |
//!
//! Lattice parts are matched on the **whole 32-bit value** rather than
//! on the low 28 bits the way [`super::xilinx::idcode_matches`] matches
//! a Xilinx part. That is not an oversight. On an ECP5 the top nibble is
//! not a silicon revision: `0x2…` is the LFE5U-12F, `0x4…` the LFE5U
//! parts above it, `0x8…` the LFE5UM5G and `0x0…` the LFE5UM, and three
//! of those share the device field `0x1111`. Masking the nibble off
//! would merge four different parts.

/// The 11-bit JEDEC manufacturer identity of Lattice Semiconductor:
/// JEP106 bank 1, code `0x21`, which lands in `IDCODE` bits 11..1.
pub const MANUFACTURER_LATTICE: u32 = 0x021;

/// The low twelve bits of every Lattice `IDCODE`: the manufacturer
/// field with IEEE 1149.1's mandatory `1` under it.
pub const LATTICE_IDCODE_TAIL: u32 = (MANUFACTURER_LATTICE << 1) | 1;

/// The 11-bit JEDEC manufacturer field of an `IDCODE`.
#[must_use]
pub fn manufacturer(idcode: u32) -> u32 {
    (idcode >> 1) & 0x7FF
}

/// The 16-bit part number field of an `IDCODE` (bits 27..12).
#[must_use]
pub fn part_number(idcode: u32) -> u32 {
    (idcode >> 12) & 0xFFFF
}

/// The version field of an `IDCODE` (bits 31..28).
#[must_use]
pub fn version(idcode: u32) -> u32 {
    idcode >> 28
}

/// True when an `IDCODE` names a Lattice part.
///
/// This is only the manufacturer field and the mandatory bit, so it says
/// who made the part and nothing about which one it is. It is also the
/// check that tells a real answer from a dead chain: an undriven TDO
/// reads as all ones or all zeros, and neither of those is a Lattice
/// identifier.
#[must_use]
pub fn is_lattice(idcode: u32) -> bool {
    idcode & 0xFFF == LATTICE_IDCODE_TAIL
}

/// Every ECP5 `IDCODE` this crate can name, with the part it belongs
/// to.
///
/// These are the values published for the family: they appear in
/// Lattice's own BSDL files for each part and, identically, in every
/// open tool that speaks to an ECP5 — OpenOCD's `src/flash/nor/lattice.c`
/// device table and Project Trellis' device database, which were the two
/// independent sources checked against each other while writing this.
/// Where a part has an automotive sibling with the same identifier, both
/// names are given, because the identifier genuinely cannot tell them
/// apart.
///
/// HIGH confidence in the numbers; **none of these parts has been
/// configured by Reticle** and this table claims nothing about that.
pub const ECP5_PARTS: [(u32, &str); 10] = [
    (0x2111_1043, "LFE5U-12F (or LAE5U-12F)"),
    (0x4111_1043, "LFE5U-25F"),
    (0x4111_2043, "LFE5U-45F"),
    (0x4111_3043, "LFE5U-85F"),
    (0x0111_1043, "LFE5UM-25F (or LAE5UM-25F)"),
    (0x0111_2043, "LFE5UM-45F (or LAE5UM-45F)"),
    (0x0111_3043, "LFE5UM-85F (or LAE5UM-85F)"),
    (0x8111_1043, "LFE5UM5G-25F"),
    (0x8111_2043, "LFE5UM5G-45F"),
    (0x8111_3043, "LFE5UM5G-85F"),
];

/// The ECP5 an `IDCODE` names, or `None`.
#[must_use]
pub fn ecp5_part(idcode: u32) -> Option<&'static str> {
    ECP5_PARTS
        .iter()
        .find(|(id, _)| *id == idcode)
        .map(|(_, name)| *name)
}

/// A sentence describing an `IDCODE`, for a report.
///
/// It always names the raw value first and the reading second, so a
/// wrong reading cannot hide the number it was read from.
#[must_use]
pub fn describe(idcode: u32) -> String {
    let mut text = format!("IDCODE {idcode:#010x}");
    if !is_lattice(idcode) {
        text.push_str(&format!(
            " (manufacturer field {:#05x}, which is not Lattice's {MANUFACTURER_LATTICE:#05x})",
            manufacturer(idcode)
        ));
        return text;
    }
    text.push_str(&format!(
        ": manufacturer {:#05x} (Lattice), part number {:#06x}, version {}",
        manufacturer(idcode),
        part_number(idcode),
        version(idcode)
    ));
    match ecp5_part(idcode) {
        Some(part) => text.push_str(&format!(" — {part}")),
        None => text.push_str(" — a Lattice part this crate has no name for"),
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields of an `IDCODE` are IEEE 1149.1 clause 12's, and every
    /// entry in the table is a Lattice one with bit 0 set.
    #[test]
    fn every_listed_part_is_a_well_formed_lattice_idcode() {
        for (idcode, name) in ECP5_PARTS {
            assert_eq!(idcode & 1, 1, "{name} must have bit 0 set");
            assert!(is_lattice(idcode), "{name} must be a Lattice identifier");
            assert_eq!(manufacturer(idcode), MANUFACTURER_LATTICE, "{name}");
            assert_eq!(ecp5_part(idcode), Some(name));
        }
    }

    /// Every identifier in the table is distinct, which is the whole
    /// reason the version nibble is not masked off.
    #[test]
    fn the_table_has_no_duplicates_and_needs_the_version_nibble() {
        let mut seen = Vec::new();
        for (idcode, name) in ECP5_PARTS {
            assert!(!seen.contains(&idcode), "{name} is listed twice");
            seen.push(idcode);
        }
        // Four parts share the device field 0x1111 and differ only in
        // the top nibble; masking it would merge them.
        let sharing: Vec<&str> = ECP5_PARTS
            .iter()
            .filter(|(id, _)| part_number(*id) == 0x1111)
            .map(|(_, name)| *name)
            .collect();
        assert_eq!(sharing.len(), 4, "got {sharing:?}");
    }

    /// A dead chain does not look like a part. This is the check that
    /// stops "the board is not answering" from being reported as an
    /// exotic Lattice device.
    #[test]
    fn an_undriven_chain_is_not_a_lattice_part() {
        assert!(!is_lattice(0x0000_0000));
        assert!(!is_lattice(0xFFFF_FFFF));
        // A Xilinx part is not one either.
        assert!(!is_lattice(super::super::xilinx::IDCODE_XC7A35T));
        assert!(describe(0xFFFF_FFFF).contains("not Lattice"));
        assert!(describe(0x0000_0000).contains("0x00000000"));
    }

    /// The description leads with the raw value, whatever it decodes
    /// to, and names the part when it knows one.
    #[test]
    fn a_description_leads_with_the_number() {
        let text = describe(0x4111_1043);
        assert!(text.starts_with("IDCODE 0x41111043"), "{text}");
        assert!(text.contains("LFE5U-25F"), "{text}");
        assert!(text.contains("Lattice"), "{text}");

        // A Lattice identifier that is not in the table is still
        // reported, as a Lattice part with no name.
        let text = describe(0x0127_0043);
        assert!(text.starts_with("IDCODE 0x01270043"), "{text}");
        assert!(text.contains("no name for"), "{text}");
    }
}
