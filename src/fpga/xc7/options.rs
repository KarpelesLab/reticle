//! `write_bitstream`'s options on the 7 series, and the packet stream they
//! make.
//!
//! # Where every number here comes from
//!
//! **Vivado 2026.1, measured, on an XC7A100T and where it says so an XC7A35T.**
//! `tools/vivado-parity/bitstream_options.tcl` wrote one `.bit` per value of
//! each `BITSTREAM.*` property from one routed design, and
//! `tests/fpga_xray_bitstream.rs` holds what this module writes to the bytes of
//! each. UG470 names the registers and most of their fields, and it was a
//! guide to reading the dumps rather than the source of anything here: it does
//! not say what a tool writes when.
//!
//! `docs/fpga-vivado-parity.md` has which options these are, which are not
//! done and why.
//!
//! # For a caller that has text
//!
//! [`BitstreamOptions::set`] takes a property's name and value, in Vivado's
//! spelling, and says why it refuses one; [`option_table`] lists what it knows.
//! Reading `set_property BITSTREAM.*` from an XDC file is one call to `set` per
//! property, and it is what `reticle fpga --bitstream-option` is.

use std::fmt;

use super::{
    Command, FrameAddress, FrameData, FrameLayout, LEADING_DUMMY_WORDS, PARKED_FAR, Part,
    RESERVED_REGISTER_19, Register, Stream, WORDS_PER_FRAME, Xc7Error,
};

/// One option Vivado's `write_bitstream` reads, as [`option_table`] lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionSpec {
    /// The property's name, as Vivado spells it.
    pub name: &'static str,
    /// The values it takes, or what kind of value.
    pub values: &'static str,
    /// What Vivado does when it is not set.
    pub default: &'static str,
    /// Whether Reticle does what Vivado does with it.
    pub support: Support,
}

/// How far Reticle goes with an option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// Every value takes effect, and the result is held to Vivado's bytes.
    Done,
    /// Accepted. On what was measured it changes nothing in the stream.
    NoEffect,
    /// Refused unless left at its default, for the reason given.
    Refused(&'static str),
}

const SPI_MODES: &str = "SPIx1 SPIx2 SPIx4 M_SERIAL S_SERIAL BPI8 BPI16 S_SELECTMAP \
                         S_SELECTMAP16 S_SELECTMAP32 B_SCAN M_SELECTMAP M_SELECTMAP16";
const PULLS2: &str = "PULLUP PULLNONE";
const PULLS3: &str = "PULLUP PULLDOWN PULLNONE";

const fn spec(
    name: &'static str,
    values: &'static str,
    default: &'static str,
    support: Support,
) -> OptionSpec {
    OptionSpec {
        name,
        values,
        default,
        support,
    }
}

const PERSIST_WHY: &str = "Vivado also configures every pin of the CONFIG_MODE's interface \
                           for it, and Reticle has no table of those pins";
const ENCRYPT_WHY: &str = "Reticle has no AES-256 and no HMAC, and a stream that claimed to be encrypted and was not \
     would be worse than a refusal";

/// Every property [`BitstreamOptions::set`] knows: the output
/// of `list_property [current_design] BITSTREAM.*` on an XC7A100T under
/// Vivado 2026.1, and the three design properties that reach the stream or are
/// checked with it. This is the table for a caller that reads `set_property`
/// and wants to warn about a name without trying it.
pub fn option_table() -> &'static [OptionSpec] {
    use Support::{Done, NoEffect, Refused};
    const TABLE: &[OptionSpec] = &[
        spec("BITSTREAM.CONFIG.BPI_1ST_READ_CYCLE", "1 2 3 4", "1", Done),
        spec("BITSTREAM.CONFIG.BPI_PAGE_SIZE", "1 4 8", "1", Done),
        spec(
            "BITSTREAM.CONFIG.BPI_SYNC_MODE",
            "DISABLE TYPE1 TYPE2",
            "(no BSPI packet)",
            Done,
        ),
        spec("BITSTREAM.CONFIG.CCLKPIN", PULLS2, "PULLUP", Done),
        spec(
            "BITSTREAM.CONFIG.CCLK_TRISTATE",
            "FALSE TRUE",
            "FALSE",
            NoEffect,
        ),
        spec(
            "BITSTREAM.CONFIG.CONFIGFALLBACK",
            "DISABLE ENABLE",
            "DISABLE, or ENABLE with NEXT_CONFIG_ADDR",
            Done,
        ),
        spec(
            "BITSTREAM.CONFIG.CONFIGRATE",
            "3 6 9 12 16 22 26 33 40 50 66",
            "3",
            Done,
        ),
        spec(
            "BITSTREAM.CONFIG.DCIUPDATEMODE",
            "ASREQUIRED CONTINUOUS QUIET",
            "ASREQUIRED",
            NoEffect,
        ),
        spec("BITSTREAM.CONFIG.DONEPIN", PULLS2, "PULLUP", Done),
        spec(
            "BITSTREAM.CONFIG.EXTMASTERCCLK_EN",
            "DISABLE DIV-8 DIV-4 DIV-2 DIV-1",
            "DISABLE",
            Done,
        ),
        spec("BITSTREAM.CONFIG.INITPIN", PULLS2, "PULLUP", Done),
        spec(
            "BITSTREAM.CONFIG.INITSIGNALSERROR",
            "ENABLE DISABLE",
            "ENABLE",
            Done,
        ),
        spec("BITSTREAM.CONFIG.M0PIN", PULLS3, "PULLUP", Done),
        spec("BITSTREAM.CONFIG.M1PIN", PULLS3, "PULLUP", Done),
        spec("BITSTREAM.CONFIG.M2PIN", PULLS3, "PULLUP", Done),
        spec(
            "BITSTREAM.CONFIG.NEXT_CONFIG_ADDR",
            "a 29-bit address",
            "none",
            Done,
        ),
        spec(
            "BITSTREAM.CONFIG.NEXT_CONFIG_REBOOT",
            "ENABLE DISABLE",
            "ENABLE",
            Done,
        ),
        spec(
            "BITSTREAM.CONFIG.OVERTEMPPOWERDOWN",
            "DISABLE ENABLE",
            "DISABLE",
            Done,
        ),
        spec(
            "BITSTREAM.CONFIG.PERSIST",
            "NO YES",
            "NO",
            Refused(PERSIST_WHY),
        ),
        spec("BITSTREAM.CONFIG.PROGPIN", PULLS2, "PULLUP", Done),
        spec("BITSTREAM.CONFIG.REVISIONSELECT", "00 01 10 11", "00", Done),
        spec(
            "BITSTREAM.CONFIG.REVISIONSELECT_TRISTATE",
            "DISABLE ENABLE",
            "DISABLE",
            Done,
        ),
        spec(
            "BITSTREAM.CONFIG.SELECTMAPABORT",
            "ENABLE DISABLE",
            "ENABLE",
            Done,
        ),
        spec(
            "BITSTREAM.CONFIG.SPI_32BIT_ADDR",
            "NO YES",
            "(no BSPI packet)",
            Done,
        ),
        spec("BITSTREAM.CONFIG.SPI_BUSWIDTH", "NONE 1 2 4", "NONE", Done),
        spec("BITSTREAM.CONFIG.SPI_FALL_EDGE", "NO YES", "NO", Done),
        spec("BITSTREAM.CONFIG.TCKPIN", PULLS3, "PULLUP", Done),
        spec("BITSTREAM.CONFIG.TDIPIN", PULLS3, "PULLUP", Done),
        spec("BITSTREAM.CONFIG.TDOPIN", PULLS3, "PULLUP", Done),
        spec("BITSTREAM.CONFIG.TIMER_CFG", "a 30-bit count", "off", Done),
        spec("BITSTREAM.CONFIG.TIMER_USR", "a 30-bit count", "off", Done),
        spec("BITSTREAM.CONFIG.TMSPIN", PULLS3, "PULLUP", Done),
        spec(
            "BITSTREAM.CONFIG.UNUSEDPIN",
            "PULLDOWN PULLUP PULLNONE",
            "PULLDOWN",
            Done,
        ),
        spec(
            "BITSTREAM.CONFIG.USERID",
            "a 32-bit value",
            "0XFFFFFFFF",
            Done,
        ),
        spec(
            "BITSTREAM.CONFIG.USR_ACCESS",
            "a 32-bit value, TIMESTAMP or NONE",
            "NONE",
            Done,
        ),
        spec(
            "BITSTREAM.ENCRYPTION.ENCRYPT",
            "NO YES",
            "NO",
            Refused(ENCRYPT_WHY),
        ),
        spec(
            "BITSTREAM.ENCRYPTION.ENCRYPTKEYSELECT",
            "BBRAM EFUSE",
            "BBRAM",
            Done,
        ),
        spec(
            "BITSTREAM.ENCRYPTION.HKEY",
            "a key",
            "none",
            Refused(ENCRYPT_WHY),
        ),
        spec(
            "BITSTREAM.ENCRYPTION.KEY0",
            "a key",
            "none",
            Refused(ENCRYPT_WHY),
        ),
        spec(
            "BITSTREAM.ENCRYPTION.KEYFILE",
            "a file",
            "none",
            Refused(ENCRYPT_WHY),
        ),
        spec(
            "BITSTREAM.ENCRYPTION.STARTCBC",
            "a value",
            "none",
            Refused(ENCRYPT_WHY),
        ),
        spec("BITSTREAM.GENERAL.COMPRESS", "FALSE TRUE", "FALSE", Done),
        spec("BITSTREAM.GENERAL.CRC", "ENABLE DISABLE", "ENABLE", Done),
        spec("BITSTREAM.GENERAL.DEBUGBITSTREAM", "NO YES", "NO", Done),
        spec("BITSTREAM.GENERAL.DISABLE_JTAG", "NO YES", "NO", Done),
        spec(
            "BITSTREAM.GENERAL.JTAG_XADC",
            "ENABLE DISABLE STATUSONLY",
            "ENABLE",
            Done,
        ),
        spec("BITSTREAM.GENERAL.PERFRAMECRC", "NO YES", "NO", Done),
        spec(
            "BITSTREAM.GENERAL.XADCENHANCEDLINEARITY",
            "OFF ON",
            "OFF",
            Done,
        ),
        spec(
            "BITSTREAM.GENERAL.XADCPOWERDOWN",
            "DISABLE ENABLE",
            "DISABLE",
            Done,
        ),
        spec(
            "BITSTREAM.READBACK.ACTIVERECONFIG",
            "NO YES",
            "NO",
            NoEffect,
        ),
        spec(
            "BITSTREAM.READBACK.ICAP_SELECT",
            "AUTO TOP BOTTOM",
            "AUTO",
            Done,
        ),
        spec(
            "BITSTREAM.READBACK.SECURITY",
            "NONE LEVEL1 LEVEL2",
            "NONE",
            Done,
        ),
        spec(
            "BITSTREAM.READBACK.XADCPARTIALRECONFIG",
            "DISABLE ENABLE",
            "DISABLE",
            Done,
        ),
        spec("BITSTREAM.STARTUP.DONEPIPE", "YES NO", "YES", Done),
        spec("BITSTREAM.STARTUP.DONE_CYCLE", "1 2 3 4 5 6", "4", Done),
        spec(
            "BITSTREAM.STARTUP.GTS_CYCLE",
            "1 2 3 4 5 6 DONE KEEP",
            "5",
            Done,
        ),
        spec(
            "BITSTREAM.STARTUP.GWE_CYCLE",
            "1 2 3 4 5 6 DONE KEEP",
            "6",
            Done,
        ),
        spec(
            "BITSTREAM.STARTUP.LCK_CYCLE",
            "NOWAIT 0 1 2 3 4 5 6",
            "NOWAIT",
            Done,
        ),
        spec(
            "BITSTREAM.STARTUP.MATCH_CYCLE",
            "AUTO NOWAIT 0 1 2 3 4 5 6",
            "AUTO",
            Done,
        ),
        spec(
            "BITSTREAM.STARTUP.STARTUPCLK",
            "CCLK JTAGCLK USERCLK",
            "CCLK",
            Done,
        ),
        spec("CFGBVS", "GND VCCO", "unset", NoEffect),
        spec("CONFIG_MODE", SPI_MODES, "unset", Done),
        spec("CONFIG_VOLTAGE", "1.5 1.8 2.5 3.3", "unset", NoEffect),
    ];
    TABLE
}

