//! The configuration primitives: `BSCANE2`, `CAPTUREE2`, `DNA_PORT`,
//! `EFUSE_USR`, `FRAME_ECCE2`, `ICAPE2`, `STARTUPE2` and `USR_ACCESSE2`, on the
//! sites of the `CFG_CENTER_MID` and `CFG_CENTER_TOP` tiles, and what Vivado
//! writes for each.
//!
//! # What was asked, and what Vivado wrote
//!
//! `testdata/fpga/vivado/config/`, by `tools/vivado-parity/config.sh`: one small
//! design per parameter, per wiring of a pin and per `write_bitstream` option
//! beside one, and `vivado.txt` there has the configuration tiles' bits of each,
//! as `--decode` reads them, and the stream's packets. **Checked** against that
//! by `tests/fpga_xray_config.rs`. **Nothing here has been on a board**, and
//! none of it is to be loaded into a part from here.
//!
//! | primitive | what Vivado wrote |
//! |---|---|
//! | `BSCANE2` | `BSCAN.JTAG_CHAIN_<n>` on `BSCAN_X0Y<n-1>`, which is the only site chain `n` may take (`PDRC-2`); a chain outside 1 to 4 is built as 1. `DISABLE_JTAG` is `BITSTREAM.GENERAL.DISABLE_JTAG`'s three bits. `TDO` must be connected (`PDCN-2`) |
//! | `CAPTUREE2` | no bit in any tile; `ONESHOT` `TRUE`, its default, is `COR0` bit 23 |
//! | `DNA_PORT` | three bits in `CFG_CENTER_MID`'s frames 26 and 27 the database does not name, `DNA_PORT.IN_USE` here. A `DNA_PORT` whose `DOUT` nothing reads is removed |
//! | `EFUSE_USR` | nothing at all but its routes |
//! | `FRAME_ECCE2` | no bit in any tile; `CTL0`'s bit 8 (`GLUTMASK_B`) clear, and bit 7 set for `FARSRC = "FAR"` |
//! | `ICAPE2` | three bits for each site in use, `ICAP_X0Y0.IN_USE` and `ICAP_X0Y1.IN_USE` here; `ICAP.ICAP_WIDTH_X8` or `_X16`, which both sites share: two of them set both their widths' bits. One with no `LOC` goes on `ICAP_X0Y1`. On `ICAP_X0Y0` alone, `CTL0` bit 30 |
//! | `STARTUPE2` | `STARTUP.PROG_USR`; three bits when `GSR` or `GTS` is a signal, `STARTUP.GSR_OR_GTS` here, and three when `USRCCLKO`, `USRDONEO` or `USRDONETS` is one, `STARTUP.USRCCLKO_OR_USRDONE`. A pin tied to a constant sets neither. A `CLK` that is a signal makes `STARTUPCLK` `USERCLK` (`COR0` bit 15) unless an option says otherwise |
//! | `USR_ACCESSE2` | nothing but its routes; the value is `BITSTREAM.CONFIG.USR_ACCESS`'s, in `AXSS` |
//!
//! The names `IN_USE`, `GSR_OR_GTS` and `USRCCLKO_OR_USRDONE` are this module's,
//! as `XADC.IN_USE` is `super::xadc`'s: each is three bits set together in every
//! design that has the case and in none that has not, and nothing says what the
//! silicon does with them. Which of a `STARTUPE2`'s pins sets which was read
//! one pin at a time: `USRCCLKTS`, `KEYCLEARB` and `PACK` as signals set nothing.
//!
//! # The pins
//!
//! Every site pin of these sites is on a wire of the tile's own, joined by one
//! fixed connection to a `CFG_CENTER_IMUX<n>_<k>`, `CFG_CENTER_LOGIC_OUTS_B<n>_<k>`
//! or `CFG_CENTER_CLK1_<k>` wire that `tileconn.json` joins to the interconnect,
//! through `VFRAME`. [`PINS`] was transcribed by script from prjxray-db's
//! `artix7/tile_type_CFG_CENTER_MID.json` and `tile_type_CFG_CENTER_TOP.json` at
//! 0a0adde, every one of its 319 lines: the database has no `ppips` file for
//! either tile. `DNA_PORT` and `EFUSE_USR` are on the `TOP` tile, which has no
//! bits, and their bits are in `MID`'s frames, so their bels are declared on the
//! `MID` tile with their pins ten rows up ([`far_pins`]).
//!
//! # What an input nobody drives reads
//!
//! Vivado ties every input of a `STARTUPE2` the design leaves open, `KEYCLEARB`,
//! `USRCCLKTS` and `USRDONETS` to one and the rest to zero, and a `CAPTUREE2`'s
//! `CAP` to zero; it leaves a `DNA_PORT`'s and an `ICAPE2`'s unrouted, and a
//! `CAPTUREE2`'s `CLK`. A pin tied to a constant is tied, on all of them.
//! [`wants`] asks the same of the block RAMs' ties, with a zero over a ground
//! fan and a one left at the interconnect's `VCC_WIRE` default, which Vivado
//! routes and no bit says. That matters more than anywhere else here: a
//! `STARTUPE2`'s `GSR` or `GTS` left unrouted reads one, which holds every
//! flip-flop of the part in reset or every output in tristate.
//!
//! A clock pin is on a `CLK_L` wire no ground fan reaches in two hops, and
//! Vivado's zero there comes over four (`GFAN1`, `FAN_ALT5`, `FAN_BOUNCE5`,
//! `CLK_L1`), which `bram::TiePolicy::ClockZero` takes; when it cannot be had
//! the pin is left, where it reads one, on a clock nothing selects. **Checked**:
//! every tie of the column the pins enter from is Vivado's, on all 108 designs
//! Vivado builds (`ground` lines in `vivado.txt`).

use super::Feature;
use super::XrayFabric;
use super::bram::{TiePolicy, Want};
use super::sites::{BelPin, ExtraBel, PassThrough};
use crate::fpga::arch::{ArchSite, ConfigBit, ConfigEntry, RoutingGraph, WireRef};
use crate::fpga::bitstream::Bitstream;
use crate::fpga::place::{Instance, Netlist, Placement};
use crate::fpga::xc7::options::ConfigUse;
use crate::ir::{Design, ModuleId};
use crate::logic::Bit;

/// The tile every bel here is declared on.
const MID: &str = "CFG_CENTER_MID";
/// The tile above it, ten rows up, which holds `DNA_PORT` and `EFUSE_USR`.
const TOP: &str = "CFG_CENTER_TOP";
/// Where [`TOP`] is from [`MID`]: `tileconn.json`'s `[0, -10]`.
const TOP_DY: i32 = -10;
/// The tile below it, 21 rows down, which holds no site and through which
/// thirteen of `USR_ACCESS`'s `DATA` reach the interconnect.
const BOT: &str = "CFG_CENTER_BOT";

/// Whether a tile type is one of the three of the configuration sites, whose
/// bels and wires a design with no such primitive does not have.
pub(super) fn is_config_tile(tile_type: &str) -> bool {
    matches!(tile_type, MID | TOP | BOT)
}

/// The bels whose sites are on [`TOP`].
const ON_TOP: [&str; 2] = ["DNA_PORT", "EFUSE_USR"];

/// One pin: the bel, the role, the site pin's wire and the wire it is joined to,
/// both without their `CFG_CENTER_` prefix, and whether it is an input.
type Pin = (&'static str, &'static str, &'static str, &'static str, bool);

