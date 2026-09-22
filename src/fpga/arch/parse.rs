//! Reading and writing the `.arch` text format.
//!
//! The grammar is in the [module docs](super). It is flat on purpose: an
//! `arch` block has exactly one `end`, and a tile type's members name
//! their tile type instead of nesting, which means a file may hold
//! `device` blocks and `arch` blocks side by side and either parser can
//! skip the other's. This one skips a `device` block by counting the one
//! construct that nests inside it, `bram`.

use super::{Arch, BelDecl, ConfigBit, ConfigEntry, PipDecl, TileType, WireDecl, WireRef};
use crate::diag::{Diagnostic, Diagnostics};
use crate::fpga::text::{Line, quote, tokenize};
use crate::source::{SourceId, Span};

/// Diagnostic code for a malformed line in an `.arch` file.
pub const ARCH_SYNTAX: &str = "F0120";
/// Diagnostic code for an `.arch` construct that is well formed but
/// unknown.
pub const ARCH_UNKNOWN: &str = "F0121";

impl Arch {
    /// Parses every `arch` block in `text`.
    ///
    /// `device` blocks are skipped, so one file may describe a part and
    /// its fabric. Malformed lines are reported through `diags` and
    /// skipped, so one bad directive does not lose the file.
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> Vec<Arch> {
        let lines = tokenize(text, file);
        let mut p = Parser {
            lines: &lines,
            pos: 0,
            diags,
        };
        let mut out = Vec::new();
        while let Some(line) = p.peek() {
            match line.keyword() {
                "arch" => {
                    if let Some(arch) = p.arch() {
                        out.push(arch);
                    }
                }
                "device" => p.skip_device(),
                other => {
                    let (span, other) = (line.tokens[0].span, other.to_owned());
                    p.pos += 1;
                    p.error(span, format!("expected `arch`, found `{other}`"));
                }
            }
        }
        out
    }

    /// Renders the architecture in the `.arch` text format.
    ///
    /// The result parses back to an equal value. The one normalisation is
    /// that a `tiles ... rect` in the input comes back as one `tile` line
    /// per position.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("arch {}\n", quote(&self.name)));
        out.push_str(&format!("  family {}\n", quote(&self.family)));
        for part in &self.parts {
            out.push_str(&format!("  part {}\n", quote(part)));
        }
        out.push_str(&format!("  grid {} {}\n", self.width, self.height));
        if !self.asc_device.is_empty() {
            out.push_str(&format!("  asc_device {}\n", quote(&self.asc_device)));
        }
        for global in &self.globals {
            out.push_str(&format!("  global {}\n", quote(global)));
        }
        for tile in &self.tile_types {
            out.push_str(&format!(
                "  tiletype {} asc {} bits {} {}\n",
                quote(&tile.name),
                quote(&tile.asc_keyword),
                tile.bit_rows,
                tile.bit_cols
            ));
            for wire in &tile.wires {
                out.push_str(&format!(
                    "  wire {} {} span {} {}\n",
                    quote(&tile.name),
                    quote(&wire.name),
                    wire.dx,
                    wire.dy
                ));
            }
            for bel in &tile.bels {
                let mut line = format!(
                    "  bel {} {} {}",
                    quote(&tile.name),
                    quote(&bel.name),
                    quote(&bel.kind)
                );
                for (role, wire) in &bel.pins {
                    line.push_str(&format!(" pin {}={}", quote(role), quote(&wire.to_text())));
                }
                out.push_str(&line);
                out.push('\n');
                for entry in &bel.config {
                    let head = format!("  config {} {}", quote(&tile.name), quote(&bel.name));
                    match entry {
                        ConfigEntry::Cell { primitive, bits } => {
                            out.push_str(&format!("{head} cell {}", quote(primitive)));
                            if !bits.is_empty() {
                                out.push_str(&format!(" bits {}", write_bits(bits)));
                            }
                            out.push('\n');
                        }
                        ConfigEntry::Param { name, index, at } => {
                            out.push_str(&format!(
                                "{head} param {} {index} {}.{}\n",
                                quote(name),
                                at.row,
                                at.col
                            ));
                        }
                    }
                }
            }
            for pip in &tile.pips {
                let mut line = format!(
                    "  pip {} {} {}",
                    quote(&tile.name),
                    quote(&pip.from.to_text()),
                    quote(&pip.to.to_text())
                );
                if !pip.bits.is_empty() {
                    line.push_str(&format!(" bits {}", write_bits(&pip.bits)));
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
        for y in 0..self.height {
            for x in 0..self.width {
                if let Some(index) = self.tile_index_at(x, y) {
                    out.push_str(&format!(
                        "  tile {x} {y} {}\n",
                        quote(&self.tile_types[index].name)
                    ));
                }
            }
        }
        for (pin, site) in &self.pinmap {
            out.push_str(&format!("  pinmap {} {}\n", quote(pin), quote(site)));
        }
        out.push_str("end\n");
        out
    }
}

/// Renders a list of config bits as `row.col,row.col`.
fn write_bits(bits: &[ConfigBit]) -> String {
    bits.iter()
        .map(|b| format!("{}.{}", b.row, b.col))
        .collect::<Vec<_>>()
        .join(",")
}

/// The line-at-a-time parser.
struct Parser<'a> {
    lines: &'a [Line],
    pos: usize,
    diags: &'a mut Diagnostics,
}

/// A tile placement, kept until the grid is known.
struct TilePlacement {
    x: u32,
    y: u32,
    type_name: String,
    span: Span,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Line> {
        self.lines.get(self.pos)
    }