/// Why [`BitstreamOptions::set`] or [`BitstreamOptions::check`] refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionError {
    /// A name Vivado has no property of. Vivado's own message is the same
    /// kind of thing, and that includes `DRIVEDONE`, `DONEPROG` and
    /// `OVERTEMPSHUTDOWN`, which the 7 series under Vivado 2026.1 does not
    /// have.
    Unknown(String),
    /// A name it has with a value it does not take.
    BadValue {
        /// The property.
        name: String,
        /// The value given.
        value: String,
        /// What it takes.
        expected: String,
    },
    /// A property Reticle knows and does not do.
    NotImplemented {
        /// The property.
        name: String,
        /// Why.
        why: String,
    },
    /// Two values Vivado will not write together.
    Conflict(String),
}

impl fmt::Display for OptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OptionError::Unknown(name) => write!(
                f,
                "`{name}` is not a bitstream option on the 7 series under Vivado 2026.1"
            ),
            OptionError::BadValue {
                name,
                value,
                expected,
            } => write!(f, "`{name}` does not take `{value}`: it takes {expected}"),
            OptionError::NotImplemented { name, why } => {
                write!(f, "`{name}` is not done here: {why}")
            }
            OptionError::Conflict(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for OptionError {}

/// A pull on a dedicated configuration pin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PinPull {
    /// The default, with no bit set.
    #[default]
    Up,
    /// A pull-down, where the pin takes one.
    Down,
    /// No pull.
    None,
}

/// A pad nothing in the design uses: [`BitstreamOptions::unused_pin`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnusedPull {
    /// The default: the pull field with every bit clear.
    #[default]
    Down,
    /// `IOB_Y<n>.PULLTYPE.PULLUP`.
    Up,
    /// `IOB_Y<n>.PULLTYPE.NONE`.
    None,
}

/// What the design's configuration primitives ask of the stream, which no
/// `write_bitstream` option says: `xray::XrayFabric::configure_config` finds
/// it, and [`BitstreamOptions::set_design`] takes it.
///
/// **Measured**, `testdata/fpga/vivado/config/`, each against Vivado's stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ConfigUse {
    /// A `CAPTUREE2` with `ONESHOT = "TRUE"`, its default: `COR0` bit 23.
    pub capture_oneshot: bool,
    /// A `STARTUPE2` whose `CLK` is a signal: `STARTUPCLK` is `USERCLK` unless
    /// an option said otherwise, and an option of `USERCLK` needs it.
    pub startup_clock: bool,
    /// A `FRAME_ECCE2`, and whether its `FARSRC` is `FAR`: `CTL0` bit 8
    /// (`GLUTMASK_B`) clear, and bit 7 for `FAR`.
    pub frame_ecc: Option<bool>,
    /// An `ICAPE2` on `ICAP_X0Y1`, the top one.
    pub icap_top: bool,
    /// An `ICAPE2` on `ICAP_X0Y0`, the bottom one: alone, it is `CTL0` bit 30.
    pub icap_bottom: bool,
    /// A `BSCANE2` with `DISABLE_JTAG = "TRUE"`: the same bits as the option.
    pub disable_jtag: bool,
}

/// `READBACK.ICAP_SELECT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IcapSelect {
    /// The default: the bottom one when it is the only one in use.
    #[default]
    Auto,
    /// `ICAP_X0Y1`.
    Top,
    /// `ICAP_X0Y0`.
    Bottom,
}

/// What `USR_ACCESS` writes to the `AXSS` register.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UsrAccess {
    /// No `AXSS` packet at all.
    #[default]
    None,
    /// This value.
    Value(u32),
    /// The time of the build, which a caller with a clock fills in with
    /// [`BitstreamOptions::resolve_timestamp`]; the library has no clock.
    Timestamp,
}

/// The value of `set_property`, once understood.
///
/// The defaults are Vivado's. A field is an `Option` where Vivado tells "not
/// set" apart from any value: `SPI_BUSWIDTH` unset writes no `BSPI` packet, and
/// neither does `NONE`, but `SPI_32BIT_ADDR=NO`, which sounds like the default,
/// writes one.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct BitstreamOptions {
    /// `BITSTREAM.GENERAL.COMPRESS`: identical frames of a row by `MFWR`.
    pub compress: bool,
    /// `BITSTREAM.GENERAL.PERFRAMECRC`: a CRC after every frame.
    pub per_frame_crc: bool,
    /// `BITSTREAM.GENERAL.DEBUGBITSTREAM`: one packet per frame, and `LOUT`.
    pub debug: bool,
    /// `BITSTREAM.GENERAL.CRC`: false writes `RCRC` where the CRCs were.
    pub crc: bool,
    /// `CONFIGRATE`, in MHz.
    pub config_rate: u32,
    /// `EXTMASTERCCLK_EN`: the divider as written, 1, 2, 4 or 8, when enabled.
    pub ext_master_cclk: Option<u32>,
    /// `STARTUP.DONE_CYCLE`, 1 to 6.
    pub done_cycle: u32,
    /// `STARTUP.GTS_CYCLE`: the field's value, 0 to 7.
    pub gts_cycle: u32,
    /// `STARTUP.GWE_CYCLE`: the field's value, 0 to 7.
    pub gwe_cycle: u32,
    /// `STARTUP.LCK_CYCLE`: the field's value, 0 to 7.
    pub lck_cycle: u32,
    /// `STARTUP.MATCH_CYCLE`: the field's value, 0 to 7.
    pub match_cycle: u32,
    /// `STARTUP.STARTUPCLK` is `JTAGCLK`, rather than `CCLK`.
    pub startup_jtag_clock: bool,
    /// `STARTUP.STARTUPCLK` is `USERCLK`.
    pub startup_user_clock: bool,
    /// `STARTUP.STARTUPCLK` was given at all: with a `STARTUPE2` clock and no
    /// option, Vivado writes `USERCLK`.
    pub startup_clock_set: bool,
    /// `READBACK.ICAP_SELECT`.
    pub icap_select: IcapSelect,
    /// What the design's configuration primitives ask of the stream.
    pub design: ConfigUse,
    /// `STARTUP.DONEPIPE`.
    pub done_pipe: bool,
    /// `BPI_PAGE_SIZE`, 1, 4 or 8.
    pub bpi_page_size: u32,
    /// `BPI_1ST_READ_CYCLE`, 1 to 4.
    pub bpi_first_read_cycle: u32,
    /// `BPI_SYNC_MODE`, as the `BSPI` register's high half.
    pub bpi_sync: Option<u32>,
    /// `SPI_BUSWIDTH`: 1, 2 or 4, `None` when unset or `NONE`.
    pub spi_width: Option<u32>,
    /// `SPI_BUSWIDTH` was `NONE` by name, which Vivado tells apart from unset.
    pub spi_width_none: bool,
    /// `SPI_32BIT_ADDR`, when set at all.
    pub spi_32bit: Option<bool>,
    /// `SPI_FALL_EDGE`.
    pub spi_fall_edge: bool,
    /// `CONFIG_MODE`, when set.
    pub config_mode: Option<String>,
    /// `CFGBVS`, when set.
    pub cfgbvs: Option<String>,
    /// `CONFIG_VOLTAGE`, when set.
    pub config_voltage: Option<String>,
    /// `NEXT_CONFIG_ADDR`, when set.
    pub next_config_addr: Option<u32>,
    /// `NEXT_CONFIG_REBOOT`.
    pub next_config_reboot: bool,
    /// `CONFIGFALLBACK`, when set; unset follows `NEXT_CONFIG_ADDR`.
    pub config_fallback: Option<bool>,
    /// `REVISIONSELECT`, 0 to 3.
    pub revision_select: u32,
    /// `REVISIONSELECT_TRISTATE`.
    pub revision_tristate: bool,
    /// `TIMER_CFG`, when set.
    pub timer_cfg: Option<u32>,
    /// `TIMER_USR`, when set.
    pub timer_usr: Option<u32>,
    /// `USR_ACCESS`.
    pub usr_access: UsrAccess,
    /// `USERID`.
    pub user_id: u32,
    /// `INITSIGNALSERROR`.
    pub init_signals_error: bool,
    /// `OVERTEMPPOWERDOWN`.
    pub over_temp_power_down: bool,
    /// `SELECTMAPABORT`.
    pub select_map_abort: bool,
    /// `READBACK.SECURITY`, 0 to 2.
    pub security: u32,
    /// `ENCRYPTKEYSELECT` is `EFUSE`. Vivado sets the bit with no encryption.
    pub key_in_efuse: bool,
    /// `READBACK.XADCPARTIALRECONFIG`.
    pub xadc_partial_reconfig: bool,
    /// `UNUSEDPIN`.
    pub unused_pin: UnusedPull,
    /// `M0PIN`, `M1PIN`, `M2PIN`.
    pub mode_pins: [PinPull; 3],
    /// `TCKPIN`, `TDIPIN`, `TDOPIN`, `TMSPIN`.
    pub jtag_pins: [PinPull; 4],
    /// `CCLKPIN`, `DONEPIN`, `INITPIN`, `PROGPIN`.
    pub config_pins: [PinPull; 4],
    /// `DISABLE_JTAG`.
    pub disable_jtag: bool,
    /// `JTAG_XADC`: 0 `ENABLE`, 1 `DISABLE`, 2 `STATUSONLY`.
    pub jtag_xadc: u32,
    /// `XADCENHANCEDLINEARITY`.
    pub xadc_enhanced_linearity: bool,
    /// `XADCPOWERDOWN`.
    pub xadc_power_down: bool,
}

