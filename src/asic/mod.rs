//! ASIC file formats: Liberty, LEF and DEF.
//!
//! This module holds the readers and writers for the three text formats an
//! ASIC flow exchanges with a standard-cell library and a place-and-route
//! tool:
//!
//! - [`liberty`]: the Synopsys Liberty (`.lib`) format that describes a
//!   cell library's functions, pins, timing and power tables. Reticle reads
//!   it into a generic group tree ([`liberty::Group`]) and a typed view
//!   ([`liberty::Library`]) that the standard-cell mapper and the static
//!   timing analyser consume.
//! - [`lef`]: the Library Exchange Format that describes the technology
//!   (layers, vias, sites) and the physical abstract of each cell (size,
//!   pins, obstructions).
//! - [`def`]: the Design Exchange Format that describes one design's
//!   floorplan, components, pins and nets. [`def::from_netlist`] turns a
//!   Reticle netlist into an unplaced DEF so it can be handed to OpenROAD.
//!
//! All three follow the crate's rules: no I/O (text in, values out),
//! problems reported as [`crate::diag::Diagnostic`]s with spans into the
//! [`crate::source::SourceMap`], unknown constructs preserved rather than
//! rejected, and deterministic output from the writers so the files can be
//! snapshot-tested.
//!
//! The types here are plain data. Nothing depends on the synthesis passes;
//! the mapper in `synth` is a consumer of [`liberty::Library`], not the other
//! way round.

pub mod def;
pub mod lef;
mod lefdef;
pub mod liberty;

use std::fmt;

/// A placement orientation, shared by LEF (`FOREIGN`) and DEF
/// (`COMPONENTS`, `PINS`, `ROW`).
///
/// The names follow the LEF/DEF convention: `N` is the unrotated cell, `S`
/// is rotated by 180 degrees, `W`/`E` by 90/270, and the `F` variants are
/// mirrored about the Y axis before rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Orient {
    /// North: no rotation.
    #[default]
    N,
    /// South: rotated by 180 degrees.
    S,
    /// West: rotated by 90 degrees counter-clockwise.
    W,
    /// East: rotated by 270 degrees counter-clockwise.
    E,
    /// Flipped north (mirrored about the Y axis).
    FN,
    /// Flipped south.
    FS,
    /// Flipped west.
    FW,
    /// Flipped east.
    FE,
}

impl Orient {
    /// The LEF/DEF keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            Orient::N => "N",
            Orient::S => "S",
            Orient::W => "W",
            Orient::E => "E",
            Orient::FN => "FN",
            Orient::FS => "FS",
            Orient::FW => "FW",
            Orient::FE => "FE",
        }
    }

    /// Parses a LEF/DEF orientation keyword (also accepting the `R0`,
    /// `R90`, `MX`, ... aliases some tools write).
    pub fn from_keyword(word: &str) -> Option<Orient> {
        Some(match word {
            "N" | "R0" => Orient::N,
            "S" | "R180" => Orient::S,
            "W" | "R90" => Orient::W,
            "E" | "R270" => Orient::E,
            "FN" | "MY" => Orient::FN,
            "FS" | "MX" => Orient::FS,
            "FW" | "MX90" => Orient::FW,
            "FE" | "MY90" => Orient::FE,
            _ => return None,
        })
    }
}

impl fmt::Display for Orient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.keyword())
    }
}

/// A LEF/DEF statement the typed readers do not model, kept as its tokens
/// (strings re-quoted) without the terminating `;`. When `block` is set
/// the tokens span a whole `KEYWORD ... END KEYWORD` section, internal
/// `;` included, and no `;` is appended on output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raw {
    /// The tokens as written.
    pub tokens: Vec<String>,
    /// True for a multi-statement section.
    pub block: bool,
}

impl Raw {
    /// A single statement from its tokens.
    pub fn statement(tokens: Vec<String>) -> Raw {
        Raw {
            tokens,
            block: false,
        }
    }
}

/// A LEF/DEF `PROPERTY name value` pair.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    /// The property name, declared in `PROPERTYDEFINITIONS`.
    pub name: String,
    /// The value.
    pub value: PropertyValue,
}

/// The value of a LEF/DEF property.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// A number.
    Number(f64),
    /// A string (or a bare word that is not a number).
    String(String),
}

