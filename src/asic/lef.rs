//! LEF 5.8 (Library Exchange Format) reader and writer.
//!
//! A LEF file describes the physical side of a technology and its cell
//! library: the routing and cut layers with their pitch, width and
//! spacing rules, the vias between them, the placement `SITE`s, and one
//! `MACRO` per cell with its size, pin geometry and obstructions. It is
//! what a placer needs next to the Liberty file.
//!
//! [`parse_lef`] turns text into a [`Lef`]; [`write_lef`] renders one back
//! in a canonical layout (two-space indent, one statement per line) that
//! is stable under parse/write round-trips. The reader models the
//! statements that Reticle and the open-source flows around it use;
//! everything else is preserved verbatim as [`Raw`] statements on the
//! enclosing object so a file survives a round-trip without losing rules
//! the typed structs do not describe. Coordinates are microns as written
//! (`f64`); a consumer that wants database units multiplies by
//! [`Lef::database_microns`].

use super::lefdef::{Cursor, Kind, PResult};
use super::{Orient, Property, PropertyDefinition, PropertyValue, Raw, fmt_num, quote_if_needed};
use crate::diag::Diagnostics;
use crate::source::SourceId;

/// A point in microns.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Point {
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
}

/// An axis-aligned rectangle in microns, `(x1, y1)` to `(x2, y2)`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    /// Left.
    pub x1: f64,
    /// Bottom.
    pub y1: f64,
    /// Right.
    pub x2: f64,
    /// Top.
    pub y2: f64,
}

/// One `UNITS` entry: `DATABASE MICRONS 1000`, `TIME NANOSECONDS 100`, ...
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    /// `DATABASE`, `TIME`, `CAPACITANCE`, `RESISTANCE`, ...
    pub kind: String,
    /// `MICRONS`, `NANOSECONDS`, `PICOFARADS`, ...
    pub unit: String,
    /// The factor.
    pub value: f64,
}

/// The `TYPE` of a layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerKind {
    /// `ROUTING`.
    Routing,
    /// `CUT`.
    Cut,
    /// `MASTERSLICE`.
    Masterslice,
    /// `OVERLAP`.
    Overlap,
    /// `IMPLANT`.
    Implant,
    /// Anything else, as written.
    Other(String),
}

impl LayerKind {
    /// The keyword.
    pub fn as_str(&self) -> &str {
        match self {
            LayerKind::Routing => "ROUTING",
            LayerKind::Cut => "CUT",
            LayerKind::Masterslice => "MASTERSLICE",
            LayerKind::Overlap => "OVERLAP",
            LayerKind::Implant => "IMPLANT",
            LayerKind::Other(s) => s,
        }
    }

    fn parse(s: &str) -> LayerKind {
        match s {
            "ROUTING" => LayerKind::Routing,
            "CUT" => LayerKind::Cut,
            "MASTERSLICE" => LayerKind::Masterslice,
            "OVERLAP" => LayerKind::Overlap,
            "IMPLANT" => LayerKind::Implant,
            other => LayerKind::Other(other.to_string()),
        }
    }
}

/// The preferred routing direction of a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// `HORIZONTAL`.
    Horizontal,
    /// `VERTICAL`.
    Vertical,
    /// `DIAG45`.
    Diag45,
    /// `DIAG135`.
    Diag135,
}

impl Direction {
    /// The keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Horizontal => "HORIZONTAL",
            Direction::Vertical => "VERTICAL",
            Direction::Diag45 => "DIAG45",
            Direction::Diag135 => "DIAG135",
        }
    }

    fn parse(s: &str) -> Option<Direction> {
        Some(match s {
            "HORIZONTAL" => Direction::Horizontal,
            "VERTICAL" => Direction::Vertical,
            "DIAG45" => Direction::Diag45,
            "DIAG135" => Direction::Diag135,
            _ => return None,
        })
    }
}

/// A `SPACING` rule of a layer: the minimum spacing plus any qualifiers
/// (`RANGE a b`, `ADJACENTCUTS n WITHIN w`, `SAMENET`, ...) kept as
/// tokens.
#[derive(Clone, Debug, PartialEq)]
pub struct Spacing {
    /// The spacing in microns.
    pub value: f64,
    /// The qualifying tokens after the value.
    pub qualifiers: Vec<String>,
}

/// A `LAYER` section.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// The layer name.
    pub name: String,
    /// `TYPE`.
    pub kind: Option<LayerKind>,
    /// `DIRECTION`.
    pub direction: Option<Direction>,
    /// `PITCH x [y]`; a single value is stored for both axes.
    pub pitch: Option<(f64, f64)>,
    /// `OFFSET x [y]`.
    pub offset: Option<(f64, f64)>,
    /// `WIDTH`.
    pub width: Option<f64>,
    /// `MINWIDTH`.
    pub min_width: Option<f64>,
    /// `MAXWIDTH`.
    pub max_width: Option<f64>,
    /// `AREA`.
    pub area: Option<f64>,
    /// `THICKNESS`.
    pub thickness: Option<f64>,
    /// `HEIGHT`.
    pub height: Option<f64>,
    /// `SPACING` rules, in order.
    pub spacing: Vec<Spacing>,
    /// `RESISTANCE RPERSQ` (routing layers).
    pub resistance_per_square: Option<f64>,
    /// `RESISTANCE` (cut layers).
    pub resistance: Option<f64>,
    /// `CAPACITANCE CPERSQDIST`.
    pub capacitance_per_square: Option<f64>,
    /// `EDGECAPACITANCE`.
    pub edge_capacitance: Option<f64>,
    /// `PROPERTY` values.
    pub properties: Vec<Property>,
    /// Statements not modelled above (`SPACINGTABLE`, antenna rules,
    /// `MINIMUMCUT`, ...).
    pub extras: Vec<Raw>,
}

impl Layer {
    /// A layer with only a name.
    pub fn new(name: impl Into<String>) -> Layer {
        Layer {
            name: name.into(),
            kind: None,
            direction: None,
            pitch: None,
            offset: None,
            width: None,
            min_width: None,
            max_width: None,
            area: None,
            thickness: None,
            height: None,
            spacing: Vec::new(),
            resistance_per_square: None,
            resistance: None,
            capacitance_per_square: None,
            edge_capacitance: None,
            properties: Vec::new(),
            extras: Vec::new(),
        }
    }

    /// True for `TYPE ROUTING`.
    pub fn is_routing(&self) -> bool {
        self.kind == Some(LayerKind::Routing)
    }
}

/// A geometric shape inside a layer.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// `RECT [MASK m] x1 y1 x2 y2`.
    Rect {
        /// `MASK` number.
        mask: Option<u32>,
        /// The rectangle.
        rect: Rect,
    },
    /// `POLYGON [MASK m] x y x y ...`.
    Polygon {
        /// `MASK` number.
        mask: Option<u32>,
        /// The vertices.
        points: Vec<Point>,
    },
    /// `PATH [MASK m] x y x y ...`, a centre line of the layer's width.
    Path {
        /// `MASK` number.
        mask: Option<u32>,
        /// The vertices.
        points: Vec<Point>,
    },
}

/// The shapes of one `LAYER` inside a `VIA`.
#[derive(Clone, Debug, PartialEq)]
pub struct ViaLayer {
    /// The layer name.
    pub name: String,
    /// The shapes on it.
    pub shapes: Vec<Shape>,
}

/// The `VIARULE`-generated form of a via.
#[derive(Clone, Debug, PartialEq)]
pub struct ViaGenerate {
    /// `VIARULE`.
    pub rule: String,
    /// `CUTSIZE x y`.
    pub cut_size: Option<(f64, f64)>,
    /// `LAYERS bottom cut top`.
    pub layers: Option<(String, String, String)>,
    /// `CUTSPACING x y`.
    pub cut_spacing: Option<(f64, f64)>,
    /// `ENCLOSURE xb yb xt yt`.
    pub enclosure: Option<(f64, f64, f64, f64)>,
    /// `ROWCOL rows cols`.
    pub row_col: Option<(i64, i64)>,
    /// `ORIGIN x y`.
    pub origin: Option<Point>,
    /// `OFFSET xb yb xt yt`.
    pub offset: Option<(f64, f64, f64, f64)>,
    /// `PATTERN`.
    pub pattern: Option<String>,
}

/// A `VIA` section.
#[derive(Clone, Debug, PartialEq)]
pub struct Via {
    /// The via name.
    pub name: String,
    /// `DEFAULT`.
    pub default: bool,
    /// `GENERATED`.
    pub generated: bool,
    /// The generate-rule form, when the via is defined by a `VIARULE`.
    pub via_rule: Option<ViaGenerate>,
    /// `RESISTANCE`.
    pub resistance: Option<f64>,
    /// The fixed form: shapes per layer.
    pub layers: Vec<ViaLayer>,
    /// `PROPERTY` values.
    pub properties: Vec<Property>,
    /// Unmodelled statements.
    pub extras: Vec<Raw>,
}

/// One `LAYER` of a `VIARULE`.
#[derive(Clone, Debug, PartialEq)]
pub struct ViaRuleLayer {
    /// The layer name.
    pub name: String,
    /// `DIRECTION` (non-generate rules).
    pub direction: Option<Direction>,
    /// `WIDTH min TO max`.
    pub width: Option<(f64, f64)>,
    /// `ENCLOSURE x y` (generate rules, metal layers).
    pub enclosure: Option<(f64, f64)>,
    /// `RECT` (generate rules, cut layer).
    pub rect: Option<Rect>,
    /// `SPACING x BY y` (generate rules, cut layer).
    pub spacing: Option<(f64, f64)>,
    /// `RESISTANCE`.
    pub resistance: Option<f64>,
    /// Unmodelled statements.
    pub extras: Vec<Raw>,
}