impl Default for BitstreamOptions {
    fn default() -> BitstreamOptions {
        BitstreamOptions {
            compress: false,
            per_frame_crc: false,
            debug: false,
            crc: true,
            config_rate: 3,
            ext_master_cclk: None,
            done_cycle: 4,
            gts_cycle: 4,
            gwe_cycle: 5,
            lck_cycle: 7,
            match_cycle: 7,
            startup_jtag_clock: false,
            startup_user_clock: false,
            startup_clock_set: false,
            icap_select: IcapSelect::Auto,
            design: ConfigUse::default(),
            done_pipe: true,
            bpi_page_size: 1,
            bpi_first_read_cycle: 1,
            bpi_sync: None,
            spi_width: None,
            spi_width_none: false,
            spi_32bit: None,
            spi_fall_edge: false,
            config_mode: None,
            cfgbvs: None,
            config_voltage: None,
            next_config_addr: None,
            next_config_reboot: true,
            config_fallback: None,
            revision_select: 0,
            revision_tristate: false,
            timer_cfg: None,
            timer_usr: None,
            usr_access: UsrAccess::None,
            user_id: 0xFFFF_FFFF,
            init_signals_error: true,
            over_temp_power_down: false,
            select_map_abort: true,
            security: 0,
            key_in_efuse: false,
            xadc_partial_reconfig: false,
            unused_pin: UnusedPull::Down,
            mode_pins: [PinPull::Up; 3],
            jtag_pins: [PinPull::Up; 4],
            config_pins: [PinPull::Up; 4],
            disable_jtag: false,
            jtag_xadc: 0,
            xadc_enhanced_linearity: false,
            xadc_power_down: false,
        }
    }
}

