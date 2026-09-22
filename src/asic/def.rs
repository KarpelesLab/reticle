//! DEF 5.8 (Design Exchange Format) reader and writer.
//!
//! A DEF file describes one design's physical state: the die area, rows
//! and routing tracks of the floorplan, the components (instances of LEF
//! macros) with their placement, the I/O pins, and the nets with their
//! connections and, after routing, their wires. It is the file handed to
//! and returned by a place-and-route tool such as OpenROAD.
//!
//! [`parse_def`] reads text into a [`Def`] and [`write_def`] renders one
//! back in a canonical layout that round-trips. [`from_netlist`] builds an
//! unplaced DEF from a Reticle netlist: every cell carrying a `lib_cell`
//! attribute or of kind [`CellKind::Blackbox`], and every instance of a
//! black-box module, becomes a `COMPONENT`; every bit of every connected
//! net becomes a `NET`; every port bit becomes a `PIN`.
//!
//! Coordinates are database units (`i64`, see [`Def::units`]). Sections
//! the typed reader does not model (`SLOTS`, `FILLS`, `SCANCHAINS`,
//! `NONDEFAULTRULES`, ...) are preserved verbatim as [`Raw`] blocks, as
//! are unknown `+ OPTION` clauses on the objects that have them, so a
//! file written by another tool survives a round-trip.

use std::collections::BTreeMap;

use super::lefdef::{Cursor, Kind, PResult};
use super::{Orient, Property, PropertyDefinition, PropertyValue, Raw, fmt_num, quote_if_needed};
use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{CellKind, Design, ExprId, ExprKind, Module, ModuleId, ModuleRef, NetId, PortDir};
use crate::source::SourceId;

/// A point in database units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Point {
    /// X coordinate.
    pub x: i64,
    /// Y coordinate.
    pub y: i64,
}

/// An axis-aligned rectangle in database units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    /// Left.
    pub x1: i64,
    /// Bottom.
    pub y1: i64,
    /// Right.
    pub x2: i64,
    /// Top.
    pub y2: i64,
}

/// A `ROW` statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The row name.
    pub name: String,
    /// The site name.
    pub site: String,
    /// The origin.
    pub origin: Point,
    /// The site orientation.
    pub orient: Orient,
    /// `DO num_x BY num_y STEP step_x step_y`.
    pub repeat: Option<RowRepeat>,
    /// `+ PROPERTY` values.
    pub properties: Vec<Property>,
}

/// The `DO ... BY ... STEP ...` part of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowRepeat {
    /// Sites along X.
    pub num_x: i64,
    /// Sites along Y (1 for a normal row).
    pub num_y: i64,
    /// X step.
    pub step_x: i64,
    /// Y step.
    pub step_y: i64,
}

/// A track or grid axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// `X`: lines at constant X (vertical tracks).
    X,
    /// `Y`: lines at constant Y (horizontal tracks).
    Y,
}

impl Axis {
    /// The keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
        }
    }

    fn parse(s: &str) -> Option<Axis> {
        match s {
            "X" => Some(Axis::X),
            "Y" => Some(Axis::Y),
            _ => None,
        }
    }
}

/// A `TRACKS` statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// `X` or `Y`.
    pub axis: Axis,
    /// First track coordinate.
    pub start: i64,
    /// `DO`: number of tracks.
    pub num: i64,
    /// `STEP`: pitch.
    pub step: i64,
    /// `MASK`.
    pub mask: Option<u32>,
    /// `SAMEMASK`.
    pub same_mask: bool,
    /// `LAYER` names.
    pub layers: Vec<String>,
}

/// A `GCELLGRID` statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GcellGrid {
    /// `X` or `Y`.
    pub axis: Axis,
    /// First line.
    pub start: i64,
    /// `DO`: number of lines.
    pub num: i64,
    /// `STEP`.
    pub step: i64,
}

/// A shape on a layer inside a `VIAS` entry or a pin.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerShape {
    /// The layer name.
    pub layer: String,
    /// `+ MASK`.
    pub mask: Option<u32>,
    /// `+ SPACING` (pins only).
    pub spacing: Option<i64>,
    /// `+ DESIGNRULEWIDTH` (pins only).
    pub design_rule_width: Option<i64>,
    /// The geometry.
    pub shape: ShapeKind,
}

/// Rectangle or polygon.
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeKind {
    /// `RECT`.
    Rect(Rect),
    /// `POLYGON`.
    Polygon(Vec<Point>),
}

/// The `VIARULE`-generated form of a DEF via.
#[derive(Clone, Debug, PartialEq)]
pub struct ViaGenerate {
    /// `+ VIARULE`.
    pub rule: String,
    /// `+ CUTSIZE x y`.
    pub cut_size: Option<(i64, i64)>,
    /// `+ LAYERS bottom cut top`.
    pub layers: Option<(String, String, String)>,
    /// `+ CUTSPACING x y`.
    pub cut_spacing: Option<(i64, i64)>,
    /// `+ ENCLOSURE xb yb xt yt`.
    pub enclosure: Option<(i64, i64, i64, i64)>,
    /// `+ ROWCOL rows cols`.
    pub row_col: Option<(i64, i64)>,
    /// `+ ORIGIN x y`.
    pub origin: Option<Point>,
    /// `+ OFFSET xb yb xt yt`.
    pub offset: Option<(i64, i64, i64, i64)>,
    /// `+ PATTERN`.
    pub pattern: Option<String>,
}

/// A `VIAS` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Via {
    /// The via name.
    pub name: String,
    /// The generate-rule form.
    pub via_rule: Option<ViaGenerate>,
    /// The fixed form: shapes per layer.
    pub shapes: Vec<LayerShape>,
    /// Unknown `+` clauses.
    pub extras: Vec<Raw>,
}

/// Placement status of a component or pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlacementStatus {
    /// `UNPLACED` (or no placement clause).
    #[default]
    Unplaced,
    /// `PLACED`: may be moved by the placer.
    Placed,
    /// `FIXED`: must not be moved.
    Fixed,
    /// `COVER`: fixed and part of the floorplan (a hard macro cover).
    Cover,
}

impl PlacementStatus {
    /// The keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            PlacementStatus::Unplaced => "UNPLACED",
            PlacementStatus::Placed => "PLACED",
            PlacementStatus::Fixed => "FIXED",
            PlacementStatus::Cover => "COVER",
        }
    }
}

/// The placement of a component or pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Placement {
    /// The status.
    pub status: PlacementStatus,
    /// Location and orientation; `None` when unplaced.
    pub location: Option<(Point, Orient)>,
}

impl Placement {
    /// A placed location.
    pub fn placed(x: i64, y: i64, orient: Orient) -> Placement {
        Placement {
            status: PlacementStatus::Placed,
            location: Some((Point { x, y }, orient)),
        }
    }

    /// A fixed location.
    pub fn fixed(x: i64, y: i64, orient: Orient) -> Placement {
        Placement {
            status: PlacementStatus::Fixed,
            location: Some((Point { x, y }, orient)),
        }
    }
}

/// A `COMPONENTS` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    /// The instance name.
    pub name: String,
    /// The LEF macro name.
    pub master: String,
    /// `+ EEQ`.
    pub eeq: Option<String>,
    /// `+ SOURCE` (`NETLIST`, `DIST`, `USER`, `TIMING`).
    pub source: Option<String>,
    /// The placement.
    pub placement: Placement,
    /// `+ MASKSHIFT`.
    pub mask_shift: Option<String>,
    /// `+ HALO [SOFT] left bottom right top`.
    pub halo: Option<Halo>,
    /// `+ WEIGHT`.
    pub weight: Option<i64>,
    /// `+ REGION`.
    pub region: Option<String>,
    /// `+ PROPERTY` values.
    pub properties: Vec<Property>,
    /// Unknown `+` clauses (`ROUTEHALO`, ...).
    pub extras: Vec<Raw>,
}

impl Component {
    /// An unplaced component.
    pub fn new(name: impl Into<String>, master: impl Into<String>) -> Component {
        Component {
            name: name.into(),
            master: master.into(),
            eeq: None,
            source: None,
            placement: Placement::default(),
            mask_shift: None,
            halo: None,
            weight: None,
            region: None,
            properties: Vec::new(),
            extras: Vec::new(),
        }
    }
}

/// A placement halo around a component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Halo {
    /// `SOFT`.
    pub soft: bool,
    /// Left extension.
    pub left: i64,
    /// Bottom extension.
    pub bottom: i64,
    /// Right extension.
    pub right: i64,
    /// Top extension.
    pub top: i64,
}

/// Pin direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinDirection {
    /// `INPUT`.
    Input,
    /// `OUTPUT`.
    Output,
    /// `INOUT`.
    Inout,
    /// `FEEDTHRU`.
    Feedthru,
}

impl PinDirection {
    /// The keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            PinDirection::Input => "INPUT",
            PinDirection::Output => "OUTPUT",
            PinDirection::Inout => "INOUT",
            PinDirection::Feedthru => "FEEDTHRU",
        }
    }

    fn parse(s: &str) -> Option<PinDirection> {
        Some(match s {
            "INPUT" => PinDirection::Input,
            "OUTPUT" => PinDirection::Output,
            "INOUT" => PinDirection::Inout,
            "FEEDTHRU" => PinDirection::Feedthru,
            _ => return None,
        })
    }
}

/// One `+ PORT` of a pin, or the pin's own geometry when it has no
/// explicit ports.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PinPort {
    /// `+ LAYER` / `+ POLYGON` shapes.
    pub shapes: Vec<LayerShape>,
    /// `+ VIA name ( x y )`.
    pub vias: Vec<(String, Point)>,
    /// `+ PLACED` / `+ FIXED` / `+ COVER`.
    pub placement: Placement,
}

/// A `PINS` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    /// The pin name.
    pub name: String,
    /// `+ NET`.
    pub net: String,
    /// `+ SPECIAL`.
    pub special: bool,
    /// `+ DIRECTION`.
    pub direction: Option<PinDirection>,
    /// `+ USE` (`SIGNAL`, `POWER`, `GROUND`, `CLOCK`, ...).
    pub use_: Option<String>,
    /// `+ NETEXPR`.
    pub net_expr: Option<String>,
    /// `+ SUPPLYSENSITIVITY`.
    pub supply_sensitivity: Option<String>,
    /// `+ GROUNDSENSITIVITY`.
    pub ground_sensitivity: Option<String>,
    /// Geometry and placement given directly on the pin.
    pub port: PinPort,
    /// Explicit `+ PORT` groups.
    pub ports: Vec<PinPort>,
    /// `+ PROPERTY` values.
    pub properties: Vec<Property>,
    /// Unknown `+` clauses (antenna values, ...).
    pub extras: Vec<Raw>,
}