/// A `VIARULE` or `VIARULE GENERATE` section.
#[derive(Clone, Debug, PartialEq)]
pub struct ViaRule {
    /// The rule name.
    pub name: String,
    /// `GENERATE`.
    pub generate: bool,
    /// `DEFAULT`.
    pub default: bool,
    /// The layers.
    pub layers: Vec<ViaRuleLayer>,
    /// `VIA name ;` references (non-generate rules).
    pub vias: Vec<String>,
    /// `PROPERTY` values.
    pub properties: Vec<Property>,
    /// Unmodelled statements.
    pub extras: Vec<Raw>,
}

/// A `SYMMETRY` flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symmetry {
    /// `X`.
    X,
    /// `Y`.
    Y,
    /// `R90`.
    R90,
}

impl Symmetry {
    /// The keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            Symmetry::X => "X",
            Symmetry::Y => "Y",
            Symmetry::R90 => "R90",
        }
    }
}

/// A `SITE` section.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    /// The site name.
    pub name: String,
    /// `CLASS` (`CORE`, `PAD`, ...).
    pub class: Option<String>,
    /// `SYMMETRY`.
    pub symmetry: Vec<Symmetry>,
    /// `SIZE w BY h`.
    pub size: Option<(f64, f64)>,
    /// `ROWPATTERN site orient ...`.
    pub row_pattern: Vec<(String, Orient)>,
    /// Unmodelled statements.
    pub extras: Vec<Raw>,
}

/// A `FOREIGN` reference of a macro.
#[derive(Clone, Debug, PartialEq)]
pub struct Foreign {
    /// The foreign cell name.
    pub name: String,
    /// The offset.
    pub point: Option<Point>,
    /// The orientation.
    pub orient: Option<Orient>,
}

/// Pin `DIRECTION`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinDirection {
    /// `INPUT`.
    Input,
    /// `OUTPUT`.
    Output,
    /// `OUTPUT TRISTATE`.
    OutputTristate,
    /// `INOUT`.
    Inout,
    /// `FEEDTHRU`.
    Feedthru,
}

impl PinDirection {
    /// The keyword(s).
    pub fn as_str(self) -> &'static str {
        match self {
            PinDirection::Input => "INPUT",
            PinDirection::Output => "OUTPUT",
            PinDirection::OutputTristate => "OUTPUT TRISTATE",
            PinDirection::Inout => "INOUT",
            PinDirection::Feedthru => "FEEDTHRU",
        }
    }
}

/// Pin `USE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinUse {
    /// `SIGNAL`.
    Signal,
    /// `ANALOG`.
    Analog,
    /// `POWER`.
    Power,
    /// `GROUND`.
    Ground,
    /// `CLOCK`.
    Clock,
    /// Anything else.
    Other(String),
}

impl PinUse {
    /// The keyword.
    pub fn as_str(&self) -> &str {
        match self {
            PinUse::Signal => "SIGNAL",
            PinUse::Analog => "ANALOG",
            PinUse::Power => "POWER",
            PinUse::Ground => "GROUND",
            PinUse::Clock => "CLOCK",
            PinUse::Other(s) => s,
        }
    }

    /// Parses the keyword.
    pub fn parse(s: &str) -> PinUse {
        match s {
            "SIGNAL" => PinUse::Signal,
            "ANALOG" => PinUse::Analog,
            "POWER" => PinUse::Power,
            "GROUND" => PinUse::Ground,
            "CLOCK" => PinUse::Clock,
            other => PinUse::Other(other.to_string()),
        }
    }

    /// True for `POWER` and `GROUND`.
    pub fn is_supply(&self) -> bool {
        matches!(self, PinUse::Power | PinUse::Ground)
    }
}

/// Pin `SHAPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinShape {
    /// `ABUTMENT`.
    Abutment,
    /// `RING`.
    Ring,
    /// `FEEDTHRU`.
    Feedthru,
}

impl PinShape {
    /// The keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            PinShape::Abutment => "ABUTMENT",
            PinShape::Ring => "RING",
            PinShape::Feedthru => "FEEDTHRU",
        }
    }
}

/// The shapes on one layer inside a `PORT` or `OBS`.
#[derive(Clone, Debug, PartialEq)]
pub struct GeometryLayer {
    /// The layer name.
    pub name: String,
    /// `EXCEPTPGNET`.
    pub except_pg_net: bool,
    /// `SPACING`.
    pub spacing: Option<f64>,
    /// `DESIGNRULEWIDTH`.
    pub design_rule_width: Option<f64>,
    /// `WIDTH` for paths.
    pub width: Option<f64>,
    /// The shapes.
    pub shapes: Vec<Shape>,
}

/// One geometry item of a `PORT` or `OBS`.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    /// A `LAYER` with its shapes.
    Layer(GeometryLayer),
    /// `VIA [MASK m] x y name`.
    Via {
        /// `MASK` number.
        mask: Option<u32>,
        /// The via location.
        point: Point,
        /// The via name.
        name: String,
    },
}

/// A `PORT` of a macro pin.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Port {
    /// `CLASS` (`CORE`, `BUMP`, ...).
    pub class: Option<String>,
    /// The geometry.
    pub geometry: Vec<Geometry>,
    /// Unmodelled statements (`ITERATE` shapes and the like).
    pub extras: Vec<Raw>,
}

/// A `PIN` of a macro.
#[derive(Clone, Debug, PartialEq)]
pub struct MacroPin {
    /// The pin name.
    pub name: String,
    /// `DIRECTION`.
    pub direction: Option<PinDirection>,
    /// `USE`.
    pub use_: Option<PinUse>,
    /// `SHAPE`.
    pub shape: Option<PinShape>,
    /// `TAPERRULE`.
    pub taper_rule: Option<String>,
    /// `MUSTJOIN`.
    pub must_join: Option<String>,
    /// `NETEXPR`.
    pub net_expr: Option<String>,
    /// `SUPPLYSENSITIVITY`.
    pub supply_sensitivity: Option<String>,
    /// `GROUNDSENSITIVITY`.
    pub ground_sensitivity: Option<String>,
    /// The ports.
    pub ports: Vec<Port>,
    /// `PROPERTY` values.
    pub properties: Vec<Property>,
    /// Unmodelled statements (antenna values and the like).
    pub extras: Vec<Raw>,
}

impl MacroPin {
    /// A pin with only a name.
    pub fn new(name: impl Into<String>) -> MacroPin {
        MacroPin {
            name: name.into(),
            direction: None,
            use_: None,
            shape: None,
            taper_rule: None,
            must_join: None,
            net_expr: None,
            supply_sensitivity: None,
            ground_sensitivity: None,
            ports: Vec::new(),
            properties: Vec::new(),
            extras: Vec::new(),
        }
    }

    /// True for `USE POWER` / `USE GROUND`.
    pub fn is_supply(&self) -> bool {
        self.use_.as_ref().is_some_and(PinUse::is_supply)
    }
}

/// A `MACRO` section: the physical abstract of one cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Macro {
    /// The macro name.
    pub name: String,
    /// `CLASS` tokens (`CORE`, `CORE SPACER`, `PAD INPUT`, `BLOCK`, ...).
    pub class: Vec<String>,
    /// `FIXEDMASK`.
    pub fixed_mask: bool,
    /// `ORIGIN`.
    pub origin: Option<Point>,
    /// `FOREIGN` references.
    pub foreign: Vec<Foreign>,
    /// `SIZE w BY h`.
    pub size: Option<(f64, f64)>,
    /// `SYMMETRY`.
    pub symmetry: Vec<Symmetry>,
    /// `SITE`.
    pub site: Option<String>,
    /// `EEQ`.
    pub eeq: Option<String>,
    /// The pins.
    pub pins: Vec<MacroPin>,
    /// `OBS` geometry.
    pub obs: Vec<Geometry>,
    /// `PROPERTY` values.
    pub properties: Vec<Property>,
    /// Unmodelled statements (`DENSITY`, ...).
    pub extras: Vec<Raw>,
}

impl Macro {
    /// A macro with only a name.
    pub fn new(name: impl Into<String>) -> Macro {
        Macro {
            name: name.into(),
            class: Vec::new(),
            fixed_mask: false,
            origin: None,
            foreign: Vec::new(),
            size: None,
            symmetry: Vec::new(),
            site: None,
            eeq: None,
            pins: Vec::new(),
            obs: Vec::new(),
            properties: Vec::new(),
            extras: Vec::new(),
        }
    }

    /// The pin with this name.
    pub fn pin(&self, name: &str) -> Option<&MacroPin> {
        self.pins.iter().find(|p| p.name == name)
    }

    /// The signal (non-supply) pins.
    pub fn signal_pins(&self) -> impl Iterator<Item = &MacroPin> {
        self.pins.iter().filter(|p| !p.is_supply())
    }
}

/// A parsed LEF file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Lef {
    /// `VERSION`.
    pub version: Option<String>,
    /// `BUSBITCHARS`.
    pub bus_bit_chars: Option<String>,
    /// `DIVIDERCHAR`.
    pub divider_char: Option<String>,
    /// `UNITS` entries.
    pub units: Vec<Unit>,
    /// `MANUFACTURINGGRID`.
    pub manufacturing_grid: Option<f64>,
    /// `CLEARANCEMEASURE`.
    pub clearance_measure: Option<String>,
    /// `USEMINSPACING OBS|PIN ON|OFF`.
    pub use_min_spacing: Vec<(String, bool)>,
    /// `PROPERTYDEFINITIONS`.
    pub property_definitions: Vec<PropertyDefinition>,
    /// `LAYER` sections, in order.
    pub layers: Vec<Layer>,
    /// `VIA` sections.
    pub vias: Vec<Via>,
    /// `VIARULE` sections.
    pub via_rules: Vec<ViaRule>,
    /// `SITE` sections.
    pub sites: Vec<Site>,
    /// `MACRO` sections.
    pub macros: Vec<Macro>,
    /// Unmodelled top-level statements and sections.
    pub extras: Vec<Raw>,
}