/// The value of a `0X...` property. Vivado's form, and a bare hexadecimal too.
fn hex_value(text: &str) -> Option<u32> {
    let digits = text
        .strip_prefix("0X")
        .or_else(|| text.strip_prefix("0x"))
        .unwrap_or(text);
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

impl BitstreamOptions {
    /// Sets one property, as `set_property <name> <value> [current_design]`.
    ///
    /// Names and enumerated values are compared without regard to case, as
    /// Vivado compares them. A property [`option_table`] marks
    /// [`Support::Refused`] is refused unless at its default, and a value that
    /// only makes sense alongside another is checked by
    /// [`BitstreamOptions::check`], when they have all been seen.
    ///
    /// # Errors
    ///
    /// [`OptionError`], which says which of those it was.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), OptionError> {
        let name = name.to_ascii_uppercase();
        let upper = value.to_ascii_uppercase();
        let Some(spec) = option_table().iter().find(|s| s.name == name) else {
            return Err(OptionError::Unknown(name));
        };
        let bad = || OptionError::BadValue {
            name: name.clone(),
            value: value.to_owned(),
            expected: spec.values.to_owned(),
        };
        let one_of = |values: &str| values.split(' ').any(|v| v == upper);
        if let Support::Refused(why) = spec.support {
            let at_default = matches!(upper.as_str(), "NO" | "AUTO");
            if !at_default {
                return Err(OptionError::NotImplemented {
                    name,
                    why: why.to_owned(),
                });
            }
            return Ok(());
        }
        let yes_no = |yes: &str, no: &str| -> Result<bool, OptionError> {
            if upper == yes {
                Ok(true)
            } else if upper == no {
                Ok(false)
            } else {
                Err(bad())
            }
        };
        let pull3 = || -> Result<PinPull, OptionError> {
            match upper.as_str() {
                "PULLUP" => Ok(PinPull::Up),
                "PULLDOWN" => Ok(PinPull::Down),
                "PULLNONE" => Ok(PinPull::None),
                _ => Err(bad()),
            }
        };
        let pull2 = || -> Result<PinPull, OptionError> {
            match upper.as_str() {
                "PULLUP" => Ok(PinPull::Up),
                "PULLNONE" => Ok(PinPull::None),
                _ => Err(bad()),
            }
        };
        // A cycle field: `1` to `6` is that less one, then its two names.
        let cycle = || -> Result<u32, OptionError> {
            match upper.as_str() {
                "DONE" => Ok(6),
                "KEEP" => Ok(7),
                "1" | "2" | "3" | "4" | "5" | "6" => {
                    Ok(upper.parse::<u32>().map_err(|_| bad())? - 1)
                }
                _ => Err(bad()),
            }
        };
        // A cycle field that counts from zero, with `NOWAIT` for no cycle.
        let wait = || -> Result<u32, OptionError> {
            match upper.as_str() {
                "NOWAIT" | "AUTO" => Ok(7),
                "0" | "1" | "2" | "3" | "4" | "5" | "6" => upper.parse().map_err(|_| bad()),
                _ => Err(bad()),
            }
        };
        let field = name.rsplit('.').next().unwrap_or(&name);
        match field {
            "BPI_1ST_READ_CYCLE" => {
                self.bpi_first_read_cycle = match upper.as_str() {
                    "1" | "2" | "3" | "4" => upper.parse().map_err(|_| bad())?,
                    _ => return Err(bad()),
                };
            }
            "BPI_PAGE_SIZE" => {
                self.bpi_page_size = match upper.as_str() {
                    "1" | "4" | "8" => upper.parse().map_err(|_| bad())?,
                    _ => return Err(bad()),
                };
            }
            "BPI_SYNC_MODE" => {
                self.bpi_sync = Some(match upper.as_str() {
                    "DISABLE" => 0,
                    "TYPE1" => 0x0D9C_F000,
                    "TYPE2" => 0x0A9C_F000,
                    _ => return Err(bad()),
                });
            }
            "CCLKPIN" => self.config_pins[0] = pull2()?,
            "DONEPIN" => self.config_pins[1] = pull2()?,
            "INITPIN" => self.config_pins[2] = pull2()?,
            "PROGPIN" => self.config_pins[3] = pull2()?,
            "M0PIN" => self.mode_pins[0] = pull3()?,
            "M1PIN" => self.mode_pins[1] = pull3()?,
            "M2PIN" => self.mode_pins[2] = pull3()?,
            "TCKPIN" => self.jtag_pins[0] = pull3()?,
            "TDIPIN" => self.jtag_pins[1] = pull3()?,
            "TDOPIN" => self.jtag_pins[2] = pull3()?,
            "TMSPIN" => self.jtag_pins[3] = pull3()?,
            "CONFIGRATE" => {
                if !one_of(spec.values) {
                    return Err(bad());
                }
                self.config_rate = upper.parse().map_err(|_| bad())?;
            }
            "CONFIGFALLBACK" => self.config_fallback = Some(yes_no("ENABLE", "DISABLE")?),
            "EXTMASTERCCLK_EN" => {
                self.ext_master_cclk = match upper.as_str() {
                    "DISABLE" => None,
                    "DIV-1" => Some(1),
                    "DIV-2" => Some(2),
                    "DIV-4" => Some(4),
                    "DIV-8" => Some(8),
                    _ => return Err(bad()),
                };
            }
            "INITSIGNALSERROR" => self.init_signals_error = yes_no("ENABLE", "DISABLE")?,
            "NEXT_CONFIG_ADDR" => {
                let address = hex_value(&upper).ok_or_else(bad)?;
                if address > 0x1FFF_FFFF {
                    return Err(bad());
                }
                self.next_config_addr = Some(address);
            }
            "NEXT_CONFIG_REBOOT" => self.next_config_reboot = yes_no("ENABLE", "DISABLE")?,
            "OVERTEMPPOWERDOWN" => self.over_temp_power_down = yes_no("ENABLE", "DISABLE")?,
            "REVISIONSELECT" => {
                self.revision_select = match upper.as_str() {
                    "00" => 0,
                    "01" => 1,
                    "10" => 2,
                    "11" => 3,
                    _ => return Err(bad()),
                };
            }
            "REVISIONSELECT_TRISTATE" => self.revision_tristate = yes_no("ENABLE", "DISABLE")?,
            "SELECTMAPABORT" => self.select_map_abort = yes_no("ENABLE", "DISABLE")?,
            "SPI_32BIT_ADDR" => self.spi_32bit = Some(yes_no("YES", "NO")?),
            "SPI_BUSWIDTH" => {
                self.spi_width_none = upper == "NONE";
                self.spi_width = match upper.as_str() {
                    "NONE" => None,
                    "1" => Some(1),
                    "2" => Some(2),
                    "4" => Some(4),
                    _ => return Err(bad()),
                };
            }
            "SPI_FALL_EDGE" => self.spi_fall_edge = yes_no("YES", "NO")?,
            "TIMER_CFG" | "TIMER_USR" => {
                let count = hex_value(&upper).ok_or_else(bad)?;
                if count > 0x3FFF_FFFF {
                    return Err(bad());
                }
                if field == "TIMER_CFG" {
                    self.timer_cfg = Some(count);
                } else {
                    self.timer_usr = Some(count);
                }
            }
            "UNUSEDPIN" => {
                self.unused_pin = match upper.as_str() {
                    "PULLDOWN" => UnusedPull::Down,
                    "PULLUP" => UnusedPull::Up,
                    "PULLNONE" => UnusedPull::None,
                    _ => return Err(bad()),
                };
            }
            "USERID" => self.user_id = hex_value(&upper).ok_or_else(bad)?,
            "USR_ACCESS" => {
                self.usr_access = match upper.as_str() {
                    "NONE" => UsrAccess::None,
                    "TIMESTAMP" => UsrAccess::Timestamp,
                    _ => UsrAccess::Value(hex_value(&upper).ok_or_else(bad)?),
                };
            }
            "ENCRYPTKEYSELECT" => self.key_in_efuse = yes_no("EFUSE", "BBRAM")?,
            "COMPRESS" => self.compress = yes_no("TRUE", "FALSE")?,
            "CRC" => self.crc = yes_no("ENABLE", "DISABLE")?,
            "DEBUGBITSTREAM" => self.debug = yes_no("YES", "NO")?,
            "DISABLE_JTAG" => self.disable_jtag = yes_no("YES", "NO")?,
            "JTAG_XADC" => {
                self.jtag_xadc = match upper.as_str() {
                    "ENABLE" => 0,
                    "DISABLE" => 1,
                    "STATUSONLY" => 2,
                    _ => return Err(bad()),
                };
            }
            "PERFRAMECRC" => self.per_frame_crc = yes_no("YES", "NO")?,
            "XADCENHANCEDLINEARITY" => self.xadc_enhanced_linearity = yes_no("ON", "OFF")?,
            "XADCPOWERDOWN" => self.xadc_power_down = yes_no("ENABLE", "DISABLE")?,
            "SECURITY" => {
                self.security = match upper.as_str() {
                    "NONE" => 0,
                    "LEVEL1" => 1,
                    "LEVEL2" => 2,
                    _ => return Err(bad()),
                };
            }
            "XADCPARTIALRECONFIG" => self.xadc_partial_reconfig = yes_no("ENABLE", "DISABLE")?,
            "DONEPIPE" => self.done_pipe = yes_no("YES", "NO")?,
            "DONE_CYCLE" => {
                self.done_cycle = match upper.as_str() {
                    "1" | "2" | "3" | "4" | "5" | "6" => upper.parse().map_err(|_| bad())?,
                    _ => return Err(bad()),
                };
            }
            "GTS_CYCLE" => self.gts_cycle = cycle()?,
            "GWE_CYCLE" => self.gwe_cycle = cycle()?,
            "LCK_CYCLE" => {
                if upper == "AUTO" {
                    return Err(bad());
                }
                self.lck_cycle = wait()?;
            }
            "MATCH_CYCLE" => self.match_cycle = wait()?,
            "STARTUPCLK" => {
                (self.startup_jtag_clock, self.startup_user_clock) = match upper.as_str() {
                    "CCLK" => (false, false),
                    "JTAGCLK" => (true, false),
                    "USERCLK" => (false, true),
                    _ => return Err(bad()),
                };
                self.startup_clock_set = true;
            }
            "ICAP_SELECT" => {
                self.icap_select = match upper.as_str() {
                    "AUTO" => IcapSelect::Auto,
                    "TOP" => IcapSelect::Top,
                    "BOTTOM" => IcapSelect::Bottom,
                    _ => return Err(bad()),
                };
            }
            "CONFIG_MODE" => {
                if !one_of(&SPI_MODES.to_ascii_uppercase()) {
                    return Err(bad());
                }
                self.config_mode = Some(upper);
            }
            "CFGBVS" => {
                if !one_of(spec.values) {
                    return Err(bad());
                }
                self.cfgbvs = Some(upper);
            }
            "CONFIG_VOLTAGE" => {
                if !one_of(spec.values) {
                    return Err(bad());
                }
                self.config_voltage = Some(upper);
            }
            // Accepted and without effect on what was measured.
            _ => {
                if !one_of(spec.values) {
                    return Err(bad());
                }
            }
        }
        Ok(())
    }

    /// What the design's configuration primitives ask of the stream, as
    /// `xray::XrayFabric::configure_config` found it. A `BSCANE2`'s
    /// `DISABLE_JTAG` is the option's own bits, so it sets the option.
    pub fn set_design(&mut self, design: ConfigUse) {
        self.design = design;
        self.disable_jtag |= design.disable_jtag;
    }

    /// The refusals Vivado makes of values together, and of a value the
    /// design's configuration primitives do not allow, in Vivado's terms: its
    /// message numbers are in `docs/fpga-vivado-parity.md`. Call it when every
    /// property has been set and [`BitstreamOptions::set_design`] has been
    /// called; with no design, what is checked is a design with none of those
    /// primitives, which is what `write_bitstream` from a checkpoint of one is.
    ///
    /// # Errors
    ///
    /// [`OptionError::Conflict`] with what is wrong.
    pub fn check(&self) -> Result<(), OptionError> {
        self.check_options()?;
        let conflict = |text: &str| Err(OptionError::Conflict(text.to_owned()));
        if self.startup_user_clock && !self.design.startup_clock {
            return conflict(
                "STARTUPCLK is USERCLK and no STARTUPE2 has a signal on its CLK: Vivado refuses \
                 that (Designutils 12-1517)",
            );
        }
        match self.icap_select {
            IcapSelect::Top if !self.design.icap_top => conflict(
                "ICAP_SELECT is TOP and no ICAPE2 is on ICAP_X0Y1: Vivado refuses that \
                 (Designutils 12-1536)",
            ),
            IcapSelect::Bottom if !self.design.icap_bottom => conflict(
                "ICAP_SELECT is BOTTOM and no ICAPE2 is on ICAP_X0Y0: Vivado refuses that \
                 (Designutils 12-1536)",
            ),
            _ => Ok(()),
        }
    }

    /// The refusals of [`BitstreamOptions::check`] that are of the options
    /// alone, for a caller that has not got the design yet.
    ///
    /// # Errors
    ///
    /// [`OptionError::Conflict`] with what is wrong.
    pub fn check_options(&self) -> Result<(), OptionError> {
        let conflict = |text: &str| Err(OptionError::Conflict(text.to_owned()));
        if self.compress && self.per_frame_crc {
            return conflict("Vivado does not compress a bitstream that uses per-frame CRC");
        }
        if self.compress && self.debug {
            return conflict("Vivado does not compress a debug bitstream");
        }
        if self.per_frame_crc && !self.crc {
            return conflict("Vivado does not disable the CRC of a bitstream with per-frame CRC");
        }
        if self.spi_width_none && self.spi_32bit == Some(true) {
            return conflict(
                "BITSTREAM.CONFIG.SPI_BUSWIDTH NONE is not valid with \
                 BITSTREAM.CONFIG.SPI_32BIT_ADDR YES",
            );
        }
        if self.spi_32bit == Some(true)
            && let Some(address) = self.next_config_addr
            && address & 0xff != 0
        {
            return conflict(&format!(
                "the lower 8 bits of BITSTREAM.CONFIG.NEXT_CONFIG_ADDR {address:#010x} are not \
                 zero, and with BITSTREAM.CONFIG.SPI_32BIT_ADDR YES a MultiBoot address must be \
                 on a 256-byte boundary; Vivado refuses it too (Designutils 12-2356)"
            ));
        }
        if let (Some(mode), Some(width)) = (self.config_mode_width(), self.spi_width)
            && mode != width
        {
            return conflict(&format!(
                "the CONFIG_MODE property value `{}` is not valid when \
                 BITSTREAM.CONFIG.SPI_BUSWIDTH is set to {width}",
                self.config_mode.as_deref().unwrap_or("")
            ));
        }
        match (self.config_voltage.as_deref(), self.cfgbvs.as_deref()) {
            (Some(_), None) => {
                return conflict(
                    "CONFIG_VOLTAGE needs CFGBVS set too: Vivado's design rule CFGBVS-2 refuses \
                     it without",
                );
            }
            (Some("1.5" | "1.8"), Some(bvs)) if bvs != "GND" => {
                return conflict(
                    "CONFIG_VOLTAGE 1.5 or 1.8 needs CFGBVS GND: Vivado's design rule CFGBVS-4",
                );
            }
            _ => {}
        }
        Ok(())
    }

    /// The bus width `CONFIG_MODE` implies, when it names an SPI mode.
    fn config_mode_width(&self) -> Option<u32> {
        match self.config_mode.as_deref() {
            Some("SPIX1") => Some(1),
            Some("SPIX2") => Some(2),
            Some("SPIX4") => Some(4),
            _ => None,
        }
    }

    /// Puts the time of a build where `USR_ACCESS=TIMESTAMP` asked for one,
    /// in the register's own packed form (`ddddd MMMM yyyyyy hhhhh mmmmmm
    /// ssssss`, year from 2000), which is the form Vivado's value decodes in.
    /// The library has no clock; the caller has.
    pub fn resolve_timestamp(&mut self, year: u32, month: u32, day: u32, time: (u32, u32, u32)) {
        if self.usr_access == UsrAccess::Timestamp {
            self.usr_access = UsrAccess::Value(
                ((day & 0x1F) << 27)
                    | ((month & 0xF) << 23)
                    | ((year.saturating_sub(2000) & 0x3F) << 17)
                    | ((time.0 & 0x1F) << 12)
                    | ((time.1 & 0x3F) << 6)
                    | (time.2 & 0x3F),
            );
        }
    }

    /// The `COR0` word.
    pub fn cor0(&self) -> u32 {
        let oscillator = match self.ext_master_cclk {
            Some(1) => 0x30,
            Some(2) => 0x20,
            Some(4) => 0x10,
            Some(_) => 0x00,
            None => match self.config_rate {
                6 => 0x10,
                9 => 0x01,
                12 => 0x20,
                16 => 0x11,
                22 => 0x04,
                26 => 0x12,
                33 => 0x21,
                40 => 0x0B,
                50 => 0x15,
                66 => 0x18,
                _ => 0x00,
            },
        };
        self.gwe_cycle
            | (self.gts_cycle << 3)
            | (self.lck_cycle << 6)
            | (self.match_cycle << 9)
            | ((self.done_cycle - 1) << 12)
            | (u32::from(self.startup_user_clock()) << 15)
            | (u32::from(self.startup_jtag_clock) << 16)
            | (oscillator << 17)
            | (u32::from(self.design.capture_oneshot) << 23)
            | (u32::from(self.done_pipe) << 25)
            | (u32::from(self.ext_master_cclk.is_some()) << 26)
    }

    /// Whether the start-up sequence runs on the `STARTUPE2`'s `CLK`.
    fn startup_user_clock(&self) -> bool {
        self.startup_user_clock || (!self.startup_clock_set && self.design.startup_clock)
    }

    /// Whether `CTL0` selects the bottom `ICAPE2`.
    fn icap_bottom(&self) -> bool {
        match self.icap_select {
            IcapSelect::Top => false,
            IcapSelect::Bottom => true,
            IcapSelect::Auto => self.design.icap_bottom && !self.design.icap_top,
        }
    }

    /// The `COR1` word.
    pub fn cor1(&self) -> u32 {
        let page = match self.bpi_page_size {
            4 => 1,
            8 => 2,
            _ => 0,
        };
        page | ((self.bpi_first_read_cycle - 1) << 2)
            | (u32::from(self.xadc_partial_reconfig) << 31)
    }

    /// The `CTL0` word.
    pub fn ctl0(&self) -> u32 {
        let fallback = self
            .config_fallback
            .unwrap_or(self.next_config_addr.is_some());
        1 | (self.security << 4)
            | (u32::from(self.design.frame_ecc == Some(true)) << 7)
            | (u32::from(self.design.frame_ecc.is_none()) << 8)
            | (u32::from(!self.select_map_abort) << 9)
            | (u32::from(!fallback) << 10)
            | (u32::from(self.over_temp_power_down) << 12)
            | (u32::from(!self.init_signals_error) << 13)
            | (u32::from(self.icap_bottom()) << 30)
            | (u32::from(self.key_in_efuse) << 31)
    }

    /// The `CTL1` word: what the frame writes that follow it need.
    pub fn ctl1(&self) -> u32 {
        if self.compress {
            0x0000_1000
        } else if self.per_frame_crc {
            0x0020_0000
        } else {
            0
        }
    }

    /// The `WBSTAR` word.
    ///
    /// **With `SPI_32BIT_ADDR`, the whole word is shifted down eight bits**:
    /// Vivado 2026.1 writes `0x00004000` for an address of `0x00400000`,
    /// `0x001fc000` for `0x1fc00000`, and `0x00a04000` for `0x00400000` with
    /// `REVISIONSELECT 10` and its tristate, whose bits move with it. Found
    /// when an XDC set the two together (`xdc_startup` in
    /// `testdata/fpga/vivado/config/`); every run of the options' sweep had set
    /// one alone. [`BitstreamOptions::check_options`] refuses an address whose
    /// low eight bits that would lose, as Vivado does.
    pub fn wbstar(&self) -> u32 {
        let word = (self.revision_select << 30)
            | (u32::from(self.revision_tristate) << 29)
            | self.next_config_addr.unwrap_or(0);
        if self.spi_32bit == Some(true) {
            word >> 8
        } else {
            word
        }
    }

    /// The `TIMER` word. The register holds one count: with both set to
    /// different counts, Vivado writes `TIMER_CFG`'s and says nothing, and so
    /// does this.
    pub fn timer(&self) -> u32 {
        (u32::from(self.timer_usr.is_some()) << 31)
            | (u32::from(self.timer_cfg.is_some()) << 30)
            | self.timer_cfg.or(self.timer_usr).unwrap_or(0)
    }

    /// The `BSPI` word, when there is a packet of it.
    pub fn bspi(&self) -> Option<u32> {
        let width = self.spi_width.or_else(|| self.config_mode_width());
        let spi = match (width, self.spi_32bit) {
            (None, None) => None,
            (width, wide) => {
                let (bits, command) = match width {
                    Some(4) => (2, 0x6B),
                    Some(2) => (1, 0x3B),
                    _ => (0, 0x0B),
                };
                Some((bits << 8) | (command + u32::from(wide == Some(true))))
            }
        };
        match (self.bpi_sync, spi) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) | b.unwrap_or(0)),
        }
    }

    /// Dummy words before the bus-width pattern. A flash with 32-bit addresses
    /// is read from further in, so Vivado leaves it more.
    pub fn leading_words(&self) -> usize {
        LEADING_DUMMY_WORDS + if self.spi_32bit == Some(true) { 64 } else { 0 }
    }

    /// The `.bit` header's `a` field, with this user code.
    pub fn design_field(&self, name: &str) -> String {
        format!("{name};UserID=0X{:08X};Version=reticle", self.user_id)
    }

    /// The bits these options set in the configuration tiles' frames, as
    /// frame address, word and bit. What is not here sets nothing: every
    /// default is a clear bit.
    pub fn config_tile_bits(&self, tiles: ConfigTiles) -> Vec<(FrameAddress, usize, u32)> {
        let mut out = Vec::new();
        for (tile, _, bits) in config_tile_table(self) {
            let base = match tile {
                ConfigTile::Center => tiles.center,
                ConfigTile::Monitor => tiles.monitor,
            };
            for (frame, word, bit) in bits {
                out.push((FrameAddress::from_u32(base + frame), word, bit));
            }
        }
        out
    }
}

