//! The XADC: a bel on the `MONITOR_BOT` tile, and the configuration bits
//! of its initial registers, which the database does not describe.
//!
//! # Why this module derives what prjxray never fuzzed
//!
//! prjxray-db has no `segbits_monitor_bot.db` and no `ppips` for the tile:
//! an XADC's bits are unexplained in every image that uses one, and its
//! pins reach nothing in this loader's graph. NewAE's ChipWhisperer-Husky
//! uses one — through Vivado's `xadc_wiz` — for its temperature and supply
//! alarms, so both halves are supplied here.
//!
//! **The pins** come from prjxray's own `tile_type_MONITOR_BOT.json`, which
//! the fetch does not take and which was read once, by script, to write
//! [`PINS`]: every fabric-facing site pin of the `XADC` site, its wire, and
//! the one fixed pass-transistor connection between that wire and the
//! interconnect side (`MONITOR_IMUX<n>_<k>`, `MONITOR_CLK1_<k>`,
//! `MONITOR_CTRL1_2`, `MONITOR_LOGIC_OUTS_B<n>_<k>`), which reaches an
//! interconnect tile over `tileconn.json`'s joins like any other. The
//! analogue pins (`VP`, `VN`, `VAUX*`) are the dedicated pads' and are not
//! offered.
//!
//! **The register layout** was derived from NewAE's image, and the
//! derivation is the evidence: the wizard's `.xci` records the 27 initial
//! registers it writes (`C_CONFIGURATION_R0..2` are `INIT_40..42`,
//! `C_SEQUENCE_R0..7` are `INIT_48..4F`, `C_ALARM_LIMIT_R0..15` are
//! `INIT_50..5F`). Dropping the six that exist only on a Zynq
//! (`INIT_59..5B`, `INIT_5D..5F`) leaves 92 ones, and the tile holds
//! exactly 92 bits besides frame ECC: 89 in frames 28 and 29 and three
//! elsewhere. The layout prjxray found for a PLL's DRP registers —
//! register `r`, bit `b`, index `i = (r - 0x40) * 16 + b`, frame
//! `28 + i % 2`, bit `1696 + i / 2` — places **88 of the 89** exactly; the
//! other is register `0x5C`, the VBRAM lower alarm, which Vivado wrote as
//! `0x5111` where the wizard records `0x9999`: a question of which wizard
//! field feeds that register, not of the layout, and with `0x5111` all 89
//! land. The three bits outside frames 28 and 29 (`25_1613`, `26_1888`,
//! `27_1928`) belong to no register and are set whenever an XADC is used,
//! as Vivado set them; they are `XADC.IN_USE` here, a name this module
//! gives them.
//!
//! What a single image cannot show: a register bit that is zero in all of
//! Vivado's values. The layout is regular, and it is the PLL's, so it is
//! taken to hold for those bits too — a reading, not a measurement, until a
//! second image with other values confirms it.

use super::Feature;
use super::sites::{BelPin, ExtraBel, PassThrough};
use crate::fpga::arch::ConfigBit;

/// The first bit of the `INIT` register file in the tile, as the layout
/// above places it.
const XADC_BASE: u32 = 1696;