impl Lef {
    /// `UNITS DATABASE MICRONS`, the number of database units per micron.
    pub fn database_microns(&self) -> Option<f64> {
        self.units
            .iter()
            .find(|u| u.kind == "DATABASE")
            .map(|u| u.value)
    }

    /// The layer with this name.
    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }

    /// Routing layers in order (bottom first, as listed).
    pub fn routing_layers(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter().filter(|l| l.is_routing())
    }

    /// The macro with this name.
    pub fn macro_(&self, name: &str) -> Option<&Macro> {
        self.macros.iter().find(|m| m.name == name)
    }

    /// The site with this name.
    pub fn site(&self, name: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.name == name)
    }

    /// The via with this name.
    pub fn via(&self, name: &str) -> Option<&Via> {
        self.vias.iter().find(|v| v.name == name)
    }
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Parses LEF text. Syntax errors are reported to `diags` with spans and
/// the offending statement is skipped; the returned file holds everything
/// that parsed.
pub fn parse_lef(text: &str, file: SourceId, diags: &mut Diagnostics) -> Lef {
    let mut c = Cursor::new(text, file);
    let mut lef = Lef::default();
    while !c.at_end() {
        if c.eat_kind(Kind::Semi) {
            continue;
        }
        let Some(tok) = c.peek() else { break };
        if tok.kind != Kind::Word {
            c.recover(c.error("expected a statement keyword"), diags);
            continue;
        }
        let result = match tok.text {
            "END" => {
                c.next();
                if c.eat_word("LIBRARY") {
                    break;
                }
                Err(c.error("expected `END LIBRARY`"))
            }
            "VERSION" => {
                c.next();
                c.name().and_then(|v| {
                    lef.version = Some(v.to_string());
                    c.expect_semi()
                })
            }
            "BUSBITCHARS" => {
                c.next();
                c.name().and_then(|v| {
                    lef.bus_bit_chars = Some(v.to_string());
                    c.expect_semi()
                })
            }
            "DIVIDERCHAR" => {
                c.next();
                c.name().and_then(|v| {
                    lef.divider_char = Some(v.to_string());
                    c.expect_semi()
                })
            }
            "MANUFACTURINGGRID" => {
                c.next();
                c.number().and_then(|v| {
                    lef.manufacturing_grid = Some(v);
                    c.expect_semi()
                })
            }
            "CLEARANCEMEASURE" => {
                c.next();
                c.word().and_then(|v| {
                    lef.clearance_measure = Some(v.to_string());
                    c.expect_semi()
                })
            }
            "USEMINSPACING" => {
                c.next();
                (|| {
                    let what = c.word()?.to_string();
                    let on = c.word()? == "ON";
                    lef.use_min_spacing.push((what, on));
                    c.expect_semi()
                })()
            }
            "UNITS" => {
                c.next();
                parse_units(&mut c, &mut lef.units, diags)
            }
            "PROPERTYDEFINITIONS" => {
                c.next();
                parse_property_definitions(&mut c, &mut lef.property_definitions, diags)
            }
            "LAYER" => {
                c.next();
                parse_layer(&mut c, diags).map(|l| lef.layers.push(l))
            }
            "VIA" => {
                c.next();
                parse_via(&mut c, diags).map(|v| lef.vias.push(v))
            }
            "VIARULE" => {
                c.next();
                parse_via_rule(&mut c, diags).map(|v| lef.via_rules.push(v))
            }
            "SITE" => {
                c.next();
                parse_site(&mut c, diags).map(|s| lef.sites.push(s))
            }
            "MACRO" => {
                c.next();
                parse_macro(&mut c, diags).map(|m| lef.macros.push(m))
            }
            "NONDEFAULTRULE" | "ARRAY" => {
                // `KEYWORD name ... END name`
                let raw = raw_block(&mut c, true);
                lef.extras.push(raw);
                Ok(())
            }
            "SPACING" | "IRDROP" | "NOISETABLE" | "CORRECTIONTABLE" | "BEGINEXT" => {
                // `KEYWORD ... END KEYWORD` (`BEGINEXT ... ENDEXT`).
                let raw = raw_block(&mut c, false);
                lef.extras.push(raw);
                Ok(())
            }
            _ => {
                lef.extras.push(Raw::statement(c.raw_statement()));
                Ok(())
            }
        };
        if let Err(e) = result {
            c.recover(e, diags);
        }
    }
    lef
}

/// Collects a `KEYWORD [name] ... END KEYWORD|name` section verbatim.
fn raw_block(c: &mut Cursor<'_>, named: bool) -> Raw {
    let mut tokens = Vec::new();
    let keyword = c.next().map(|t| t.text.to_string()).unwrap_or_default();
    tokens.push(keyword.clone());
    let terminator = if named {
        let name = c.next().map(|t| t.text.to_string()).unwrap_or_default();
        tokens.push(name.clone());
        name
    } else if keyword == "BEGINEXT" {
        "ENDEXT".to_string()
    } else {
        keyword
    };
    while let Some(t) = c.next() {
        let text = match t.kind {
            Kind::Str => format!("\"{}\"", t.text),
            _ => t.text.to_string(),
        };
        if terminator == "ENDEXT" && text == "ENDEXT" {
            tokens.push(text);
            break;
        }
        let is_end = text == "END" && c.is_word(&terminator);
        tokens.push(text);
        if is_end {
            if let Some(n) = c.next() {
                tokens.push(n.text.to_string());
            }
            break;
        }
    }
    Raw {
        tokens,
        block: true,
    }
}

/// Consumes `END [name]` and checks the name when given.
fn expect_end(c: &mut Cursor<'_>, name: &str, diags: &mut Diagnostics) -> PResult<()> {
    c.expect_word("END")?;
    if c.is_kind(Kind::Word) || c.is_kind(Kind::Str) {
        let span = c.span();
        let got = c.name()?;
        if got != name {
            diags.push(
                crate::diag::Diagnostic::warning(format!("`END {got}` closes section `{name}`"))
                    .with_span(span),
            );
        }
    }
    Ok(())
}

fn parse_units(c: &mut Cursor<'_>, units: &mut Vec<Unit>, diags: &mut Diagnostics) -> PResult<()> {
    loop {
        if c.at_end() {
            return Err(c.error("unterminated UNITS"));
        }
        if c.is_word("END") {
            return expect_end(c, "UNITS", diags);
        }
        let r = (|| {
            let kind = c.word()?.to_string();
            let unit = c.word()?.to_string();
            let value = c.number()?;
            c.expect_semi()?;
            units.push(Unit { kind, unit, value });
            Ok(())
        })();
        if let Err(e) = r {
            c.recover(e, diags);
        }
    }
}

fn parse_property_value(c: &mut Cursor<'_>) -> PResult<PropertyValue> {
    match c.peek() {
        Some(t) if t.kind == Kind::Str => {
            c.next();
            Ok(PropertyValue::String(t.text.to_string()))
        }
        Some(t) if t.kind == Kind::Word => {
            c.next();
            Ok(match t.text.parse::<f64>() {
                Ok(v) => PropertyValue::Number(v),
                Err(_) => PropertyValue::String(t.text.to_string()),
            })
        }
        _ => Err(c.error("expected a property value")),
    }
}

fn parse_properties(c: &mut Cursor<'_>, out: &mut Vec<Property>) -> PResult<()> {
    while !c.is_kind(Kind::Semi) && !c.at_end() {
        let name = c.name()?.to_string();
        let value = parse_property_value(c)?;
        out.push(Property { name, value });
    }
    c.expect_semi()
}

fn parse_property_definitions(
    c: &mut Cursor<'_>,
    out: &mut Vec<PropertyDefinition>,
    diags: &mut Diagnostics,
) -> PResult<()> {
    loop {
        if c.at_end() {
            return Err(c.error("unterminated PROPERTYDEFINITIONS"));
        }
        if c.is_word("END") {
            return expect_end(c, "PROPERTYDEFINITIONS", diags);
        }
        let r = (|| {
            let object = c.word()?.to_string();
            let name = c.name()?.to_string();
            let ty = c.word()?.to_string();
            let mut def = PropertyDefinition {
                object,
                name,
                ty,
                range: None,
                value: None,
            };
            while !c.is_kind(Kind::Semi) && !c.at_end() {
                if c.eat_word("RANGE") {
                    let lo = c.number()?;
                    let hi = c.number()?;
                    def.range = Some((lo, hi));
                } else {
                    def.value = Some(parse_property_value(c)?);
                }
            }
            c.expect_semi()?;
            out.push(def);
            Ok(())
        })();
        if let Err(e) = r {
            c.recover(e, diags);
        }
    }
}

fn parse_pair(c: &mut Cursor<'_>) -> PResult<(f64, f64)> {
    let x = c.number()?;
    let y = if c.is_kind(Kind::Semi) {
        x
    } else {
        c.number()?
    };
    Ok((x, y))
}

fn parse_rect(c: &mut Cursor<'_>) -> PResult<Rect> {
    Ok(Rect {
        x1: c.number()?,
        y1: c.number()?,
        x2: c.number()?,
        y2: c.number()?,
    })
}

fn parse_points(c: &mut Cursor<'_>) -> PResult<Vec<Point>> {
    let mut pts = Vec::new();
    while !c.is_kind(Kind::Semi) && !c.at_end() {
        let x = c.number()?;
        let y = c.number()?;
        pts.push(Point { x, y });
    }
    Ok(pts)
}

