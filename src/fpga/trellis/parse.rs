//! One reader per file of the Project Trellis database, and the two rules
//! the files imply and do not state.
//!
//! Four of the five files are JSON and are read by [`crate::json`]; the
//! fifth, `bits.db`, is a line-oriented text format read by
//! [`TileDatabase::parse`].

use std::collections::{BTreeMap, BTreeSet};

use crate::json::Json;

use super::super::ecp5::{FrameFormat, TileWindow};
use super::TrellisError;

/// One part, from `devices.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The family directory the part's files live in (`ECP5`).
    pub family: String,
    /// The part (`LFE5U-12F`).
    pub name: String,
    /// Its JTAG IDCODE.
    pub idcode: u32,
    /// The shape of its configuration memory.
    pub format: FrameFormat,
    /// The highest grid row, so the grid is `max_row + 1` rows tall.
    pub max_row: u32,
    /// The highest grid column.
    pub max_col: u32,
    /// The packages its pinout file describes, in file order.
    pub packages: Vec<String>,
}

impl DeviceInfo {
    /// Grid columns.
    pub fn width(&self) -> u32 {
        self.max_col + 1
    }

    /// Grid rows.
    pub fn height(&self) -> u32 {
        self.max_row + 1
    }

    /// The prefix a wire carries when it exists only on dice of this size.
    ///
    /// Project Trellis shares one `bits.db` per tile *type* between every
    /// ECP5, and the handful of wires that exist on only some of them are
    /// spelled `25K_…`, `45K_…` or `85K_…`. The 12F and the 25F are the
    /// same die, so both are `25K_`.
    pub fn chip_prefix(&self) -> &'static str {
        let n = &self.name;
        if n.contains("12F") || n.contains("25F") {
            "25K_"
        } else if n.contains("45F") {
            "45K_"
        } else if n.contains("85F") {
            "85K_"
        } else {
            ""
        }
    }
}