/// Every fabric-facing pin of the XADC: role, the site pin's wire, the
/// wire it is joined to, and whether it is an input. Transcribed by script
/// from prjxray-db's `artix7/tile_type_MONITOR_BOT.json` at 0a0adde.
const PINS: [(&str, &str, &str, bool); 71] = [
    ("alm0", "MONITOR_ALM0", "MONITOR_LOGIC_OUTS_B11_2", false),
    ("alm1", "MONITOR_ALM1", "MONITOR_LOGIC_OUTS_B12_2", false),
    ("alm2", "MONITOR_ALM2", "MONITOR_LOGIC_OUTS_B13_2", false),
    ("alm3", "MONITOR_ALM3", "MONITOR_LOGIC_OUTS_B14_2", false),
    ("alm4", "MONITOR_ALM4", "MONITOR_LOGIC_OUTS_B15_2", false),
    ("alm5", "MONITOR_ALM5", "MONITOR_LOGIC_OUTS_B16_2", false),
    ("alm6", "MONITOR_ALM6", "MONITOR_LOGIC_OUTS_B17_2", false),
    ("alm7", "MONITOR_ALM7", "MONITOR_LOGIC_OUTS_B18_2", false),
    ("busy", "MONITOR_BUSY", "MONITOR_LOGIC_OUTS_B20_1", false),
    (
        "channel0",
        "MONITOR_CHANNEL0",
        "MONITOR_LOGIC_OUTS_B15_1",
        false,
    ),
    (
        "channel1",
        "MONITOR_CHANNEL1",
        "MONITOR_LOGIC_OUTS_B16_1",
        false,
    ),
    (
        "channel2",
        "MONITOR_CHANNEL2",
        "MONITOR_LOGIC_OUTS_B17_1",
        false,
    ),
    (
        "channel3",
        "MONITOR_CHANNEL3",
        "MONITOR_LOGIC_OUTS_B18_1",
        false,
    ),
    (
        "channel4",
        "MONITOR_CHANNEL4",
        "MONITOR_LOGIC_OUTS_B19_1",
        false,
    ),
    ("convst", "MONITOR_CONVST", "MONITOR_IMUX43_1", true),
    ("convstclk", "MONITOR_CONVSTCLK", "MONITOR_CLK1_1", true),
    ("daddr0", "MONITOR_DADDR0", "MONITOR_IMUX34_1", true),
    ("daddr1", "MONITOR_DADDR1", "MONITOR_IMUX35_1", true),
    ("daddr2", "MONITOR_DADDR2", "MONITOR_IMUX36_1", true),
    ("daddr3", "MONITOR_DADDR3", "MONITOR_IMUX37_1", true),
    ("daddr4", "MONITOR_DADDR4", "MONITOR_IMUX38_1", true),
    ("daddr5", "MONITOR_DADDR5", "MONITOR_IMUX39_1", true),
    ("daddr6", "MONITOR_DADDR6", "MONITOR_IMUX40_1", true),
    ("dclk", "MONITOR_DCLK", "MONITOR_CLK1_0", true),
    ("den", "MONITOR_DEN", "MONITOR_IMUX41_1", true),
    ("di0", "MONITOR_DI0", "MONITOR_IMUX28_0", true),
    ("di1", "MONITOR_DI1", "MONITOR_IMUX29_0", true),
    ("di2", "MONITOR_DI2", "MONITOR_IMUX30_0", true),
    ("di3", "MONITOR_DI3", "MONITOR_IMUX31_0", true),
    ("di4", "MONITOR_DI4", "MONITOR_IMUX32_0", true),
    ("di5", "MONITOR_DI5", "MONITOR_IMUX33_0", true),
    ("di6", "MONITOR_DI6", "MONITOR_IMUX34_0", true),
    ("di7", "MONITOR_DI7", "MONITOR_IMUX35_0", true),
    ("di8", "MONITOR_DI8", "MONITOR_IMUX36_0", true),
    ("di9", "MONITOR_DI9", "MONITOR_IMUX37_0", true),
    ("di10", "MONITOR_DI10", "MONITOR_IMUX38_0", true),
    ("di11", "MONITOR_DI11", "MONITOR_IMUX39_0", true),
    ("di12", "MONITOR_DI12", "MONITOR_IMUX40_0", true),
    ("di13", "MONITOR_DI13", "MONITOR_IMUX41_0", true),
    ("di14", "MONITOR_DI14", "MONITOR_IMUX42_0", true),
    ("di15", "MONITOR_DI15", "MONITOR_IMUX43_0", true),
    ("do0", "MONITOR_DO0", "MONITOR_LOGIC_OUTS_B8_0", false),
    ("do1", "MONITOR_DO1", "MONITOR_LOGIC_OUTS_B9_0", false),
    ("do2", "MONITOR_DO2", "MONITOR_LOGIC_OUTS_B10_0", false),
    ("do3", "MONITOR_DO3", "MONITOR_LOGIC_OUTS_B11_0", false),
    ("do4", "MONITOR_DO4", "MONITOR_LOGIC_OUTS_B12_0", false),
    ("do5", "MONITOR_DO5", "MONITOR_LOGIC_OUTS_B13_0", false),
    ("do6", "MONITOR_DO6", "MONITOR_LOGIC_OUTS_B14_0", false),
    ("do7", "MONITOR_DO7", "MONITOR_LOGIC_OUTS_B15_0", false),
    ("do8", "MONITOR_DO8", "MONITOR_LOGIC_OUTS_B16_0", false),
    ("do9", "MONITOR_DO9", "MONITOR_LOGIC_OUTS_B17_0", false),
    ("do10", "MONITOR_DO10", "MONITOR_LOGIC_OUTS_B18_0", false),
    ("do11", "MONITOR_DO11", "MONITOR_LOGIC_OUTS_B19_0", false),
    ("do12", "MONITOR_DO12", "MONITOR_LOGIC_OUTS_B20_0", false),
    ("do13", "MONITOR_DO13", "MONITOR_LOGIC_OUTS_B21_0", false),
    ("do14", "MONITOR_DO14", "MONITOR_LOGIC_OUTS_B22_0", false),
    ("do15", "MONITOR_DO15", "MONITOR_LOGIC_OUTS_B23_0", false),
    ("drdy", "MONITOR_DRDY", "MONITOR_LOGIC_OUTS_B14_1", false),
    ("dwe", "MONITOR_DWE", "MONITOR_IMUX42_1", true),
    ("eoc", "MONITOR_EOC", "MONITOR_LOGIC_OUTS_B22_1", false),
    ("eos", "MONITOR_EOS", "MONITOR_LOGIC_OUTS_B23_1", false),
    (
        "jtagbusy",
        "MONITOR_JTAGBUSY",
        "MONITOR_LOGIC_OUTS_B13_1",
        false,
    ),
    (
        "jtaglocked",
        "MONITOR_JTAGLOCKED",
        "MONITOR_LOGIC_OUTS_B11_1",
        false,
    ),
    (
        "jtagmodified",
        "MONITOR_JTAGMODIFIED",
        "MONITOR_LOGIC_OUTS_B12_1",
        false,
    ),
    (
        "muxaddr0",
        "MONITOR_MUXADDR0",
        "MONITOR_LOGIC_OUTS_B19_2",
        false,
    ),
    (
        "muxaddr1",
        "MONITOR_MUXADDR1",
        "MONITOR_LOGIC_OUTS_B20_2",
        false,
    ),
    (
        "muxaddr2",
        "MONITOR_MUXADDR2",
        "MONITOR_LOGIC_OUTS_B21_2",
        false,
    ),
    (
        "muxaddr3",
        "MONITOR_MUXADDR3",
        "MONITOR_LOGIC_OUTS_B22_2",
        false,
    ),
    (
        "muxaddr4",
        "MONITOR_MUXADDR4",
        "MONITOR_LOGIC_OUTS_B23_2",
        false,
    ),
    ("ot", "MONITOR_OT", "MONITOR_LOGIC_OUTS_B21_1", false),
    ("reset", "MONITOR_RESET", "MONITOR_CTRL1_2", true),
];