impl Pin {
    /// An unplaced pin on `net` with the given direction.
    pub fn new(name: impl Into<String>, net: impl Into<String>, direction: PinDirection) -> Pin {
        Pin {
            name: name.into(),
            net: net.into(),
            special: false,
            direction: Some(direction),
            use_: None,
            net_expr: None,
            supply_sensitivity: None,
            ground_sensitivity: None,
            port: PinPort::default(),
            ports: Vec::new(),
            properties: Vec::new(),
            extras: Vec::new(),
        }
    }
}

/// One `( component pin )` of a net; `component` is `PIN` for a design
/// I/O pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    /// The component name, or `PIN`.
    pub component: String,
    /// The pin name.
    pub pin: String,
    /// `+ SYNTHESIZED`.
    pub synthesized: bool,
}

impl Connection {
    /// A connection to a component pin.
    pub fn new(component: impl Into<String>, pin: impl Into<String>) -> Connection {
        Connection {
            component: component.into(),
            pin: pin.into(),
            synthesized: false,
        }
    }

    /// A connection to a design I/O pin.
    pub fn io(pin: impl Into<String>) -> Connection {
        Connection::new("PIN", pin)
    }

    /// True for a design I/O pin.
    pub fn is_io(&self) -> bool {
        self.component == "PIN"
    }
}

/// The status of a routed wire group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteStatus {
    /// `+ ROUTED`.
    Routed,
    /// `+ FIXED`.
    Fixed,
    /// `+ COVER`.
    Cover,
    /// `+ NOSHIELD`.
    Noshield,
}

impl RouteStatus {
    /// The keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            RouteStatus::Routed => "ROUTED",
            RouteStatus::Fixed => "FIXED",
            RouteStatus::Cover => "COVER",
            RouteStatus::Noshield => "NOSHIELD",
        }
    }
}

/// One element of a wire's point list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteItem {
    /// `( x y [ext] )`; `None` for `*` (same as the previous point).
    Point {
        /// X, or `None` for `*`.
        x: Option<i64>,
        /// Y, or `None` for `*`.
        y: Option<i64>,
        /// Wire extension.
        ext: Option<i64>,
        /// `MASK` before the point.
        mask: Option<u32>,
    },
    /// A via placed at the previous point.
    Via {
        /// The via name.
        name: String,
        /// Orientation.
        orient: Option<Orient>,
        /// `MASK` before the via.
        mask: Option<u32>,
    },
    /// `RECT ( dx1 dy1 dx2 dy2 )` relative to the previous point.
    Rect(Rect),
    /// `VIRTUAL ( x y )`.
    Virtual(Point),
}

/// One wire of a route: a layer and a point list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wire {
    /// The layer name.
    pub layer: String,
    /// `TAPER` or `TAPERRULE name`.
    pub taper: Option<Option<String>>,
    /// `STYLE`.
    pub style: Option<i64>,
    /// Points and vias.
    pub items: Vec<RouteItem>,
}

/// A `+ ROUTED` / `+ FIXED` / `+ COVER` / `+ NOSHIELD` clause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// The status.
    pub status: RouteStatus,
    /// The wires (`NEW`-separated).
    pub wires: Vec<Wire>,
}

/// A `NETS` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Net {
    /// The net name.
    pub name: String,
    /// The connections.
    pub connections: Vec<Connection>,
    /// `+ USE`.
    pub use_: Option<String>,
    /// `+ SOURCE`.
    pub source: Option<String>,
    /// `+ WEIGHT`.
    pub weight: Option<i64>,
    /// `+ NONDEFAULTRULE`.
    pub nondefault_rule: Option<String>,
    /// `+ FIXEDBUMP`.
    pub fixed_bump: bool,
    /// `+ ORIGINAL`.
    pub original: Option<String>,
    /// `+ PATTERN`.
    pub pattern: Option<String>,
    /// `+ ESTCAP`.
    pub est_cap: Option<f64>,
    /// `+ FREQUENCY`.
    pub frequency: Option<f64>,
    /// `+ XTALK`.
    pub xtalk: Option<i64>,
    /// Routed wires.
    pub routes: Vec<Route>,
    /// `+ PROPERTY` values.
    pub properties: Vec<Property>,
    /// Unknown `+` clauses (`SHIELDNET`, `VPIN`, `SUBNET`, ...).
    pub extras: Vec<Raw>,
}

impl Net {
    /// An unrouted net with no connections.
    pub fn new(name: impl Into<String>) -> Net {
        Net {
            name: name.into(),
            connections: Vec::new(),
            use_: None,
            source: None,
            weight: None,
            nondefault_rule: None,
            fixed_bump: false,
            original: None,
            pattern: None,
            est_cap: None,
            frequency: None,
            xtalk: None,
            routes: Vec::new(),
            properties: Vec::new(),
            extras: Vec::new(),
        }
    }
}

/// A `SPECIALNETS` entry. Routing is kept shallow: every `+` clause
/// other than `USE`, `SOURCE`, `WEIGHT` and `PROPERTY` is preserved
/// verbatim.
#[derive(Clone, Debug, PartialEq)]
pub struct SpecialNet {
    /// The net name.
    pub name: String,
    /// The connections.
    pub connections: Vec<Connection>,
    /// `+ USE`.
    pub use_: Option<String>,
    /// `+ SOURCE`.
    pub source: Option<String>,
    /// `+ WEIGHT`.
    pub weight: Option<i64>,
    /// `+ PROPERTY` values.
    pub properties: Vec<Property>,
    /// Remaining `+` clauses (`ROUTED`, `SHAPE`, `VOLTAGE`, ...).
    pub extras: Vec<Raw>,
}

/// What a blockage blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockageKind {
    /// `- LAYER name`: routing on that layer.
    Layer(String),
    /// `- PLACEMENT`: cell placement.
    Placement,
}

/// A `BLOCKAGES` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Blockage {
    /// Layer or placement.
    pub kind: BlockageKind,
    /// `+ COMPONENT`.
    pub component: Option<String>,
    /// Other `+` qualifiers (`SLOTS`, `FILLS`, `PUSHDOWN`, `SOFT`,
    /// `PARTIAL d`, `SPACING s`, ...), each without the `+`.
    pub options: Vec<Raw>,
    /// `RECT` shapes.
    pub rects: Vec<Rect>,
    /// `POLYGON` shapes.
    pub polygons: Vec<Vec<Point>>,
}

/// A `REGIONS` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// The region name.
    pub name: String,
    /// The rectangles.
    pub rects: Vec<Rect>,
    /// `+ TYPE FENCE|GUIDE`.
    pub kind: Option<String>,
    /// `+ PROPERTY` values.
    pub properties: Vec<Property>,
}

/// A `GROUPS` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The group name.
    pub name: String,
    /// Component name patterns.
    pub members: Vec<String>,
    /// `+ REGION`.
    pub region: Option<String>,
    /// `+ PROPERTY` values.
    pub properties: Vec<Property>,
    /// Unknown `+` clauses.
    pub extras: Vec<Raw>,
}

/// A parsed DEF file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Def {
    /// `VERSION`.
    pub version: Option<String>,
    /// `DIVIDERCHAR`.
    pub divider_char: Option<String>,
    /// `BUSBITCHARS`.
    pub bus_bit_chars: Option<String>,
    /// `DESIGN`.
    pub design: String,
    /// `TECHNOLOGY`.
    pub technology: Option<String>,
    /// `UNITS DISTANCE MICRONS`: database units per micron.
    pub units: Option<i64>,
    /// `PROPERTYDEFINITIONS`.
    pub property_definitions: Vec<PropertyDefinition>,
    /// `DIEAREA`: two points for a rectangle, more for a polygon.
    pub die_area: Vec<Point>,
    /// `ROW` statements.
    pub rows: Vec<Row>,
    /// `TRACKS` statements.
    pub tracks: Vec<Track>,
    /// `GCELLGRID` statements.
    pub gcell_grids: Vec<GcellGrid>,
    /// `VIAS`.
    pub vias: Vec<Via>,
    /// `REGIONS`.
    pub regions: Vec<Region>,
    /// `COMPONENTS`.
    pub components: Vec<Component>,
    /// `PINS`.
    pub pins: Vec<Pin>,
    /// `BLOCKAGES`.
    pub blockages: Vec<Blockage>,
    /// `SPECIALNETS`.
    pub special_nets: Vec<SpecialNet>,
    /// `NETS`.
    pub nets: Vec<Net>,
    /// `GROUPS`.
    pub groups: Vec<Group>,
    /// Unmodelled statements and sections, in file order.
    pub extras: Vec<Raw>,
}