    fn bump(&mut self) -> Option<&'a Line> {
        let line = self.lines.get(self.pos);
        self.pos += 1;
        line
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(
            Diagnostic::error(message)
                .with_code(ARCH_SYNTAX)
                .with_span(span),
        );
    }

    fn unknown(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(
            Diagnostic::error(message)
                .with_code(ARCH_UNKNOWN)
                .with_span(span),
        );
    }

    /// Skips a `device` block, whose only nested block is `bram`.
    fn skip_device(&mut self) {
        self.pos += 1;
        let mut depth = 0usize;
        while let Some(line) = self.bump() {
            match line.keyword() {
                "bram" => depth += 1,
                "end" if depth > 0 => depth -= 1,
                "end" => return,
                _ => {}
            }
        }
    }

    fn word(&mut self, line: &'a Line, index: usize, what: &str) -> Option<&'a str> {
        match line.get(index) {
            Some(token) => Some(token.as_str()),
            None => {
                self.error(line.span, format!("expected {what}"));
                None
            }
        }
    }

    fn number(&mut self, line: &'a Line, index: usize, what: &str) -> Option<u32> {
        let token = line.get(index)?;
        match token.as_str().parse::<u32>() {
            Ok(v) => Some(v),
            Err(_) => {
                let span = token.span;
                self.error(span, format!("expected {what}, found `{token}`"));
                None
            }
        }
    }

    fn signed(&mut self, line: &'a Line, index: usize, what: &str) -> Option<i32> {
        let token = line.get(index)?;
        match token.as_str().parse::<i32>() {
            Ok(v) => Some(v),
            Err(_) => {
                let span = token.span;
                self.error(span, format!("expected {what}, found `{token}`"));
                None
            }
        }
    }

    /// The index of the tile type named by token `index`, reporting one
    /// that was never declared.
    fn tile_type(&mut self, line: &'a Line, index: usize, arch: &Arch) -> Option<usize> {
        let name = self.word(line, index, "a tile type name")?;
        match arch.tile_type_index(name) {
            Some(i) => Some(i),
            None => {
                let span = line.tokens[index].span;
                let name = name.to_owned();
                self.error(span, format!("no tile type `{name}` is declared"));
                None
            }
        }
    }

    /// Parses one `arch` block; the cursor is on its header line.
    fn arch(&mut self) -> Option<Arch> {
        let header = self.bump()?;
        let name = self.word(header, 1, "an architecture name")?.to_owned();
        let mut arch = Arch::new(name, "", 0, 0);
        let mut placements: Vec<TilePlacement> = Vec::new();
        let mut seen_grid = false;
        loop {
            let Some(line) = self.peek() else {
                self.error(header.span, "unterminated `arch` block");
                break;
            };
            match line.keyword() {
                "end" => {
                    self.bump();
                    break;
                }
                "arch" | "device" => {
                    self.error(header.span, "unterminated `arch` block");
                    break;
                }
                _ => {}
            }
            self.bump();
            seen_grid |= line.keyword() == "grid";
            self.directive(line, &mut arch, &mut placements);
        }
        if arch.family.is_empty() {
            self.error(
                header.span,
                format!("architecture `{}` has no `family` line", arch.name),
            );
        }
        if !seen_grid {
            self.error(
                header.span,
                format!("architecture `{}` has no `grid` line", arch.name),
            );
        }
        for place in placements {
            match arch.tile_type_index(&place.type_name) {
                Some(index) if place.x < arch.width && place.y < arch.height => {
                    arch.set_tile(place.x, place.y, index);
                }
                Some(_) => {
                    let (x, y) = (place.x, place.y);
                    let (w, h) = (arch.width, arch.height);
                    self.error(
                        place.span,
                        format!("tile ({x}, {y}) is outside the {w} x {h} grid"),
                    );
                }
                None => {
                    let name = place.type_name.clone();
                    self.error(place.span, format!("no tile type `{name}` is declared"));
                }
            }
        }
        Some(arch)
    }

    fn directive(&mut self, line: &'a Line, arch: &mut Arch, tiles: &mut Vec<TilePlacement>) {
        match line.keyword() {
            "family" => {
                if let Some(word) = self.word(line, 1, "a family name") {
                    arch.family = word.to_owned();
                }
            }
            "part" => {
                if let Some(word) = self.word(line, 1, "a device name") {
                    arch.parts.push(word.to_owned());
                }
            }
            "asc_device" => {
                if let Some(word) = self.word(line, 1, "a device word") {
                    arch.asc_device = word.to_owned();
                }
            }
            "global" => {
                if let Some(word) = self.word(line, 1, "a wire name") {
                    arch.globals.push(word.to_owned());
                }
            }
            "grid" => {
                let width = self.number(line, 1, "the grid width");
                let height = self.number(line, 2, "the grid height");
                if let (Some(width), Some(height)) = (width, height) {
                    arch.width = width;
                    arch.height = height;
                    arch.tiles = vec![None; width as usize * height as usize];
                }
            }
            "tiletype" => self.tiletype(line, arch),
            "wire" => self.wire(line, arch),
            "bel" => self.bel(line, arch),
            "config" => self.config(line, arch),
            "pip" => self.pip(line, arch),
            "tile" => {
                let x = self.number(line, 1, "a column");
                let y = self.number(line, 2, "a row");
                let name = self.word(line, 3, "a tile type name").map(str::to_owned);
                if let (Some(x), Some(y), Some(type_name)) = (x, y, name) {
                    tiles.push(TilePlacement {
                        x,
                        y,
                        type_name,
                        span: line.span,
                    });
                }
            }
            "tiles" => self.tiles(line, tiles),
            "pinmap" => {
                let pin = self.word(line, 1, "a package pin").map(str::to_owned);
                let site = self.word(line, 2, "a site name").map(str::to_owned);
                if let (Some(pin), Some(site)) = (pin, site) {
                    arch.pinmap.push((pin, site));
                }
            }
            other => {
                let message = format!("unknown architecture directive `{other}`");
                self.unknown(line.tokens[0].span, message);
            }
        }
    }

    fn tiletype(&mut self, line: &'a Line, arch: &mut Arch) {
        let Some(name) = self.word(line, 1, "a tile type name").map(str::to_owned) else {
            return;
        };
        if arch.tile_type_index(&name).is_some() {
            self.error(line.span, format!("tile type `{name}` is declared twice"));
            return;
        }
        let mut keyword = name.clone();
        let mut rows = 0;
        let mut cols = 0;
        let mut index = 2;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "asc" => {
                    if let Some(word) = self.word(line, index, "an `.asc` keyword") {
                        keyword = word.to_owned();
                    }
                    index += 1;
                }
                "bits" => {
                    rows = self.number(line, index, "the bitmap rows").unwrap_or(0);
                    cols = self
                        .number(line, index + 1, "the bitmap columns")
                        .unwrap_or(0);
                    index += 2;
                }
                other => {
                    let (span, other) = (token.span, other.to_owned());
                    self.unknown(span, format!("unknown `tiletype` option `{other}`"));
                }
            }
        }
        arch.tile_types
            .push(TileType::new(name, keyword, rows, cols));
    }

    fn wire(&mut self, line: &'a Line, arch: &mut Arch) {
        let Some(tile) = self.tile_type(line, 1, arch) else {
            return;
        };
        let Some(name) = self.word(line, 2, "a wire name").map(str::to_owned) else {
            return;
        };
        let mut dx = 0;
        let mut dy = 0;
        if line.get(3).is_some_and(|t| t.is("span")) {
            dx = self.signed(line, 4, "a column span").unwrap_or(0);
            dy = self.signed(line, 5, "a row span").unwrap_or(0);
        }
        arch.tile_types[tile].wires.push(WireDecl { name, dx, dy });
    }

    fn bel(&mut self, line: &'a Line, arch: &mut Arch) {
        let Some(tile) = self.tile_type(line, 1, arch) else {
            return;
        };
        let name = self.word(line, 2, "a bel name").map(str::to_owned);
        let kind = self.word(line, 3, "a bel kind").map(str::to_owned);
        let (Some(name), Some(kind)) = (name, kind) else {
            return;
        };
        let mut bel = BelDecl::new(name, kind);
        let mut index = 4;
        while let Some(token) = line.get(index) {
            index += 1;
            match token.as_str() {
                "pin" => {
                    let Some(pair) = line.get(index) else {
                        self.error(line.span, "expected `role=wire` after `pin`");
                        break;
                    };
                    index += 1;
                    let Some((role, wire)) = pair.pair() else {
                        let span = pair.span;
                        self.error(span, "expected `role=wire`");
                        continue;
                    };
                    let (role, wire) = (role.to_owned(), wire.to_owned());
                    match parse_wire_ref(&wire) {
                        Some(wref) => bel.pins.push((role, wref)),
                        None => {
                            let span = pair.span;
                            self.error(span, format!("`{wire}` is not a wire reference"));
                        }
                    }
                }
                other => {
                    let (span, other) = (token.span, other.to_owned());
                    self.unknown(span, format!("unknown `bel` option `{other}`"));
                }
            }
        }
        arch.tile_types[tile].bels.push(bel);
    }

    fn config(&mut self, line: &'a Line, arch: &mut Arch) {
        let Some(tile) = self.tile_type(line, 1, arch) else {
            return;
        };
        let Some(bel_name) = self.word(line, 2, "a bel name").map(str::to_owned) else {
            return;
        };
        if arch.tile_types[tile].bel(&bel_name).is_none() {
            let name = arch.tile_types[tile].name.clone();
            self.error(
                line.span,
                format!("tile type `{name}` has no bel `{bel_name}`"),
            );
            return;
        }
        let Some(what) = self.word(line, 3, "`cell` or `param`").map(str::to_owned) else {
            return;
        };
        let entry = match what.as_str() {
            "cell" => {
                let primitive = self.word(line, 4, "a primitive name").map(str::to_owned);
                // A variant with no bits set is a legitimate code, so the
                // `bits` clause is optional here.
                let bits = if line.get(5).is_some() {
                    self.bits_clause(line, 5)
                } else {
                    Some(Vec::new())
                };
                match (primitive, bits) {
                    (Some(primitive), Some(bits)) => ConfigEntry::Cell { primitive, bits },
                    _ => return,
                }
            }
            "param" => {
                let name = self.word(line, 4, "a parameter name").map(str::to_owned);
                let index = self.number(line, 5, "a bit index");
                let at = line.get(6).and_then(|t| parse_bit(t.as_str()));
                match (name, index, at) {
                    (Some(name), Some(index), Some(at)) => ConfigEntry::Param { name, index, at },
                    _ => {
                        self.error(line.span, "expected `param <name> <index> <row>.<col>`");
                        return;
                    }
                }
            }
            other => {
                let span = line.tokens[3].span;
                self.unknown(span, format!("unknown `config` kind `{other}`"));
                return;
            }
        };
        let bel = arch.tile_types[tile]
            .bels
            .iter_mut()
            .find(|b| b.name == bel_name)
            .expect("checked above");
        bel.config.push(entry);
    }

    fn pip(&mut self, line: &'a Line, arch: &mut Arch) {
        let Some(tile) = self.tile_type(line, 1, arch) else {
            return;
        };
        let from = self.word(line, 2, "a source wire").map(str::to_owned);
        let to = self.word(line, 3, "a destination wire").map(str::to_owned);
        let (Some(from), Some(to)) = (from, to) else {
            return;
        };
        let (Some(from), Some(to)) = (parse_wire_ref(&from), parse_wire_ref(&to)) else {
            self.error(line.span, "expected two wire references");
            return;
        };
        let bits = if line.get(4).is_some() {
            match self.bits_clause(line, 4) {
                Some(bits) => bits,
                None => return,
            }
        } else {
            Vec::new()
        };
        arch.tile_types[tile].pips.push(PipDecl { from, to, bits });
    }

    /// Reads `bits <r>.<c>,<r>.<c>` starting at `index`.
    fn bits_clause(&mut self, line: &'a Line, index: usize) -> Option<Vec<ConfigBit>> {
        if !line.get(index).is_some_and(|t| t.is("bits")) {
            self.error(line.span, "expected `bits <row>.<col>`");
            return None;
        }
        let Some(list) = line.get(index + 1) else {
            self.error(line.span, "expected a bit list after `bits`");
            return None;
        };
        let mut bits = Vec::new();
        for part in list.as_str().split(',').filter(|p| !p.is_empty()) {
            match parse_bit(part) {
                Some(bit) => bits.push(bit),
                None => {
                    let span = list.span;
                    self.error(span, format!("`{part}` is not a `row.col` bit position"));
                    return None;
                }
            }
        }
        Some(bits)
    }

    fn tiles(&mut self, line: &'a Line, tiles: &mut Vec<TilePlacement>) {
        let Some(name) = self.word(line, 1, "a tile type name").map(str::to_owned) else {
            return;
        };
        if !line.get(2).is_some_and(|t| t.is("rect")) {
            self.error(line.span, "expected `rect <x0> <y0> <x1> <y1>`");
            return;
        }
        let x0 = self.number(line, 3, "a left edge");
        let y0 = self.number(line, 4, "a bottom edge");
        let x1 = self.number(line, 5, "a right edge");
        let y1 = self.number(line, 6, "a top edge");
        let (Some(x0), Some(y0), Some(x1), Some(y1)) = (x0, y0, x1, y1) else {
            return;
        };
        if x1 < x0 || y1 < y0 {
            self.error(
                line.span,
                "the rectangle's far corner is before its near one",
            );
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                tiles.push(TilePlacement {
                    x,
                    y,
                    type_name: name.clone(),
                    span: line.span,
                });
            }
        }
    }
}