/// The XADC bel of a `MONITOR_BOT` tile, or nothing.
pub(super) fn xadc_bels(tile_type: &str) -> Vec<ExtraBel> {
    if tile_type != "MONITOR_BOT" {
        return Vec::new();
    }
    vec![ExtraBel {
        name: "XADC".to_owned(),
        kind: "sysmon",
        pins: PINS
            .iter()
            .map(|(role, wire, _, _)| BelPin {
                role,
                wire: (*wire).to_owned(),
            })
            .collect(),
        cells: vec![("XADC", vec!["XADC.IN_USE".to_owned()])],
    }]
}

/// The fixed connections between the XADC's pins and the interconnect,
/// which the database has no `ppips` file to say.
pub(super) fn xadc_pass_throughs(tile_type: &str) -> Vec<PassThrough> {
    if tile_type != "MONITOR_BOT" {
        return Vec::new();
    }
    PINS.iter()
        .map(|(_, pin, joined, input)| {
            let (to, from) = if *input { (pin, joined) } else { (joined, pin) };
            PassThrough {
                to: (*to).to_owned(),
                from: (*from).to_owned(),
                features: Vec::new(),
            }
        })
        .collect()
}

/// The bit of the tile holding bit `bit` of initial register `register`
/// (`0x40..=0x5F`), as the module documentation derives it.
#[must_use]
pub fn xadc_init_bit(register: u32, bit: u32) -> ConfigBit {
    let index = (register - 0x40) * 16 + bit;
    ConfigBit::new(28 + index % 2, XADC_BASE + index / 2)
}