/// The fixed hops on the way to a pin of another tile, on the tile they are in:
/// `MID`'s `CLK1_8` up to `DNA_PORT`'s clock, `TOP`'s `CLK1_0` down to
/// `ICAP_X0Y1`'s, and `USR_ACCESS`'s `DATA[14:2]`, which leave `MID` on a wire
/// of their own (`MID_USR_ACCESS_DATA<n>`, [`PINS`]' join) that `tileconn.json`
/// takes down to `BOT`, where this joins it to the interconnect. From the same
/// files as [`PINS`] and `tile_type_CFG_CENTER_BOT.json`, whose thirteen pips
/// are the last thirteen here. **The first build of a `USR_ACCESSE2` had
/// none of these**: its stream and its configuration tile were Vivado's, and
/// it routed nothing from `DATA[14:2]` (`the router gave up`), which a
/// comparison of the tiles alone does not see.
const HOPS: [(&str, &str, &str); 15] = [
    (MID, "CLK1_8", "MID_DNA_PORT_CLK"),
    (TOP, "CLK1_0", "TOP_ICAP1_CLK"),
    (BOT, "BOT_USR_ACCESS_DATA2", "LOGIC_OUTS_B13_19"),
    (BOT, "BOT_USR_ACCESS_DATA3", "LOGIC_OUTS_B14_19"),
    (BOT, "BOT_USR_ACCESS_DATA4", "LOGIC_OUTS_B15_19"),
    (BOT, "BOT_USR_ACCESS_DATA5", "LOGIC_OUTS_B16_19"),
    (BOT, "BOT_USR_ACCESS_DATA6", "LOGIC_OUTS_B17_19"),
    (BOT, "BOT_USR_ACCESS_DATA7", "LOGIC_OUTS_B18_19"),
    (BOT, "BOT_USR_ACCESS_DATA8", "LOGIC_OUTS_B19_19"),
    (BOT, "BOT_USR_ACCESS_DATA9", "LOGIC_OUTS_B20_19"),
    (BOT, "BOT_USR_ACCESS_DATA10", "LOGIC_OUTS_B21_19"),
    (BOT, "BOT_USR_ACCESS_DATA11", "LOGIC_OUTS_B22_19"),
    (BOT, "BOT_USR_ACCESS_DATA12", "LOGIC_OUTS_B23_19"),
    (BOT, "BOT_USR_ACCESS_DATA13", "LOGIC_OUTS_B10_19"),
    (BOT, "BOT_USR_ACCESS_DATA14", "LOGIC_OUTS_B11_19"),
];