impl fmt::Display for PropertyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyValue::Number(v) => f.write_str(&fmt_num(*v)),
            PropertyValue::String(s) => f.write_str(&quote_if_needed(s)),
        }
    }
}

/// One entry of a LEF/DEF `PROPERTYDEFINITIONS` section.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyDefinition {
    /// The object type: `LAYER`, `MACRO`, `PIN`, `COMPONENT`, `NET`, ...
    pub object: String,
    /// The property name.
    pub name: String,
    /// `STRING`, `INTEGER` or `REAL`.
    pub ty: String,
    /// `RANGE min max`.
    pub range: Option<(f64, f64)>,
    /// The default value.
    pub value: Option<PropertyValue>,
}

/// Formats a real number the way the writers print it: plain decimal with
/// at most six places, trailing zeros trimmed, and `-0` normalised to `0`.
///
/// LEF coordinates are microns on a manufacturing grid of a few nanometres
/// and Liberty values rarely carry more than six decimals, so that form
/// covers the numbers that matter and keeps the output stable across
/// platforms. When six decimals would *not* reproduce the value — a small
/// capacitance such as `2.5e-17`, or a ratio with a long expansion — the
/// shortest exactly round-tripping form is used instead, which may carry
/// an exponent. Writing a value that does not read back is never worth the
/// tidier column.
pub fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let mut s = format!("{v:.6}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".to_string();
    }
    if s.parse::<f64>() == Ok(v) {
        s
    } else {
        // `{}` on f64 is the shortest decimal that reads back exactly.
        v.to_string()
    }
}

/// Converts a number that must be integral (a bit index, a bank width, a
/// DEF coordinate written as `100.0`) to `i64`; `None` when it has a
/// fractional part or does not fit.
///
/// This is the one place the module narrows a float to an integer; the
/// range check makes the `as` exact.
#[allow(clippy::cast_possible_truncation)]
pub fn float_to_int(v: f64) -> Option<i64> {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 9.0e15 {
        Some(v as i64)
    } else {
        None
    }
}

/// Quotes a string for LEF/DEF/Liberty output when it contains characters
/// that would otherwise split it into several tokens.
fn quote_if_needed(s: &str) -> String {
    let needs = s.is_empty()
        || s.chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | ';' | '(' | ')' | '{' | '}'));
    if needs {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            if c == '"' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting_is_trimmed() {
        assert_eq!(fmt_num(1.0), "1");
        assert_eq!(fmt_num(0.46), "0.46");
        assert_eq!(fmt_num(2.72), "2.72");
        assert_eq!(fmt_num(-0.0), "0");
        assert_eq!(fmt_num(1234.5), "1234.5");
        assert_eq!(fmt_num(0.000001), "0.000001");
        assert_eq!(fmt_num(2.5e-5), "0.000025");
    }

    #[test]
    fn number_formatting_never_loses_a_value() {
        // Too small for six decimals: the exact form wins over the tidy one.
        assert_eq!(fmt_num(1e-7), "0.0000001");
        assert_eq!(fmt_num(2.5e-17), "0.000000000000000025");
        assert_eq!(fmt_num(1.0 / 3.0), "0.3333333333333333");
        for v in [
            0.0,
            -0.0,
            1.0,
            0.005,
            0.46,
            2.72,
            -0.085,
            1e-7,
            2.5e-17,
            1.0 / 3.0,
            1234.5678,
            f64::MIN_POSITIVE,
        ] {
            let s = fmt_num(v);
            assert_eq!(
                s.parse::<f64>().unwrap(),
                if v == 0.0 { 0.0 } else { v },
                "{v} printed as {s}"
            );
        }
    }

    #[test]
    fn orientation_keywords_round_trip() {
        for o in [
            Orient::N,
            Orient::S,
            Orient::W,
            Orient::E,
            Orient::FN,
            Orient::FS,
            Orient::FW,
            Orient::FE,
        ] {
            assert_eq!(Orient::from_keyword(o.keyword()), Some(o));
        }
        assert_eq!(Orient::from_keyword("MX"), Some(Orient::FS));
        assert_eq!(Orient::from_keyword("bogus"), None);
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_if_needed("abc"), "abc");
        assert_eq!(quote_if_needed("a b"), "\"a b\"");
        assert_eq!(quote_if_needed(""), "\"\"");
    }
}