/// Every part `devices.json` describes, in file order.
///
/// # Errors
///
/// [`TrellisError::Malformed`] when the file is not the shape
/// `devices.json` has.
pub fn devices(text: &str) -> Result<Vec<DeviceInfo>, TrellisError> {
    let root = Json::parse(text).map_err(|e| TrellisError::Malformed {
        path: "devices.json".to_owned(),
        what: e.to_string(),
    })?;
    let bad = |what: &str| TrellisError::Malformed {
        path: "devices.json".to_owned(),
        what: what.to_owned(),
    };
    let families = root
        .get("families")
        .and_then(Json::as_object)
        .ok_or_else(|| bad("no `families` object"))?;
    let mut out = Vec::new();
    for (family, body) in families {
        let Some(devices) = body.get("devices").and_then(Json::as_object) else {
            continue;
        };
        for (name, dev) in devices {
            // A part with no `idcode` is one Project Trellis describes only
            // through its variants; nothing here needs those.
            let Some(idcode) = dev.get("idcode").and_then(Json::as_str) else {
                continue;
            };
            let idcode =
                parse_u32(idcode).ok_or_else(|| bad("an `idcode` that is not a number"))?;
            let field = |key: &str| -> Result<u32, TrellisError> {
                dev.get(key)
                    .and_then(Json::as_u32)
                    .ok_or_else(|| bad(&format!("{name} has no `{key}`")))
            };
            out.push(DeviceInfo {
                family: family.clone(),
                name: name.clone(),
                idcode,
                format: FrameFormat {
                    frames: field("frames")?,
                    bits_per_frame: field("bits_per_frame")?,
                    pad_bits_before: field("pad_bits_before_frame")?,
                    pad_bits_after: field("pad_bits_after_frame")?,
                },
                max_row: field("max_row")?,
                max_col: field("max_col")?,
                packages: dev
                    .get("packages")
                    .and_then(Json::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Json::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    if out.is_empty() {
        return Err(bad("no part in the file has an `idcode`"));
    }
    Ok(out)
}

/// A `0x…` or decimal integer, which is how `devices.json` spells an
/// IDCODE (JSON has no hexadecimal, so it is a string).
fn parse_u32(text: &str) -> Option<u32> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// One tile of one part, from `tilegrid.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileEntry {
    /// The key, `<lattice name>:<type>`, which is unique where the Lattice
    /// name alone is not.
    pub key: String,
    /// The tile type, which names its `tiledata/<type>/bits.db`.
    pub ty: String,
    /// Its grid row, from the `R<row>C<col>` in its Lattice name.
    pub row: u32,
    /// Its grid column.
    pub col: u32,
    /// Where its bits live.
    pub window: TileWindow,
    /// The sites it holds, as the database names them.
    pub sites: Vec<String>,
}

/// Every tile of one part, sorted by key so a position's members come out
/// in the same order on every run.
///
/// **A grid position holds up to six tiles.** That is the one structural
/// surprise of this database: Project Trellis splits a position of the
/// ECP5's fabric into separate tiles with separate bit regions, and they
/// share one wire namespace. See [`super`] for what is done about it.
///
/// # Errors
///
/// [`TrellisError::Malformed`] when the file is not an object of tiles, or
/// a tile's name carries no `R<row>C<col>`.
pub fn tilegrid(text: &str, path: &str) -> Result<Vec<TileEntry>, TrellisError> {
    let root = Json::parse(text).map_err(|e| TrellisError::Malformed {
        path: path.to_owned(),
        what: e.to_string(),
    })?;
    let bad = |what: String| TrellisError::Malformed {
        path: path.to_owned(),
        what,
    };
    let tiles = root
        .as_object()
        .ok_or_else(|| bad("the root is not an object of tiles".to_owned()))?;
    let mut out = Vec::with_capacity(tiles.len());
    for (key, body) in tiles {
        let lattice = key.split(':').next().unwrap_or(key);
        let (row, col) =
            row_col(lattice).ok_or_else(|| bad(format!("`{key}` carries no R<row>C<col>")))?;
        let field = |name: &str| -> Result<u32, TrellisError> {
            body.get(name)
                .and_then(Json::as_u32)
                .ok_or_else(|| bad(format!("`{key}` has no `{name}`")))
        };
        let ty = body
            .get("type")
            .and_then(Json::as_str)
            .ok_or_else(|| bad(format!("`{key}` has no `type`")))?
            .to_owned();
        out.push(TileEntry {
            key: key.clone(),
            ty,
            row,
            col,
            window: TileWindow {
                start_frame: field("start_frame")?,
                start_bit: field("start_bit")?,
                // The names are the database's and they are the other way
                // round from what they look like: `cols` counts frames and
                // `rows` counts bits within a frame. libtrellis reads them
                // as `num_frames` and `bits_per_frame`, and this follows it.
                frames: field("cols")?,
                bits: field("rows")?,
            },
            sites: body
                .get("sites")
                .and_then(Json::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.get("name").and_then(Json::as_str))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

/// The `R<row>C<col>` a Lattice tile name carries.
///
/// Every ECP5 tile name has one; libtrellis has a dozen further patterns
/// for the MachXO families and none of them is needed here, so a name
/// without one is an error rather than a guess.
pub fn row_col(name: &str) -> Option<(u32, u32)> {
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'R' {
            let mut j = i + 1;
            let start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start && j < bytes.len() && bytes[j] == b'C' {
                let row: u32 = name[start..j].parse().ok()?;
                let mut k = j + 1;
                let cstart = k;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                if k > cstart {
                    let col: u32 = name[cstart..k].parse().ok()?;
                    return Some((row, col));
                }
            }
        }
        i += 1;
    }
    None
}

/// One package's ball map, from `iodb.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pinout {
    /// The package, as `iodb.json` spells it (`CABGA256`).
    pub package: String,
    /// Ball name to the grid position and PIO letter it reaches.
    pub balls: BTreeMap<String, (u32, u32, String)>,
}

/// Every package `iodb.json` describes.
///
/// # Errors
///
/// [`TrellisError::Malformed`] when the file is not an object of packages
/// of balls.
pub fn iodb(text: &str, path: &str) -> Result<Vec<Pinout>, TrellisError> {
    let root = Json::parse(text).map_err(|e| TrellisError::Malformed {
        path: path.to_owned(),
        what: e.to_string(),
    })?;
    let packages = root
        .get("packages")
        .and_then(Json::as_object)
        .ok_or_else(|| TrellisError::Malformed {
            path: path.to_owned(),
            what: "no `packages` object".to_owned(),
        })?;
    let mut out = Vec::new();
    for (package, body) in packages {
        let mut balls = BTreeMap::new();
        for (ball, entry) in body.as_object().unwrap_or(&[]) {
            let (Some(row), Some(col), Some(pio)) = (
                entry.get("row").and_then(Json::as_u32),
                entry.get("col").and_then(Json::as_u32),
                entry.get("pio").and_then(Json::as_str),
            ) else {
                continue;
            };
            balls.insert(ball.clone(), (row, col, pio.to_owned()));
        }
        out.push(Pinout {
            package: package.clone(),
            balls,
        });
    }
    Ok(out)
}

/// Which IO bank each PIO of the die belongs to, from `iodb.json`'s
/// `pio_metadata`.
///
/// This is a property of the **die**, not of a package: the array sits
/// beside `packages` rather than inside one, and a ball of any package
/// reaches the bank its `(row, col, pio)` is in. Hence a map keyed by
/// position rather than by ball.
///
/// It is needed because a bank has one configuration bit of its own that
/// no pad's tiles hold — `BANK.VCCIO`, in a `BANKREF<bank>` tile far from
/// the pad — and nothing else in the database says which bank a pad is
/// wired to.
///
/// An entry without the three position fields or without `bank` is
/// skipped. A file with no `pio_metadata` at all yields an empty map,
/// which is not an error here: the caller is what decides whether a pad
/// whose bank is unknown can be configured.
///
/// # Errors
///
/// [`TrellisError::Malformed`] when the file is not JSON.
pub fn pio_banks(
    text: &str,
    path: &str,
) -> Result<BTreeMap<(u32, u32, String), u32>, TrellisError> {
    let root = Json::parse(text).map_err(|e| TrellisError::Malformed {
        path: path.to_owned(),
        what: e.to_string(),
    })?;
    let mut out = BTreeMap::new();
    for entry in root
        .get("pio_metadata")
        .and_then(Json::as_array)
        .unwrap_or(&[])
    {
        let (Some(row), Some(col), Some(pio), Some(bank)) = (
            entry.get("row").and_then(Json::as_u32),
            entry.get("col").and_then(Json::as_u32),
            entry.get("pio").and_then(Json::as_str),
            entry.get("bank").and_then(Json::as_u32),
        ) else {
            continue;
        };
        out.insert((row, col, pio.to_owned()), bank);
    }
    Ok(out)
}

/// One configuration bit of a tile, as `bits.db` writes it: `F<frame>B<bit>`
/// for a bit that must be **set** and `!F<frame>B<bit>` for one that must be
/// **clear**.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DbBit {
    /// The frame inside the tile.
    pub frame: u32,
    /// The bit inside that frame.
    pub bit: u32,
    /// True when the feature wants the bit clear rather than set.
    pub inverted: bool,
}

/// One `.mux` source: the sink, the source, and the bits that select it.
pub type MuxBits = (String, String, Vec<DbBit>);

/// One `.config` word: its name, its default as a binary string, and one
/// bit group per bit of the value, bit 0 first.
pub type WordBits = (String, Option<String>, Vec<Vec<DbBit>>);

/// One `.config_enum` field: its name, its default value, and one
/// `(value, bits)` pair per setting, in file order.
pub type EnumBits = (String, Option<String>, Vec<(String, Vec<DbBit>)>);

/// One tile type's `bits.db`.
///
/// Four record kinds, in the order the file writes them:
///
/// - `.mux <sink>` and then `<source> <bits…>`: a programmable connection.
///   A source with **no** bits is the state the mux is in when none of its
///   bits is set, and that is still a connection — see
///   [`TileDatabase::arcs`], which is where this was got wrong once.
/// - `.config <name> <default>` and then one bit group per bit of the
///   word, bit 0 first: a multi-bit field such as a lookup table's
///   `INIT`.
/// - `.config_enum <name> [default]` and then `<value> <bits…>`: a field
///   with named settings.
/// - `.fixed_conn <sink> <source>`: a connection that is simply there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileDatabase {
    /// The programmable connections, in file order.
    pub muxes: Vec<MuxBits>,
    /// The multi-bit fields, in file order.
    pub words: Vec<WordBits>,
    /// The named fields, in file order.
    pub enums: Vec<EnumBits>,
    /// The unconditional connections, as `(sink, source)`.
    pub fixed: Vec<(String, String)>,
}

impl TileDatabase {
    /// Reads one `bits.db`.
    ///
    /// # Errors
    ///
    /// [`TrellisError::Malformed`] for a bit that is not `[!]F<n>B<n>`, or
    /// a line of bits before any record has been opened.
    pub fn parse(text: &str, path: &str) -> Result<TileDatabase, TrellisError> {
        let bad = |what: String| TrellisError::Malformed {
            path: path.to_owned(),
            what,
        };
        let mut out = TileDatabase::default();
        // Which record the following lines belong to.
        enum Open {
            None,
            Mux(String),
            Word,
            Enum,
        }
        let mut open = Open::None;
        for (number, line) in text.lines().enumerate() {
            let line = match line.find('#') {
                Some(at) => &line[..at],
                None => line,
            };
            let line = line.trim();
            if line.is_empty() {
                open = Open::None;
                continue;
            }
            let mut words = line.split_whitespace();
            let head = words.next().unwrap_or_default();
            match head {
                ".mux" => {
                    let sink = words.next().unwrap_or_default().to_owned();
                    open = Open::Mux(sink);
                }
                ".config" => {
                    let name = words.next().unwrap_or_default().to_owned();
                    let default = words.next().map(str::to_owned);
                    out.words.push((name, default, Vec::new()));
                    open = Open::Word;
                }
                ".config_enum" => {
                    let name = words.next().unwrap_or_default().to_owned();
                    let default = words.next().map(str::to_owned);
                    out.enums.push((name, default, Vec::new()));
                    open = Open::Enum;
                }
                ".fixed_conn" => {
                    let sink = words.next().unwrap_or_default().to_owned();
                    let source = words.next().unwrap_or_default().to_owned();
                    out.fixed.push((sink, source));
                    open = Open::None;
                }
                other => match &open {
                    Open::Mux(sink) => {
                        let bits = bits(words, path, number)?;
                        out.muxes.push((sink.clone(), other.to_owned(), bits));
                    }
                    Open::Word => {
                        let mut all = vec![other];
                        all.extend(words);
                        let bits = bits(all.into_iter(), path, number)?;
                        if let Some(last) = out.words.last_mut() {
                            last.2.push(bits);
                        }
                    }
                    Open::Enum => {
                        let bits = bits(words, path, number)?;
                        if let Some(last) = out.enums.last_mut() {
                            last.2.push((other.to_owned(), bits));
                        }
                    }
                    Open::None => {
                        return Err(bad(format!(
                            "line {} (`{line}`) belongs to no record",
                            number + 1
                        )));
                    }
                },
            }
        }
        Ok(out)
    }

    /// Every `.mux` source, **including the ones with no bits**.
    ///
    /// # The mistake this used to make
    ///
    /// This returned only the sources with bits, on the grounds that a
    /// bitless source is "the state of the mux when nothing drives it"
    /// rather than a connection. That reading is wrong, and it is wrong in
    /// a way that no structural check would have caught: it disconnects
    /// every lookup table in the part from the interconnect.
    ///
    /// A `PLC2`'s output mux is
    ///
    /// ```text
    /// .mux F0
    /// F0_SLICE -
    /// F5A_SLICE F8B10
    /// ```
    ///
    /// `F0` is the routing wire and `F0_SLICE` is the lookup table's
    /// output. With `F8B10` clear the mux carries `F0_SLICE`, which is the
    /// *usual* case and the only way a LUT reaches the fabric; `F8B10` set
    /// switches it to the carry chain's cascade output instead. So the
    /// bitless source is the connection a design almost always wants, and
    /// dropping it left every LUT output driving nothing. It was found by
    /// walking the graph from a pad to a LUT and back before any of this
    /// was written in Rust: the walk reached `F0` and never `F0_SLICE`.
    ///
    /// A bitless source becomes a pip with an empty bit list, which is
    /// what [`Arch`](crate::fpga::arch::Arch) already means by a connection
    /// that is always there, and it is what `ecppack` writes for one:
    /// `ChipConfig::add_arc` of a bitless arc sets no bits.
    ///
    /// The four `…BOUNCE` sources are bitless too and stay harmless: no
    /// `.mux` has a `…BOUNCE` as its *sink*, so nothing drives one and a
    /// router can never route through one.
    pub fn arcs(&self) -> impl Iterator<Item = &MuxBits> {
        self.muxes.iter()
    }

    /// The named field of that name.
    pub fn enum_bits(&self, name: &str, value: &str) -> Option<&[DbBit]> {
        self.enums
            .iter()
            .find(|(n, _, _)| n == name)
            .and_then(|(_, _, options)| options.iter().find(|(v, _)| v == value))
            .map(|(_, bits)| bits.as_slice())
    }

    /// True when the type declares a named field called `name`.
    pub fn has_enum(&self, name: &str) -> bool {
        self.enums.iter().any(|(n, _, _)| n == name)
    }

    /// The bit groups of the multi-bit field `name`, bit 0 first.
    pub fn word_bits(&self, name: &str) -> Option<&[Vec<DbBit>]> {
        self.words
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, _, bits)| bits.as_slice())
    }

    /// Every wire name the type mentions, in whatever spelling the file
    /// uses.
    pub fn wire_names(&self) -> BTreeSet<&str> {
        let mut out = BTreeSet::new();
        for (sink, source, _) in &self.muxes {
            out.insert(sink.as_str());
            out.insert(source.as_str());
        }
        for (sink, source) in &self.fixed {
            out.insert(sink.as_str());
            out.insert(source.as_str());
        }
        out
    }
}

/// A whitespace-separated list of bits, with `-` for none.
fn bits<'a>(
    words: impl Iterator<Item = &'a str>,
    path: &str,
    line: usize,
) -> Result<Vec<DbBit>, TrellisError> {
    let mut out = Vec::new();
    for word in words {
        if word == "-" {
            continue;
        }
        out.push(parse_bit(word).ok_or_else(|| TrellisError::Malformed {
            path: path.to_owned(),
            what: format!("line {}: `{word}` is not [!]F<frame>B<bit>", line + 1),
        })?);
    }
    Ok(out)
}

/// One `[!]F<frame>B<bit>`.
pub fn parse_bit(text: &str) -> Option<DbBit> {
    let (inverted, rest) = match text.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let rest = rest.strip_prefix('F')?;
    let (frame, bit) = rest.split_once('B')?;
    Some(DbBit {
        frame: frame.parse().ok()?,
        bit: bit.parse().ok()?,
        inverted,
    })
}

/// What a wire name in a `bits.db` refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireTarget {
    /// A wire of a tile, `(dx, dy)` positions from the tile that named it,
    /// under the base name. `(0, 0)` is the tile's own.
    Tile {
        /// Columns east.
        dx: i32,
        /// Rows **south**, which is the direction grid rows increase in.
        dy: i32,
        /// The name in that tile.
        name: String,
    },
    /// A wire that reaches the whole die: one node for the part.
    Global {
        /// Its name.
        name: String,
    },
}