/// Every fabric-facing pin of every site here, as [`pins`] reads it. Transcribed by
/// script from prjxray-db's `artix7/tile_type_CFG_CENTER_MID.json` and
/// `tile_type_CFG_CENTER_TOP.json` at 0a0adde: a pin with a capacitance is an
/// input, and the one pip whose end is the pin's wire is the join.
static PINS: [&str; 319] = [
    "BSCAN_X0Y0 capture BSCAN1_CAPTURE LOGIC_OUTS_B22_2 out",
    "BSCAN_X0Y0 drck BSCAN1_DRCK LOGIC_OUTS_B20_2 out",
    "BSCAN_X0Y0 reset BSCAN1_RESET LOGIC_OUTS_B18_2 out",
    "BSCAN_X0Y0 runtest BSCAN1_RUNTEST LOGIC_OUTS_B8_2 out",
    "BSCAN_X0Y0 sel BSCAN1_SEL LOGIC_OUTS_B14_2 out",
    "BSCAN_X0Y0 shift BSCAN1_SHIFT LOGIC_OUTS_B16_3 out",
    "BSCAN_X0Y0 tck BSCAN1_TCK LOGIC_OUTS_B14_3 out",
    "BSCAN_X0Y0 tdi BSCAN1_TDI LOGIC_OUTS_B12_3 out",
    "BSCAN_X0Y0 tdo BSCAN1_TDO IMUX33_3 in",
    "BSCAN_X0Y0 tms BSCAN1_TMS LOGIC_OUTS_B10_3 out",
    "BSCAN_X0Y0 update BSCAN1_UPDATE LOGIC_OUTS_B16_2 out",
    "BSCAN_X0Y1 capture BSCAN2_CAPTURE LOGIC_OUTS_B23_2 out",
    "BSCAN_X0Y1 drck BSCAN2_DRCK LOGIC_OUTS_B21_2 out",
    "BSCAN_X0Y1 reset BSCAN2_RESET LOGIC_OUTS_B19_2 out",
    "BSCAN_X0Y1 runtest BSCAN2_RUNTEST LOGIC_OUTS_B9_2 out",
    "BSCAN_X0Y1 sel BSCAN2_SEL LOGIC_OUTS_B15_2 out",
    "BSCAN_X0Y1 shift BSCAN2_SHIFT LOGIC_OUTS_B17_3 out",
    "BSCAN_X0Y1 tck BSCAN2_TCK LOGIC_OUTS_B15_3 out",
    "BSCAN_X0Y1 tdi BSCAN2_TDI LOGIC_OUTS_B13_3 out",
    "BSCAN_X0Y1 tdo BSCAN2_TDO IMUX34_3 in",
    "BSCAN_X0Y1 tms BSCAN2_TMS LOGIC_OUTS_B11_3 out",
    "BSCAN_X0Y1 update BSCAN2_UPDATE LOGIC_OUTS_B17_2 out",
    "BSCAN_X0Y2 capture BSCAN3_CAPTURE LOGIC_OUTS_B11_11 out",
    "BSCAN_X0Y2 drck BSCAN3_DRCK LOGIC_OUTS_B22_10 out",
    "BSCAN_X0Y2 reset BSCAN3_RESET LOGIC_OUTS_B20_10 out",
    "BSCAN_X0Y2 runtest BSCAN3_RUNTEST LOGIC_OUTS_B13_11 out",
    "BSCAN_X0Y2 sel BSCAN3_SEL LOGIC_OUTS_B16_10 out",
    "BSCAN_X0Y2 shift BSCAN3_SHIFT LOGIC_OUTS_B21_11 out",
    "BSCAN_X0Y2 tck BSCAN3_TCK LOGIC_OUTS_B19_11 out",
    "BSCAN_X0Y2 tdi BSCAN3_TDI LOGIC_OUTS_B17_11 out",
    "BSCAN_X0Y2 tdo BSCAN3_TDO IMUX38_11 in",
    "BSCAN_X0Y2 tms BSCAN3_TMS LOGIC_OUTS_B15_11 out",
    "BSCAN_X0Y2 update BSCAN3_UPDATE LOGIC_OUTS_B18_10 out",
    "BSCAN_X0Y3 capture BSCAN4_CAPTURE LOGIC_OUTS_B12_11 out",
    "BSCAN_X0Y3 drck BSCAN4_DRCK LOGIC_OUTS_B23_10 out",
    "BSCAN_X0Y3 reset BSCAN4_RESET LOGIC_OUTS_B21_10 out",
    "BSCAN_X0Y3 runtest BSCAN4_RUNTEST LOGIC_OUTS_B14_11 out",
    "BSCAN_X0Y3 sel BSCAN4_SEL LOGIC_OUTS_B17_10 out",
    "BSCAN_X0Y3 shift BSCAN4_SHIFT LOGIC_OUTS_B22_11 out",
    "BSCAN_X0Y3 tck BSCAN4_TCK LOGIC_OUTS_B20_11 out",
    "BSCAN_X0Y3 tdi BSCAN4_TDI LOGIC_OUTS_B18_11 out",
    "BSCAN_X0Y3 tdo BSCAN4_TDO IMUX39_11 in",
    "BSCAN_X0Y3 tms BSCAN4_TMS LOGIC_OUTS_B16_11 out",
    "BSCAN_X0Y3 update BSCAN4_UPDATE LOGIC_OUTS_B19_10 out",
    "CAPTURE cap CAPTURE_CAP IMUX37_8 in",
    "CAPTURE clk CAPTURE_CLK CLK1_9 in",
    "FRAME_ECC crcerror FRAME_ECC_CRCERROR LOGIC_OUTS_B15_10 out",
    "FRAME_ECC eccerror FRAME_ECC_ECCERROR LOGIC_OUTS_B20_9 out",
    "FRAME_ECC eccerrorsingle FRAME_ECC_ECCERRORSINGLE LOGIC_OUTS_B23_8 out",
    "FRAME_ECC far0 FRAME_ECC_FAR0 LOGIC_OUTS_B10_6 out",
    "FRAME_ECC far1 FRAME_ECC_FAR1 LOGIC_OUTS_B11_6 out",
    "FRAME_ECC far2 FRAME_ECC_FAR2 LOGIC_OUTS_B12_6 out",
    "FRAME_ECC far3 FRAME_ECC_FAR3 LOGIC_OUTS_B13_6 out",
    "FRAME_ECC far4 FRAME_ECC_FAR4 LOGIC_OUTS_B14_6 out",
    "FRAME_ECC far5 FRAME_ECC_FAR5 LOGIC_OUTS_B15_6 out",
    "FRAME_ECC far6 FRAME_ECC_FAR6 LOGIC_OUTS_B16_6 out",
    "FRAME_ECC far7 FRAME_ECC_FAR7 LOGIC_OUTS_B17_6 out",
    "FRAME_ECC far8 FRAME_ECC_FAR8 LOGIC_OUTS_B18_6 out",
    "FRAME_ECC far9 FRAME_ECC_FAR9 LOGIC_OUTS_B19_6 out",
    "FRAME_ECC far10 FRAME_ECC_FAR10 LOGIC_OUTS_B20_6 out",
    "FRAME_ECC far11 FRAME_ECC_FAR11 LOGIC_OUTS_B21_6 out",
    "FRAME_ECC far12 FRAME_ECC_FAR12 LOGIC_OUTS_B22_6 out",
    "FRAME_ECC far13 FRAME_ECC_FAR13 LOGIC_OUTS_B23_6 out",
    "FRAME_ECC far14 FRAME_ECC_FAR14 LOGIC_OUTS_B8_6 out",
    "FRAME_ECC far15 FRAME_ECC_FAR15 LOGIC_OUTS_B9_6 out",
    "FRAME_ECC far16 FRAME_ECC_FAR16 LOGIC_OUTS_B10_7 out",
    "FRAME_ECC far17 FRAME_ECC_FAR17 LOGIC_OUTS_B11_7 out",
    "FRAME_ECC far18 FRAME_ECC_FAR18 LOGIC_OUTS_B12_7 out",
    "FRAME_ECC far19 FRAME_ECC_FAR19 LOGIC_OUTS_B13_7 out",
    "FRAME_ECC far20 FRAME_ECC_FAR20 LOGIC_OUTS_B14_7 out",
    "FRAME_ECC far21 FRAME_ECC_FAR21 LOGIC_OUTS_B15_7 out",
    "FRAME_ECC far22 FRAME_ECC_FAR22 LOGIC_OUTS_B16_7 out",
    "FRAME_ECC far23 FRAME_ECC_FAR23 LOGIC_OUTS_B17_7 out",
    "FRAME_ECC far24 FRAME_ECC_FAR24 LOGIC_OUTS_B18_7 out",
    "FRAME_ECC far25 FRAME_ECC_FAR25 LOGIC_OUTS_B19_7 out",
    "FRAME_ECC synbit0 FRAME_ECC_SYNBIT0 LOGIC_OUTS_B18_8 out",
    "FRAME_ECC synbit1 FRAME_ECC_SYNBIT1 LOGIC_OUTS_B19_8 out",
    "FRAME_ECC synbit2 FRAME_ECC_SYNBIT2 LOGIC_OUTS_B20_8 out",
    "FRAME_ECC synbit3 FRAME_ECC_SYNBIT3 LOGIC_OUTS_B21_8 out",
    "FRAME_ECC synbit4 FRAME_ECC_SYNBIT4 LOGIC_OUTS_B22_8 out",
    "FRAME_ECC syndrome0 FRAME_ECC_SYNDROME0 LOGIC_OUTS_B14_1 out",
    "FRAME_ECC syndrome1 FRAME_ECC_SYNDROME1 LOGIC_OUTS_B15_1 out",
    "FRAME_ECC syndrome2 FRAME_ECC_SYNDROME2 LOGIC_OUTS_B16_1 out",
    "FRAME_ECC syndrome3 FRAME_ECC_SYNDROME3 LOGIC_OUTS_B17_1 out",
    "FRAME_ECC syndrome4 FRAME_ECC_SYNDROME4 LOGIC_OUTS_B18_1 out",
    "FRAME_ECC syndrome5 FRAME_ECC_SYNDROME5 LOGIC_OUTS_B19_1 out",
    "FRAME_ECC syndrome6 FRAME_ECC_SYNDROME6 LOGIC_OUTS_B20_1 out",
    "FRAME_ECC syndrome7 FRAME_ECC_SYNDROME7 LOGIC_OUTS_B21_1 out",
    "FRAME_ECC syndrome8 FRAME_ECC_SYNDROME8 LOGIC_OUTS_B22_1 out",
    "FRAME_ECC syndrome9 FRAME_ECC_SYNDROME9 LOGIC_OUTS_B23_1 out",
    "FRAME_ECC syndrome10 FRAME_ECC_SYNDROME10 LOGIC_OUTS_B10_2 out",
    "FRAME_ECC syndrome11 FRAME_ECC_SYNDROME11 LOGIC_OUTS_B11_2 out",
    "FRAME_ECC syndrome12 FRAME_ECC_SYNDROME12 LOGIC_OUTS_B12_2 out",
    "FRAME_ECC syndromevalid FRAME_ECC_SYNDROMEVALID LOGIC_OUTS_B13_2 out",
    "FRAME_ECC synword0 FRAME_ECC_SYNWORD0 LOGIC_OUTS_B20_7 out",
    "FRAME_ECC synword1 FRAME_ECC_SYNWORD1 LOGIC_OUTS_B21_7 out",
    "FRAME_ECC synword2 FRAME_ECC_SYNWORD2 LOGIC_OUTS_B22_7 out",
    "FRAME_ECC synword3 FRAME_ECC_SYNWORD3 LOGIC_OUTS_B23_7 out",
    "FRAME_ECC synword4 FRAME_ECC_SYNWORD4 LOGIC_OUTS_B8_7 out",
    "FRAME_ECC synword5 FRAME_ECC_SYNWORD5 LOGIC_OUTS_B9_7 out",
    "FRAME_ECC synword6 FRAME_ECC_SYNWORD6 LOGIC_OUTS_B17_8 out",
    "ICAP_X0Y0 clk ICAP0_CLK CLK1_6 in",
    "ICAP_X0Y0 csib ICAP0_CSIB IMUX40_11 in",
    "ICAP_X0Y0 i0 ICAP0_I0 IMUX28_4 in",
    "ICAP_X0Y0 i1 ICAP0_I1 IMUX29_4 in",
    "ICAP_X0Y0 i2 ICAP0_I2 IMUX30_4 in",
    "ICAP_X0Y0 i3 ICAP0_I3 IMUX31_4 in",
    "ICAP_X0Y0 i4 ICAP0_I4 IMUX32_4 in",
    "ICAP_X0Y0 i5 ICAP0_I5 IMUX33_4 in",
    "ICAP_X0Y0 i6 ICAP0_I6 IMUX34_4 in",
    "ICAP_X0Y0 i7 ICAP0_I7 IMUX35_4 in",
    "ICAP_X0Y0 i8 ICAP0_I8 IMUX36_4 in",
    "ICAP_X0Y0 i9 ICAP0_I9 IMUX37_4 in",
    "ICAP_X0Y0 i10 ICAP0_I10 IMUX38_4 in",
    "ICAP_X0Y0 i11 ICAP0_I11 IMUX39_4 in",
    "ICAP_X0Y0 i12 ICAP0_I12 IMUX40_4 in",
    "ICAP_X0Y0 i13 ICAP0_I13 IMUX41_4 in",
    "ICAP_X0Y0 i14 ICAP0_I14 IMUX42_4 in",
    "ICAP_X0Y0 i15 ICAP0_I15 IMUX43_4 in",
    "ICAP_X0Y0 i16 ICAP0_I16 IMUX27_5 in",
    "ICAP_X0Y0 i17 ICAP0_I17 IMUX28_5 in",
    "ICAP_X0Y0 i18 ICAP0_I18 IMUX29_5 in",
    "ICAP_X0Y0 i19 ICAP0_I19 IMUX30_5 in",
    "ICAP_X0Y0 i20 ICAP0_I20 IMUX31_5 in",
    "ICAP_X0Y0 i21 ICAP0_I21 IMUX32_5 in",
    "ICAP_X0Y0 i22 ICAP0_I22 IMUX33_5 in",
    "ICAP_X0Y0 i23 ICAP0_I23 IMUX34_5 in",
    "ICAP_X0Y0 i24 ICAP0_I24 IMUX35_5 in",
    "ICAP_X0Y0 i25 ICAP0_I25 IMUX36_5 in",
    "ICAP_X0Y0 i26 ICAP0_I26 IMUX37_5 in",
    "ICAP_X0Y0 i27 ICAP0_I27 IMUX38_5 in",
    "ICAP_X0Y0 i28 ICAP0_I28 IMUX39_5 in",
    "ICAP_X0Y0 i29 ICAP0_I29 IMUX40_5 in",
    "ICAP_X0Y0 i30 ICAP0_I30 IMUX41_5 in",
    "ICAP_X0Y0 i31 ICAP0_I31 IMUX42_5 in",
    "ICAP_X0Y0 o0 ICAP0_O0 LOGIC_OUTS_B16_4 out",
    "ICAP_X0Y0 o1 ICAP0_O1 LOGIC_OUTS_B17_4 out",
    "ICAP_X0Y0 o2 ICAP0_O2 LOGIC_OUTS_B18_3 out",
    "ICAP_X0Y0 o3 ICAP0_O3 LOGIC_OUTS_B19_3 out",
    "ICAP_X0Y0 o4 ICAP0_O4 LOGIC_OUTS_B20_3 out",
    "ICAP_X0Y0 o5 ICAP0_O5 LOGIC_OUTS_B21_3 out",
    "ICAP_X0Y0 o6 ICAP0_O6 LOGIC_OUTS_B22_3 out",
    "ICAP_X0Y0 o7 ICAP0_O7 LOGIC_OUTS_B23_3 out",
    "ICAP_X0Y0 o8 ICAP0_O8 LOGIC_OUTS_B8_3 out",
    "ICAP_X0Y0 o9 ICAP0_O9 LOGIC_OUTS_B9_3 out",
    "ICAP_X0Y0 o10 ICAP0_O10 LOGIC_OUTS_B10_4 out",
    "ICAP_X0Y0 o11 ICAP0_O11 LOGIC_OUTS_B11_4 out",
    "ICAP_X0Y0 o12 ICAP0_O12 LOGIC_OUTS_B12_4 out",
    "ICAP_X0Y0 o13 ICAP0_O13 LOGIC_OUTS_B13_4 out",
    "ICAP_X0Y0 o14 ICAP0_O14 LOGIC_OUTS_B14_4 out",
    "ICAP_X0Y0 o15 ICAP0_O15 LOGIC_OUTS_B15_4 out",
    "ICAP_X0Y0 o16 ICAP0_O16 LOGIC_OUTS_B16_5 out",
    "ICAP_X0Y0 o17 ICAP0_O17 LOGIC_OUTS_B17_5 out",
    "ICAP_X0Y0 o18 ICAP0_O18 LOGIC_OUTS_B18_4 out",
    "ICAP_X0Y0 o19 ICAP0_O19 LOGIC_OUTS_B19_4 out",
    "ICAP_X0Y0 o20 ICAP0_O20 LOGIC_OUTS_B20_4 out",
    "ICAP_X0Y0 o21 ICAP0_O21 LOGIC_OUTS_B21_4 out",
    "ICAP_X0Y0 o22 ICAP0_O22 LOGIC_OUTS_B22_4 out",
    "ICAP_X0Y0 o23 ICAP0_O23 LOGIC_OUTS_B23_4 out",
    "ICAP_X0Y0 o24 ICAP0_O24 LOGIC_OUTS_B8_4 out",
    "ICAP_X0Y0 o25 ICAP0_O25 LOGIC_OUTS_B9_4 out",
    "ICAP_X0Y0 o26 ICAP0_O26 LOGIC_OUTS_B10_5 out",
    "ICAP_X0Y0 o27 ICAP0_O27 LOGIC_OUTS_B11_5 out",
    "ICAP_X0Y0 o28 ICAP0_O28 LOGIC_OUTS_B12_5 out",
    "ICAP_X0Y0 o29 ICAP0_O29 LOGIC_OUTS_B13_5 out",
    "ICAP_X0Y0 o30 ICAP0_O30 LOGIC_OUTS_B14_5 out",
    "ICAP_X0Y0 o31 ICAP0_O31 LOGIC_OUTS_B15_5 out",
    "ICAP_X0Y0 rdwrb ICAP0_RDWRB IMUX41_11 in",
    "ICAP_X0Y1 clk ICAP1_CLK MID_ICAP1_CLK in",
    "ICAP_X0Y1 csib ICAP1_CSIB IMUX43_11 in",
    "ICAP_X0Y1 i0 ICAP1_I0 IMUX28_12 in",
    "ICAP_X0Y1 i1 ICAP1_I1 IMUX29_12 in",
    "ICAP_X0Y1 i2 ICAP1_I2 IMUX30_12 in",
    "ICAP_X0Y1 i3 ICAP1_I3 IMUX31_12 in",
    "ICAP_X0Y1 i4 ICAP1_I4 IMUX32_12 in",
    "ICAP_X0Y1 i5 ICAP1_I5 IMUX33_12 in",
    "ICAP_X0Y1 i6 ICAP1_I6 IMUX34_12 in",
    "ICAP_X0Y1 i7 ICAP1_I7 IMUX35_12 in",
    "ICAP_X0Y1 i8 ICAP1_I8 IMUX36_12 in",
    "ICAP_X0Y1 i9 ICAP1_I9 IMUX37_12 in",
    "ICAP_X0Y1 i10 ICAP1_I10 IMUX38_12 in",
    "ICAP_X0Y1 i11 ICAP1_I11 IMUX39_12 in",
    "ICAP_X0Y1 i12 ICAP1_I12 IMUX40_12 in",
    "ICAP_X0Y1 i13 ICAP1_I13 IMUX41_12 in",
    "ICAP_X0Y1 i14 ICAP1_I14 IMUX42_12 in",
    "ICAP_X0Y1 i15 ICAP1_I15 IMUX43_12 in",
    "ICAP_X0Y1 i16 ICAP1_I16 IMUX28_13 in",
    "ICAP_X0Y1 i17 ICAP1_I17 IMUX29_13 in",
    "ICAP_X0Y1 i18 ICAP1_I18 IMUX30_13 in",
    "ICAP_X0Y1 i19 ICAP1_I19 IMUX31_13 in",
    "ICAP_X0Y1 i20 ICAP1_I20 IMUX32_13 in",
    "ICAP_X0Y1 i21 ICAP1_I21 IMUX33_13 in",
    "ICAP_X0Y1 i22 ICAP1_I22 IMUX34_13 in",
    "ICAP_X0Y1 i23 ICAP1_I23 IMUX35_13 in",
    "ICAP_X0Y1 i24 ICAP1_I24 IMUX36_13 in",
    "ICAP_X0Y1 i25 ICAP1_I25 IMUX37_13 in",
    "ICAP_X0Y1 i26 ICAP1_I26 IMUX38_13 in",
    "ICAP_X0Y1 i27 ICAP1_I27 IMUX39_13 in",
    "ICAP_X0Y1 i28 ICAP1_I28 IMUX40_13 in",
    "ICAP_X0Y1 i29 ICAP1_I29 IMUX41_13 in",
    "ICAP_X0Y1 i30 ICAP1_I30 IMUX42_13 in",
    "ICAP_X0Y1 i31 ICAP1_I31 IMUX43_13 in",
    "ICAP_X0Y1 o0 ICAP1_O0 LOGIC_OUTS_B23_11 out",
    "ICAP_X0Y1 o1 ICAP1_O1 LOGIC_OUTS_B12_12 out",
    "ICAP_X0Y1 o2 ICAP1_O2 LOGIC_OUTS_B13_12 out",
    "ICAP_X0Y1 o3 ICAP1_O3 LOGIC_OUTS_B14_12 out",
    "ICAP_X0Y1 o4 ICAP1_O4 LOGIC_OUTS_B15_12 out",
    "ICAP_X0Y1 o5 ICAP1_O5 LOGIC_OUTS_B16_12 out",
    "ICAP_X0Y1 o6 ICAP1_O6 LOGIC_OUTS_B17_12 out",
    "ICAP_X0Y1 o7 ICAP1_O7 LOGIC_OUTS_B18_12 out",
    "ICAP_X0Y1 o8 ICAP1_O8 LOGIC_OUTS_B19_12 out",
    "ICAP_X0Y1 o9 ICAP1_O9 LOGIC_OUTS_B20_12 out",
    "ICAP_X0Y1 o10 ICAP1_O10 LOGIC_OUTS_B21_12 out",
    "ICAP_X0Y1 o11 ICAP1_O11 LOGIC_OUTS_B22_12 out",
    "ICAP_X0Y1 o12 ICAP1_O12 LOGIC_OUTS_B23_12 out",
    "ICAP_X0Y1 o13 ICAP1_O13 LOGIC_OUTS_B12_13 out",
    "ICAP_X0Y1 o14 ICAP1_O14 LOGIC_OUTS_B13_13 out",
    "ICAP_X0Y1 o15 ICAP1_O15 LOGIC_OUTS_B14_13 out",
    "ICAP_X0Y1 o16 ICAP1_O16 LOGIC_OUTS_B15_13 out",
    "ICAP_X0Y1 o17 ICAP1_O17 LOGIC_OUTS_B16_13 out",
    "ICAP_X0Y1 o18 ICAP1_O18 LOGIC_OUTS_B17_13 out",
    "ICAP_X0Y1 o19 ICAP1_O19 LOGIC_OUTS_B18_13 out",
    "ICAP_X0Y1 o20 ICAP1_O20 LOGIC_OUTS_B19_13 out",
    "ICAP_X0Y1 o21 ICAP1_O21 LOGIC_OUTS_B20_13 out",
    "ICAP_X0Y1 o22 ICAP1_O22 LOGIC_OUTS_B21_13 out",
    "ICAP_X0Y1 o23 ICAP1_O23 LOGIC_OUTS_B22_13 out",
    "ICAP_X0Y1 o24 ICAP1_O24 LOGIC_OUTS_B23_13 out",
    "ICAP_X0Y1 o25 ICAP1_O25 LOGIC_OUTS_B10_14 out",
    "ICAP_X0Y1 o26 ICAP1_O26 LOGIC_OUTS_B11_14 out",
    "ICAP_X0Y1 o27 ICAP1_O27 LOGIC_OUTS_B12_14 out",
    "ICAP_X0Y1 o28 ICAP1_O28 LOGIC_OUTS_B13_14 out",
    "ICAP_X0Y1 o29 ICAP1_O29 LOGIC_OUTS_B14_14 out",
    "ICAP_X0Y1 o30 ICAP1_O30 LOGIC_OUTS_B15_14 out",
    "ICAP_X0Y1 o31 ICAP1_O31 LOGIC_OUTS_B16_14 out",
    "ICAP_X0Y1 rdwrb ICAP1_RDWRB IMUX42_11 in",
    "STARTUP cfgclk STARTUP_CFGCLK LOGIC_OUTS_B14_10 out",
    "STARTUP cfgmclk STARTUP_CFGMCLK LOGIC_OUTS_B18_5 out",
    "STARTUP clk STARTUP_CLK CLK1_5 in",
    "STARTUP eos STARTUP_EOS LOGIC_OUTS_B23_9 out",
    "STARTUP gsr STARTUP_GSR IMUX40_8 in",
    "STARTUP gts STARTUP_GTS IMUX39_8 in",
    "STARTUP keyclearb STARTUP_KEYCLEARB IMUX36_8 in",
    "STARTUP pack STARTUP_PACK IMUX43_8 in",
    "STARTUP preq STARTUP_PREQ LOGIC_OUTS_B22_9 out",
    "STARTUP usrcclko STARTUP_USRCCLKO CLK1_7 in",
    "STARTUP usrcclkts STARTUP_USRCCLKTS IMUX38_8 in",
    "STARTUP usrdoneo STARTUP_USRDONEO IMUX42_8 in",
    "STARTUP usrdonets STARTUP_USRDONETS IMUX41_8 in",
    "USR_ACCESS cfgclk USR_ACCESS_CFGCLK LOGIC_OUTS_B9_5 out",
    "USR_ACCESS data0 USR_ACCESS_DATA0 LOGIC_OUTS_B11_0 out",
    "USR_ACCESS data1 USR_ACCESS_DATA1 LOGIC_OUTS_B12_0 out",
    "USR_ACCESS data2 USR_ACCESS_DATA2 MID_USR_ACCESS_DATA2 out",
    "USR_ACCESS data3 USR_ACCESS_DATA3 MID_USR_ACCESS_DATA3 out",
    "USR_ACCESS data4 USR_ACCESS_DATA4 MID_USR_ACCESS_DATA4 out",
    "USR_ACCESS data5 USR_ACCESS_DATA5 MID_USR_ACCESS_DATA5 out",
    "USR_ACCESS data6 USR_ACCESS_DATA6 MID_USR_ACCESS_DATA6 out",
    "USR_ACCESS data7 USR_ACCESS_DATA7 MID_USR_ACCESS_DATA7 out",
    "USR_ACCESS data8 USR_ACCESS_DATA8 MID_USR_ACCESS_DATA8 out",
    "USR_ACCESS data9 USR_ACCESS_DATA9 MID_USR_ACCESS_DATA9 out",
    "USR_ACCESS data10 USR_ACCESS_DATA10 MID_USR_ACCESS_DATA10 out",
    "USR_ACCESS data11 USR_ACCESS_DATA11 MID_USR_ACCESS_DATA11 out",
    "USR_ACCESS data12 USR_ACCESS_DATA12 MID_USR_ACCESS_DATA12 out",
    "USR_ACCESS data13 USR_ACCESS_DATA13 MID_USR_ACCESS_DATA13 out",
    "USR_ACCESS data14 USR_ACCESS_DATA14 MID_USR_ACCESS_DATA14 out",
    "USR_ACCESS data15 USR_ACCESS_DATA15 LOGIC_OUTS_B10_0 out",
    "USR_ACCESS data16 USR_ACCESS_DATA16 LOGIC_OUTS_B11_1 out",
    "USR_ACCESS data17 USR_ACCESS_DATA17 LOGIC_OUTS_B12_1 out",
    "USR_ACCESS data18 USR_ACCESS_DATA18 LOGIC_OUTS_B13_0 out",
    "USR_ACCESS data19 USR_ACCESS_DATA19 LOGIC_OUTS_B14_0 out",
    "USR_ACCESS data20 USR_ACCESS_DATA20 LOGIC_OUTS_B15_0 out",
    "USR_ACCESS data21 USR_ACCESS_DATA21 LOGIC_OUTS_B16_0 out",
    "USR_ACCESS data22 USR_ACCESS_DATA22 LOGIC_OUTS_B17_0 out",
    "USR_ACCESS data23 USR_ACCESS_DATA23 LOGIC_OUTS_B18_0 out",
    "USR_ACCESS data24 USR_ACCESS_DATA24 LOGIC_OUTS_B19_0 out",
    "USR_ACCESS data25 USR_ACCESS_DATA25 LOGIC_OUTS_B20_0 out",
    "USR_ACCESS data26 USR_ACCESS_DATA26 LOGIC_OUTS_B21_0 out",
    "USR_ACCESS data27 USR_ACCESS_DATA27 LOGIC_OUTS_B22_0 out",
    "USR_ACCESS data28 USR_ACCESS_DATA28 LOGIC_OUTS_B23_0 out",
    "USR_ACCESS data29 USR_ACCESS_DATA29 LOGIC_OUTS_B8_0 out",
    "USR_ACCESS data30 USR_ACCESS_DATA30 LOGIC_OUTS_B9_0 out",
    "USR_ACCESS data31 USR_ACCESS_DATA31 LOGIC_OUTS_B10_1 out",
    "USR_ACCESS datavalid USR_ACCESS_DATAVALID LOGIC_OUTS_B13_1 out",
    "DNA_PORT clk DNA_PORT_CLK TOP_DNA_PORT_CLK in",
    "DNA_PORT din DNA_PORT_DIN IMUX43_5 in",
    "DNA_PORT dout DNA_PORT_DOUT LOGIC_OUTS_B23_5 out",
    "DNA_PORT read DNA_PORT_READ IMUX41_5 in",
    "DNA_PORT shift DNA_PORT_SHIFT IMUX42_5 in",
    "EFUSE_USR efuseusr0 EFUSE_USR_EFUSEUSR0 LOGIC_OUTS_B16_6 out",
    "EFUSE_USR efuseusr1 EFUSE_USR_EFUSEUSR1 LOGIC_OUTS_B17_6 out",
    "EFUSE_USR efuseusr2 EFUSE_USR_EFUSEUSR2 LOGIC_OUTS_B18_6 out",
    "EFUSE_USR efuseusr3 EFUSE_USR_EFUSEUSR3 LOGIC_OUTS_B19_6 out",
    "EFUSE_USR efuseusr4 EFUSE_USR_EFUSEUSR4 LOGIC_OUTS_B20_6 out",
    "EFUSE_USR efuseusr5 EFUSE_USR_EFUSEUSR5 LOGIC_OUTS_B21_6 out",
    "EFUSE_USR efuseusr6 EFUSE_USR_EFUSEUSR6 LOGIC_OUTS_B22_6 out",
    "EFUSE_USR efuseusr7 EFUSE_USR_EFUSEUSR7 LOGIC_OUTS_B23_6 out",
    "EFUSE_USR efuseusr8 EFUSE_USR_EFUSEUSR8 LOGIC_OUTS_B16_7 out",
    "EFUSE_USR efuseusr9 EFUSE_USR_EFUSEUSR9 LOGIC_OUTS_B17_7 out",
    "EFUSE_USR efuseusr10 EFUSE_USR_EFUSEUSR10 LOGIC_OUTS_B18_7 out",
    "EFUSE_USR efuseusr11 EFUSE_USR_EFUSEUSR11 LOGIC_OUTS_B19_7 out",
    "EFUSE_USR efuseusr12 EFUSE_USR_EFUSEUSR12 LOGIC_OUTS_B20_7 out",
    "EFUSE_USR efuseusr13 EFUSE_USR_EFUSEUSR13 LOGIC_OUTS_B21_7 out",
    "EFUSE_USR efuseusr14 EFUSE_USR_EFUSEUSR14 LOGIC_OUTS_B22_7 out",
    "EFUSE_USR efuseusr15 EFUSE_USR_EFUSEUSR15 LOGIC_OUTS_B23_7 out",
    "EFUSE_USR efuseusr16 EFUSE_USR_EFUSEUSR16 LOGIC_OUTS_B16_8 out",
    "EFUSE_USR efuseusr17 EFUSE_USR_EFUSEUSR17 LOGIC_OUTS_B17_8 out",
    "EFUSE_USR efuseusr18 EFUSE_USR_EFUSEUSR18 LOGIC_OUTS_B18_8 out",
    "EFUSE_USR efuseusr19 EFUSE_USR_EFUSEUSR19 LOGIC_OUTS_B19_8 out",
    "EFUSE_USR efuseusr20 EFUSE_USR_EFUSEUSR20 LOGIC_OUTS_B20_8 out",
    "EFUSE_USR efuseusr21 EFUSE_USR_EFUSEUSR21 LOGIC_OUTS_B21_8 out",
    "EFUSE_USR efuseusr22 EFUSE_USR_EFUSEUSR22 LOGIC_OUTS_B22_8 out",
    "EFUSE_USR efuseusr23 EFUSE_USR_EFUSEUSR23 LOGIC_OUTS_B23_8 out",
    "EFUSE_USR efuseusr24 EFUSE_USR_EFUSEUSR24 LOGIC_OUTS_B16_9 out",
    "EFUSE_USR efuseusr25 EFUSE_USR_EFUSEUSR25 LOGIC_OUTS_B17_9 out",
    "EFUSE_USR efuseusr26 EFUSE_USR_EFUSEUSR26 LOGIC_OUTS_B18_9 out",
    "EFUSE_USR efuseusr27 EFUSE_USR_EFUSEUSR27 LOGIC_OUTS_B19_9 out",
    "EFUSE_USR efuseusr28 EFUSE_USR_EFUSEUSR28 LOGIC_OUTS_B20_9 out",
    "EFUSE_USR efuseusr29 EFUSE_USR_EFUSEUSR29 LOGIC_OUTS_B21_9 out",
    "EFUSE_USR efuseusr30 EFUSE_USR_EFUSEUSR30 LOGIC_OUTS_B22_9 out",
    "EFUSE_USR efuseusr31 EFUSE_USR_EFUSEUSR31 LOGIC_OUTS_B23_9 out",
];