/// Which tile an option's bits are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigTile {
    Center,
    Monitor,
}

/// Where the two tiles that hold an option's bits start: `CFG_CENTER_MID`'s
/// and `MONITOR_BOT`'s `baseaddr` in `tilegrid.json`.
///
/// The bits below are given relative to these and the same on the XC7A100T and
/// the XC7A35T, whose tiles are elsewhere, which is what says they are the
/// tile's bits. Both tiles fill their frames, `offset` 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfigTiles {
    /// `CFG_CENTER_MID`'s first frame.
    pub center: u32,
    /// `MONITOR_BOT`'s first frame.
    pub monitor: u32,
}

/// A name, and a bit: frame within the tile, word, bit.
type TileBit = (u32, usize, u32);

/// A name, and its bits.
type Named = (&'static str, Vec<TileBit>);

/// A pin, the names of its two pulls, and their bits.
type PinRow = (
    PinPull,
    (&'static str, &'static str),
    TileBit,
    Option<TileBit>,
);

/// A pull's bits: no pull is one bit, and the pull-down is that and another.
fn pull(
    pull: PinPull,
    name: (&'static str, &'static str),
    none: TileBit,
    down: Option<TileBit>,
) -> Option<Named> {
    match pull {
        PinPull::Up => None,
        PinPull::None => Some((name.0, vec![none])),
        PinPull::Down => {
            let mut bits = vec![none];
            bits.extend(down);
            Some((name.1, bits))
        }
    }
}

/// Every set bit of the configuration tiles for these options. Measured, each
/// pin alone: `tests/fpga_xray_bitstream.rs` holds it to Vivado's frames.
fn config_tile_table(o: &BitstreamOptions) -> Vec<(ConfigTile, &'static str, Vec<TileBit>)> {
    let mut center: Vec<Named> = Vec::new();
    let pins: [PinRow; 7] = [
        (
            o.mode_pins[0],
            ("M0PIN.PULLNONE", "M0PIN.PULLDOWN"),
            (26, 62, 21),
            Some((27, 62, 18)),
        ),
        (
            o.mode_pins[1],
            ("M1PIN.PULLNONE", "M1PIN.PULLDOWN"),
            (26, 62, 26),
            Some((27, 62, 23)),
        ),
        (
            o.mode_pins[2],
            ("M2PIN.PULLNONE", "M2PIN.PULLDOWN"),
            (27, 62, 31),
            Some((27, 62, 28)),
        ),
        (
            o.jtag_pins[0],
            ("TCKPIN.PULLNONE", "TCKPIN.PULLDOWN"),
            (27, 69, 27),
            Some((26, 69, 25)),
        ),
        (
            o.jtag_pins[1],
            ("TDIPIN.PULLNONE", "TDIPIN.PULLDOWN"),
            (26, 64, 21),
            Some((27, 64, 18)),
        ),
        (
            o.jtag_pins[2],
            ("TDOPIN.PULLNONE", "TDOPIN.PULLDOWN"),
            (27, 64, 31),
            Some((27, 64, 28)),
        ),
        (
            o.jtag_pins[3],
            ("TMSPIN.PULLNONE", "TMSPIN.PULLDOWN"),
            (26, 64, 26),
            Some((27, 64, 23)),
        ),
    ];
    for (value, name, none, down) in pins {
        center.extend(pull(value, name, none, down));
    }
    let config: [(PinPull, &str, TileBit); 4] = [
        (o.config_pins[0], "CCLKPIN.PULLNONE", (27, 69, 31)),
        (o.config_pins[1], "DONEPIN.PULLNONE", (27, 64, 15)),
        (o.config_pins[2], "INITPIN.PULLNONE", (27, 64, 12)),
        (o.config_pins[3], "PROGPIN.PULLNONE", (26, 64, 10)),
    ];
    for (value, name, none) in config {
        if value != PinPull::Up {
            center.push((name, vec![none]));
        }
    }
    if o.disable_jtag {
        center.push((
            "DISABLE_JTAG",
            vec![(26, 68, 26), (27, 68, 27), (27, 68, 28)],
        ));
    }
    // The user code, inverted, its bits taking turns between two frames. An
    // unset one, all ones, is therefore no bits at all.
    if o.user_id != u32::MAX {
        let mut bits = Vec::new();
        for i in 0..32u32 {
            if (o.user_id >> i) & 1 == 0 {
                bits.push((26 + (i % 2), 68, i / 2));
            }
        }
        center.push(("USERID", bits));
    }
    let mut monitor: Vec<Named> = Vec::new();
    match o.jtag_xadc {
        1 => monitor.push((
            "JTAG_XADC.DISABLE",
            vec![(26, 60, 0), (26, 60, 1), (27, 60, 0)],
        )),
        2 => monitor.push((
            "JTAG_XADC.STATUSONLY",
            vec![(26, 60, 3), (27, 60, 1), (27, 60, 3)],
        )),
        _ => {}
    }
    if o.xadc_enhanced_linearity {
        monitor.push(("XADCENHANCEDLINEARITY.ON", vec![(26, 59, 18)]));
    }
    if o.xadc_power_down {
        monitor.push(("XADCPOWERDOWN.ENABLE", vec![(28, 53, 18), (29, 53, 18)]));
    }
    let mut out: Vec<_> = center
        .into_iter()
        .map(|(n, b)| (ConfigTile::Center, n, b))
        .collect();
    out.extend(
        monitor
            .into_iter()
            .map(|(n, b)| (ConfigTile::Monitor, n, b)),
    );
    out
}

/// What one option-carrying bit of a configuration tile is, for a reader who
/// has found it set: `--decode` says this rather than `?`. The name is
/// Vivado's property and value.
///
/// `tile_type` is the tile's type and `row` and `col` its bit in a `segbits`
/// file's coordinates, which for these two tiles is frame and `word * 32 +
/// bit`.
pub fn config_tile_bit_name(tile_type: &str, row: u32, col: u32) -> Option<String> {
    let word = usize::try_from(col / 32).ok()?;
    let bit = col % 32;
    let wanted = match tile_type {
        "CFG_CENTER_MID" => ConfigTile::Center,
        "MONITOR_BOT" => ConfigTile::Monitor,
        _ => return None,
    };
    if wanted == ConfigTile::Center && word == 68 && bit < 16 && (row == 26 || row == 27) {
        // USERID's bits, each named for the bit of the code it holds.
        return Some(format!("USERID[{}]=0", bit * 2 + (row - 26)));
    }
    // Every pull twice: a pin's pull-down is the bit of no pull and one more,
    // so a pin pulled down reads as both of these names.
    let everything = |pull: PinPull, jtag_xadc: u32| BitstreamOptions {
        mode_pins: [pull; 3],
        jtag_pins: [pull; 4],
        config_pins: [PinPull::None; 4],
        disable_jtag: true,
        jtag_xadc,
        xadc_enhanced_linearity: true,
        xadc_power_down: true,
        ..BitstreamOptions::default()
    };
    for options in [everything(PinPull::None, 1), everything(PinPull::Down, 2)] {
        for (tile, name, bits) in config_tile_table(&options) {
            if tile == wanted && bits.contains(&(row, word, bit)) {
                return Some(name.to_owned());
            }
        }
    }
    None
}

/// The trailing no-operations Vivado writes: the file's length stays what it
/// would have been, so what a `BSPI`, `AXSS` or longer lead-in added comes out
/// of here.
fn tail_nops(options: &BitstreamOptions) -> usize {
    let mut used = options.leading_words() - LEADING_DUMMY_WORDS;
    if options.bspi().is_some() {
        used += 5;
    }
    if matches!(options.usr_access, UsrAccess::Value(_)) {
        used += 2;
    }
    if options.spi_fall_edge {
        used += 2;
    }
    400usize.saturating_sub(used)
}

/// Writes the frames as `BITSTREAM.GENERAL.COMPRESS` does.
///
/// A run of frames goes by `FDRI`, from the first not yet written. It ends at
/// the first frame with a duplicate in its row, and that frame then goes to its
/// own address and to each duplicate by `MFWR`. It also ends at a frame already
/// written, with a zero frame after it, which is never committed: a frame is
/// committed by the one behind it. A run that reaches the end of a row carries
/// on over the row's two pad frames into the next row; at the end of the last
/// row those two are all that follows it. So only frames of one row are ever
/// duplicates of each other, though a run can span rows.
///
/// The block RAM bus wants eight no-operations after each `MFWR`, and the
/// configuration bus none.
///
/// **Measured**: read from Vivado's streams, and exact on four, two of them not
/// committed because a design that fills a row of block RAM is hundreds of
/// kilobytes of frames: see `docs/fpga-vivado-parity.md`.
fn write_compressed(stream: &mut Stream, data: &FrameData) {
    const MFW_PAD: usize = 12;
    const BRAM_NOPS: usize = 8;
    let rows = rows_of(data.layout());
    let zero = [0u32; WORDS_PER_FRAME];
    let frames: Vec<Vec<&[u32]>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|(_, position)| data.frame(*position).unwrap_or(&[]))
                .collect()
        })
        .collect();
    let groups: Vec<std::collections::HashMap<&[u32], Vec<usize>>> = frames
        .iter()
        .map(|row| {
            let mut groups = std::collections::HashMap::<&[u32], Vec<usize>>::new();
            for (index, frame) in row.iter().enumerate() {
                groups.entry(frame).or_default().push(index);
            }
            groups
        })
        .collect();
    let mut done: Vec<Vec<bool>> = rows.iter().map(|row| vec![false; row.len()]).collect();
    let mut first = true;
    let (mut row, mut next) = (0, 0);
    loop {
        while row < rows.len() {
            while next < rows[row].len() && done[row][next] {
                next += 1;
            }
            if next < rows[row].len() {
                break;
            }
            row += 1;
            next = 0;
        }
        if row >= rows.len() {
            break;
        }
        let mut run: Vec<u32> = Vec::new();
        let mut twins = None;
        let (mut at_row, mut at) = (row, next);
        loop {
            if done[at_row][at] {
                run.extend_from_slice(&zero);
                break;
            }
            run.extend_from_slice(frames[at_row][at]);
            done[at_row][at] = true;
            if let Some(group) = groups[at_row]
                .get(frames[at_row][at])
                .filter(|g| g.len() > 1)
            {
                twins = Some((at_row, group));
                break;
            }
            at += 1;
            if at >= rows[at_row].len() {
                run.extend_from_slice(&zero);
                run.extend_from_slice(&zero);
                at_row += 1;
                at = 0;
                if at_row >= rows.len() {
                    break;
                }
            }
        }
        let far = rows[row][next].0.to_u32();
        if first {
            stream.write(Register::Far, far);
            stream.command(Command::Wcfg);
            stream.nops(1);
            first = false;
        } else {
            stream.command(Command::Wcfg);
            stream.nops(1);
            stream.write(Register::Far, far);
            stream.nops(1);
        }
        stream.frames_short(&run);
        if let Some((twin_row, twins)) = twins {
            let settle = if rows[twin_row][0].0.block == 1 {
                BRAM_NOPS
            } else {
                0
            };
            stream.command(Command::Mfw);
            stream.nops(MFW_PAD);
            stream.write_block(Register::Mfwr, &[0; 8]);
            stream.nops(settle);
            for twin in twins.iter().skip(1) {
                done[twin_row][*twin] = true;
                stream.write(Register::Far, rows[twin_row][*twin].0.to_u32());
                stream.write_block(Register::Mfwr, &[0; 4]);
                stream.nops(settle);
            }
        }
    }
}