/// What a wire name in a tile's `bits.db` refers to, or `None` when it
/// belongs to a different die.
///
/// This is the rule the database implies and does not state, and it is
/// `libtrellis`' `RoutingGraph::globalise_net_ecp5`:
///
/// 1. A name starting `25K_`, `45K_` or `85K_` exists **only** on dice of
///    that size. With the die's own prefix it is that wire without the
///    prefix; with another's it does not exist here at all, which is what
///    `None` means.
/// 2. A name starting `G_`, `L_` or `R_` is a **global**: one node for the
///    whole part. The exceptions are `G_…VPTX…`, `G_…HPBX…` and
///    `G_…HPRX…`, and every `L_`/`R_` name, which are per-tile after all —
///    they are the clock spines and branches, and one tile's is not
///    another's.
/// 3. Anything else may carry `N<n>`, `S<n>`, `W<n>` and `E<n>` prefixes,
///    joined to the base name by an underscore: `S1E1_JA0` is the wire
///    `JA0` one row south and one column east. `N` decreases the row, `S`
///    increases it, `W` decreases the column and `E` increases it.
///
/// Rule 3 is why this loader needs no join pips at all, where
/// [`super::super::xray`] spends a third of its graph edges on them: the
/// mapping from a neighbour's spelling to the wire it means is arithmetic,
/// so the wire is declared once in the tile that owns it and every
/// reference resolves straight to it.
pub fn globalise(name: &str, chip_prefix: &str) -> Option<WireTarget> {
    Some(match globalise_ref(name, chip_prefix)? {
        WireTargetRef::Tile { dx, dy, name } => WireTarget::Tile {
            dx,
            dy,
            name: name.to_owned(),
        },
        WireTargetRef::Global { name } => WireTarget::Global {
            name: name.to_owned(),
        },
    })
}