/// [`PINS`], read: each line is the bel, the role, the wire, the wire it is
/// joined to and `in` or `out`.
fn pins() -> impl Iterator<Item = Pin> {
    PINS.iter().map(|line| {
        let mut words = line.split(' ');
        let mut next = || words.next().unwrap_or("");
        (next(), next(), next(), next(), next() == "in")
    })
}

/// The site kind of a bel, which is what `place::Netlist::build` gives a cell:
/// the device's role keyword, with a `BSCANE2`'s chain and an `ICAPE2`'s site.
fn kind_of(bel: &str) -> &'static str {
    match bel {
        "BSCAN_X0Y0" => "bscan1",
        "BSCAN_X0Y1" => "bscan2",
        "BSCAN_X0Y2" => "bscan3",
        "BSCAN_X0Y3" => "bscan4",
        "ICAP_X0Y0" => "icap0",
        "ICAP_X0Y1" => "icap1",
        "CAPTURE" => "capture",
        "FRAME_ECC" => "frame_ecc",
        "STARTUP" => "startup",
        "USR_ACCESS" => "usr_access",
        "DNA_PORT" => "dna",
        "EFUSE_USR" => "efuse",
        _ => "other",
    }
}

/// What a cell of each bel sets, and the features [`XrayFabric::configure_config`]
/// chooses among by name, each under its own name with a leading `=` so that no
/// primitive selects it.
fn cells_of(bel: &str) -> Vec<(&'static str, Vec<String>)> {
    if let Some(y) = bel.strip_prefix("BSCAN_X0Y") {
        let chain = y.parse::<u32>().unwrap_or(0) + 1;
        return vec![("BSCANE2", vec![format!("BSCAN.JTAG_CHAIN_{chain}")])];
    }
    if bel.starts_with("ICAP_X0Y") {
        return vec![
            ("ICAPE2", vec![format!("{bel}.IN_USE")]),
            ("=ICAP_WIDTH_X8", vec!["ICAP.ICAP_WIDTH_X8".to_owned()]),
            ("=ICAP_WIDTH_X16", vec!["ICAP.ICAP_WIDTH_X16".to_owned()]),
        ];
    }
    match bel {
        "CAPTURE" => vec![("CAPTUREE2", Vec::new())],
        "FRAME_ECC" => vec![("FRAME_ECCE2", Vec::new())],
        "STARTUP" => vec![
            ("STARTUPE2", Vec::new()),
            ("=PROG_USR", vec!["STARTUP.PROG_USR".to_owned()]),
            ("=GSR_OR_GTS", vec!["STARTUP.GSR_OR_GTS".to_owned()]),
            (
                "=USRCCLKO_OR_USRDONE",
                vec!["STARTUP.USRCCLKO_OR_USRDONE".to_owned()],
            ),
        ],
        "USR_ACCESS" => vec![("USR_ACCESSE2", Vec::new())],
        "DNA_PORT" => vec![("DNA_PORT", vec!["DNA_PORT.IN_USE".to_owned()])],
        "EFUSE_USR" => vec![("EFUSE_USR", Vec::new())],
        _ => Vec::new(),
    }
}