/// The layout's rows: runs of real frames between pad frames, with each frame's
/// address and position in the stream.
fn rows_of(layout: &FrameLayout) -> Vec<Vec<(FrameAddress, usize)>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    for (position, address) in layout.order().iter().enumerate() {
        match address {
            Some(address) => row.push((*address, position)),
            None => {
                if !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                }
            }
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

/// `CRC` where the option is on, `RCRC` where it is off.
fn check(stream: &mut Stream, options: &BitstreamOptions) {
    if options.crc {
        stream.checkpoint();
    } else {
        stream.command(Command::Rcrc);
    }
}

/// The packets that follow the sync word, as Vivado writes them with these
/// options.
///
/// # Errors
///
/// [`Xc7Error::Malformed`] when the frame data is in a layout that is not the
/// part's.
pub fn stream_words(
    part: &Part,
    data: &FrameData,
    options: &BitstreamOptions,
) -> Result<Vec<u32>, Xc7Error> {
    if data.layout() != &part.layout {
        return Err(Xc7Error::Malformed(
            "the frame data was built in a different layout from the part's".to_owned(),
        ));
    }
    let data = data.with_ecc();
    let mut stream = Stream::new();
    stream.nops(1);
    if options.debug {
        stream.write_uncounted(Register::Lout, 0);
    }
    if let Some(bspi) = options.bspi() {
        stream.write(Register::Bspi, bspi);
        stream.command(Command::BspiRead);
        stream.nops(1);
    }
    stream.write(Register::Timer, options.timer());
    stream.write(Register::Wbstar, options.wbstar());
    let reboot = options.next_config_addr.is_some() && options.next_config_reboot;
    stream.command(if reboot {
        Command::Iprog
    } else {
        Command::Null
    });
    stream.nops(1);
    stream.command(Command::Rcrc);
    stream.nops(2);
    if let UsrAccess::Value(value) = options.usr_access {
        stream.write(Register::Axss, value);
    }
    stream.write_raw(RESERVED_REGISTER_19, 0);
    stream.write(Register::Cor0, options.cor0());
    stream.write(Register::Cor1, options.cor1());
    stream.write(Register::Idcode, part.idcode);
    if options.spi_fall_edge {
        stream.command(Command::FallEdge);
    }
    stream.command(Command::Switch);
    stream.nops(1);
    let ctl0 = options.ctl0();
    stream.write(Register::Mask, ctl0 & !0x100);
    stream.write(Register::Ctl0, ctl0);
    stream.write(Register::Mask, options.ctl1());
    stream.write(Register::Ctl1, options.ctl1());
    stream.nops(8);

    if options.compress {
        write_compressed(&mut stream, &data);
        check(&mut stream, options);
    } else if options.per_frame_crc {
        for row in &rows_of(data.layout()) {
            stream.command(Command::Wcfg);
            stream.nops(1);
            stream.write(Register::Far, row[0].0.to_u32());
            stream.nops(1);
            let mut last = 0;
            for (address, position) in row {
                let frame = data.frame(*position).unwrap_or(&[]);
                stream.frames_short(frame);
                last = address.to_u32();
                stream.write(Register::Far, last);
                stream.checkpoint();
            }
            // The zero frame that commits the row's last one.
            let zero = vec![0u32; WORDS_PER_FRAME];
            stream.frames_short(&zero);
            stream.write(Register::Far, last);
            stream.checkpoint();
        }
        stream.command(Command::Null);
    } else if options.debug {
        stream.write(Register::Far, 0);
        stream.command(Command::Wcfg);
        stream.nops(1);
        for (position, address) in data.layout().order().iter().enumerate() {
            let frame = data.frame(position).unwrap_or(&[]);
            match address {
                Some(address) => {
                    stream.frames_short(frame);
                    stream.write_uncounted(Register::Lout, address.to_u32());
                }
                // A pad frame in this form is bare zero words, with no packet
                // around them, and Vivado's CRC still counts them as `FDRI`.
                None => stream.bare_frame(frame),
            }
        }
        check(&mut stream, options);
    } else {
        stream.write(Register::Far, 0);
        stream.command(Command::Wcfg);
        stream.nops(1);
        stream.frames(data.words());
        check(&mut stream, options);
    }
    stream.nops(2);
    if options.per_frame_crc {
        stream.write(Register::Mask, options.ctl1());
        stream.write(Register::Ctl1, 0);
    }
    stream.command(Command::Grestore);
    stream.nops(1);
    stream.command(Command::Lfrm);
    if options.compress {
        stream.write(Register::Mask, options.ctl1());
        stream.write(Register::Ctl1, 0);
    }
    stream.nops(100);
    stream.command(Command::Start);
    stream.nops(1);
    stream.write(Register::Far, PARKED_FAR);
    // `GLUTMASK_B` (bit 8) is in this mask whether it is set or not, and out
    // of the first: a `FRAME_ECCE2` clears it, and Vivado's masks are then
    // 0x401 and 0x501 around a `CTL0` of 0x401 (`testdata/fpga/vivado/config/`).
    stream.write(Register::Mask, ctl0 | 0x100);
    stream.write(Register::Ctl0, ctl0);
    check(&mut stream, options);
    stream.nops(2);
    stream.command(Command::Desync);
    stream.nops(tail_nops(options));
    Ok(stream.words)
}

/// A `.rbt`: what Vivado's `-raw_bitfile` writes, the configuration stream as a
/// line of 32 ones and zeros per word under a header.
///
/// `payload` is the stream as a `.bin` holds it. The header differs from
/// Vivado's in saying who wrote it and in the date, which is [`super::BitHeader`]'s
/// fixed one so that two runs give the same bytes.
pub fn raw_bitfile(header: &super::BitHeader, architecture: &str, payload: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(payload.len() * 9 + 256);
    let _ = writeln!(out, "Xilinx ASCII Bitstream");
    let _ = writeln!(out, "Created by Reticle");
    let _ = writeln!(out, "Design name: \t{}", header.design);
    let _ = writeln!(out, "Architecture:\t{architecture}");
    let _ = writeln!(out, "Part:        \t{}", header.part);
    let _ = writeln!(out, "Date:        \t{} {}", header.date, header.time);
    let _ = writeln!(out, "Bits:        \t{}", payload.len() * 8);
    for word in payload.chunks(4) {
        for byte in word {
            let _ = write!(out, "{byte:08b}");
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`SPI_32BIT_ADDR` shifts `WBSTAR` down eight bits, revision bits and
    /// all**, as Vivado 2026.1 wrote it for four streams, and an address it
    /// would cut is refused as Vivado refuses it (`Designutils 12-2356`). Would
    /// catch the shift lost, the address alone shifted, or the refusal let
    /// through; the streams themselves are `tests/fpga_xray_config.rs`'s
    /// (`xdc_startup`).
    #[test]
    fn a_32_bit_spi_address_is_written_a_byte_down() {
        let with = |pairs: &[(&str, &str)]| {
            let mut o = BitstreamOptions::default();
            for (name, value) in pairs {
                o.set(name, value).unwrap();
            }
            o
        };
        let next = ("BITSTREAM.CONFIG.NEXT_CONFIG_ADDR", "0x00400000");
        let wide = ("BITSTREAM.CONFIG.SPI_32BIT_ADDR", "YES");
        assert_eq!(with(&[next]).wbstar(), 0x0040_0000);
        assert_eq!(with(&[next, wide]).wbstar(), 0x0000_4000);
        let top = ("BITSTREAM.CONFIG.NEXT_CONFIG_ADDR", "0x1FC00000");
        assert_eq!(with(&[top, wide]).wbstar(), 0x001f_c000);
        let revision = [
            next,
            wide,
            ("BITSTREAM.CONFIG.REVISIONSELECT", "10"),
            ("BITSTREAM.CONFIG.REVISIONSELECT_TRISTATE", "ENABLE"),
        ];
        assert_eq!(with(&revision).wbstar(), 0x00a0_4000);
        let odd = ("BITSTREAM.CONFIG.NEXT_CONFIG_ADDR", "0x00123456");
        assert!(with(&[odd]).check_options().is_ok());
        let refused = with(&[odd, wide]).check_options().unwrap_err().to_string();
        assert!(refused.contains("Designutils 12-2356"), "{refused}");
    }

    #[test]
    fn the_defaults_are_the_words_every_vivado_stream_carries() {
        let o = BitstreamOptions::default();
        assert_eq!(o.cor0(), 0x0200_3FE5);
        assert_eq!(o.cor1(), 0);
        assert_eq!(o.ctl0(), 0x0000_0501);
        assert_eq!(o.ctl1(), 0);
        assert_eq!(o.wbstar(), 0);
        assert_eq!(o.timer(), 0);
        assert_eq!(o.bspi(), None);
        assert_eq!(o.leading_words(), 8);
        assert!(o.config_tile_bits(ConfigTiles::default()).is_empty());
        assert_eq!(tail_nops(&o), 400);
    }

    #[test]
    fn a_name_vivado_lacks_and_a_value_it_refuses_are_said_to_be() {
        let mut o = BitstreamOptions::default();
        for name in [
            "BITSTREAM.STARTUP.DRIVEDONE",
            "BITSTREAM.CONFIG.DONEPROG",
            "BITSTREAM.CONFIG.OVERTEMPSHUTDOWN",
        ] {
            assert!(matches!(o.set(name, "YES"), Err(OptionError::Unknown(_))));
        }
        assert!(
            o.set("BITSTREAM.CONFIG.CONFIGRATE", "4")
                .unwrap_err()
                .to_string()
                .contains("3 6 9")
        );
        assert!(matches!(
            o.set("BITSTREAM.ENCRYPTION.ENCRYPT", "YES"),
            Err(OptionError::NotImplemented { .. })
        ));
        // A value that is the default of a refused option is no request at all.
        o.set("BITSTREAM.ENCRYPTION.ENCRYPT", "NO").unwrap();
        o.set("bitstream.config.configrate", "33").unwrap();
        assert_eq!(o.config_rate, 33);
    }

    #[test]
    fn every_option_in_the_table_is_one_set_can_reach() {
        // Each property's first value, or a number for the free-form ones: what
        // would catch a table entry whose field `set` has no arm for, which
        // would otherwise be accepted and silently do nothing.
        for spec in option_table() {
            let value = match spec.values.split(' ').next().unwrap_or("") {
                "a" => "0X00000001",
                other => other,
            };
            let mut o = BitstreamOptions::default();
            let result = o.set(spec.name, value);
            match spec.support {
                Support::Refused(_) => {}
                _ => assert!(result.is_ok(), "{}={value}: {result:?}", spec.name),
            }
        }
    }

    /// The words a design's configuration primitives make, each against what
    /// Vivado wrote for the design in `testdata/fpga/vivado/config/` named
    /// beside it. Would catch a bit moved or dropped, and the two rules that the
    /// integration test's gate does not build a design for: two `ICAPE2`s with
    /// no `ICAP_SELECT` select neither (`icap_two`), and an option of `CCLK`
    /// wins over a `STARTUPE2`'s clock (`startup_clk_cclk`).
    #[test]
    fn a_designs_configuration_primitives_write_vivados_words() {
        let with = |design: ConfigUse, sets: &[(&str, &str)]| {
            let mut o = BitstreamOptions::default();
            for (name, value) in sets {
                o.set(name, value).unwrap();
            }
            o.set_design(design);
            o.check().unwrap();
            (o.cor0(), o.ctl0())
        };
        let none = ConfigUse::default();
        assert_eq!(with(none, &[]), (0x0200_3FE5, 0x0000_0501), "base");
        let capture = ConfigUse {
            capture_oneshot: true,
            ..none
        };
        assert_eq!(with(capture, &[]).0, 0x0280_3FE5, "capture_plain");
        let clock = ConfigUse {
            startup_clock: true,
            ..none
        };
        assert_eq!(with(clock, &[]).0, 0x0200_BFE5, "startup_clk");
        let cclk = [("BITSTREAM.STARTUP.STARTUPCLK", "CCLK")];
        assert_eq!(with(clock, &cclk).0, 0x0200_3FE5, "startup_clk_cclk");
        let jtag = [("BITSTREAM.STARTUP.STARTUPCLK", "JTAGCLK")];
        assert_eq!(with(clock, &jtag).0, 0x0201_3FE5, "startup_clk_jtagclk");
        let efar = ConfigUse {
            frame_ecc: Some(false),
            ..none
        };
        assert_eq!(with(efar, &[]).1, 0x0000_0401, "ecc_plain");
        let far = ConfigUse {
            frame_ecc: Some(true),
            ..none
        };
        assert_eq!(with(far, &[]).1, 0x0000_0481, "ecc_far");
        let bottom = ConfigUse {
            icap_bottom: true,
            ..none
        };
        assert_eq!(with(bottom, &[]).1, 0x4000_0501, "icap_loc0");
        let two = ConfigUse {
            icap_bottom: true,
            icap_top: true,
            ..none
        };
        assert_eq!(with(two, &[]).1, 0x0000_0501, "icap_two");
        let pick = [("BITSTREAM.READBACK.ICAP_SELECT", "BOTTOM")];
        assert_eq!(with(two, &pick).1, 0x4000_0501, "icap_two_select_bottom");
        let jtag_off = ConfigUse {
            disable_jtag: true,
            ..none
        };
        let mut o = BitstreamOptions::default();
        o.set_design(jtag_off);
        assert!(o.disable_jtag, "bscan_disable");
    }

    /// What Vivado refuses of an option the design's primitives do not allow,
    /// with its rule in the message. Would catch either check dropped or made of
    /// the wrong site: each builds a stream Vivado would not write.
    #[test]
    fn an_option_the_design_does_not_allow_is_refused() {
        let refused = |design: ConfigUse, name: &str, value: &str| {
            let mut o = BitstreamOptions::default();
            o.set(name, value).unwrap();
            o.check_options().unwrap();
            o.set_design(design);
            o.check().map_err(|e| e.to_string())
        };
        let none = ConfigUse::default();
        let top = ConfigUse {
            icap_top: true,
            ..none
        };
        let select = "BITSTREAM.READBACK.ICAP_SELECT";
        assert!(
            refused(none, select, "TOP")
                .unwrap_err()
                .contains("12-1536")
        );
        assert!(
            refused(top, select, "BOTTOM")
                .unwrap_err()
                .contains("12-1536")
        );
        assert!(refused(top, select, "TOP").is_ok());
        let clock = "BITSTREAM.STARTUP.STARTUPCLK";
        assert!(
            refused(none, clock, "USERCLK")
                .unwrap_err()
                .contains("12-1517")
        );
        let started = ConfigUse {
            startup_clock: true,
            ..none
        };
        assert!(refused(started, clock, "USERCLK").is_ok());
    }

    #[test]
    fn the_conflicts_vivado_refuses_are_refused() {
        let cases: &[&[(&str, &str)]] = &[
            &[
                ("BITSTREAM.GENERAL.COMPRESS", "TRUE"),
                ("BITSTREAM.GENERAL.PERFRAMECRC", "YES"),
            ],
            &[
                ("BITSTREAM.GENERAL.COMPRESS", "TRUE"),
                ("BITSTREAM.GENERAL.DEBUGBITSTREAM", "YES"),
            ],
            &[
                ("BITSTREAM.GENERAL.CRC", "DISABLE"),
                ("BITSTREAM.GENERAL.PERFRAMECRC", "YES"),
            ],
            &[
                ("BITSTREAM.CONFIG.SPI_BUSWIDTH", "4"),
                ("CONFIG_MODE", "SPIx1"),
            ],
            &[
                ("BITSTREAM.CONFIG.SPI_BUSWIDTH", "NONE"),
                ("BITSTREAM.CONFIG.SPI_32BIT_ADDR", "YES"),
            ],
            &[("CONFIG_VOLTAGE", "3.3")],
            &[("CONFIG_VOLTAGE", "1.8"), ("CFGBVS", "VCCO")],
        ];
        for case in cases {
            let mut o = BitstreamOptions::default();
            for (name, value) in *case {
                o.set(name, value).unwrap();
            }
            assert!(o.check().is_err(), "{case:?}");
        }
        BitstreamOptions::default().check().unwrap();
    }

    #[test]
    fn a_timestamp_packs_as_vivados_decodes() {
        // 0x25B4FA88, which Vivado wrote at 15:42:08 on 4 November 2026.
        let mut o = BitstreamOptions::default();
        o.set("BITSTREAM.CONFIG.USR_ACCESS", "TIMESTAMP").unwrap();
        o.resolve_timestamp(2026, 11, 4, (15, 42, 8));
        assert_eq!(o.usr_access, UsrAccess::Value(0x25B4_FA88));
    }

    #[test]
    fn a_configuration_bit_found_set_has_a_name() {
        assert_eq!(
            config_tile_bit_name("CFG_CENTER_MID", 27, 68 * 32 + 3).as_deref(),
            Some("USERID[7]=0")
        );
        assert_eq!(
            config_tile_bit_name("CFG_CENTER_MID", 26, 62 * 32 + 21).as_deref(),
            Some("M0PIN.PULLNONE")
        );
        assert_eq!(
            config_tile_bit_name("MONITOR_BOT", 28, 53 * 32 + 18).as_deref(),
            Some("XADCPOWERDOWN.ENABLE")
        );
        assert_eq!(config_tile_bit_name("MONITOR_BOT", 28, 0), None);
        assert_eq!(config_tile_bit_name("CLBLL_L", 26, 68 * 32), None);
    }

    /// Three rows, by hand. The first: a frame, then a pair, the second of whose
    /// frames is not next to it. The second is two lone frames, which carry on
    /// into the third, over the pads, and the third ends the stream. This is
    /// the shape of Vivado's stream for a design that fills a row of block RAM,
    /// which is too many frames to commit, and it would catch the pads, the row
    /// crossing or the end of the stream being got wrong.
    #[test]
    fn a_compressed_run_crosses_a_row_over_its_pads() {
        use super::super::{ConfigRow, FrameLayout};
        let layout = FrameLayout::new(vec![
            ConfigRow {
                block: 0,
                half: 0,
                row: 0,
                columns: vec![4],
            },
            ConfigRow {
                block: 0,
                half: 0,
                row: 1,
                columns: vec![2],
            },
            ConfigRow {
                block: 0,
                half: 0,
                row: 2,
                columns: vec![1],
            },
        ]);
        let mut words = vec![0u32; layout.words()];
        let mut fill = |position: usize, value: u32| {
            words[position * WORDS_PER_FRAME] = value;
        };
        // Row 0: 1, 2, 1, 3. Row 1 (after two pads): 4, 5. Row 2: 6.
        for (position, value) in [(0, 1), (1, 2), (2, 1), (3, 3), (6, 4), (7, 5), (10, 6)] {
            fill(position, value);
        }
        let data = FrameData::from_stream(layout.clone(), words).unwrap();
        let mut stream = Stream::new();
        write_compressed(&mut stream, &data);
        let packets = super::super::packets(&stream.words).unwrap();
        let mut seen = Vec::new();
        for packet in &packets {
            let super::super::Packet::Write { register, words } = packet else {
                continue;
            };
            match Register::from_u32(*register) {
                Some(Register::Fdri) => seen.push(format!(
                    "fdri {:?}",
                    words
                        .chunks(WORDS_PER_FRAME)
                        .map(|f| f[0])
                        .collect::<Vec<u32>>()
                )),
                Some(Register::Far) => seen.push(format!("far {:x}", words[0])),
                Some(Register::Mfwr) => seen.push("mfwr".to_owned()),
                _ => {}
            }
        }
        assert_eq!(
            seen,
            [
                "far 0",
                "fdri [1]",
                "mfwr",
                "far 2",
                "mfwr",
                // Frame 2 of the row is written, so this run stops at frame 3's
                // end: a lone frame at a row's end goes on over the pads, and
                // the third row ends it with its own two.
                "far 1",
                "fdri [2, 0]",
                "far 3",
                "fdri [3, 0, 0, 4, 5, 0, 0, 6, 0, 0]",
            ]
        );
        // A second opinion: the replay of a whole stream says the same.
        let part = Part {
            name: "tiny".to_owned(),
            idcode: 1,
            layout: layout.clone(),
        };
        let options = BitstreamOptions {
            compress: true,
            ..BitstreamOptions::default()
        };
        let bytes = super::super::write_bit_with(
            &super::super::BitHeader::new("t", "tiny"),
            &part,
            &data,
            &options,
        )
        .unwrap();
        let back = super::super::read_bit(&bytes).unwrap();
        assert_eq!(back.image(layout).unwrap().words(), data.with_ecc().words());
    }

    #[test]
    fn a_raw_file_is_the_stream_in_ones_and_zeros() {
        let header = super::super::BitHeader::new("t", "7atiny");
        let text = raw_bitfile(&header, "artix7", &[0xAA, 0x99, 0x55, 0x66]);
        assert!(text.contains("Bits:        \t32\n"));
        assert!(text.ends_with("10101010100110010101010101100110\n"));
    }
}