/// [`WireTarget`] that borrows the name it found rather than owning it.
///
/// The loader resolves about a million and a half wire names — every name
/// of every tile of the grid — and does it twice, once to decide which
/// position owns a name and once per pip. An owned `String` per resolution
/// is a few million allocations for nothing, since every name is a slice of
/// a `bits.db` already in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireTargetRef<'a> {
    /// A wire of the tile `(dx, dy)` positions away, under `name`.
    Tile {
        /// Columns east.
        dx: i32,
        /// Rows **south**.
        dy: i32,
        /// The name in that tile.
        name: &'a str,
    },
    /// A wire that reaches the whole die.
    Global {
        /// Its name.
        name: &'a str,
    },
}

/// [`globalise`] without the allocation; see [`WireTargetRef`].
pub fn globalise_ref<'a>(name: &'a str, chip_prefix: &str) -> Option<WireTargetRef<'a>> {
    let mut rest = name;
    for prefix in ["25K_", "45K_", "85K_"] {
        if let Some(stripped) = name.strip_prefix(prefix) {
            if prefix != chip_prefix {
                return None;
            }
            rest = stripped;
            break;
        }
    }
    if rest.starts_with("G_") || rest.starts_with("L_") || rest.starts_with("R_") {
        let per_tile = !rest.starts_with("G_")
            || rest.contains("VPTX")
            || rest.contains("HPBX")
            || rest.contains("HPRX");
        if per_tile {
            return Some(WireTargetRef::Tile {
                dx: 0,
                dy: 0,
                name: rest,
            });
        }
        return Some(WireTargetRef::Global { name: rest });
    }
    // `[NS]<n>` then `[EW]<n>`, then `_`, then the base name. Anything
    // that does not match that shape is a wire of this very tile.
    let Some(under) = rest.find('_') else {
        return Some(WireTargetRef::Tile {
            dx: 0,
            dy: 0,
            name: rest,
        });
    };
    let (head, tail) = (&rest[..under], &rest[under + 1..]);
    let mut dx = 0i32;
    let mut dy = 0i32;
    let mut at = 0usize;
    let bytes = head.as_bytes();
    let mut matched = 0;
    for expect in [b"NS".as_slice(), b"EW".as_slice()] {
        if at < bytes.len() && expect.contains(&bytes[at]) {
            let sign = bytes[at];
            let start = at + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end == start {
                break;
            }
            let Ok(count) = head[start..end].parse::<i32>() else {
                break;
            };
            match sign {
                b'N' => dy -= count,
                b'S' => dy += count,
                b'W' => dx -= count,
                b'E' => dx += count,
                _ => {}
            }
            at = end;
            matched += 1;
        }
    }
    if matched == 0 || at != head.len() {
        // The underscore was part of the name, not a direction prefix.
        return Some(WireTargetRef::Tile {
            dx: 0,
            dy: 0,
            name: rest,
        });
    }
    Some(WireTargetRef::Tile { dx, dy, name: tail })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_name_gives_its_grid_position() {
        assert_eq!(row_col("CIB_R10C1"), Some((10, 1)));
        assert_eq!(row_col("MIB_R0C67"), Some((0, 67)));
        assert_eq!(row_col("TAP_R0C60"), Some((0, 60)));
        assert_eq!(row_col("R51C72"), Some((51, 72)));
        assert_eq!(row_col("PLC2"), None);
        assert_eq!(row_col("RC"), None);
        assert_eq!(row_col("R12"), None);
    }

    /// The wire-naming rule, which is the one thing in this database that
    /// a reader has to get exactly right: a wrong offset joins two nets
    /// that are not the same metal, and no structural check would notice.
    #[test]
    fn the_relative_prefixes_are_the_directions_grid_rows_and_columns_run_in() {
        let t = |name: &str| globalise(name, "25K_");
        assert_eq!(
            t("H01E0001"),
            Some(WireTarget::Tile {
                dx: 0,
                dy: 0,
                name: "H01E0001".to_owned()
            })
        );
        assert_eq!(
            t("E1_H01E0001"),
            Some(WireTarget::Tile {
                dx: 1,
                dy: 0,
                name: "H01E0001".to_owned()
            })
        );
        assert_eq!(
            t("W3_H06W0003"),
            Some(WireTarget::Tile {
                dx: -3,
                dy: 0,
                name: "H06W0003".to_owned()
            })
        );
        // S increases the row, N decreases it.
        assert_eq!(
            t("S1_JA0"),
            Some(WireTarget::Tile {
                dx: 0,
                dy: 1,
                name: "JA0".to_owned()
            })
        );
        assert_eq!(
            t("N13_V06N0303"),
            Some(WireTarget::Tile {
                dx: 0,
                dy: -13,
                name: "V06N0303".to_owned()
            })
        );
        // Both at once, row first.
        assert_eq!(
            t("S1E1_JA0"),
            Some(WireTarget::Tile {
                dx: 1,
                dy: 1,
                name: "JA0".to_owned()
            })
        );
        assert_eq!(
            t("N2W1_H00L0000"),
            Some(WireTarget::Tile {
                dx: -1,
                dy: -2,
                name: "H00L0000".to_owned()
            })
        );
    }

    #[test]
    fn a_name_with_an_underscore_that_is_not_a_direction_stays_local() {
        let t = |name: &str| globalise(name, "25K_");
        for name in [
            "A0_SLICE",
            "PADDOA_PIO",
            "JPADDIA_PIO",
            "IOLDOA_SIOLOGIC",
            "WDO0C_SLICE",
        ] {
            assert_eq!(
                t(name),
                Some(WireTarget::Tile {
                    dx: 0,
                    dy: 0,
                    name: name.to_owned()
                }),
                "{name}"
            );
        }
    }

    #[test]
    fn a_global_is_one_node_and_a_clock_spine_is_not() {
        let t = |name: &str| globalise(name, "25K_");
        assert_eq!(
            t("G_JCE0"),
            Some(WireTarget::Global {
                name: "G_JCE0".to_owned()
            })
        );
        // The spines and branches are per tile, because one tile's is not
        // another's.
        for name in ["G_HPBX0000", "G_VPTX0000", "G_HPRX0000"] {
            assert!(
                matches!(t(name), Some(WireTarget::Tile { dx: 0, dy: 0, .. })),
                "{name}"
            );
        }
        assert!(matches!(
            t("L_HFSN0000"),
            Some(WireTarget::Tile { dx: 0, dy: 0, .. })
        ));
    }

    #[test]
    fn a_wire_of_another_die_does_not_exist_here() {
        assert_eq!(globalise("45K_PCSA_TXREFCLK", "25K_"), None);
        assert_eq!(globalise("85K_FOO", "25K_"), None);
        assert_eq!(
            globalise("25K_FOO", "25K_"),
            Some(WireTarget::Tile {
                dx: 0,
                dy: 0,
                name: "FOO".to_owned()
            })
        );
    }

    #[test]
    fn a_bits_db_reads_back_record_by_record() {
        let text = "\
# Routing Mux Bits
.mux A0
F5 F1B3 F4B2
WBOUNCE -

.mux CLK0
G_HPBX0000 F45B0 F46B0

# Non-Routing Configuration
.config SLICEA.K0.INIT 1111
!F25B10
!F24B10
!F23B10
!F22B10

.config_enum PIOA.CLAMP
OFF !F9B0
ON F9B0

.config_enum PIOA.PULLMODE DOWN
NONE F7B0 !F8B0

# Fixed Connections
.fixed_conn A0_SLICE A0
";
        let db = TileDatabase::parse(text, "bits.db").expect("parses");
        assert_eq!(db.muxes.len(), 3);
        // **Every** source is an arc, the bitless one included: see
        // `TileDatabase::arcs`, where believing otherwise disconnected
        // every lookup table on the die.
        assert_eq!(db.arcs().count(), 3);
        assert_eq!(
            db.arcs()
                .filter(|(_, _, bits)| bits.is_empty())
                .map(|(sink, source, _)| (sink.as_str(), source.as_str()))
                .collect::<Vec<_>>(),
            vec![("A0", "WBOUNCE")]
        );
        assert_eq!(
            db.muxes[0].2,
            vec![
                DbBit {
                    frame: 1,
                    bit: 3,
                    inverted: false
                },
                DbBit {
                    frame: 4,
                    bit: 2,
                    inverted: false
                }
            ]
        );
        assert_eq!(db.words.len(), 1);
        assert_eq!(db.words[0].0, "SLICEA.K0.INIT");
        assert_eq!(db.words[0].1.as_deref(), Some("1111"));
        assert_eq!(db.word_bits("SLICEA.K0.INIT").map(<[_]>::len), Some(4));
        assert!(db.word_bits("nope").is_none());
        // Bit 0 of the word comes first, and it is inverted.
        assert_eq!(
            db.word_bits("SLICEA.K0.INIT").unwrap()[0],
            vec![DbBit {
                frame: 25,
                bit: 10,
                inverted: true
            }]
        );
        assert_eq!(db.enums.len(), 2);
        assert!(db.has_enum("PIOA.CLAMP"));
        assert!(!db.has_enum("PIOA.DRIVE"));
        assert_eq!(db.enums[0].1, None);
        assert_eq!(db.enums[1].1.as_deref(), Some("DOWN"));
        assert_eq!(
            db.enum_bits("PIOA.CLAMP", "ON"),
            Some(
                &[DbBit {
                    frame: 9,
                    bit: 0,
                    inverted: false
                }][..]
            )
        );
        assert!(db.enum_bits("PIOA.CLAMP", "nope").is_none());
        assert_eq!(db.fixed, vec![("A0_SLICE".to_owned(), "A0".to_owned())]);
        assert!(db.wire_names().contains("A0_SLICE"));
        assert!(db.wire_names().contains("G_HPBX0000"));
    }

    #[test]
    fn a_malformed_bits_db_is_refused_with_the_line() {
        let err = TileDatabase::parse(".mux A0\nnope notabit\n", "x/bits.db").expect_err("refused");
        assert!(err.to_string().contains("notabit"), "{err}");
        let err = TileDatabase::parse("F1B2\n", "x/bits.db").expect_err("refused");
        assert!(err.to_string().contains("belongs to no record"), "{err}");
        assert_eq!(
            parse_bit("F1B2"),
            Some(DbBit {
                frame: 1,
                bit: 2,
                inverted: false
            })
        );
        assert_eq!(
            parse_bit("!F1B2"),
            Some(DbBit {
                frame: 1,
                bit: 2,
                inverted: true
            })
        );
        assert_eq!(parse_bit("G1B2"), None);
        assert_eq!(parse_bit("F1"), None);
    }

    #[test]
    fn devices_json_gives_the_frame_shape_and_the_grid() {
        let text = r#"{ "families": { "ECP5": { "devices": {
            "LFE5U-12F": { "packages": ["caBGA256"], "idcode": "0x21111043",
              "frames": 7562, "bits_per_frame": 592, "pad_bits_after_frame": 0,
              "pad_bits_before_frame": 0, "max_row": 50, "max_col": 72 },
            "LFE5U-45F": { "packages": ["caBGA381"], "idcode": "0x41112043",
              "frames": 9470, "bits_per_frame": 846, "pad_bits_after_frame": 0,
              "pad_bits_before_frame": 2, "max_row": 71, "max_col": 90 },
            "LFE5U-NOID": { "frames": 1 }
        } } } }"#;
        let parts = devices(text).expect("parses");
        assert_eq!(parts.len(), 2, "the part with no idcode is skipped");
        let twelve = &parts[0];
        assert_eq!(twelve.name, "LFE5U-12F");
        assert_eq!(twelve.family, "ECP5");
        assert_eq!(twelve.idcode, 0x2111_1043);
        assert_eq!(twelve.format.frames, 7562);
        assert_eq!(twelve.format.bytes_per_frame(), 74);
        assert_eq!((twelve.width(), twelve.height()), (73, 51));
        assert_eq!(twelve.chip_prefix(), "25K_");
        assert_eq!(parts[1].chip_prefix(), "45K_");
        assert_eq!(parts[1].format.bytes_per_frame(), 106);
        assert!(devices(r#"{"families":{}}"#).is_err());
        assert!(devices("nope").is_err());
    }

    #[test]
    fn a_tilegrid_entry_carries_a_window_and_its_position() {
        let text = r#"{
          "CIB_R1C67:CIB": { "cols": 106, "rows": 11, "start_bit": 1,
             "start_frame": 7032, "type": "CIB", "sites": [{"name":"CIBTEST"}] },
          "MIB_R0C67:PIOT0": { "cols": 106, "rows": 1, "start_bit": 0,
             "start_frame": 7032, "type": "PIOT0",
             "sites": [{"name":"PIOA"},{"name":"PIOB"}] }
        }"#;
        let tiles = tilegrid(text, "tilegrid.json").expect("parses");
        assert_eq!(tiles.len(), 2);
        // Sorted by key, so CIB_… comes before MIB_….
        assert_eq!(tiles[0].ty, "CIB");
        assert_eq!((tiles[0].row, tiles[0].col), (1, 67));
        assert_eq!(tiles[0].window.frames, 106);
        assert_eq!(tiles[0].window.bits, 11);
        assert_eq!(tiles[1].sites, vec!["PIOA", "PIOB"]);
        assert!(tilegrid(r#"{"NOPOS:X":{}}"#, "t.json").is_err());
    }

    #[test]
    fn an_iodb_gives_the_ball_map() {
        let text = r#"{"packages": { "CABGA256": {
            "E13": { "col": 62, "pio": "B", "row": 0 },
            "A8":  { "col": 29, "pio": "B", "row": 0 }
        } } }"#;
        let packages = iodb(text, "iodb.json").expect("parses");
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package, "CABGA256");
        assert_eq!(packages[0].balls.get("E13"), Some(&(0, 62, "B".to_owned())));
        assert!(iodb("{}", "iodb.json").is_err());
    }
}