/// The bels of the configuration tile, or nothing for any other tile type.
/// `DNA_PORT`'s and `EFUSE_USR`'s pins are [`far_pins`].
pub(super) fn config_bels(tile_type: &str) -> Vec<ExtraBel> {
    if tile_type != MID {
        return Vec::new();
    }
    let mut names: Vec<&str> = pins().map(|p| p.0).collect();
    names.dedup();
    names
        .into_iter()
        .map(|bel| ExtraBel {
            name: bel.to_owned(),
            kind: kind_of(bel),
            pins: if ON_TOP.contains(&bel) {
                Vec::new()
            } else {
                pins()
                    .filter(|p| p.0 == bel)
                    .map(|p| BelPin {
                        role: p.1,
                        wire: format!("CFG_CENTER_{}", p.2),
                    })
                    .collect()
            },
            cells: cells_of(bel),
        })
        .collect()
}

/// The pins of the bels whose sites are on the tile above, as references from
/// [`MID`] to [`TOP`]'s wires: the bel, the role and the wire.
pub(super) fn far_pins(tile_type: &str) -> Vec<(&'static str, String, WireRef)> {
    if tile_type != MID {
        return Vec::new();
    }
    pins()
        .filter(|p| ON_TOP.contains(&p.0))
        .map(|p| {
            (
                p.0,
                p.1.to_owned(),
                WireRef::at_in(format!("CFG_CENTER_{}", p.2), 0, TOP_DY, TOP),
            )
        })
        .collect()
}