/// Parses a `row.col` bit position.
fn parse_bit(text: &str) -> Option<ConfigBit> {
    let (row, col) = text.split_once('.')?;
    Some(ConfigBit {
        row: row.parse().ok()?,
        col: col.parse().ok()?,
    })
}

/// Parses a wire reference: `name`, `name@dx,dy` or `*name`.
fn parse_wire_ref(text: &str) -> Option<WireRef> {
    if let Some(name) = text.strip_prefix('*') {
        if name.is_empty() {
            return None;
        }
        return Some(WireRef::global(name));
    }
    match text.split_once('@') {
        None if text.is_empty() => None,
        None => Some(WireRef::local(text)),
        Some((name, offset)) => {
            let (dx, dy) = offset.split_once(',')?;
            Some(WireRef::at(name, dx.parse().ok()?, dy.parse().ok()?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn parse(text: &str) -> (Vec<Arch>, String) {
        let mut map = SourceMap::new();
        let file = map.add("t.arch", text).unwrap();
        let mut diags = Diagnostics::new();
        let archs = Arch::parse(text, file, &mut diags);
        (archs, diags.render(&map))
    }

    const TINY: &str = "\
arch tiny
  family test
  part test-part
  grid 3 2
  asc_device 1k
  global glb
  tiletype logic asc logic_tile bits 2 4
  wire logic local span 0 0
  wire logic sp2e span 2 0
  bel logic lut lut pin i0=local pin o=sp2e
  config logic lut cell SB_LUT4 bits 0.0,0.1
  config logic lut param LUT_INIT 3 1.3
  pip logic sp2e@-2,0 local bits 1.1
  pip logic *glb local
  tiles logic rect 0 0 2 1
  pinmap 1 X0Y0/lut
end
";

    #[test]
    fn parses_and_round_trips() {
        let (archs, diags) = parse(TINY);
        assert_eq!(diags, "");
        assert_eq!(archs.len(), 1);
        let arch = &archs[0];
        assert_eq!(arch.name, "tiny");
        assert_eq!(arch.family, "test");
        assert_eq!(arch.parts, vec!["test-part"]);
        assert_eq!((arch.width, arch.height), (3, 2));
        assert_eq!(arch.asc_device, "1k");
        assert_eq!(arch.globals, vec!["glb"]);
        assert_eq!(arch.tile_types.len(), 1);
        let tile = &arch.tile_types[0];
        assert_eq!(tile.asc_keyword, "logic_tile");
        assert_eq!(tile.bit_count(), 8);
        assert!(tile.has_wire("sp2e"));
        assert_eq!(tile.bels.len(), 1);
        assert_eq!(tile.bels[0].config.len(), 2);
        assert_eq!(tile.pips.len(), 2);
        assert!(tile.pips[1].bits.is_empty());
        assert_eq!(arch.tiles.iter().filter(|t| t.is_some()).count(), 6);
        assert_eq!(arch.pinmap, vec![("1".to_owned(), "X0Y0/lut".to_owned())]);

        let text = arch.to_text();
        let (again, diags) = parse(&text);
        assert_eq!(diags, "");
        assert_eq!(again.len(), 1);
        assert_eq!(&again[0], arch);
        assert_eq!(again[0].to_text(), text);
        // The rectangle became explicit tiles.
        assert!(text.contains("  tile 2 1 logic\n"), "{text}");
    }

    #[test]
    fn device_blocks_are_skipped() {
        let text = format!(
            "device d\n  family f\n  bram B\n    ports 1\n  end\n  grid 2 2\nend\n\n{TINY}"
        );
        let (archs, diags) = parse(&text);
        assert_eq!(diags, "");
        assert_eq!(archs.len(), 1);
        assert_eq!(archs[0].name, "tiny");
    }

    /// The same file parses as a device database too, which is what makes
    /// one file able to hold both.
    #[test]
    fn a_combined_file_parses_both_ways() {
        let text = format!(
            "{}\n{TINY}",
            crate::fpga::target("generic").unwrap().to_text()
        );
        let mut map = SourceMap::new();
        let file = map.add("both.dev", &text).unwrap();
        let mut diags = Diagnostics::new();
        let db = crate::fpga::DeviceDb::parse(&text, file, &mut diags);
        assert_eq!(diags.render(&map), "");
        assert_eq!(db.names(), vec!["generic"]);
        let archs = Arch::parse(&text, file, &mut diags);
        assert_eq!(diags.render(&map), "");
        assert_eq!(archs.len(), 1);
    }

    #[test]
    fn malformed_lines_are_reported_one_by_one() {
        let text = "\
arch bad
  grid 1 1
  wire nosuch w span 0 0
  tiletype t asc t bits 1 1
  tiletype t asc t bits 1 1
  bel t b lut pin bad
  config t nosuch cell C bits 0.0
  pip t a
  pip t a b bits 9
  tile 5 5 t
  tiles t rect 2 2 1 1
  nonsense
end
";
        let (archs, diags) = parse(text);
        assert_eq!(archs.len(), 1);
        for wanted in [
            "has no `family` line",
            "no tile type `nosuch` is declared",
            "tile type `t` is declared twice",
            "expected `role=wire`",
            "has no bel `nosuch`",
            "expected a destination wire",
            "is not a `row.col` bit position",
            "outside the 1 x 1 grid",
            "far corner is before its near one",
            "unknown architecture directive `nonsense`",
        ] {
            assert!(diags.contains(wanted), "missing {wanted:?} in\n{diags}");
        }
    }

    #[test]
    fn an_unterminated_block_is_reported() {
        let (_, diags) = parse("arch a\n  family f\n  grid 1 1\n");
        assert!(diags.contains("unterminated `arch` block"), "{diags}");
        let (_, diags) =
            parse("arch a\n  family f\n  grid 1 1\narch b\n  family f\n  grid 1 1\nend\n");
        assert!(diags.contains("unterminated `arch` block"), "{diags}");
        let (_, diags) = parse("stray\n");
        assert!(diags.contains("expected `arch`, found `stray`"), "{diags}");
    }

    #[test]
    fn wire_references_parse() {
        assert_eq!(parse_wire_ref("a"), Some(WireRef::local("a")));
        assert_eq!(parse_wire_ref("a@-4,2"), Some(WireRef::at("a", -4, 2)));
        assert_eq!(parse_wire_ref("*g"), Some(WireRef::global("g")));
        assert_eq!(parse_wire_ref(""), None);
        assert_eq!(parse_wire_ref("*"), None);
        assert_eq!(parse_wire_ref("a@1"), None);
        assert_eq!(parse_wire_ref("a@x,1"), None);
        assert_eq!(parse_bit("3.20"), Some(ConfigBit::new(3, 20)));
        assert_eq!(parse_bit("3"), None);
        assert_eq!(parse_bit("a.b"), None);
    }
}