impl Def {
    /// The component with this name.
    pub fn component(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.name == name)
    }

    /// The net with this name.
    pub fn net(&self, name: &str) -> Option<&Net> {
        self.nets.iter().find(|n| n.name == name)
    }

    /// The pin with this name.
    pub fn pin(&self, name: &str) -> Option<&Pin> {
        self.pins.iter().find(|p| p.name == name)
    }

    /// The die area as a rectangle, when it is one.
    pub fn die_rect(&self) -> Option<Rect> {
        match self.die_area.as_slice() {
            [a, b] => Some(Rect {
                x1: a.x.min(b.x),
                y1: a.y.min(b.y),
                x2: a.x.max(b.x),
                y2: a.y.max(b.y),
            }),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Parses DEF text. Syntax errors are reported to `diags` with spans and
/// the offending statement is skipped.
pub fn parse_def(text: &str, file: SourceId, diags: &mut Diagnostics) -> Def {
    let mut c = Cursor::new(text, file);
    let mut def = Def::default();
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
                if c.eat_word("DESIGN") {
                    break;
                }
                Err(c.error("expected `END DESIGN`"))
            }
            "VERSION" | "DIVIDERCHAR" | "BUSBITCHARS" | "DESIGN" | "TECHNOLOGY" => {
                let key = tok.text;
                c.next();
                c.name().and_then(|v| {
                    let v = v.to_string();
                    match key {
                        "VERSION" => def.version = Some(v),
                        "DIVIDERCHAR" => def.divider_char = Some(v),
                        "BUSBITCHARS" => def.bus_bit_chars = Some(v),
                        "DESIGN" => def.design = v,
                        _ => def.technology = Some(v),
                    }
                    c.expect_semi()
                })
            }
            "UNITS" => {
                c.next();
                (|| {
                    c.expect_word("DISTANCE")?;
                    c.expect_word("MICRONS")?;
                    def.units = Some(c.integer()?);
                    c.expect_semi()
                })()
            }
            "PROPERTYDEFINITIONS" => {
                c.next();
                parse_property_definitions(&mut c, &mut def.property_definitions, diags)
            }
            "DIEAREA" => {
                c.next();
                parse_points(&mut c).and_then(|p| {
                    def.die_area = p;
                    c.expect_semi()
                })
            }
            "ROW" => {
                c.next();
                parse_row(&mut c).map(|r| def.rows.push(r))
            }
            "TRACKS" => {
                c.next();
                parse_tracks(&mut c).map(|t| def.tracks.push(t))
            }
            "GCELLGRID" => {
                c.next();
                (|| {
                    let axis = parse_axis(&mut c)?;
                    let start = c.integer()?;
                    c.expect_word("DO")?;
                    let num = c.integer()?;
                    c.expect_word("STEP")?;
                    let step = c.integer()?;
                    c.expect_semi()?;
                    def.gcell_grids.push(GcellGrid {
                        axis,
                        start,
                        num,
                        step,
                    });
                    Ok(())
                })()
            }
            "VIAS" => {
                c.next();
                parse_section(&mut c, "VIAS", diags, |c| {
                    parse_via(c).map(|v| def.vias.push(v))
                })
            }
            "REGIONS" => {
                c.next();
                parse_section(&mut c, "REGIONS", diags, |c| {
                    parse_region(c).map(|r| def.regions.push(r))
                })
            }
            "COMPONENTS" => {
                c.next();
                parse_section(&mut c, "COMPONENTS", diags, |c| {
                    parse_component(c).map(|x| def.components.push(x))
                })
            }
            "PINS" => {
                c.next();
                parse_section(&mut c, "PINS", diags, |c| {
                    parse_pin(c).map(|p| def.pins.push(p))
                })
            }
            "BLOCKAGES" => {
                c.next();
                parse_section(&mut c, "BLOCKAGES", diags, |c| {
                    parse_blockage(c).map(|b| def.blockages.push(b))
                })
            }
            "SPECIALNETS" => {
                c.next();
                parse_section(&mut c, "SPECIALNETS", diags, |c| {
                    parse_special_net(c).map(|n| def.special_nets.push(n))
                })
            }
            "NETS" => {
                c.next();
                parse_section(&mut c, "NETS", diags, |c| {
                    parse_net(c).map(|n| def.nets.push(n))
                })
            }
            "GROUPS" => {
                c.next();
                parse_section(&mut c, "GROUPS", diags, |c| {
                    parse_group(c).map(|g| def.groups.push(g))
                })
            }
            "NONDEFAULTRULES" | "PINPROPERTIES" | "SLOTS" | "FILLS" | "SCANCHAINS" | "STYLES"
            | "BEGINEXT" => {
                def.extras.push(raw_section(&mut c));
                Ok(())
            }
            _ => {
                def.extras.push(Raw::statement(c.raw_statement()));
                Ok(())
            }
        };
        if let Err(e) = result {
            c.recover(e, diags);
        }
    }
    def
}