/// The fixed connections between the sites' pins and the interconnect side of
/// both tiles, which the database has no `ppips` file to say.
pub(super) fn config_pass_throughs(tile_type: &str) -> Vec<PassThrough> {
    let top = match tile_type {
        MID => false,
        TOP => true,
        BOT => {
            return HOPS
                .iter()
                .filter(|h| h.0 == BOT)
                .map(|(_, from, to)| PassThrough {
                    to: format!("CFG_CENTER_{to}"),
                    from: format!("CFG_CENTER_{from}"),
                    features: Vec::new(),
                })
                .collect();
        }
        _ => return Vec::new(),
    };
    let mut out: Vec<PassThrough> = pins()
        .filter(|p| ON_TOP.contains(&p.0) == top)
        .map(|(_, _, pin, joined, input)| {
            let pin = format!("CFG_CENTER_{pin}");
            let joined = format!("CFG_CENTER_{joined}");
            let (to, from) = if input { (pin, joined) } else { (joined, pin) };
            PassThrough {
                to,
                from,
                features: Vec::new(),
            }
        })
        .collect();
    for (tile, from, to) in HOPS {
        if tile == tile_type {
            out.push(PassThrough {
                to: format!("CFG_CENTER_{to}"),
                from: format!("CFG_CENTER_{from}"),
                features: Vec::new(),
            });
        }
    }
    out
}