fn parse_mask(c: &mut Cursor<'_>) -> PResult<Option<u32>> {
    if c.eat_word("MASK") {
        let v = c.integer()?;
        return u32::try_from(v)
            .map(Some)
            .map_err(|_| c.error("mask number out of range"));
    }
    Ok(None)
}

fn parse_layer(c: &mut Cursor<'_>, diags: &mut Diagnostics) -> PResult<Layer> {
    let name = c.name()?.to_string();
    let mut layer = Layer::new(&name);
    loop {
        if c.at_end() {
            return Err(c.error(format!("unterminated LAYER {name}")));
        }
        if c.is_word("END") {
            expect_end(c, &name, diags)?;
            return Ok(layer);
        }
        let Some(tok) = c.peek() else { break };
        let r = match tok.text {
            "TYPE" => {
                c.next();
                c.word().and_then(|t| {
                    layer.kind = Some(LayerKind::parse(t));
                    c.expect_semi()
                })
            }
            "DIRECTION" => {
                c.next();
                c.word().and_then(|d| match Direction::parse(d) {
                    Some(d) => {
                        layer.direction = Some(d);
                        c.expect_semi()
                    }
                    None => Err(c.error(format!("unknown direction `{d}`"))),
                })
            }
            "PITCH" => {
                c.next();
                parse_pair(c).and_then(|p| {
                    layer.pitch = Some(p);
                    c.expect_semi()
                })
            }
            "OFFSET" => {
                c.next();
                parse_pair(c).and_then(|p| {
                    layer.offset = Some(p);
                    c.expect_semi()
                })
            }
            "WIDTH" | "MINWIDTH" | "MAXWIDTH" | "AREA" | "THICKNESS" | "HEIGHT"
            | "EDGECAPACITANCE" => {
                let key = tok.text;
                c.next();
                c.number().and_then(|v| {
                    match key {
                        "WIDTH" => layer.width = Some(v),
                        "MINWIDTH" => layer.min_width = Some(v),
                        "MAXWIDTH" => layer.max_width = Some(v),
                        "AREA" => layer.area = Some(v),
                        "THICKNESS" => layer.thickness = Some(v),
                        "HEIGHT" => layer.height = Some(v),
                        _ => layer.edge_capacitance = Some(v),
                    }
                    c.expect_semi()
                })
            }
            "SPACING" => {
                c.next();
                c.number().and_then(|v| {
                    let qualifiers = c.raw_until(&[]);
                    layer.spacing.push(Spacing {
                        value: v,
                        qualifiers,
                    });
                    c.expect_semi()
                })
            }
            "RESISTANCE" => {
                c.next();
                if c.eat_word("RPERSQ") {
                    c.number().and_then(|v| {
                        layer.resistance_per_square = Some(v);
                        c.expect_semi()
                    })
                } else {
                    c.number().and_then(|v| {
                        layer.resistance = Some(v);
                        c.expect_semi()
                    })
                }
            }
            "CAPACITANCE" => {
                c.next();
                c.eat_word("CPERSQDIST");
                c.number().and_then(|v| {
                    layer.capacitance_per_square = Some(v);
                    c.expect_semi()
                })
            }
            "PROPERTY" => {
                c.next();
                parse_properties(c, &mut layer.properties)
            }
            _ => {
                layer.extras.push(Raw::statement(c.raw_statement()));
                Ok(())
            }
        };
        if let Err(e) = r {
            c.recover(e, diags);
        }
    }
    Ok(layer)
}

fn parse_shape(c: &mut Cursor<'_>, keyword: &str) -> PResult<Shape> {
    let mask = parse_mask(c)?;
    match keyword {
        "RECT" => {
            let rect = parse_rect(c)?;
            c.expect_semi()?;
            Ok(Shape::Rect { mask, rect })
        }
        "POLYGON" => {
            let points = parse_points(c)?;
            c.expect_semi()?;
            Ok(Shape::Polygon { mask, points })
        }
        _ => {
            let points = parse_points(c)?;
            c.expect_semi()?;
            Ok(Shape::Path { mask, points })
        }
    }
}

fn parse_via(c: &mut Cursor<'_>, diags: &mut Diagnostics) -> PResult<Via> {
    let name = c.name()?.to_string();
    let mut via = Via {
        name: name.clone(),
        default: false,
        generated: false,
        via_rule: None,
        resistance: None,
        layers: Vec::new(),
        properties: Vec::new(),
        extras: Vec::new(),
    };
    loop {
        if c.eat_word("DEFAULT") {
            via.default = true;
        } else if c.eat_word("GENERATED") {
            via.generated = true;
        } else {
            break;
        }
    }
    loop {
        if c.at_end() {
            return Err(c.error(format!("unterminated VIA {name}")));
        }
        if c.is_word("END") {
            expect_end(c, &name, diags)?;
            return Ok(via);
        }
        let Some(tok) = c.peek() else { break };
        let r = match tok.text {
            "VIARULE" => {
                c.next();
                c.name().and_then(|rule| {
                    via.via_rule = Some(ViaGenerate {
                        rule: rule.to_string(),
                        cut_size: None,
                        layers: None,
                        cut_spacing: None,
                        enclosure: None,
                        row_col: None,
                        origin: None,
                        offset: None,
                        pattern: None,
                    });
                    c.expect_semi()
                })
            }
            "CUTSIZE" | "CUTSPACING" | "LAYERS" | "ENCLOSURE" | "ROWCOL" | "ORIGIN" | "OFFSET"
            | "PATTERN" => {
                let key = tok.text;
                c.next();
                match via.via_rule.as_mut() {
                    None => Err(c.error(format!("`{key}` needs a preceding VIARULE"))),
                    Some(g) => (|| {
                        match key {
                            "CUTSIZE" => g.cut_size = Some((c.number()?, c.number()?)),
                            "CUTSPACING" => g.cut_spacing = Some((c.number()?, c.number()?)),
                            "LAYERS" => {
                                g.layers = Some((
                                    c.name()?.to_string(),
                                    c.name()?.to_string(),
                                    c.name()?.to_string(),
                                ));
                            }
                            "ENCLOSURE" => {
                                g.enclosure =
                                    Some((c.number()?, c.number()?, c.number()?, c.number()?));
                            }
                            "ROWCOL" => g.row_col = Some((c.integer()?, c.integer()?)),
                            "ORIGIN" => {
                                g.origin = Some(Point {
                                    x: c.number()?,
                                    y: c.number()?,
                                });
                            }
                            "OFFSET" => {
                                g.offset =
                                    Some((c.number()?, c.number()?, c.number()?, c.number()?));
                            }
                            _ => g.pattern = Some(c.name()?.to_string()),
                        }
                        c.expect_semi()
                    })(),
                }
            }
            "RESISTANCE" => {
                c.next();
                c.number().and_then(|v| {
                    via.resistance = Some(v);
                    c.expect_semi()
                })
            }
            "LAYER" => {
                c.next();
                c.name().and_then(|l| {
                    via.layers.push(ViaLayer {
                        name: l.to_string(),
                        shapes: Vec::new(),
                    });
                    c.expect_semi()
                })
            }
            "RECT" | "POLYGON" => {
                let key = tok.text;
                c.next();
                match via.layers.last_mut() {
                    None => Err(c.error(format!("`{key}` needs a preceding LAYER"))),
                    Some(layer) => parse_shape(c, key).map(|s| layer.shapes.push(s)),
                }
            }
            "PROPERTY" => {
                c.next();
                parse_properties(c, &mut via.properties)
            }
            _ => {
                via.extras.push(Raw::statement(c.raw_statement()));
                Ok(())
            }
        };
        if let Err(e) = r {
            c.recover(e, diags);
        }
    }
    Ok(via)
}

fn parse_via_rule(c: &mut Cursor<'_>, diags: &mut Diagnostics) -> PResult<ViaRule> {
    let name = c.name()?.to_string();
    let mut rule = ViaRule {
        name: name.clone(),
        generate: false,
        default: false,
        layers: Vec::new(),
        vias: Vec::new(),
        properties: Vec::new(),
        extras: Vec::new(),
    };
    loop {
        if c.eat_word("GENERATE") {
            rule.generate = true;
        } else if c.eat_word("DEFAULT") {
            rule.default = true;
        } else {
            break;
        }
    }
    loop {
        if c.at_end() {
            return Err(c.error(format!("unterminated VIARULE {name}")));
        }
        if c.is_word("END") {
            expect_end(c, &name, diags)?;
            return Ok(rule);
        }
        let Some(tok) = c.peek() else { break };
        let r = match tok.text {
            "LAYER" => {
                c.next();
                c.name().and_then(|l| {
                    rule.layers.push(ViaRuleLayer {
                        name: l.to_string(),
                        direction: None,
                        width: None,
                        enclosure: None,
                        rect: None,
                        spacing: None,
                        resistance: None,
                        extras: Vec::new(),
                    });
                    c.expect_semi()
                })
            }
            "VIA" => {
                c.next();
                c.name().and_then(|v| {
                    rule.vias.push(v.to_string());
                    c.expect_semi()
                })
            }
            "PROPERTY" => {
                c.next();
                parse_properties(c, &mut rule.properties)
            }
            "DIRECTION" | "WIDTH" | "ENCLOSURE" | "RECT" | "SPACING" | "RESISTANCE" => {
                let key = tok.text;
                c.next();
                match rule.layers.last_mut() {
                    None => Err(c.error(format!("`{key}` needs a preceding LAYER"))),
                    Some(layer) => (|| {
                        match key {
                            "DIRECTION" => {
                                let d = c.word()?;
                                layer.direction = Direction::parse(d);
                                if layer.direction.is_none() {
                                    return Err(c.error(format!("unknown direction `{d}`")));
                                }
                            }
                            "WIDTH" => {
                                let lo = c.number()?;
                                c.expect_word("TO")?;
                                let hi = c.number()?;
                                layer.width = Some((lo, hi));
                            }
                            "ENCLOSURE" => layer.enclosure = Some((c.number()?, c.number()?)),
                            "RECT" => layer.rect = Some(parse_rect(c)?),
                            "SPACING" => {
                                let x = c.number()?;
                                c.expect_word("BY")?;
                                let y = c.number()?;
                                layer.spacing = Some((x, y));
                            }
                            _ => layer.resistance = Some(c.number()?),
                        }
                        c.expect_semi()
                    })(),
                }
            }
            _ => {
                let raw = Raw::statement(c.raw_statement());
                match rule.layers.last_mut() {
                    Some(layer) => layer.extras.push(raw),
                    None => rule.extras.push(raw),
                }
                Ok(())
            }
        };
        if let Err(e) = r {
            c.recover(e, diags);
        }
    }
    Ok(rule)
}