/// Collects `KEYWORD ... END KEYWORD` (or `BEGINEXT ... ENDEXT`) verbatim.
fn raw_section(c: &mut Cursor<'_>) -> Raw {
    let mut tokens = Vec::new();
    let keyword = c.next().map(|t| t.text.to_string()).unwrap_or_default();
    tokens.push(keyword.clone());
    while let Some(t) = c.next() {
        let text = match t.kind {
            Kind::Str => format!("\"{}\"", t.text),
            _ => t.text.to_string(),
        };
        if keyword == "BEGINEXT" && text == "ENDEXT" {
            tokens.push(text);
            break;
        }
        let is_end = text == "END" && c.is_word(&keyword);
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

/// Parses `n ; - item ; ... END KEYWORD`, recovering per item.
fn parse_section(
    c: &mut Cursor<'_>,
    keyword: &str,
    diags: &mut Diagnostics,
    mut item: impl FnMut(&mut Cursor<'_>) -> PResult<()>,
) -> PResult<()> {
    c.integer()?;
    c.expect_semi()?;
    loop {
        if c.at_end() {
            return Err(c.error(format!("unterminated {keyword} section")));
        }
        if c.eat_word("END") {
            return c.expect_word(keyword);
        }
        if let Err(e) = c.expect_word("-").and_then(|()| item(c)) {
            c.recover(e, diags);
        }
    }
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
        if c.eat_word("END") {
            return c.expect_word("PROPERTYDEFINITIONS");
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
                    def.range = Some((c.number()?, c.number()?));
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

/// `+ PROPERTY name value ...` up to the next `+` or `;`.
fn parse_properties(c: &mut Cursor<'_>, out: &mut Vec<Property>) -> PResult<()> {
    while !c.is_kind(Kind::Semi) && !c.is_word("+") && !c.at_end() {
        let name = c.name()?.to_string();
        let value = parse_property_value(c)?;
        out.push(Property { name, value });
    }
    Ok(())
}

fn parse_axis(c: &mut Cursor<'_>) -> PResult<Axis> {
    let w = c.word()?;
    Axis::parse(w).ok_or_else(|| c.error(format!("expected `X` or `Y`, found `{w}`")))
}

fn parse_orient(c: &mut Cursor<'_>) -> PResult<Orient> {
    let w = c.word()?;
    Orient::from_keyword(w).ok_or_else(|| c.error(format!("unknown orientation `{w}`")))
}

/// `( x y )`.
fn parse_point(c: &mut Cursor<'_>) -> PResult<Point> {
    c.expect_lparen()?;
    let x = c.integer()?;
    let y = c.integer()?;
    c.expect_rparen()?;
    Ok(Point { x, y })
}

/// A sequence of `( x y )` up to something that is not `(`.
fn parse_points(c: &mut Cursor<'_>) -> PResult<Vec<Point>> {
    let mut pts = Vec::new();
    while c.is_kind(Kind::LParen) {
        pts.push(parse_point(c)?);
    }
    Ok(pts)
}

/// `( x1 y1 ) ( x2 y2 )`.
fn parse_rect(c: &mut Cursor<'_>) -> PResult<Rect> {
    let a = parse_point(c)?;
    let b = parse_point(c)?;
    Ok(Rect {
        x1: a.x,
        y1: a.y,
        x2: b.x,
        y2: b.y,
    })
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

fn parse_row(c: &mut Cursor<'_>) -> PResult<Row> {
    let name = c.name()?.to_string();
    let site = c.name()?.to_string();
    let x = c.integer()?;
    let y = c.integer()?;
    let orient = parse_orient(c)?;
    let mut row = Row {
        name,
        site,
        origin: Point { x, y },
        orient,
        repeat: None,
        properties: Vec::new(),
    };
    if c.eat_word("DO") {
        let num_x = c.integer()?;
        c.expect_word("BY")?;
        let num_y = c.integer()?;
        let (mut step_x, mut step_y) = (0, 0);
        if c.eat_word("STEP") {
            step_x = c.integer()?;
            step_y = c.integer()?;
        }
        row.repeat = Some(RowRepeat {
            num_x,
            num_y,
            step_x,
            step_y,
        });
    }
    while c.eat_word("+") {
        if c.eat_word("PROPERTY") {
            parse_properties(c, &mut row.properties)?;
        } else {
            return Err(c.error("unexpected ROW option"));
        }
    }
    c.expect_semi()?;
    Ok(row)
}

fn parse_tracks(c: &mut Cursor<'_>) -> PResult<Track> {
    let axis = parse_axis(c)?;
    let start = c.integer()?;
    c.expect_word("DO")?;
    let num = c.integer()?;
    c.expect_word("STEP")?;
    let step = c.integer()?;
    let mut track = Track {
        axis,
        start,
        num,
        step,
        mask: None,
        same_mask: false,
        layers: Vec::new(),
    };
    track.mask = parse_mask(c)?;
    if track.mask.is_some() {
        track.same_mask = c.eat_word("SAMEMASK");
    }
    if c.eat_word("LAYER") {
        while !c.is_kind(Kind::Semi) && !c.at_end() {
            track.layers.push(c.name()?.to_string());
        }
    }
    c.expect_semi()?;
    Ok(track)
}

/// The tokens of one unknown `+ ...` clause (the `+` already consumed).
fn raw_clause(c: &mut Cursor<'_>) -> Raw {
    Raw::statement(c.raw_until(&["+"]))
}

fn parse_via(c: &mut Cursor<'_>) -> PResult<Via> {
    let name = c.name()?.to_string();
    let mut via = Via {
        name,
        via_rule: None,
        shapes: Vec::new(),
        extras: Vec::new(),
    };
    while c.eat_word("+") {
        let key = c.word()?;
        match key {
            "VIARULE" => {
                via.via_rule = Some(ViaGenerate {
                    rule: c.name()?.to_string(),
                    cut_size: None,
                    layers: None,
                    cut_spacing: None,
                    enclosure: None,
                    row_col: None,
                    origin: None,
                    offset: None,
                    pattern: None,
                });
            }
            "CUTSIZE" | "CUTSPACING" | "LAYERS" | "ENCLOSURE" | "ROWCOL" | "ORIGIN" | "OFFSET"
            | "PATTERN" => {
                let Some(g) = via.via_rule.as_mut() else {
                    return Err(c.error(format!("`{key}` needs a preceding VIARULE")));
                };
                match key {
                    "CUTSIZE" => g.cut_size = Some((c.integer()?, c.integer()?)),
                    "CUTSPACING" => g.cut_spacing = Some((c.integer()?, c.integer()?)),
                    "LAYERS" => {
                        g.layers = Some((
                            c.name()?.to_string(),
                            c.name()?.to_string(),
                            c.name()?.to_string(),
                        ));
                    }
                    "ENCLOSURE" => {
                        g.enclosure =
                            Some((c.integer()?, c.integer()?, c.integer()?, c.integer()?));
                    }
                    "ROWCOL" => g.row_col = Some((c.integer()?, c.integer()?)),
                    "ORIGIN" => {
                        g.origin = Some(Point {
                            x: c.integer()?,
                            y: c.integer()?,
                        });
                    }
                    "OFFSET" => {
                        g.offset = Some((c.integer()?, c.integer()?, c.integer()?, c.integer()?));
                    }
                    _ => g.pattern = Some(c.name()?.to_string()),
                }
            }
            "RECT" | "POLYGON" => {
                let layer = c.name()?.to_string();
                let mut mask = None;
                if c.eat_word("+") {
                    c.expect_word("MASK")?;
                    let v = c.integer()?;
                    mask = Some(u32::try_from(v).map_err(|_| c.error("mask out of range"))?);
                }
                let shape = if key == "RECT" {
                    ShapeKind::Rect(parse_rect(c)?)
                } else {
                    ShapeKind::Polygon(parse_points(c)?)
                };
                via.shapes.push(LayerShape {
                    layer,
                    mask,
                    spacing: None,
                    design_rule_width: None,
                    shape,
                });
            }
            _ => {
                let mut raw = vec![key.to_string()];
                raw.extend(raw_clause(c).tokens);
                via.extras.push(Raw::statement(raw));
            }
        }
    }
    c.expect_semi()?;
    Ok(via)
}

fn parse_placement(c: &mut Cursor<'_>, key: &str) -> PResult<Placement> {
    let status = match key {
        "PLACED" => PlacementStatus::Placed,
        "FIXED" => PlacementStatus::Fixed,
        "COVER" => PlacementStatus::Cover,
        _ => return Ok(Placement::default()),
    };
    let point = parse_point(c)?;
    let orient = parse_orient(c)?;
    Ok(Placement {
        status,
        location: Some((point, orient)),
    })
}

fn parse_component(c: &mut Cursor<'_>) -> PResult<Component> {
    let name = c.name()?.to_string();
    let master = c.name()?.to_string();
    let mut comp = Component::new(name, master);
    while c.eat_word("+") {
        let key = c.word()?;
        match key {
            "EEQ" => comp.eeq = Some(c.name()?.to_string()),
            "SOURCE" => comp.source = Some(c.word()?.to_string()),
            "PLACED" | "FIXED" | "COVER" | "UNPLACED" => {
                comp.placement = parse_placement(c, key)?;
            }
            "MASKSHIFT" => comp.mask_shift = Some(c.word()?.to_string()),
            "HALO" => {
                let soft = c.eat_word("SOFT");
                comp.halo = Some(Halo {
                    soft,
                    left: c.integer()?,
                    bottom: c.integer()?,
                    right: c.integer()?,
                    top: c.integer()?,
                });
            }
            "WEIGHT" => comp.weight = Some(c.integer()?),
            "REGION" => comp.region = Some(c.name()?.to_string()),
            "PROPERTY" => parse_properties(c, &mut comp.properties)?,
            _ => {
                let mut raw = vec![key.to_string()];
                raw.extend(raw_clause(c).tokens);
                comp.extras.push(Raw::statement(raw));
            }
        }
    }
    c.expect_semi()?;
    Ok(comp)
}

/// `LAYER name [+ MASK m] [+ SPACING s | + DESIGNRULEWIDTH d] rect` or
/// `POLYGON name [+ MASK m] ... points`, with the keyword consumed.
fn parse_pin_shape(c: &mut Cursor<'_>, key: &str) -> PResult<LayerShape> {
    let layer = c.name()?.to_string();
    let mut shape = LayerShape {
        layer,
        mask: None,
        spacing: None,
        design_rule_width: None,
        shape: ShapeKind::Rect(Rect::default()),
    };
    // Qualifiers are `+ MASK`, `+ SPACING`, `+ DESIGNRULEWIDTH` before the
    // points; a `+` followed by anything else belongs to the pin.
    while c.is_word("+") {
        let Some(next) = c.peek_at(1) else { break };
        match next.text {
            "MASK" => {
                c.next();
                c.next();
                let v = c.integer()?;
                shape.mask = Some(u32::try_from(v).map_err(|_| c.error("mask out of range"))?);
            }
            "SPACING" => {
                c.next();
                c.next();
                shape.spacing = Some(c.integer()?);
            }
            "DESIGNRULEWIDTH" => {
                c.next();
                c.next();
                shape.design_rule_width = Some(c.integer()?);
            }
            _ => break,
        }
    }
    shape.shape = if key == "LAYER" {
        ShapeKind::Rect(parse_rect(c)?)
    } else {
        ShapeKind::Polygon(parse_points(c)?)
    };
    Ok(shape)
}

fn parse_pin(c: &mut Cursor<'_>) -> PResult<Pin> {
    let name = c.name()?.to_string();
    let mut pin = Pin {
        name,
        net: String::new(),
        special: false,
        direction: None,
        use_: None,
        net_expr: None,
        supply_sensitivity: None,
        ground_sensitivity: None,
        port: PinPort::default(),
        ports: Vec::new(),
        properties: Vec::new(),
        extras: Vec::new(),
    };
    while c.eat_word("+") {
        let key = c.word()?;
        // Geometry and placement go to the current explicit port when
        // there is one, else to the pin itself.
        match key {
            "NET" => pin.net = c.name()?.to_string(),
            "SPECIAL" => pin.special = true,
            "DIRECTION" => {
                let d = c.word()?;
                pin.direction = Some(
                    PinDirection::parse(d)
                        .ok_or_else(|| c.error(format!("unknown pin direction `{d}`")))?,
                );
            }
            "USE" => pin.use_ = Some(c.word()?.to_string()),
            "NETEXPR" => pin.net_expr = Some(c.name()?.to_string()),
            "SUPPLYSENSITIVITY" => pin.supply_sensitivity = Some(c.name()?.to_string()),
            "GROUNDSENSITIVITY" => pin.ground_sensitivity = Some(c.name()?.to_string()),
            "PORT" => pin.ports.push(PinPort::default()),
            "LAYER" | "POLYGON" => {
                let shape = parse_pin_shape(c, key)?;
                current_port(&mut pin).shapes.push(shape);
            }
            "VIA" => {
                let via = c.name()?.to_string();
                let point = parse_point(c)?;
                current_port(&mut pin).vias.push((via, point));
            }
            "PLACED" | "FIXED" | "COVER" | "UNPLACED" => {
                let p = parse_placement(c, key)?;
                current_port(&mut pin).placement = p;
            }
            "PROPERTY" => parse_properties(c, &mut pin.properties)?,
            _ => {
                let mut raw = vec![key.to_string()];
                raw.extend(raw_clause(c).tokens);
                pin.extras.push(Raw::statement(raw));
            }
        }
    }
    c.expect_semi()?;
    Ok(pin)
}

fn current_port(pin: &mut Pin) -> &mut PinPort {
    match pin.ports.last_mut() {
        Some(p) => p,
        None => &mut pin.port,
    }
}

/// `( comp pin [+ SYNTHESIZED] )` connections up to the first `+` or `;`.
fn parse_connections(c: &mut Cursor<'_>) -> PResult<Vec<Connection>> {
    let mut out = Vec::new();
    while c.eat_kind(Kind::LParen) {
        let component = c.name()?.to_string();
        let pin = c.name()?.to_string();
        let mut synthesized = false;
        if c.eat_word("+") {
            c.expect_word("SYNTHESIZED")?;
            synthesized = true;
        }
        c.expect_rparen()?;
        out.push(Connection {
            component,
            pin,
            synthesized,
        });
    }
    Ok(out)
}

/// The wires of a `+ ROUTED`-style clause (keyword consumed).
fn parse_route(c: &mut Cursor<'_>, status: RouteStatus) -> PResult<Route> {
    let mut wires = Vec::new();
    loop {
        let layer = c.name()?.to_string();
        let mut wire = Wire {
            layer,
            taper: None,
            style: None,
            items: Vec::new(),
        };
        loop {
            if c.eat_word("TAPER") {
                wire.taper = Some(None);
            } else if c.eat_word("TAPERRULE") {
                wire.taper = Some(Some(c.name()?.to_string()));
            } else if c.eat_word("STYLE") {
                wire.style = Some(c.integer()?);
            } else {
                break;
            }
        }
        loop {
            let mask = parse_mask(c)?;
            if c.eat_kind(Kind::LParen) {
                let coord = |c: &mut Cursor<'_>| -> PResult<Option<i64>> {
                    if c.eat_word("*") {
                        Ok(None)
                    } else {
                        c.integer().map(Some)
                    }
                };
                let x = coord(c)?;
                let y = coord(c)?;
                let ext = if c.is_kind(Kind::RParen) {
                    None
                } else {
                    Some(c.integer()?)
                };
                c.expect_rparen()?;
                wire.items.push(RouteItem::Point { x, y, ext, mask });
            } else if c.eat_word("RECT") {
                c.expect_lparen()?;
                let r = Rect {
                    x1: c.integer()?,
                    y1: c.integer()?,
                    x2: c.integer()?,
                    y2: c.integer()?,
                };
                c.expect_rparen()?;
                wire.items.push(RouteItem::Rect(r));
            } else if c.eat_word("VIRTUAL") {
                wire.items.push(RouteItem::Virtual(parse_point(c)?));
            } else if c.is_kind(Kind::Word) && !c.is_word("NEW") && !c.is_word("+") {
                let name = c.name()?.to_string();
                let orient = match c.peek() {
                    Some(t) if t.kind == Kind::Word => Orient::from_keyword(t.text),
                    _ => None,
                };
                if orient.is_some() {
                    c.next();
                }
                wire.items.push(RouteItem::Via { name, orient, mask });
            } else {
                if mask.is_some() {
                    return Err(c.error("MASK must precede a point or via"));
                }
                break;
            }
        }
        wires.push(wire);
        if !c.eat_word("NEW") {
            break;
        }
    }
    Ok(Route { status, wires })
}

fn parse_net(c: &mut Cursor<'_>) -> PResult<Net> {
    let name = c.name()?.to_string();
    let mut net = Net::new(name);
    net.connections = parse_connections(c)?;
    while c.eat_word("+") {
        let key = c.word()?;
        match key {
            "USE" => net.use_ = Some(c.word()?.to_string()),
            "SOURCE" => net.source = Some(c.word()?.to_string()),
            "WEIGHT" => net.weight = Some(c.integer()?),
            "NONDEFAULTRULE" => net.nondefault_rule = Some(c.name()?.to_string()),
            "FIXEDBUMP" => net.fixed_bump = true,
            "ORIGINAL" => net.original = Some(c.name()?.to_string()),
            "PATTERN" => net.pattern = Some(c.word()?.to_string()),
            "ESTCAP" => net.est_cap = Some(c.number()?),
            "FREQUENCY" => net.frequency = Some(c.number()?),
            "XTALK" => net.xtalk = Some(c.integer()?),
            "ROUTED" => net.routes.push(parse_route(c, RouteStatus::Routed)?),
            "FIXED" => net.routes.push(parse_route(c, RouteStatus::Fixed)?),
            "COVER" => net.routes.push(parse_route(c, RouteStatus::Cover)?),
            "NOSHIELD" => net.routes.push(parse_route(c, RouteStatus::Noshield)?),
            "PROPERTY" => parse_properties(c, &mut net.properties)?,
            _ => {
                let mut raw = vec![key.to_string()];
                raw.extend(raw_clause(c).tokens);
                net.extras.push(Raw::statement(raw));
            }
        }
    }
    c.expect_semi()?;
    Ok(net)
}

fn parse_special_net(c: &mut Cursor<'_>) -> PResult<SpecialNet> {
    let name = c.name()?.to_string();
    let mut net = SpecialNet {
        name,
        connections: parse_connections(c)?,
        use_: None,
        source: None,
        weight: None,
        properties: Vec::new(),
        extras: Vec::new(),
    };
    while c.eat_word("+") {
        let key = c.word()?;
        match key {
            "USE" => net.use_ = Some(c.word()?.to_string()),
            "SOURCE" => net.source = Some(c.word()?.to_string()),
            "WEIGHT" => net.weight = Some(c.integer()?),
            "PROPERTY" => parse_properties(c, &mut net.properties)?,
            _ => {
                let mut raw = vec![key.to_string()];
                raw.extend(raw_clause(c).tokens);
                net.extras.push(Raw::statement(raw));
            }
        }
    }
    c.expect_semi()?;
    Ok(net)
}

fn parse_blockage(c: &mut Cursor<'_>) -> PResult<Blockage> {
    let kind = if c.eat_word("LAYER") {
        BlockageKind::Layer(c.name()?.to_string())
    } else if c.eat_word("PLACEMENT") {
        BlockageKind::Placement
    } else {
        return Err(c.error("expected `LAYER` or `PLACEMENT`"));
    };
    let mut b = Blockage {
        kind,
        component: None,
        options: Vec::new(),
        rects: Vec::new(),
        polygons: Vec::new(),
    };
    while c.eat_word("+") {
        if c.eat_word("COMPONENT") {
            b.component = Some(c.name()?.to_string());
        } else {
            b.options
                .push(Raw::statement(c.raw_until(&["+", "RECT", "POLYGON"])));
        }
    }
    loop {
        if c.eat_word("RECT") {
            b.rects.push(parse_rect(c)?);
        } else if c.eat_word("POLYGON") {
            b.polygons.push(parse_points(c)?);
        } else {
            break;
        }
    }
    c.expect_semi()?;
    Ok(b)
}

fn parse_region(c: &mut Cursor<'_>) -> PResult<Region> {
    let name = c.name()?.to_string();
    let mut r = Region {
        name,
        rects: Vec::new(),
        kind: None,
        properties: Vec::new(),
    };
    while c.is_kind(Kind::LParen) {
        r.rects.push(parse_rect(c)?);
    }
    while c.eat_word("+") {
        if c.eat_word("TYPE") {
            r.kind = Some(c.word()?.to_string());
        } else if c.eat_word("PROPERTY") {
            parse_properties(c, &mut r.properties)?;
        } else {
            return Err(c.error("unexpected REGION option"));
        }
    }
    c.expect_semi()?;
    Ok(r)
}

fn parse_group(c: &mut Cursor<'_>) -> PResult<Group> {
    let name = c.name()?.to_string();
    let mut g = Group {
        name,
        members: Vec::new(),
        region: None,
        properties: Vec::new(),
        extras: Vec::new(),
    };
    while !c.is_word("+") && !c.is_kind(Kind::Semi) && !c.at_end() {
        g.members.push(c.name()?.to_string());
    }
    while c.eat_word("+") {
        let key = c.word()?;
        match key {
            "REGION" => g.region = Some(c.name()?.to_string()),
            "PROPERTY" => parse_properties(c, &mut g.properties)?,
            _ => {
                let mut raw = vec![key.to_string()];
                raw.extend(raw_clause(c).tokens);
                g.extras.push(Raw::statement(raw));
            }
        }
    }
    c.expect_semi()?;
    Ok(g)
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

fn point(p: Point) -> String {
    format!("( {} {} )", p.x, p.y)
}

fn rect(r: &Rect) -> String {
    format!("( {} {} ) ( {} {} )", r.x1, r.y1, r.x2, r.y2)
}

fn points(pts: &[Point]) -> String {
    pts.iter().map(|p| point(*p)).collect::<Vec<_>>().join(" ")
}

fn properties(out: &mut String, props: &[Property]) {
    if props.is_empty() {
        return;
    }
    out.push_str(" + PROPERTY");
    for p in props {
        out.push(' ');
        out.push_str(&p.name);
        out.push(' ');
        out.push_str(&p.value.to_string());
    }
}

fn raw_clauses(out: &mut String, extras: &[Raw]) {
    for e in extras {
        out.push_str(" + ");
        out.push_str(&e.tokens.join(" "));
    }
}

fn placement(out: &mut String, p: &Placement) {
    match (p.status, p.location) {
        (PlacementStatus::Unplaced, _) => {}
        (status, Some((pt, o))) => {
            out.push_str(&format!(" + {} {} {o}", status.as_str(), point(pt)));
        }
        (status, None) => out.push_str(&format!(" + {}", status.as_str())),
    }
}

fn write_raw(out: &mut String, raw: &Raw) {
    if raw.block {
        let mut line = String::new();
        for t in &raw.tokens {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(t);
            if t == ";" {
                out.push_str(&line);
                out.push('\n');
                line.clear();
            }
        }
        if !line.is_empty() {
            out.push_str(&line);
            out.push('\n');
        }
    } else {
        out.push_str(&raw.tokens.join(" "));
        out.push_str(" ;\n");
    }
}

fn layer_shape(out: &mut String, key: &str, s: &LayerShape) {
    out.push_str(&format!(" + {key} {}", s.layer));
    if let Some(m) = s.mask {
        out.push_str(&format!(" + MASK {m}"));
    }
    if let Some(v) = s.spacing {
        out.push_str(&format!(" + SPACING {v}"));
    }
    if let Some(v) = s.design_rule_width {
        out.push_str(&format!(" + DESIGNRULEWIDTH {v}"));
    }
    match &s.shape {
        ShapeKind::Rect(r) => {
            out.push(' ');
            out.push_str(&rect(r));
        }
        ShapeKind::Polygon(p) => {
            out.push(' ');
            out.push_str(&points(p));
        }
    }
}

fn write_pin_port(out: &mut String, port: &PinPort) {
    for s in &port.shapes {
        let key = match s.shape {
            ShapeKind::Rect(_) => "LAYER",
            ShapeKind::Polygon(_) => "POLYGON",
        };
        layer_shape(out, key, s);
    }
    for (via, p) in &port.vias {
        out.push_str(&format!(" + VIA {via} {}", point(*p)));
    }
    placement(out, &port.placement);
}

fn write_route(out: &mut String, r: &Route) {
    out.push_str(&format!(" + {}", r.status.as_str()));
    for (i, w) in r.wires.iter().enumerate() {
        if i > 0 {
            out.push_str(" NEW");
        }
        out.push(' ');
        out.push_str(&w.layer);
        match &w.taper {
            Some(None) => out.push_str(" TAPER"),
            Some(Some(rule)) => out.push_str(&format!(" TAPERRULE {rule}")),
            None => {}
        }
        if let Some(s) = w.style {
            out.push_str(&format!(" STYLE {s}"));
        }
        for item in &w.items {
            match item {
                RouteItem::Point { x, y, ext, mask } => {
                    if let Some(m) = mask {
                        out.push_str(&format!(" MASK {m}"));
                    }
                    let c = |v: &Option<i64>| v.map_or("*".to_string(), |v| v.to_string());
                    out.push_str(&format!(" ( {} {}", c(x), c(y)));
                    if let Some(e) = ext {
                        out.push_str(&format!(" {e}"));
                    }
                    out.push_str(" )");
                }
                RouteItem::Via { name, orient, mask } => {
                    if let Some(m) = mask {
                        out.push_str(&format!(" MASK {m}"));
                    }
                    out.push(' ');
                    out.push_str(name);
                    if let Some(o) = orient {
                        out.push_str(&format!(" {o}"));
                    }
                }
                RouteItem::Rect(r) => {
                    out.push_str(&format!(" RECT ( {} {} {} {} )", r.x1, r.y1, r.x2, r.y2));
                }
                RouteItem::Virtual(p) => out.push_str(&format!(" VIRTUAL {}", point(*p))),
            }
        }
    }
}

fn connections(out: &mut String, conns: &[Connection]) {
    for c in conns {
        out.push_str(&format!(" ( {} {}", c.component, c.pin));
        if c.synthesized {
            out.push_str(" + SYNTHESIZED");
        }
        out.push_str(" )");
    }
}

/// Renders a DEF file in canonical form.
pub fn write_def(def: &Def) -> String {
    let mut out = String::new();
    if let Some(v) = &def.version {
        out.push_str(&format!("VERSION {v} ;\n"));
    }
    if let Some(v) = &def.divider_char {
        out.push_str(&format!("DIVIDERCHAR \"{v}\" ;\n"));
    }
    if let Some(v) = &def.bus_bit_chars {
        out.push_str(&format!("BUSBITCHARS \"{v}\" ;\n"));
    }
    out.push_str(&format!("DESIGN {} ;\n", quote_if_needed(&def.design)));
    if let Some(v) = &def.technology {
        out.push_str(&format!("TECHNOLOGY {v} ;\n"));
    }
    if let Some(u) = def.units {
        out.push_str(&format!("UNITS DISTANCE MICRONS {u} ;\n"));
    }
    if !def.property_definitions.is_empty() {
        out.push_str("PROPERTYDEFINITIONS\n");
        for d in &def.property_definitions {
            let mut s = format!("  {} {} {}", d.object, d.name, d.ty);
            if let Some((lo, hi)) = d.range {
                s.push_str(&format!(" RANGE {} {}", fmt_num(lo), fmt_num(hi)));
            }
            if let Some(v) = &d.value {
                s.push_str(&format!(" {v}"));
            }
            out.push_str(&s);
            out.push_str(" ;\n");
        }
        out.push_str("END PROPERTYDEFINITIONS\n");
    }
    if !def.die_area.is_empty() {
        out.push_str(&format!("DIEAREA {} ;\n", points(&def.die_area)));
    }
    for r in &def.rows {
        let mut s = format!(
            "ROW {} {} {} {} {}",
            r.name, r.site, r.origin.x, r.origin.y, r.orient
        );
        if let Some(rep) = r.repeat {
            s.push_str(&format!(
                " DO {} BY {} STEP {} {}",
                rep.num_x, rep.num_y, rep.step_x, rep.step_y
            ));
        }
        properties(&mut s, &r.properties);
        out.push_str(&s);
        out.push_str(" ;\n");
    }
    for t in &def.tracks {
        let mut s = format!(
            "TRACKS {} {} DO {} STEP {}",
            t.axis.as_str(),
            t.start,
            t.num,
            t.step
        );
        if let Some(m) = t.mask {
            s.push_str(&format!(" MASK {m}"));
            if t.same_mask {
                s.push_str(" SAMEMASK");
            }
        }
        if !t.layers.is_empty() {
            s.push_str(" LAYER ");
            s.push_str(&t.layers.join(" "));
        }
        out.push_str(&s);
        out.push_str(" ;\n");
    }
    for g in &def.gcell_grids {
        out.push_str(&format!(
            "GCELLGRID {} {} DO {} STEP {} ;\n",
            g.axis.as_str(),
            g.start,
            g.num,
            g.step
        ));
    }
    if !def.vias.is_empty() {
        out.push_str(&format!("VIAS {} ;\n", def.vias.len()));
        for v in &def.vias {
            let mut s = format!("  - {}", v.name);
            if let Some(g) = &v.via_rule {
                s.push_str(&format!(" + VIARULE {}", g.rule));
                if let Some((x, y)) = g.cut_size {
                    s.push_str(&format!(" + CUTSIZE {x} {y}"));
                }
                if let Some((a, b, c)) = &g.layers {
                    s.push_str(&format!(" + LAYERS {a} {b} {c}"));
                }
                if let Some((x, y)) = g.cut_spacing {
                    s.push_str(&format!(" + CUTSPACING {x} {y}"));
                }
                if let Some((a, b, c, d)) = g.enclosure {
                    s.push_str(&format!(" + ENCLOSURE {a} {b} {c} {d}"));
                }
                if let Some((r, c)) = g.row_col {
                    s.push_str(&format!(" + ROWCOL {r} {c}"));
                }
                if let Some(p) = g.origin {
                    s.push_str(&format!(" + ORIGIN {} {}", p.x, p.y));
                }
                if let Some((a, b, c, d)) = g.offset {
                    s.push_str(&format!(" + OFFSET {a} {b} {c} {d}"));
                }
                if let Some(p) = &g.pattern {
                    s.push_str(&format!(" + PATTERN {p}"));
                }
            }
            for sh in &v.shapes {
                let key = match sh.shape {
                    ShapeKind::Rect(_) => "RECT",
                    ShapeKind::Polygon(_) => "POLYGON",
                };
                layer_shape(&mut s, key, sh);
            }
            raw_clauses(&mut s, &v.extras);
            out.push_str(&s);
            out.push_str(" ;\n");
        }
        out.push_str("END VIAS\n");
    }
    if !def.regions.is_empty() {
        out.push_str(&format!("REGIONS {} ;\n", def.regions.len()));
        for r in &def.regions {
            let mut s = format!("  - {}", r.name);
            for rc in &r.rects {
                s.push(' ');
                s.push_str(&rect(rc));
            }
            if let Some(k) = &r.kind {
                s.push_str(&format!(" + TYPE {k}"));
            }
            properties(&mut s, &r.properties);
            out.push_str(&s);
            out.push_str(" ;\n");
        }
        out.push_str("END REGIONS\n");
    }
    if !def.components.is_empty() {
        out.push_str(&format!("COMPONENTS {} ;\n", def.components.len()));
        for c in &def.components {
            let mut s = format!("  - {} {}", c.name, c.master);
            if let Some(v) = &c.eeq {
                s.push_str(&format!(" + EEQ {v}"));
            }
            if let Some(v) = &c.source {
                s.push_str(&format!(" + SOURCE {v}"));
            }
            match c.placement.status {
                PlacementStatus::Unplaced => s.push_str(" + UNPLACED"),
                _ => placement(&mut s, &c.placement),
            }
            if let Some(v) = &c.mask_shift {
                s.push_str(&format!(" + MASKSHIFT {v}"));
            }
            if let Some(h) = c.halo {
                s.push_str(" + HALO");
                if h.soft {
                    s.push_str(" SOFT");
                }
                s.push_str(&format!(" {} {} {} {}", h.left, h.bottom, h.right, h.top));
            }
            if let Some(v) = c.weight {
                s.push_str(&format!(" + WEIGHT {v}"));
            }
            if let Some(v) = &c.region {
                s.push_str(&format!(" + REGION {v}"));
            }
            raw_clauses(&mut s, &c.extras);
            properties(&mut s, &c.properties);
            out.push_str(&s);
            out.push_str(" ;\n");
        }
        out.push_str("END COMPONENTS\n");
    }
    if !def.pins.is_empty() {
        out.push_str(&format!("PINS {} ;\n", def.pins.len()));
        for p in &def.pins {
            let mut s = format!("  - {} + NET {}", p.name, p.net);
            if p.special {
                s.push_str(" + SPECIAL");
            }
            if let Some(d) = p.direction {
                s.push_str(&format!(" + DIRECTION {}", d.as_str()));
            }
            if let Some(v) = &p.net_expr {
                s.push_str(&format!(" + NETEXPR {}", quote_if_needed(v)));
            }
            if let Some(v) = &p.supply_sensitivity {
                s.push_str(&format!(" + SUPPLYSENSITIVITY {v}"));
            }
            if let Some(v) = &p.ground_sensitivity {
                s.push_str(&format!(" + GROUNDSENSITIVITY {v}"));
            }
            if let Some(u) = &p.use_ {
                s.push_str(&format!(" + USE {u}"));
            }
            raw_clauses(&mut s, &p.extras);
            properties(&mut s, &p.properties);
            write_pin_port(&mut s, &p.port);
            for port in &p.ports {
                s.push_str(" + PORT");
                write_pin_port(&mut s, port);
            }
            out.push_str(&s);
            out.push_str(" ;\n");
        }
        out.push_str("END PINS\n");
    }
    if !def.blockages.is_empty() {
        out.push_str(&format!("BLOCKAGES {} ;\n", def.blockages.len()));
        for b in &def.blockages {
            let mut s = match &b.kind {
                BlockageKind::Layer(l) => format!("  - LAYER {l}"),
                BlockageKind::Placement => "  - PLACEMENT".to_string(),
            };
            raw_clauses(&mut s, &b.options);
            if let Some(c) = &b.component {
                s.push_str(&format!(" + COMPONENT {c}"));
            }
            for r in &b.rects {
                s.push_str(&format!(" RECT {}", rect(r)));
            }
            for p in &b.polygons {
                s.push_str(&format!(" POLYGON {}", points(p)));
            }
            out.push_str(&s);
            out.push_str(" ;\n");
        }
        out.push_str("END BLOCKAGES\n");
    }
    if !def.special_nets.is_empty() {
        out.push_str(&format!("SPECIALNETS {} ;\n", def.special_nets.len()));
        for n in &def.special_nets {
            let mut s = format!("  - {}", n.name);
            connections(&mut s, &n.connections);
            if let Some(v) = &n.use_ {
                s.push_str(&format!(" + USE {v}"));
            }
            if let Some(v) = &n.source {
                s.push_str(&format!(" + SOURCE {v}"));
            }
            if let Some(v) = n.weight {
                s.push_str(&format!(" + WEIGHT {v}"));
            }
            raw_clauses(&mut s, &n.extras);
            properties(&mut s, &n.properties);
            out.push_str(&s);
            out.push_str(" ;\n");
        }
        out.push_str("END SPECIALNETS\n");
    }
    if !def.nets.is_empty() {
        out.push_str(&format!("NETS {} ;\n", def.nets.len()));
        for n in &def.nets {
            let mut s = format!("  - {}", n.name);
            connections(&mut s, &n.connections);
            if let Some(v) = &n.use_ {
                s.push_str(&format!(" + USE {v}"));
            }
            if let Some(v) = &n.source {
                s.push_str(&format!(" + SOURCE {v}"));
            }
            if let Some(v) = n.weight {
                s.push_str(&format!(" + WEIGHT {v}"));
            }
            if let Some(v) = &n.nondefault_rule {
                s.push_str(&format!(" + NONDEFAULTRULE {v}"));
            }
            if n.fixed_bump {
                s.push_str(" + FIXEDBUMP");
            }
            if let Some(v) = &n.original {
                s.push_str(&format!(" + ORIGINAL {v}"));
            }
            if let Some(v) = &n.pattern {
                s.push_str(&format!(" + PATTERN {v}"));
            }
            if let Some(v) = n.est_cap {
                s.push_str(&format!(" + ESTCAP {}", fmt_num(v)));
            }
            if let Some(v) = n.frequency {
                s.push_str(&format!(" + FREQUENCY {}", fmt_num(v)));
            }
            if let Some(v) = n.xtalk {
                s.push_str(&format!(" + XTALK {v}"));
            }
            for r in &n.routes {
                write_route(&mut s, r);
            }
            raw_clauses(&mut s, &n.extras);
            properties(&mut s, &n.properties);
            out.push_str(&s);
            out.push_str(" ;\n");
        }
        out.push_str("END NETS\n");
    }
    if !def.groups.is_empty() {
        out.push_str(&format!("GROUPS {} ;\n", def.groups.len()));
        for g in &def.groups {
            let mut s = format!("  - {}", g.name);
            for m in &g.members {
                s.push(' ');
                s.push_str(m);
            }
            if let Some(r) = &g.region {
                s.push_str(&format!(" + REGION {r}"));
            }
            raw_clauses(&mut s, &g.extras);
            properties(&mut s, &g.properties);
            out.push_str(&s);
            out.push_str(" ;\n");
        }
        out.push_str("END GROUPS\n");
    }
    for e in &def.extras {
        write_raw(&mut out, e);
    }
    out.push_str("END DESIGN\n");
    out
}

// ---------------------------------------------------------------------------
// From a netlist
// ---------------------------------------------------------------------------

/// Options for [`from_netlist`].
#[derive(Clone, Debug, PartialEq)]
pub struct DefOptions {
    /// `DESIGN` name; the top module's name when `None`.
    pub design: Option<String>,
    /// `UNITS DISTANCE MICRONS`.
    pub units: i64,
    /// `DIEAREA`, when the floorplan is already known.
    pub die_area: Option<Rect>,
    /// The characters around a bus index, `[]` by default.
    pub bus_bit_chars: String,
    /// The hierarchy divider, `/` by default.
    pub divider_char: String,
    /// The cell attribute naming the library cell; `lib_cell` by default.
    pub lib_cell_attr: String,
}

impl Default for DefOptions {
    fn default() -> Self {
        DefOptions {
            design: None,
            units: 1000,
            die_area: None,
            bus_bit_chars: "[]".to_string(),
            divider_char: "/".to_string(),
            lib_cell_attr: "lib_cell".to_string(),
        }
    }
}

/// One bit of a net, the unit of DEF connectivity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NetBit {
    net: NetId,
    bit: u32,
}

/// Builds an unplaced DEF from the netlist of module `top`.
///
/// - Every cell whose attributes carry `opts.lib_cell_attr` (a string
///   naming the library cell), every [`CellKind::Blackbox`] cell, and
///   every instance of a black-box or unresolved module becomes a
///   `COMPONENT`. Other cells (generic gates the mapper has not replaced)
///   are reported as warnings and skipped.
/// - Multi-bit nets and ports are split into bits named `name[i]` using
///   `opts.bus_bit_chars`; a cell port wider than one bit is likewise
///   named `port[i]`.
/// - Every port bit becomes a `PIN` connected to its net through a `( PIN
///   name )` entry. Constant bits driving a cell input are skipped with a
///   warning: tying them off is the mapper's job.
///
/// Output order is deterministic: components in cell/instance order, nets
/// in net order, pins in port order.
pub fn from_netlist(
    design: &Design,
    top: ModuleId,
    opts: &DefOptions,
    diags: &mut Diagnostics,
) -> Def {
    let module = design.module(top);
    let (open, close) = bus_chars(&opts.bus_bit_chars);
    let bit_name = |net: NetId, bit: u32| -> String {
        let n = &module.nets[net];
        if n.ty.width().unwrap_or(1) <= 1 {
            n.name.as_str().to_string()
        } else {
            format!("{}{open}{bit}{close}", n.name)
        }
    };
    let port_bit_name = |port: &str, width: u32, bit: u32| -> String {
        if width <= 1 {
            port.to_string()
        } else {
            format!("{port}{open}{bit}{close}")
        }
    };

    let mut def = Def {
        version: Some("5.8".to_string()),
        divider_char: Some(opts.divider_char.clone()),
        bus_bit_chars: Some(opts.bus_bit_chars.clone()),
        design: opts
            .design
            .clone()
            .unwrap_or_else(|| module.name.as_str().to_string()),
        units: Some(opts.units),
        ..Def::default()
    };
    if let Some(r) = opts.die_area {
        def.die_area = vec![Point { x: r.x1, y: r.y1 }, Point { x: r.x2, y: r.y2 }];
    }

    let mut conns: BTreeMap<NetBit, Vec<Connection>> = BTreeMap::new();
    let mut connect = |bit: NetBit, conn: Connection| conns.entry(bit).or_default().push(conn);

    // Ports first so the `PIN` entries come first on each net.
    for port in &module.ports {
        let width = module.nets[port.net].ty.width().unwrap_or(1);
        let dir = match port.dir {
            PortDir::In => PinDirection::Input,
            PortDir::Out => PinDirection::Output,
            PortDir::InOut => PinDirection::Inout,
        };
        for bit in 0..width.max(1) {
            let name = port_bit_name(port.name.as_str(), width, bit);
            let net = bit_name(port.net, bit);
            def.pins.push(Pin::new(name.clone(), net, dir));
            connect(NetBit { net: port.net, bit }, Connection::io(name));
        }
    }

    for (_, cell) in module.cells.iter() {
        let master = match (cell.attrs.get(&opts.lib_cell_attr), &cell.kind) {
            (Some(v), _) if v.as_str().is_some() => v.as_str().unwrap_or_default().to_string(),
            (_, CellKind::Blackbox(name)) => name.as_str().to_string(),
            _ => {
                diags.push(
                    Diagnostic::warning(format!(
                        "cell `{}` is a generic `{}` without a `{}` attribute; not emitted",
                        cell.name,
                        cell.kind.keyword(),
                        opts.lib_cell_attr
                    ))
                    .with_span(cell.span),
                );
                continue;
            }
        };
        def.components
            .push(Component::new(cell.name.as_str(), master));
        for (port, expr) in &cell.inputs {
            let bits = expr_bits(module, *expr);
            let width = u32::try_from(bits.len()).unwrap_or(u32::MAX);
            for (i, b) in bits.iter().enumerate() {
                let i = u32::try_from(i).unwrap_or(u32::MAX);
                match b {
                    Some(bit) => connect(
                        *bit,
                        Connection::new(cell.name.as_str(), port_bit_name(port.as_str(), width, i)),
                    ),
                    None => diags.push(
                        Diagnostic::warning(format!(
                            "input `{port}` of cell `{}` is driven by a constant or an \
                             unsupported expression; bit {i} left unconnected",
                            cell.name
                        ))
                        .with_span(module.expr(*expr).span),
                    ),
                }
            }
        }
        for (port, net) in &cell.outputs {
            let width = module.nets[*net].ty.width().unwrap_or(1).max(1);
            for bit in 0..width {
                connect(
                    NetBit { net: *net, bit },
                    Connection::new(cell.name.as_str(), port_bit_name(port.as_str(), width, bit)),
                );
            }
        }
    }

    for (_, inst) in module.instances.iter() {
        let master = match &inst.module {
            ModuleRef::Unresolved(name) => name.as_str().to_string(),
            ModuleRef::Resolved(id) => {
                let target = design.module(*id);
                if !target.blackbox {
                    diags.push(
                        Diagnostic::warning(format!(
                            "instance `{}` of hierarchical module `{}` is not emitted; \
                             flatten the design first",
                            inst.name, target.name
                        ))
                        .with_span(inst.span),
                    );
                    continue;
                }
                target.name.as_str().to_string()
            }
        };
        def.components
            .push(Component::new(inst.name.as_str(), master));
        for (port, expr) in &inst.connections {
            let bits = expr_bits(module, *expr);
            let width = u32::try_from(bits.len()).unwrap_or(u32::MAX);
            for (i, b) in bits.iter().enumerate() {
                let i = u32::try_from(i).unwrap_or(u32::MAX);
                if let Some(bit) = b {
                    connect(
                        *bit,
                        Connection::new(inst.name.as_str(), port_bit_name(port.as_str(), width, i)),
                    );
                }
            }
        }
    }

    for (bit, connections) in conns {
        def.nets.push(Net {
            connections,
            ..Net::new(bit_name(bit.net, bit.bit))
        });
    }
    def
}

/// Splits `[]` (or any two-character string) into its open and close
/// characters, defaulting to square brackets.
fn bus_chars(s: &str) -> (char, char) {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(a), Some(b)) => (a, b),
        _ => ('[', ']'),
    }
}

/// The net bits an expression selects, least significant first; `None`
/// for bits that are constants or come from an expression DEF cannot
/// represent.
fn expr_bits(module: &Module, expr: ExprId) -> Vec<Option<NetBit>> {
    let e = module.expr(expr);
    match &e.kind {
        ExprKind::Net(net) => {
            let width = module.nets[*net].ty.width().unwrap_or(1).max(1);
            (0..width)
                .map(|bit| Some(NetBit { net: *net, bit }))
                .collect()
        }
        ExprKind::Slice { base, hi, lo } => {
            let bits = expr_bits(module, *base);
            let lo = usize::try_from(*lo).unwrap_or(usize::MAX);
            let hi = usize::try_from(*hi).unwrap_or(usize::MAX);
            (lo..=hi).map(|i| bits.get(i).copied().flatten()).collect()
        }
        ExprKind::Concat(parts) => {
            // The first part is the most significant, so build from the
            // last part up.
            let mut out = Vec::new();
            for part in parts.iter().rev() {
                out.extend(expr_bits(module, *part));
            }
            out
        }
        ExprKind::Replicate { count, expr } => {
            let bits = expr_bits(module, *expr);
            let mut out = Vec::new();
            for _ in 0..*count {
                out.extend(bits.iter().copied());
            }
            out
        }
        ExprKind::Index { base, index } => {
            match module.expr(*index).as_const().and_then(|c| c.to_u64()) {
                Some(i) => {
                    let bits = expr_bits(module, *base);
                    vec![
                        usize::try_from(i)
                            .ok()
                            .and_then(|i| bits.get(i).copied().flatten()),
                    ]
                }
                None => vec![None; e.width().unwrap_or(1) as usize],
            }
        }
        _ => vec![None; usize::try_from(e.width().unwrap_or(1)).unwrap_or(1)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    const DEF: &str = r#"
VERSION 5.8 ;
DIVIDERCHAR "/" ;
BUSBITCHARS "[]" ;
DESIGN counter ;
UNITS DISTANCE MICRONS 1000 ;
PROPERTYDEFINITIONS
  COMPONENT flow STRING ;
  NET weightClass INTEGER RANGE 0 10 ;
END PROPERTYDEFINITIONS
DIEAREA ( 0 0 ) ( 20000 20000 ) ;
ROW ROW_0 unithd 0 0 N DO 40 BY 1 STEP 460 0 ;
ROW ROW_1 unithd 0 2720 FS DO 40 BY 1 STEP 460 0 ;
TRACKS X 170 DO 58 STEP 340 LAYER met1 ;
TRACKS Y 230 DO 43 STEP 460 LAYER met1 met2 ;
GCELLGRID X 0 DO 5 STEP 4000 ;
GCELLGRID Y 0 DO 5 STEP 4000 ;
VIAS 2 ;
  - via1_2 + RECT met1 ( -145 -100 ) ( 145 100 ) + RECT via ( -75 -75 ) ( 75 75 ) + RECT met2 ( -100 -145 ) ( 100 145 ) ;
  - gen_via + VIARULE M1M2_PR + CUTSIZE 150 150 + LAYERS met1 via met2 + CUTSPACING 170 170 + ENCLOSURE 55 30 55 30 + ROWCOL 1 2 ;
END VIAS
REGIONS 1 ;
  - core ( 1000 1000 ) ( 19000 19000 ) + TYPE FENCE ;
END REGIONS
COMPONENTS 3 ;
  - u1 sky130_fd_sc_hd__inv_1 + PLACED ( 1380 2720 ) N + PROPERTY flow synth ;
  - u2 sky130_fd_sc_hd__nand2_1 + FIXED ( 2760 2720 ) FS + WEIGHT 2 + REGION core ;
  - u3 sky130_fd_sc_hd__dfxtp_1 + UNPLACED + SOURCE NETLIST + HALO SOFT 10 10 10 10 ;
END COMPONENTS
PINS 3 ;
  - clk + NET clk + DIRECTION INPUT + USE CLOCK + LAYER met2 ( -70 0 ) ( 70 400 ) + FIXED ( 5000 0 ) N ;
  - q + NET q + DIRECTION OUTPUT + USE SIGNAL + PORT + LAYER met2 ( -70 0 ) ( 70 400 ) + PLACED ( 10000 20000 ) S ;
  - d[0] + NET d[0] + DIRECTION INPUT + SPECIAL + ANTENNAPINGATEAREA 200 LAYER met1 ;
END PINS
BLOCKAGES 2 ;
  - LAYER met1 + PUSHDOWN + SPACING 100 RECT ( 100 100 ) ( 200 200 ) ;
  - PLACEMENT + SOFT + COMPONENT u1 RECT ( 0 0 ) ( 500 500 ) POLYGON ( 0 0 ) ( 1 0 ) ( 1 1 ) ;
END BLOCKAGES
SPECIALNETS 1 ;
  - VPWR ( * VPWR ) + USE POWER + ROUTED met1 480 + SHAPE STRIPE ( 0 2720 ) ( 20000 * ) ;
END SPECIALNETS
NETS 3 ;
  - clk ( PIN clk ) ( u3 CLK ) + USE CLOCK ;
  - n1 ( u1 Y ) ( u2 A + SYNTHESIZED ) + ROUTED met1 ( 1500 3000 ) ( 2500 * ) via1_2 NEW met2 ( 2500 3000 0 ) ( 2500 4000 ) + WEIGHT 3 + PROPERTY weightClass 5 ;
  - q ( PIN q ) ( u3 Q ) + FIXED met1 TAPER ( 100 100 ) MASK 1 ( 200 100 ) MASK 2 via1_2 N + SHIELDNET VPWR ;
END NETS
GROUPS 1 ;
  - g1 u1 u2 + REGION core ;
END GROUPS
SCANCHAINS 1 ;
  - sc + START PIN scan_in + STOP PIN scan_out ;
END SCANCHAINS
END DESIGN
"#;

    fn parse(text: &str) -> (Def, Diagnostics, SourceMap) {
        let mut map = SourceMap::new();
        let id = map.add("t.def", text).unwrap();
        let mut diags = Diagnostics::new();
        let def = parse_def(text, id, &mut diags);
        (def, diags, map)
    }

    #[test]
    fn reads_the_typed_view() {
        let (def, diags, map) = parse(DEF);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        assert_eq!(def.design, "counter");
        assert_eq!(def.units, Some(1000));
        assert_eq!(def.property_definitions.len(), 2);
        assert_eq!(def.property_definitions[1].range, Some((0.0, 10.0)));
        assert_eq!(
            def.die_rect(),
            Some(Rect {
                x1: 0,
                y1: 0,
                x2: 20000,
                y2: 20000
            })
        );
        assert_eq!(def.rows.len(), 2);
        assert_eq!(def.rows[1].orient, Orient::FS);
        assert_eq!(def.rows[1].repeat.unwrap().step_x, 460);
        assert_eq!(def.tracks[1].layers, vec!["met1", "met2"]);
        assert_eq!(def.gcell_grids[0].num, 5);
        assert_eq!(def.vias[0].shapes.len(), 3);
        assert_eq!(def.vias[1].via_rule.as_ref().unwrap().row_col, Some((1, 2)));
        assert_eq!(def.regions[0].kind.as_deref(), Some("FENCE"));

        let u1 = def.component("u1").unwrap();
        assert_eq!(u1.master, "sky130_fd_sc_hd__inv_1");
        assert_eq!(u1.placement, Placement::placed(1380, 2720, Orient::N));
        assert_eq!(
            u1.properties[0].value,
            PropertyValue::String("synth".into())
        );
        let u2 = def.component("u2").unwrap();
        assert_eq!(u2.placement.status, PlacementStatus::Fixed);
        assert_eq!(u2.weight, Some(2));
        assert_eq!(u2.region.as_deref(), Some("core"));
        let u3 = def.component("u3").unwrap();
        assert_eq!(u3.placement.status, PlacementStatus::Unplaced);
        assert!(u3.halo.unwrap().soft);

        let clk = def.pin("clk").unwrap();
        assert_eq!(clk.direction, Some(PinDirection::Input));
        assert_eq!(clk.use_.as_deref(), Some("CLOCK"));
        assert_eq!(clk.port.shapes[0].layer, "met2");
        assert_eq!(clk.port.placement.status, PlacementStatus::Fixed);
        let q = def.pin("q").unwrap();
        assert_eq!(q.ports.len(), 1);
        assert_eq!(
            q.ports[0].placement,
            Placement::placed(10000, 20000, Orient::S)
        );
        let d0 = def.pin("d[0]").unwrap();
        assert!(d0.special);
        assert_eq!(d0.extras[0].tokens[0], "ANTENNAPINGATEAREA");

        assert_eq!(def.blockages[0].kind, BlockageKind::Layer("met1".into()));
        assert_eq!(def.blockages[0].options.len(), 2);
        assert_eq!(def.blockages[1].component.as_deref(), Some("u1"));
        assert_eq!(def.blockages[1].polygons[0].len(), 3);

        assert_eq!(def.special_nets[0].connections[0].component, "*");
        assert_eq!(def.special_nets[0].extras.len(), 2);

        let n1 = def.net("n1").unwrap();
        assert_eq!(n1.connections.len(), 2);
        assert!(n1.connections[1].synthesized);
        assert_eq!(n1.routes[0].wires.len(), 2);
        assert_eq!(n1.routes[0].wires[0].items.len(), 3);
        assert_eq!(
            n1.routes[0].wires[0].items[1],
            RouteItem::Point {
                x: Some(2500),
                y: None,
                ext: None,
                mask: None
            }
        );
        assert_eq!(n1.weight, Some(3));
        let qn = def.net("q").unwrap();
        assert!(qn.connections[0].is_io());
        assert_eq!(qn.routes[0].status, RouteStatus::Fixed);
        assert_eq!(qn.routes[0].wires[0].taper, Some(None));
        assert!(matches!(
            qn.routes[0].wires[0].items[2],
            RouteItem::Via {
                orient: Some(Orient::N),
                mask: Some(2),
                ..
            }
        ));
        assert_eq!(qn.extras[0].tokens, vec!["SHIELDNET", "VPWR"]);
        assert_eq!(def.groups[0].members, vec!["u1", "u2"]);
        assert_eq!(def.extras.len(), 1);
        assert!(def.extras[0].block);
    }

    #[test]
    fn write_then_parse_is_a_fixed_point() {
        let (def, _, _) = parse(DEF);
        let text = write_def(&def);
        let (again, diags, map) = parse(&text);
        assert!(diags.is_empty(), "{}\n{text}", diags.render(&map));
        assert_eq!(again, def);
        assert_eq!(write_def(&again), text);
        assert!(text.contains("COMPONENTS 3 ;"));
        assert!(text.contains(
            "+ ROUTED met1 ( 1500 3000 ) ( 2500 * ) via1_2 NEW met2 ( 2500 3000 0 ) ( 2500 4000 )"
        ));
        assert!(text.ends_with("END DESIGN\n"));
    }

    #[test]
    fn errors_are_reported_per_item() {
        let text = "DESIGN x ;\nCOMPONENTS 2 ;\n  - u1 inv + PLACED ( 1 ) N ;\n  - u2 inv ;\nEND COMPONENTS\nEND DESIGN\n";
        let (def, diags, map) = parse(text);
        assert_eq!(diags.error_count(), 1, "{}", diags.render(&map));
        assert!(
            diags.render(&map).contains("t.def:3:25"),
            "{}",
            diags.render(&map)
        );
        assert_eq!(def.components.len(), 1);
        assert_eq!(def.components[0].name, "u2");
    }

    const NETLIST: &str = r#"
top mapped

module mapped
  net %clk u1 wire
  net %d u2 wire
  net %q u2 wire
  net %n1 u1 wire
  net %n2 u1 wire
  port clk in %clk
  port d in %d
  port q out %q
  attr lib_cell = "sky130_fd_sc_hd__inv_1"
  cell i0 blackbox unused (A=%d[0:0]) -> (Y=%n1)
  cell nand0 blackbox sky130_fd_sc_hd__nand2_1 (A=%n1, B=%d[1:1]) -> (Y=%n2)
  attr lib_cell = "sky130_fd_sc_hd__dfxtp_1"
  cell ff0 dff pos (clk=%clk, d={%n2, %n1}) -> (q=%q)
  cell add0 add (a=%d, b=2'd1) -> (y=%d)
  instance pad of io_pad (PAD=%clk)
end
"#;

    #[test]
    fn netlist_to_unplaced_def() {
        let mut map = SourceMap::new();
        let id = map.add("mapped.rtl", NETLIST).unwrap();
        let design =
            Design::parse_text(NETLIST, id).unwrap_or_else(|d| panic!("{}", d.render(&map)));
        let top = design.top.unwrap();
        let mut diags = Diagnostics::new();
        let def = from_netlist(&design, top, &DefOptions::default(), &mut diags);
        assert_eq!(diags.warning_count(), 1, "{}", diags.render(&map));
        assert!(
            diags
                .render(&map)
                .contains("cell `add0` is a generic `add`")
        );

        assert_eq!(def.design, "mapped");
        let comps: Vec<(&str, &str)> = def
            .components
            .iter()
            .map(|c| (c.name.as_str(), c.master.as_str()))
            .collect();
        assert_eq!(
            comps,
            vec![
                ("i0", "sky130_fd_sc_hd__inv_1"),
                ("nand0", "sky130_fd_sc_hd__nand2_1"),
                ("ff0", "sky130_fd_sc_hd__dfxtp_1"),
                ("pad", "io_pad"),
            ]
        );
        assert!(
            def.components
                .iter()
                .all(|c| c.placement.status == PlacementStatus::Unplaced)
        );
        let pins: Vec<&str> = def.pins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(pins, vec!["clk", "d[0]", "d[1]", "q[0]", "q[1]"]);
        assert_eq!(
            def.pin("q[1]").unwrap().direction,
            Some(PinDirection::Output)
        );

        let net = |n: &str| -> Vec<String> {
            def.net(n)
                .unwrap_or_else(|| panic!("no net {n}"))
                .connections
                .iter()
                .map(|c| format!("{} {}", c.component, c.pin))
                .collect()
        };
        assert_eq!(net("clk"), vec!["PIN clk", "ff0 clk", "pad PAD"]);
        assert_eq!(net("d[0]"), vec!["PIN d[0]", "i0 A"]);
        assert_eq!(net("d[1]"), vec!["PIN d[1]", "nand0 B"]);
        assert_eq!(net("n1"), vec!["i0 Y", "nand0 A", "ff0 d[0]"]);
        assert_eq!(net("n2"), vec!["nand0 Y", "ff0 d[1]"]);
        assert_eq!(net("q[0]"), vec!["PIN q[0]", "ff0 q[0]"]);

        let text = write_def(&def);
        let (again, diags, map2) = parse(&text);
        assert!(diags.is_empty(), "{}", diags.render(&map2));
        assert_eq!(again, def);
    }
}