/// The features a `MONITOR_BOT` tile has, which no `segbits` file gives:
/// `XADC.INIT_40[0]` .. `XADC.INIT_5F[15]` and `XADC.IN_USE`.
pub(super) fn derived_features(tile_type: &str) -> Option<Vec<Feature>> {
    if tile_type != "MONITOR_BOT" {
        return None;
    }
    let mut out = Vec::new();
    for register in 0x40..=0x5F_u32 {
        for bit in 0..16 {
            out.push(Feature {
                name: format!("XADC.INIT_{register:02X}[{bit}]"),
                ones: vec![xadc_init_bit(register, bit)],
                zeros: Vec::new(),
            });
        }
    }
    out.push(Feature {
        name: "XADC.IN_USE".to_owned(),
        ones: vec![
            ConfigBit::new(25, 1613),
            ConfigBit::new(26, 1888),
            ConfigBit::new(27, 1928),
        ],
        zeros: Vec::new(),
    });
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registers the Husky's wizard wrote, with `0x5C` as Vivado wrote
    /// it, placed through this layout, against the positions NewAE's image
    /// holds. Would catch the layout mis-transcribed — frame parity
    /// swapped, the base off by one, a register's bits reversed — since each
    /// moves dozens of the 89 bits. Would not catch a wrong reading of a bit
    /// Vivado never sets, which is what the module documentation says.
    #[test]
    fn the_layout_places_the_husky_registers_where_vivado_put_them() {
        let values: [(u32, u32); 14] = [
            (0x41, 0x20a0),
            (0x42, 0x0400),
            (0x48, 0x4700),
            (0x4a, 0x4700),
            (0x50, 0xb363),
            (0x51, 0x5999),
            (0x52, 0xa147),
            (0x53, 0xb5e3),
            (0x54, 0xa93a),
            (0x55, 0x5111),
            (0x56, 0x91eb),
            (0x57, 0xa93a),
            (0x58, 0x5999),
            (0x5c, 0x5111),
        ];
        let mut ours: Vec<(u32, u32)> = values
            .iter()
            .flat_map(|(r, v)| {
                (0..16)
                    .filter(move |b| v >> b & 1 == 1)
                    .map(move |b| xadc_init_bit(*r, b))
            })
            .map(|c| (c.row, c.col))
            .collect();
        ours.sort_unstable();
        assert_eq!(ours, IMAGE);
    }

    /// What NewAE's image holds in frames 28 and 29 of the XADC's tile
    /// (`MONITOR_BOT_X46Y79`), besides frame ECC.
    const IMAGE: [(u32, u32); 89] = [
        (28, 1717),
        (28, 1764),
        (28, 1765),
        (28, 1767),
        (28, 1780),
        (28, 1781),
        (28, 1783),
        (28, 1824),
        (28, 1827),
        (28, 1828),
        (28, 1830),
        (28, 1832),
        (28, 1834),
        (28, 1836),
        (28, 1838),
        (28, 1839),
        (28, 1840),
        (28, 1841),
        (28, 1843),
        (28, 1844),
        (28, 1848),
        (28, 1851),
        (28, 1852),
        (28, 1853),
        (28, 1854),
        (28, 1858),
        (28, 1860),
        (28, 1864),
        (28, 1866),
        (28, 1868),
        (28, 1870),
        (28, 1871),
        (28, 1872),
        (28, 1875),
        (28, 1876),
        (28, 1878),
        (28, 1882),
        (28, 1884),
        (28, 1888),
        (28, 1890),
        (28, 1892),
        (28, 1894),
        (28, 1895),
        (28, 1920),
        (28, 1922),
        (28, 1924),
        (28, 1926),
        (28, 1927),
        (29, 1706),
        (29, 1707),
        (29, 1710),
        (29, 1764),
        (29, 1780),
        (29, 1824),
        (29, 1826),
        (29, 1828),
        (29, 1830),
        (29, 1831),
        (29, 1833),
        (29, 1835),
        (29, 1837),
        (29, 1840),
        (29, 1846),
        (29, 1847),
        (29, 1848),
        (29, 1850),
        (29, 1851),
        (29, 1854),
        (29, 1855),
        (29, 1856),
        (29, 1857),
        (29, 1858),
        (29, 1861),
        (29, 1862),
        (29, 1863),
        (29, 1872),
        (29, 1873),
        (29, 1874),
        (29, 1875),
        (29, 1879),
        (29, 1880),
        (29, 1881),
        (29, 1882),
        (29, 1885),
        (29, 1886),
        (29, 1887),
        (29, 1889),
        (29, 1891),
        (29, 1893),
    ];
}