fn parse_symmetry(c: &mut Cursor<'_>) -> PResult<Vec<Symmetry>> {
    let mut out = Vec::new();
    while !c.is_kind(Kind::Semi) && !c.at_end() {
        let w = c.word()?;
        out.push(match w {
            "X" => Symmetry::X,
            "Y" => Symmetry::Y,
            "R90" => Symmetry::R90,
            _ => return Err(c.error(format!("unknown symmetry `{w}`"))),
        });
    }
    c.expect_semi()?;
    Ok(out)
}

fn parse_size(c: &mut Cursor<'_>) -> PResult<(f64, f64)> {
    let w = c.number()?;
    c.expect_word("BY")?;
    let h = c.number()?;
    c.expect_semi()?;
    Ok((w, h))
}

fn parse_site(c: &mut Cursor<'_>, diags: &mut Diagnostics) -> PResult<Site> {
    let name = c.name()?.to_string();
    let mut site = Site {
        name: name.clone(),
        class: None,
        symmetry: Vec::new(),
        size: None,
        row_pattern: Vec::new(),
        extras: Vec::new(),
    };
    loop {
        if c.at_end() {
            return Err(c.error(format!("unterminated SITE {name}")));
        }
        if c.is_word("END") {
            expect_end(c, &name, diags)?;
            return Ok(site);
        }
        let Some(tok) = c.peek() else { break };
        let r = match tok.text {
            "CLASS" => {
                c.next();
                c.word().and_then(|v| {
                    site.class = Some(v.to_string());
                    c.expect_semi()
                })
            }
            "SYMMETRY" => {
                c.next();
                parse_symmetry(c).map(|s| site.symmetry = s)
            }
            "SIZE" => {
                c.next();
                parse_size(c).map(|s| site.size = Some(s))
            }
            "ROWPATTERN" => {
                c.next();
                (|| {
                    while !c.is_kind(Kind::Semi) && !c.at_end() {
                        let s = c.name()?.to_string();
                        let o = c.word()?;
                        let orient = Orient::from_keyword(o)
                            .ok_or_else(|| c.error(format!("unknown orientation `{o}`")))?;
                        site.row_pattern.push((s, orient));
                    }
                    c.expect_semi()
                })()
            }
            _ => {
                site.extras.push(Raw::statement(c.raw_statement()));
                Ok(())
            }
        };
        if let Err(e) = r {
            c.recover(e, diags);
        }
    }
    Ok(site)
}

/// Parses `LAYER`/`RECT`/`POLYGON`/`PATH`/`VIA`/`WIDTH` statements up to
/// `END` (not consumed).
fn parse_geometry(
    c: &mut Cursor<'_>,
    geometry: &mut Vec<Geometry>,
    extras: &mut Vec<Raw>,
    diags: &mut Diagnostics,
) -> PResult<Option<String>> {
    let mut class = None;
    loop {
        if c.at_end() {
            return Err(c.error("unterminated geometry section"));
        }
        if c.is_word("END") {
            return Ok(class);
        }
        let Some(tok) = c.peek() else { break };
        let r = match tok.text {
            "CLASS" => {
                c.next();
                c.word().and_then(|v| {
                    class = Some(v.to_string());
                    c.expect_semi()
                })
            }
            "LAYER" => {
                c.next();
                (|| {
                    let mut layer = GeometryLayer {
                        name: c.name()?.to_string(),
                        except_pg_net: false,
                        spacing: None,
                        design_rule_width: None,
                        width: None,
                        shapes: Vec::new(),
                    };
                    while !c.is_kind(Kind::Semi) && !c.at_end() {
                        if c.eat_word("EXCEPTPGNET") {
                            layer.except_pg_net = true;
                        } else if c.eat_word("SPACING") {
                            layer.spacing = Some(c.number()?);
                        } else if c.eat_word("DESIGNRULEWIDTH") {
                            layer.design_rule_width = Some(c.number()?);
                        } else {
                            return Err(c.error("unexpected token in LAYER statement"));
                        }
                    }
                    c.expect_semi()?;
                    geometry.push(Geometry::Layer(layer));
                    Ok(())
                })()
            }
            "WIDTH" => {
                c.next();
                match geometry.last_mut() {
                    Some(Geometry::Layer(l)) => c.number().and_then(|w| {
                        l.width = Some(w);
                        c.expect_semi()
                    }),
                    _ => Err(c.error("`WIDTH` needs a preceding LAYER")),
                }
            }
            "RECT" | "POLYGON" | "PATH" => {
                let key = tok.text;
                let span = c.span();
                c.next();
                if c.is_word("ITERATE") || c.peek_at(2).is_some_and(|t| t.text == "ITERATE") {
                    // `ITERATE ... DO ... BY ... STEP ...` is rare; keep it
                    // verbatim rather than modelling step patterns.
                    let mut tokens = vec![key.to_string()];
                    tokens.extend(c.raw_statement());
                    extras.push(Raw::statement(tokens));
                    Ok(())
                } else {
                    match geometry.last_mut() {
                        Some(Geometry::Layer(l)) => parse_shape(c, key).map(|s| l.shapes.push(s)),
                        _ => Err(super::lefdef::ParseError::new(
                            span,
                            format!("`{key}` needs a preceding LAYER"),
                        )),
                    }
                }
            }
            "VIA" => {
                c.next();
                (|| {
                    let mask = parse_mask(c)?;
                    if c.eat_word("ITERATE") {
                        let mut tokens = vec!["VIA".to_string()];
                        tokens.extend(c.raw_statement());
                        extras.push(Raw::statement(tokens));
                        return Ok(());
                    }
                    let point = Point {
                        x: c.number()?,
                        y: c.number()?,
                    };
                    let name = c.name()?.to_string();
                    c.expect_semi()?;
                    geometry.push(Geometry::Via { mask, point, name });
                    Ok(())
                })()
            }
            _ => {
                extras.push(Raw::statement(c.raw_statement()));
                Ok(())
            }
        };
        if let Err(e) = r {
            c.recover(e, diags);
        }
    }
    Ok(class)
}

fn parse_macro_pin(c: &mut Cursor<'_>, diags: &mut Diagnostics) -> PResult<MacroPin> {
    let name = c.name()?.to_string();
    let mut pin = MacroPin::new(&name);
    loop {
        if c.at_end() {
            return Err(c.error(format!("unterminated PIN {name}")));
        }
        if c.is_word("END") {
            expect_end(c, &name, diags)?;
            return Ok(pin);
        }
        let Some(tok) = c.peek() else { break };
        let r = match tok.text {
            "DIRECTION" => {
                c.next();
                c.word().and_then(|d| {
                    pin.direction = Some(match d {
                        "INPUT" => PinDirection::Input,
                        "OUTPUT" => {
                            if c.eat_word("TRISTATE") {
                                PinDirection::OutputTristate
                            } else {
                                PinDirection::Output
                            }
                        }
                        "INOUT" => PinDirection::Inout,
                        "FEEDTHRU" => PinDirection::Feedthru,
                        other => return Err(c.error(format!("unknown pin direction `{other}`"))),
                    });
                    c.expect_semi()
                })
            }
            "USE" => {
                c.next();
                c.word().and_then(|u| {
                    pin.use_ = Some(PinUse::parse(u));
                    c.expect_semi()
                })
            }
            "SHAPE" => {
                c.next();
                c.word().and_then(|s| {
                    pin.shape = Some(match s {
                        "ABUTMENT" => PinShape::Abutment,
                        "RING" => PinShape::Ring,
                        "FEEDTHRU" => PinShape::Feedthru,
                        other => return Err(c.error(format!("unknown pin shape `{other}`"))),
                    });
                    c.expect_semi()
                })
            }
            "TAPERRULE" | "MUSTJOIN" | "NETEXPR" | "SUPPLYSENSITIVITY" | "GROUNDSENSITIVITY" => {
                let key = tok.text;
                c.next();
                c.name().and_then(|v| {
                    let v = v.to_string();
                    match key {
                        "TAPERRULE" => pin.taper_rule = Some(v),
                        "MUSTJOIN" => pin.must_join = Some(v),
                        "NETEXPR" => pin.net_expr = Some(v),
                        "SUPPLYSENSITIVITY" => pin.supply_sensitivity = Some(v),
                        _ => pin.ground_sensitivity = Some(v),
                    }
                    c.expect_semi()
                })
            }
            "PORT" => {
                c.next();
                let mut port = Port::default();
                parse_geometry(c, &mut port.geometry, &mut port.extras, diags).and_then(|class| {
                    port.class = class;
                    c.expect_word("END")?;
                    pin.ports.push(port);
                    Ok(())
                })
            }
            "PROPERTY" => {
                c.next();
                parse_properties(c, &mut pin.properties)
            }
            _ => {
                pin.extras.push(Raw::statement(c.raw_statement()));
                Ok(())
            }
        };
        if let Err(e) = r {
            c.recover(e, diags);
        }
    }
    Ok(pin)
}