/// The bits Vivado sets in [`MID`] that no `segbits` line names, under the names
/// the module documentation gives them. Each is the three bits of a run of
/// `testdata/fpga/vivado/config/`, and `vivado.txt` there is what says so.
pub(super) fn derived_features(tile_type: &str) -> Vec<Feature> {
    if tile_type != MID {
        return Vec::new();
    }
    let feature = |name: &str, ones: [(u32, u32); 3]| Feature {
        name: name.to_owned(),
        ones: ones.iter().map(|(f, b)| ConfigBit::new(*f, *b)).collect(),
        zeros: Vec::new(),
    };
    vec![
        feature("DNA_PORT.IN_USE", [(26, 2213), (27, 2210), (27, 2215)]),
        feature("ICAP_X0Y0.IN_USE", [(26, 2205), (27, 2206), (27, 2207)]),
        feature("ICAP_X0Y1.IN_USE", [(26, 2206), (26, 2207), (27, 2205)]),
        feature("STARTUP.GSR_OR_GTS", [(26, 2200), (26, 2201), (27, 2199)]),
        feature(
            "STARTUP.USRCCLKO_OR_USRDONE",
            [(26, 2196), (27, 2197), (27, 2198)],
        ),
    ]
}

/// The primitives here, by name.
pub(super) const PRIMITIVES: [&str; 8] = [
    "BSCANE2",
    "CAPTUREE2",
    "DNA_PORT",
    "EFUSE_USR",
    "FRAME_ECCE2",
    "ICAPE2",
    "STARTUPE2",
    "USR_ACCESSE2",
];

/// What a `STARTUPE2`'s open input is tied to, as Vivado ties it: the three
/// that a one leaves alone, and zero for the rest.
fn startup_default(role: &str) -> Bit {
    if matches!(role, "keyclearb" | "usrcclkts" | "usrdonets") {
        Bit::One
    } else {
        Bit::Zero
    }
}

/// The inputs of a configuration primitive that are tied, and to what, as
/// Vivado ties them: see the module documentation.
pub(super) fn wants(
    instance: &Instance,
    site: &ArchSite,
    netlist: &Netlist,
    wants: &mut Vec<Want>,
) {
    let inputs: &[&str] = match instance.primitive.as_str() {
        "STARTUPE2" => &[
            "clk",
            "gsr",
            "gts",
            "keyclearb",
            "pack",
            "usrcclko",
            "usrcclkts",
            "usrdoneo",
            "usrdonets",
        ],
        // Its clock is left open, as Vivado leaves it (`capture_no_clk`).
        "CAPTUREE2" => &["cap"],
        "BSCANE2" => &["tdo"],
        "DNA_PORT" | "ICAPE2" => &[],
        _ => return,
    };
    let pins = || instance.pins.iter().map(|p| &netlist.pins[*p]);
    let mut tie = |role: &str, value: Bit| {
        let Some(node) = site.pin(role) else { return };
        // A clock line has no ground fan two hops back; see the module docs.
        let clock = matches!(role, "clk" | "usrcclko");
        let policy = match value {
            Bit::One => TiePolicy::One,
            _ if clock => TiePolicy::ClockZero,
            _ => TiePolicy::Zero,
        };
        wants.push((policy, instance.name.clone(), role.to_owned(), node));
    };
    // A pin tied to a constant, on any of them.
    for pin in pins() {
        if pin.output || pin.signal.is_some() {
            continue;
        }
        if let Some(value @ (Bit::Zero | Bit::One)) = pin.constant {
            tie(&pin.role, value);
        }
    }
    // And one left open, on those Vivado ties.
    for role in inputs {
        if pins().any(|p| p.role == *role) {
            continue;
        }
        let value = if instance.primitive == "STARTUPE2" {
            startup_default(role)
        } else {
            Bit::Zero
        };
        tie(role, value);
    }
}