fn parse_macro(c: &mut Cursor<'_>, diags: &mut Diagnostics) -> PResult<Macro> {
    let name = c.name()?.to_string();
    let mut m = Macro::new(&name);
    loop {
        if c.at_end() {
            return Err(c.error(format!("unterminated MACRO {name}")));
        }
        if c.is_word("END") {
            expect_end(c, &name, diags)?;
            return Ok(m);
        }
        let Some(tok) = c.peek() else { break };
        let r = match tok.text {
            "CLASS" => {
                c.next();
                m.class = c.raw_until(&[]);
                c.expect_semi()
            }
            "FIXEDMASK" => {
                c.next();
                m.fixed_mask = true;
                c.expect_semi()
            }
            "ORIGIN" => {
                c.next();
                (|| {
                    m.origin = Some(Point {
                        x: c.number()?,
                        y: c.number()?,
                    });
                    c.expect_semi()
                })()
            }
            "FOREIGN" => {
                c.next();
                (|| {
                    let mut f = Foreign {
                        name: c.name()?.to_string(),
                        point: None,
                        orient: None,
                    };
                    if !c.is_kind(Kind::Semi) {
                        f.point = Some(Point {
                            x: c.number()?,
                            y: c.number()?,
                        });
                        if !c.is_kind(Kind::Semi) {
                            let o = c.word()?;
                            f.orient =
                                Some(Orient::from_keyword(o).ok_or_else(|| {
                                    c.error(format!("unknown orientation `{o}`"))
                                })?);
                        }
                    }
                    m.foreign.push(f);
                    c.expect_semi()
                })()
            }
            "SIZE" => {
                c.next();
                parse_size(c).map(|s| m.size = Some(s))
            }
            "SYMMETRY" => {
                c.next();
                parse_symmetry(c).map(|s| m.symmetry = s)
            }
            "SITE" => {
                c.next();
                c.name().and_then(|s| {
                    m.site = Some(s.to_string());
                    // `SITE name x y orient DO ... BY ... STEP ...` exists
                    // for pads; keep only the name.
                    c.raw_until(&[]);
                    c.expect_semi()
                })
            }
            "EEQ" => {
                c.next();
                c.name().and_then(|s| {
                    m.eeq = Some(s.to_string());
                    c.expect_semi()
                })
            }
            "PIN" => {
                c.next();
                parse_macro_pin(c, diags).map(|p| m.pins.push(p))
            }
            "OBS" => {
                c.next();
                let mut extras = Vec::new();
                let r = parse_geometry(c, &mut m.obs, &mut extras, diags)
                    .and_then(|_| c.expect_word("END"));
                m.extras.extend(extras);
                r
            }
            "PROPERTY" => {
                c.next();
                parse_properties(c, &mut m.properties)
            }
            _ => {
                m.extras.push(Raw::statement(c.raw_statement()));
                Ok(())
            }
        };
        if let Err(e) = r {
            c.recover(e, diags);
        }
    }
    Ok(m)
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

struct W {
    out: String,
    depth: usize,
}

impl W {
    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn stmt(&mut self, s: &str) {
        self.line(&format!("{s} ;"));
    }

    fn raw(&mut self, raw: &Raw) {
        if raw.block {
            // Re-flow a preserved section: newline after each `;`.
            let mut line = String::new();
            for t in &raw.tokens {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(t);
                if t == ";" {
                    self.line(&line);
                    line.clear();
                }
            }
            if !line.is_empty() {
                self.line(&line);
            }
        } else {
            self.stmt(&raw.tokens.join(" "));
        }
    }

    fn properties(&mut self, props: &[Property]) {
        if props.is_empty() {
            return;
        }
        let mut s = String::from("PROPERTY");
        for p in props {
            s.push(' ');
            s.push_str(&p.name);
            s.push(' ');
            s.push_str(&p.value.to_string());
        }
        self.stmt(&s);
    }
}

fn pair(a: f64, b: f64) -> String {
    format!("{} {}", fmt_num(a), fmt_num(b))
}

fn rect(r: &Rect) -> String {
    format!(
        "{} {} {} {}",
        fmt_num(r.x1),
        fmt_num(r.y1),
        fmt_num(r.x2),
        fmt_num(r.y2)
    )
}

fn points(pts: &[Point]) -> String {
    pts.iter()
        .map(|p| pair(p.x, p.y))
        .collect::<Vec<_>>()
        .join(" ")
}

fn mask(m: Option<u32>) -> String {
    m.map(|m| format!("MASK {m} ")).unwrap_or_default()
}

fn write_shape(w: &mut W, s: &Shape) {
    match s {
        Shape::Rect { mask: m, rect: r } => w.stmt(&format!("RECT {}{}", mask(*m), rect(r))),
        Shape::Polygon { mask: m, points: p } => {
            w.stmt(&format!("POLYGON {}{}", mask(*m), points(p)));
        }
        Shape::Path { mask: m, points: p } => w.stmt(&format!("PATH {}{}", mask(*m), points(p))),
    }
}

fn write_geometry(w: &mut W, geometry: &[Geometry]) {
    for g in geometry {
        match g {
            Geometry::Layer(l) => {
                let mut s = format!("LAYER {}", l.name);
                if l.except_pg_net {
                    s.push_str(" EXCEPTPGNET");
                }
                if let Some(v) = l.spacing {
                    s.push_str(&format!(" SPACING {}", fmt_num(v)));
                }
                if let Some(v) = l.design_rule_width {
                    s.push_str(&format!(" DESIGNRULEWIDTH {}", fmt_num(v)));
                }
                w.stmt(&s);
                w.depth += 1;
                if let Some(v) = l.width {
                    w.stmt(&format!("WIDTH {}", fmt_num(v)));
                }
                for s in &l.shapes {
                    write_shape(w, s);
                }
                w.depth -= 1;
            }
            Geometry::Via {
                mask: m,
                point,
                name,
            } => w.stmt(&format!(
                "VIA {}{} {name}",
                mask(*m),
                pair(point.x, point.y)
            )),
        }
    }
}

fn write_layer(w: &mut W, l: &Layer) {
    w.line(&format!("LAYER {}", l.name));
    w.depth += 1;
    if let Some(k) = &l.kind {
        w.stmt(&format!("TYPE {}", k.as_str()));
    }
    if let Some(d) = l.direction {
        w.stmt(&format!("DIRECTION {}", d.as_str()));
    }
    if let Some((x, y)) = l.pitch {
        if x == y {
            w.stmt(&format!("PITCH {}", fmt_num(x)));
        } else {
            w.stmt(&format!("PITCH {}", pair(x, y)));
        }
    }
    if let Some((x, y)) = l.offset {
        if x == y {
            w.stmt(&format!("OFFSET {}", fmt_num(x)));
        } else {
            w.stmt(&format!("OFFSET {}", pair(x, y)));
        }
    }
    for (key, v) in [
        ("WIDTH", l.width),
        ("MINWIDTH", l.min_width),
        ("MAXWIDTH", l.max_width),
        ("AREA", l.area),
        ("THICKNESS", l.thickness),
        ("HEIGHT", l.height),
    ] {
        if let Some(v) = v {
            w.stmt(&format!("{key} {}", fmt_num(v)));
        }
    }
    for s in &l.spacing {
        let mut line = format!("SPACING {}", fmt_num(s.value));
        for q in &s.qualifiers {
            line.push(' ');
            line.push_str(q);
        }
        w.stmt(&line);
    }
    if let Some(v) = l.resistance_per_square {
        w.stmt(&format!("RESISTANCE RPERSQ {}", fmt_num(v)));
    }
    if let Some(v) = l.resistance {
        w.stmt(&format!("RESISTANCE {}", fmt_num(v)));
    }
    if let Some(v) = l.capacitance_per_square {
        w.stmt(&format!("CAPACITANCE CPERSQDIST {}", fmt_num(v)));
    }
    if let Some(v) = l.edge_capacitance {
        w.stmt(&format!("EDGECAPACITANCE {}", fmt_num(v)));
    }
    for r in &l.extras {
        w.raw(r);
    }
    w.properties(&l.properties);
    w.depth -= 1;
    w.line(&format!("END {}", l.name));
}

fn write_via(w: &mut W, v: &Via) {
    let mut head = format!("VIA {}", v.name);
    if v.default {
        head.push_str(" DEFAULT");
    }
    if v.generated {
        head.push_str(" GENERATED");
    }
    w.line(&head);
    w.depth += 1;
    if let Some(g) = &v.via_rule {
        w.stmt(&format!("VIARULE {}", g.rule));
        if let Some((x, y)) = g.cut_size {
            w.stmt(&format!("CUTSIZE {}", pair(x, y)));
        }
        if let Some((a, b, c)) = &g.layers {
            w.stmt(&format!("LAYERS {a} {b} {c}"));
        }
        if let Some((x, y)) = g.cut_spacing {
            w.stmt(&format!("CUTSPACING {}", pair(x, y)));
        }
        if let Some((a, b, c, d)) = g.enclosure {
            w.stmt(&format!("ENCLOSURE {} {}", pair(a, b), pair(c, d)));
        }
        if let Some((r, c)) = g.row_col {
            w.stmt(&format!("ROWCOL {r} {c}"));
        }
        if let Some(p) = g.origin {
            w.stmt(&format!("ORIGIN {}", pair(p.x, p.y)));
        }
        if let Some((a, b, c, d)) = g.offset {
            w.stmt(&format!("OFFSET {} {}", pair(a, b), pair(c, d)));
        }
        if let Some(p) = &g.pattern {
            w.stmt(&format!("PATTERN {p}"));
        }
    }
    if let Some(r) = v.resistance {
        w.stmt(&format!("RESISTANCE {}", fmt_num(r)));
    }
    for l in &v.layers {
        w.stmt(&format!("LAYER {}", l.name));
        w.depth += 1;
        for s in &l.shapes {
            write_shape(w, s);
        }
        w.depth -= 1;
    }
    for r in &v.extras {
        w.raw(r);
    }
    w.properties(&v.properties);
    w.depth -= 1;
    w.line(&format!("END {}", v.name));
}

fn write_via_rule(w: &mut W, r: &ViaRule) {
    let mut head = format!("VIARULE {}", r.name);
    if r.generate {
        head.push_str(" GENERATE");
    }
    if r.default {
        head.push_str(" DEFAULT");
    }
    w.line(&head);
    w.depth += 1;
    for l in &r.layers {
        w.stmt(&format!("LAYER {}", l.name));
        w.depth += 1;
        if let Some(d) = l.direction {
            w.stmt(&format!("DIRECTION {}", d.as_str()));
        }
        if let Some((lo, hi)) = l.width {
            w.stmt(&format!("WIDTH {} TO {}", fmt_num(lo), fmt_num(hi)));
        }
        if let Some((x, y)) = l.enclosure {
            w.stmt(&format!("ENCLOSURE {}", pair(x, y)));
        }
        if let Some(rc) = &l.rect {
            w.stmt(&format!("RECT {}", rect(rc)));
        }
        if let Some((x, y)) = l.spacing {
            w.stmt(&format!("SPACING {} BY {}", fmt_num(x), fmt_num(y)));
        }
        if let Some(v) = l.resistance {
            w.stmt(&format!("RESISTANCE {}", fmt_num(v)));
        }
        for e in &l.extras {
            w.raw(e);
        }
        w.depth -= 1;
    }
    for v in &r.vias {
        w.stmt(&format!("VIA {v}"));
    }
    for e in &r.extras {
        w.raw(e);
    }
    w.properties(&r.properties);
    w.depth -= 1;
    w.line(&format!("END {}", r.name));
}

fn symmetry(s: &[Symmetry]) -> String {
    s.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")
}

fn write_site(w: &mut W, s: &Site) {
    w.line(&format!("SITE {}", s.name));
    w.depth += 1;
    if let Some(c) = &s.class {
        w.stmt(&format!("CLASS {c}"));
    }
    if !s.symmetry.is_empty() {
        w.stmt(&format!("SYMMETRY {}", symmetry(&s.symmetry)));
    }
    if !s.row_pattern.is_empty() {
        let mut line = String::from("ROWPATTERN");
        for (site, o) in &s.row_pattern {
            line.push_str(&format!(" {site} {o}"));
        }
        w.stmt(&line);
    }
    if let Some((x, y)) = s.size {
        w.stmt(&format!("SIZE {} BY {}", fmt_num(x), fmt_num(y)));
    }
    for e in &s.extras {
        w.raw(e);
    }
    w.depth -= 1;
    w.line(&format!("END {}", s.name));
}

fn write_macro(w: &mut W, m: &Macro) {
    w.line(&format!("MACRO {}", m.name));
    w.depth += 1;
    if !m.class.is_empty() {
        w.stmt(&format!("CLASS {}", m.class.join(" ")));
    }
    if m.fixed_mask {
        w.stmt("FIXEDMASK");
    }
    for f in &m.foreign {
        let mut s = format!("FOREIGN {}", f.name);
        if let Some(p) = f.point {
            s.push_str(&format!(" {}", pair(p.x, p.y)));
            if let Some(o) = f.orient {
                s.push_str(&format!(" {o}"));
            }
        }
        w.stmt(&s);
    }
    if let Some(p) = m.origin {
        w.stmt(&format!("ORIGIN {}", pair(p.x, p.y)));
    }
    if let Some(e) = &m.eeq {
        w.stmt(&format!("EEQ {e}"));
    }
    if let Some((x, y)) = m.size {
        w.stmt(&format!("SIZE {} BY {}", fmt_num(x), fmt_num(y)));
    }
    if !m.symmetry.is_empty() {
        w.stmt(&format!("SYMMETRY {}", symmetry(&m.symmetry)));
    }
    if let Some(s) = &m.site {
        w.stmt(&format!("SITE {s}"));
    }
    for p in &m.pins {
        w.line(&format!("PIN {}", p.name));
        w.depth += 1;
        if let Some(d) = p.direction {
            w.stmt(&format!("DIRECTION {}", d.as_str()));
        }
        if let Some(u) = &p.use_ {
            w.stmt(&format!("USE {}", u.as_str()));
        }
        if let Some(s) = p.shape {
            w.stmt(&format!("SHAPE {}", s.as_str()));
        }
        for (key, v) in [
            ("TAPERRULE", &p.taper_rule),
            ("MUSTJOIN", &p.must_join),
            ("NETEXPR", &p.net_expr),
            ("SUPPLYSENSITIVITY", &p.supply_sensitivity),
            ("GROUNDSENSITIVITY", &p.ground_sensitivity),
        ] {
            if let Some(v) = v {
                w.stmt(&format!("{key} {}", quote_if_needed(v)));
            }
        }
        for e in &p.extras {
            w.raw(e);
        }
        w.properties(&p.properties);
        for port in &p.ports {
            w.line("PORT");
            w.depth += 1;
            if let Some(c) = &port.class {
                w.stmt(&format!("CLASS {c}"));
            }
            write_geometry(w, &port.geometry);
            for e in &port.extras {
                w.raw(e);
            }
            w.depth -= 1;
            w.line("END");
        }
        w.depth -= 1;
        w.line(&format!("END {}", p.name));
    }
    if !m.obs.is_empty() {
        w.line("OBS");
        w.depth += 1;
        write_geometry(w, &m.obs);
        w.depth -= 1;
        w.line("END");
    }
    for e in &m.extras {
        w.raw(e);
    }
    w.properties(&m.properties);
    w.depth -= 1;
    w.line(&format!("END {}", m.name));
}

/// Renders a LEF file in canonical form.
pub fn write_lef(lef: &Lef) -> String {
    let mut w = W {
        out: String::new(),
        depth: 0,
    };
    if let Some(v) = &lef.version {
        w.stmt(&format!("VERSION {v}"));
    }
    if let Some(v) = &lef.bus_bit_chars {
        w.stmt(&format!("BUSBITCHARS \"{v}\""));
    }
    if let Some(v) = &lef.divider_char {
        w.stmt(&format!("DIVIDERCHAR \"{v}\""));
    }
    if !lef.units.is_empty() {
        w.line("UNITS");
        w.depth += 1;
        for u in &lef.units {
            w.stmt(&format!("{} {} {}", u.kind, u.unit, fmt_num(u.value)));
        }
        w.depth -= 1;
        w.line("END UNITS");
    }
    if let Some(v) = lef.manufacturing_grid {
        w.stmt(&format!("MANUFACTURINGGRID {}", fmt_num(v)));
    }
    if let Some(v) = &lef.clearance_measure {
        w.stmt(&format!("CLEARANCEMEASURE {v}"));
    }
    for (what, on) in &lef.use_min_spacing {
        w.stmt(&format!(
            "USEMINSPACING {what} {}",
            if *on { "ON" } else { "OFF" }
        ));
    }
    if !lef.property_definitions.is_empty() {
        w.line("PROPERTYDEFINITIONS");
        w.depth += 1;
        for d in &lef.property_definitions {
            let mut s = format!("{} {} {}", d.object, d.name, d.ty);
            if let Some((lo, hi)) = d.range {
                s.push_str(&format!(" RANGE {} {}", fmt_num(lo), fmt_num(hi)));
            }
            if let Some(v) = &d.value {
                s.push_str(&format!(" {v}"));
            }
            w.stmt(&s);
        }
        w.depth -= 1;
        w.line("END PROPERTYDEFINITIONS");
    }
    for l in &lef.layers {
        write_layer(&mut w, l);
    }
    for v in &lef.vias {
        write_via(&mut w, v);
    }
    for r in &lef.via_rules {
        write_via_rule(&mut w, r);
    }
    for s in &lef.sites {
        write_site(&mut w, s);
    }
    for m in &lef.macros {
        write_macro(&mut w, m);
    }
    for e in &lef.extras {
        w.raw(e);
    }
    w.line("END LIBRARY");
    w.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    const LEF: &str = r#"
VERSION 5.8 ;
BUSBITCHARS "[]" ;
DIVIDERCHAR "/" ;
UNITS
  DATABASE MICRONS 1000 ;
END UNITS
MANUFACTURINGGRID 0.005 ;
PROPERTYDEFINITIONS
  MACRO cellType STRING ;
  LAYER minArea REAL RANGE 0 100 0.5 ;
END PROPERTYDEFINITIONS
LAYER met1
  TYPE ROUTING ;
  DIRECTION HORIZONTAL ;
  PITCH 0.34 ;
  OFFSET 0.17 ;
  WIDTH 0.14 ;
  SPACING 0.14 ;
  SPACING 0.28 RANGE 3.0 10.0 ;
  RESISTANCE RPERSQ 0.125 ;
  CAPACITANCE CPERSQDIST 2.5e-05 ;
  SPACINGTABLE
    PARALLELRUNLENGTH 0 3.0
    WIDTH 0 0.14 0.14
    WIDTH 3.0 0.28 0.28 ;
  ANTENNADIFFSIDEAREARATIO PWL ( ( 0 75 ) ( 0.0125 75 ) ) ;
  PROPERTY minArea 0.083 ;
END met1
LAYER via
  TYPE CUT ;
  WIDTH 0.15 ;
  SPACING 0.17 ;
  RESISTANCE 9.0 ;
END via
VIA via1_2 DEFAULT
  LAYER met1 ;
    RECT -0.145 -0.1 0.145 0.1 ;
  LAYER via ;
    RECT -0.075 -0.075 0.075 0.075 ;
  LAYER met2 ;
    RECT -0.1 -0.145 0.1 0.145 ;
END via1_2
VIA gen_via
  VIARULE M1M2_PR ;
  CUTSIZE 0.15 0.15 ;
  LAYERS met1 via met2 ;
  CUTSPACING 0.17 0.17 ;
  ENCLOSURE 0.055 0.03 0.055 0.03 ;
  ROWCOL 1 2 ;
END gen_via
VIARULE M1M2_PR GENERATE DEFAULT
  LAYER met1 ;
    ENCLOSURE 0.055 0.03 ;
  LAYER met2 ;
    ENCLOSURE 0.055 0.03 ;
  LAYER via ;
    RECT -0.075 -0.075 0.075 0.075 ;
    SPACING 0.32 BY 0.32 ;
END M1M2_PR
SITE unithd
  CLASS CORE ;
  SYMMETRY Y ;
  SIZE 0.46 BY 2.72 ;
END unithd
MACRO inv_1
  CLASS CORE ;
  FOREIGN inv_1 0 0 ;
  ORIGIN 0.000 0.000 ;
  SIZE 1.38 BY 2.72 ;
  SYMMETRY X Y R90 ;
  SITE unithd ;
  PIN A
    DIRECTION INPUT ;
    USE SIGNAL ;
    ANTENNAGATEAREA 0.1575 ;
    PORT
      LAYER li1 ;
        RECT 0.415 0.985 0.585 1.835 ;
    END
  END A
  PIN Y
    DIRECTION OUTPUT TRISTATE ;
    USE SIGNAL ;
    PORT
      LAYER li1 ;
        RECT 0.845 0.255 1.015 2.465 ;
        POLYGON 0 0 1 0 1 1 ;
      LAYER met1 ;
        WIDTH 0.14 ;
        PATH 0.9 0.5 0.9 2.0 ;
      VIA 0.9 1.0 via1_2 ;
    END
  END Y
  PIN VPWR
    DIRECTION INOUT ;
    USE POWER ;
    SHAPE ABUTMENT ;
    PORT
      CLASS CORE ;
      LAYER met1 ;
        RECT -0.19 2.48 1.57 2.96 ;
    END
  END VPWR
  OBS
    LAYER li1 ;
      RECT 0.155 0.085 0.265 1.735 ;
  END
  PROPERTY cellType "logic gate" ;
END inv_1
END LIBRARY
"#;

    fn parse(text: &str) -> (Lef, Diagnostics, SourceMap) {
        let mut map = SourceMap::new();
        let id = map.add("t.lef", text).unwrap();
        let mut diags = Diagnostics::new();
        let lef = parse_lef(text, id, &mut diags);
        (lef, diags, map)
    }

    #[test]
    fn reads_the_typed_view() {
        let (lef, diags, map) = parse(LEF);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        assert_eq!(lef.version.as_deref(), Some("5.8"));
        assert_eq!(lef.bus_bit_chars.as_deref(), Some("[]"));
        assert_eq!(lef.database_microns(), Some(1000.0));
        assert_eq!(lef.manufacturing_grid, Some(0.005));
        assert_eq!(lef.property_definitions.len(), 2);
        assert_eq!(lef.property_definitions[1].range, Some((0.0, 100.0)));
        assert_eq!(
            lef.property_definitions[1].value,
            Some(PropertyValue::Number(0.5))
        );

        let m1 = lef.layer("met1").unwrap();
        assert!(m1.is_routing());
        assert_eq!(m1.direction, Some(Direction::Horizontal));
        assert_eq!(m1.pitch, Some((0.34, 0.34)));
        assert_eq!(m1.width, Some(0.14));
        assert_eq!(m1.spacing.len(), 2);
        assert_eq!(m1.spacing[1].qualifiers, vec!["RANGE", "3.0", "10.0"]);
        assert_eq!(m1.resistance_per_square, Some(0.125));
        assert_eq!(m1.capacitance_per_square, Some(2.5e-5));
        assert_eq!(m1.extras.len(), 2, "SPACINGTABLE and antenna kept");
        assert_eq!(m1.extras[0].tokens[0], "SPACINGTABLE");
        assert_eq!(m1.properties[0].name, "minArea");
        assert_eq!(lef.layer("via").unwrap().kind, Some(LayerKind::Cut));
        assert_eq!(lef.routing_layers().count(), 1);

        let via = lef.via("via1_2").unwrap();
        assert!(via.default);
        assert_eq!(via.layers.len(), 3);
        let generated = lef.via("gen_via").unwrap();
        let g = generated.via_rule.as_ref().unwrap();
        assert_eq!(g.rule, "M1M2_PR");
        assert_eq!(g.row_col, Some((1, 2)));
        assert_eq!(g.layers.as_ref().unwrap().1, "via");

        let rule = &lef.via_rules[0];
        assert!(rule.generate && rule.default);
        assert_eq!(rule.layers[2].spacing, Some((0.32, 0.32)));
        assert_eq!(rule.layers[0].enclosure, Some((0.055, 0.03)));

        let site = lef.site("unithd").unwrap();
        assert_eq!(site.size, Some((0.46, 2.72)));
        assert_eq!(site.symmetry, vec![Symmetry::Y]);

        let inv = lef.macro_("inv_1").unwrap();
        assert_eq!(inv.class, vec!["CORE"]);
        assert_eq!(inv.size, Some((1.38, 2.72)));
        assert_eq!(inv.site.as_deref(), Some("unithd"));
        assert_eq!(inv.symmetry, vec![Symmetry::X, Symmetry::Y, Symmetry::R90]);
        assert_eq!(inv.pins.len(), 3);
        assert_eq!(inv.signal_pins().count(), 2);
        let a = inv.pin("A").unwrap();
        assert_eq!(a.direction, Some(PinDirection::Input));
        assert_eq!(a.extras[0].tokens, vec!["ANTENNAGATEAREA", "0.1575"]);
        let y = inv.pin("Y").unwrap();
        assert_eq!(y.direction, Some(PinDirection::OutputTristate));
        assert_eq!(y.ports[0].geometry.len(), 3);
        match &y.ports[0].geometry[1] {
            Geometry::Layer(l) => {
                assert_eq!(l.width, Some(0.14));
                assert!(matches!(l.shapes[0], Shape::Path { .. }));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&y.ports[0].geometry[2], Geometry::Via { name, .. } if name == "via1_2"));
        let vpwr = inv.pin("VPWR").unwrap();
        assert!(vpwr.is_supply());
        assert_eq!(vpwr.shape, Some(PinShape::Abutment));
        assert_eq!(vpwr.ports[0].class.as_deref(), Some("CORE"));
        assert_eq!(inv.obs.len(), 1);
        assert_eq!(
            inv.properties[0].value,
            PropertyValue::String("logic gate".into())
        );
    }

    #[test]
    fn write_then_parse_is_a_fixed_point() {
        let (lef, _, _) = parse(LEF);
        let text = write_lef(&lef);
        let (again, diags, map) = parse(&text);
        assert!(diags.is_empty(), "{}\n{text}", diags.render(&map));
        assert_eq!(again, lef);
        assert_eq!(write_lef(&again), text);
        assert!(text.contains("  SPACING 0.28 RANGE 3.0 10.0 ;"));
        assert!(text.contains("PROPERTY cellType \"logic gate\" ;"));
        assert!(text.ends_with("END LIBRARY\n"));
    }

    #[test]
    fn errors_are_reported_and_skipped() {
        let text = "VERSION 5.8 ;\nLAYER m1\n  TYPE ROUTING ;\n  PITCH abc ;\n  WIDTH 0.1 ;\nEND m1\nMACRO x\n  SIZE 1 BY ;\nEND x\nEND LIBRARY\n";
        let (lef, diags, map) = parse(text);
        assert_eq!(diags.error_count(), 2, "{}", diags.render(&map));
        let rendered = diags.render(&map);
        assert!(
            rendered.contains("expected a number, found `abc`"),
            "{rendered}"
        );
        assert!(rendered.contains("t.lef:4:9"), "{rendered}");
        assert_eq!(
            lef.layers[0].width,
            Some(0.1),
            "parsing continues after the error"
        );
        assert_eq!(lef.macros.len(), 1);
    }

    #[test]
    fn unknown_sections_survive_round_trips() {
        let text = "VERSION 5.8 ;\nNONDEFAULTRULE fat\n  LAYER met1\n    WIDTH 0.5 ;\n  END met1\nEND fat\nSPACING\n  SAMENET met1 met1 0.3 ;\nEND SPACING\nNOWIREEXTENSIONATPIN ON ;\nEND LIBRARY\n";
        let (lef, diags, map) = parse(text);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        assert_eq!(lef.extras.len(), 3);
        assert!(lef.extras[0].block);
        assert_eq!(lef.extras[0].tokens.last().unwrap(), "fat");
        assert!(!lef.extras[2].block);
        let out = write_lef(&lef);
        let (again, _, _) = parse(&out);
        assert_eq!(again, lef);
        assert!(out.contains("NOWIREEXTENSIONATPIN ON ;"));
    }
}