impl XrayFabric {
    /// Configures every configuration primitive the placement put on a site:
    /// the features a parameter or a pin chooses, which `bitstream::generate`
    /// cannot, and what the stream must say of them, for
    /// [`BitstreamOptions::set_design`](crate::fpga::xc7::options::BitstreamOptions::set_design).
    ///
    /// # Errors
    ///
    /// A message for a parameter value Vivado does not have, a `BSCANE2` whose
    /// `TDO` nothing drives (Vivado's `PDCN-2`), or a feature the database has
    /// not got.
    pub fn configure_config(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
        bits: &mut Bitstream,
    ) -> Result<ConfigUse, String> {
        let mut out = ConfigUse::default();
        let Some(m) = design.modules.get(module) else {
            return Ok(out);
        };
        for (index, instance) in netlist.instances.iter().enumerate() {
            if !PRIMITIVES.contains(&instance.primitive.as_str()) {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(bel) = self.arch.tile_types[site.tile_type].bel(&site.bel) else {
                continue;
            };
            let params = m.cells.get(instance.cell).map(|cell| &cell.params);
            let text = |name: &str, default: &str| {
                params
                    .and_then(|p| p.get(name))
                    .map_or_else(
                        || default.to_owned(),
                        |v| {
                            v.as_str()
                                .map(str::to_owned)
                                .or_else(|| v.as_int().map(|i| i.to_string()))
                                .unwrap_or_default()
                        },
                    )
                    .to_ascii_uppercase()
            };
            let bad = |name: &str, value: &str, values: &str| {
                format!(
                    "`{}`: {name} is \"{value}\", and Vivado's values are {values}",
                    instance.name
                )
            };
            let truth = |name: &str, default: &str| match text(name, default).as_str() {
                "TRUE" => Ok(true),
                "FALSE" => Ok(false),
                other => Err(bad(name, other, "TRUE and FALSE")),
            };
            let signal = |role: &str| {
                instance
                    .pins
                    .iter()
                    .map(|p| &netlist.pins[*p])
                    .any(|p| p.role == role && p.signal.is_some())
            };
            let connected = |role: &str| {
                instance
                    .pins
                    .iter()
                    .map(|p| &netlist.pins[*p])
                    .any(|p| p.role == role && (p.signal.is_some() || p.constant.is_some()))
            };
            let mut names: Vec<&str> = Vec::new();
            match instance.primitive.as_str() {
                "BSCANE2" => {
                    if !connected("tdo") {
                        return Err(format!(
                            "`{}`: a `BSCANE2` must have its `TDO` connected; Vivado refuses one \
                             that has not (PDCN-2)",
                            instance.name
                        ));
                    }
                    out.disable_jtag |= truth("DISABLE_JTAG", "FALSE")?;
                }
                "CAPTUREE2" => out.capture_oneshot |= truth("ONESHOT", "TRUE")?,
                "FRAME_ECCE2" => {
                    // Beside a shift register or a lookup table used as
                    // memory it builds: Vivado writes those frames' ECC by the
                    // rule it writes them by without one, the code with the
                    // contents left out (`xc7::wanted_ecc_without`), which this
                    // flow writes too. Refused until 2026-11-06, when that rule
                    // was not yet this flow's for a shift register.
                    out.frame_ecc = Some(match text("FARSRC", "EFAR").as_str() {
                        "EFAR" => false,
                        "FAR" => true,
                        other => return Err(bad("FARSRC", other, "EFAR and FAR")),
                    });
                }
                "ICAPE2" => {
                    match text("ICAP_WIDTH", "X32").as_str() {
                        "X32" => {}
                        "X16" => names.push("=ICAP_WIDTH_X16"),
                        "X8" => names.push("=ICAP_WIDTH_X8"),
                        other => return Err(bad("ICAP_WIDTH", other, "X8, X16 and X32")),
                    }
                    if site.bel == "ICAP_X0Y0" {
                        out.icap_bottom = true;
                    } else {
                        out.icap_top = true;
                    }
                }
                "STARTUPE2" => {
                    if truth("PROG_USR", "FALSE")? {
                        names.push("=PROG_USR");
                    }
                    if signal("gsr") || signal("gts") {
                        names.push("=GSR_OR_GTS");
                    }
                    if signal("usrcclko") || signal("usrdoneo") || signal("usrdonets") {
                        names.push("=USRCCLKO_OR_USRDONE");
                    }
                    out.startup_clock |= signal("clk");
                }
                _ => {}
            }
            for name in names {
                let entry = bel.config.iter().find_map(|entry| match entry {
                    ConfigEntry::Cell { primitive, bits } if primitive == name => Some(bits),
                    _ => None,
                });
                let Some(ones) = entry else {
                    return Err(format!(
                        "the database has no feature for `{}.{}`",
                        site.bel,
                        name.trim_start_matches('=')
                    ));
                };
                for bit in ones {
                    bits.set(site.tile, *bit).map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(out)
    }
}

/// Removes every `DNA_PORT`, `EFUSE_USR`, `USR_ACCESSE2` and `FRAME_ECCE2`
/// whose outputs nothing reads, as Vivado does (`dna_dout_open`,
/// `efuse_unread`, `usr_unread`, `ecc_unread`: no cell, no bit, and the
/// `FRAME_ECCE2`'s `CTL0` bits not written). The others stay with nothing read:
/// a `BSCANE2`, a `STARTUPE2`, a `CAPTUREE2` and an `ICAPE2` do something
/// through their inputs. Returns how many went.
///
/// # Errors
///
/// A message when the module's assignments do not resolve to bits.
pub fn sweep_unread_config(module: &mut crate::ir::Module) -> Result<usize, String> {
    use crate::ir::CellKind;
    use crate::ir::emit::{BitView, SigBit};
    const SWEPT: [&str; 4] = ["DNA_PORT", "EFUSE_USR", "USR_ACCESSE2", "FRAME_ECCE2"];
    let gone: Vec<crate::ir::CellId> = {
        let view = BitView::new(module).map_err(|e| e.to_string())?;
        let reads = crate::fpga::srl::read_counts(module, &view);
        module
            .cells
            .iter()
            .filter(|(_, cell)| {
                matches!(&cell.kind, CellKind::Blackbox(p) if SWEPT.contains(&p.as_str()))
            })
            .filter(|(_, cell)| {
                cell.outputs.iter().all(|(_, net)| {
                    view.net_bits(*net, cell.span).is_ok_and(|bits| {
                        bits.iter().all(|bit| {
                            let bit: SigBit = view.canonical(*bit);
                            reads.get(&bit).copied().unwrap_or(0) == 0
                        })
                    })
                })
            })
            .map(|(id, _)| id)
            .collect()
    };
    module.cells.retain(|id, _| !gone.contains(&id));
    Ok(gone.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every bel has the pins its primitive has in the vendor's library, and
    /// each is on a wire of its own. Would catch a line of [`PINS`] dropped or
    /// doubled in a transcription, or a site's name mistyped so that its pins
    /// went to another bel. Would not catch a wire that is the wrong wire, which
    /// is the transcription's word and `tests/fpga_xray_config.rs`'s Vivado
    /// routes.
    #[test]
    fn every_site_has_its_primitives_pins() {
        let count = |bel: &str| pins().filter(|p| p.0 == bel).count();
        for n in 0..4 {
            assert_eq!(count(&format!("BSCAN_X0Y{n}")), 11);
        }
        for n in 0..2 {
            assert_eq!(count(&format!("ICAP_X0Y{n}")), 67);
        }
        assert_eq!(count("CAPTURE"), 2);
        assert_eq!(count("FRAME_ECC"), 55);
        assert_eq!(count("STARTUP"), 13);
        assert_eq!(count("USR_ACCESS"), 34);
        assert_eq!(count("DNA_PORT"), 5);
        assert_eq!(count("EFUSE_USR"), 32);
        let mut wires: Vec<&str> = pins().map(|p| p.2).collect();
        wires.sort_unstable();
        wires.dedup();
        assert_eq!(wires.len(), PINS.len());
    }

    /// The bels and their kinds, which `place::Netlist::build` must agree
    /// with: a `BSCANE2` of chain `n` is kind `bscan<n>` and only `BSCAN_X0Y<n-1>`
    /// is. Would catch the chain counted from zero on one side and one on the
    /// other, which would put every chain on the next one's site.
    #[test]
    fn the_kinds_are_the_placers() {
        let bels = config_bels(MID);
        let kind = |name: &str| bels.iter().find(|b| b.name == name).map(|b| b.kind);
        assert_eq!(kind("BSCAN_X0Y0"), Some("bscan1"));
        assert_eq!(kind("BSCAN_X0Y3"), Some("bscan4"));
        assert_eq!(kind("ICAP_X0Y1"), Some("icap1"));
        assert_eq!(kind("DNA_PORT"), Some("dna"));
        assert_eq!(bels.len(), 12);
        assert!(config_bels(TOP).is_empty());
        assert_eq!(far_pins(MID).len(), 37);
    }

    /// The bits of each name this module gives, against the unnamed bits
    /// `--decode` listed in Vivado's bitstreams before there were names, as
    /// `bit CFG_CENTER_MID <frame>_<bit>` lines of the first `vivado.txt`, by
    /// the design that set each group alone. `tests/fpga_xray_config.rs` holds
    /// the names, and so would pass a group moved to the wrong bits as long as
    /// this flow wrote the same wrong ones; this is what holds the bits.
    #[test]
    fn the_names_are_on_the_bits_vivado_set() {
        /// A design, the name, and the three bits.
        type Group = (&'static str, &'static str, [(u32, u32); 3]);
        let vivado: [Group; 5] = [
            (
                "dna_plain",
                "DNA_PORT.IN_USE",
                [(26, 2213), (27, 2210), (27, 2215)],
            ),
            (
                "icap_loc0",
                "ICAP_X0Y0.IN_USE",
                [(26, 2205), (27, 2206), (27, 2207)],
            ),
            (
                "icap_plain",
                "ICAP_X0Y1.IN_USE",
                [(26, 2206), (26, 2207), (27, 2205)],
            ),
            (
                "startup_gsr",
                "STARTUP.GSR_OR_GTS",
                [(26, 2200), (26, 2201), (27, 2199)],
            ),
            (
                "startup_usrcclko",
                "STARTUP.USRCCLKO_OR_USRDONE",
                [(26, 2196), (27, 2197), (27, 2198)],
            ),
        ];
        let derived = derived_features(MID);
        assert_eq!(derived.len(), vivado.len());
        for (design, name, bits) in vivado {
            let feature = derived.iter().find(|f| f.name == name).expect(name);
            let mut ours: Vec<(u32, u32)> = feature.ones.iter().map(|b| (b.row, b.col)).collect();
            ours.sort_unstable();
            assert_eq!(ours, bits, "{design}");
        }
    }

    /// The derived features do not share a bit with each other, or with what
    /// the database names in the tile and the options set there (`DISABLE_JTAG`
    /// at 26_2202, 27_2203 and 27_2204). Would catch a group transcribed over
    /// another, which `--decode` would then read as both.
    #[test]
    fn no_two_names_share_a_bit() {
        let mut seen: Vec<(u32, u32)> = derived_features(MID)
            .iter()
            .flat_map(|f| f.ones.iter().map(|b| (b.row, b.col)))
            .collect();
        seen.extend([(26, 2202), (27, 2203), (27, 2204)]);
        let all = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), all);
    }
}
