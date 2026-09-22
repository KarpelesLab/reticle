//! VCD waveform writer (IEEE 1364-2005 §18).
//!
//! The writer accumulates the file text in memory: a header with
//! `$timescale`, one `$scope module` per instance holding a `$var` per net
//! (aliased nets share one identifier), `$enddefinitions`, then a
//! `$dumpvars` block with every current value and, as the simulation runs,
//! `#time` stamps and value changes. Identifiers are the printable ASCII
//! base-94 encoding of the signal index, as every VCD reader expects.
//! Memories are not dumped. Nothing is written to disk; the caller takes
//! the text.

use std::fmt::Write;

use crate::ir::{NetKind, Type};
use crate::logic::Logic;

use super::Simulator;
use super::elab::{InstId, SigId};

/// The VCD text under construction.
pub(crate) struct VcdWriter {
    text: String,
    ids: Vec<Option<String>>,
    /// Multiplier from ticks to the `$timescale` unit.
    time_mult: u64,
    last_time: Option<u64>,
}

/// The base-94 identifier for signal index `i`.
pub(crate) fn identifier(mut i: usize) -> String {
    let mut out = String::new();
    loop {
        let digit = u8::try_from(i % 94).expect("digit below 94");
        out.push(char::from(b'!' + digit));
        i /= 94;
        if i == 0 {
            break;
        }
    }
    out
}

/// The `$timescale` text and the tick multiplier for a precision in
/// femtoseconds. Precisions that are 1, 10 or 100 of a unit map exactly;
/// anything else is written in femtoseconds with times scaled.
pub(crate) fn timescale(precision_fs: u64) -> (String, u64) {
    let units = [
        (1_000_000_000_000_000u64, "s"),
        (1_000_000_000_000, "ms"),
        (1_000_000_000, "us"),
        (1_000_000, "ns"),
        (1_000, "ps"),
        (1, "fs"),
    ];
    for (fs, name) in units {
        for mult in [100u64, 10, 1] {
            if precision_fs == fs * mult {
                return (format!("{mult}{name}"), 1);
            }
        }
    }
    ("1fs".to_owned(), precision_fs.max(1))
}

/// The value change text for a signal.
fn value_text(id: &str, value: &Logic, real: bool) -> String {
    if real {
        let r = value.to_u64().map_or(f64::NAN, f64::from_bits);
        return format!("r{r} {id}\n");
    }
    if value.width() == 1 {
        format!("{}{id}\n", value.bit(0).to_char())
    } else {
        format!("b{} {id}\n", value.to_binary_string())
    }
}

impl VcdWriter {
    /// Records a value change at `time`.
    pub(crate) fn change(&mut self, time: u64, sig: SigId, value: &Logic, real: bool) {
        let Some(Some(id)) = self.ids.get(sig.idx()) else {
            return;
        };
        if self.last_time != Some(time) {
            self.last_time = Some(time);
            let _ = writeln!(self.text, "#{}", time.saturating_mul(self.time_mult));
        }
        self.text.push_str(&value_text(id, value, real));
    }

    /// The text so far.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

impl Simulator<'_> {
    /// Builds a writer with the header and a `$dumpvars` snapshot of the
    /// current state.
    pub(crate) fn build_vcd(&self) -> VcdWriter {
        let (scale, time_mult) = timescale(self.precision_fs);
        let mut w = VcdWriter {
            text: String::new(),
            ids: vec![None; self.signals.len()],
            time_mult,
            last_time: None,
        };
        let _ = writeln!(w.text, "$timescale {scale} $end");
        if let Some(top) = self.instances.first() {
            let _ = top;
            self.vcd_scope(&mut w, InstId(0));
        }
        let _ = writeln!(w.text, "$enddefinitions $end");
        let _ = writeln!(w.text, "#{}", self.now.saturating_mul(time_mult));
        w.last_time = Some(self.now);
        let _ = writeln!(w.text, "$dumpvars");
        for (i, sig) in self.signals.iter().enumerate() {
            if let Some(id) = &w.ids[i] {
                w.text
                    .push_str(&value_text(id, sig.effective(), sig.ty == Type::Real));
            }
        }
        let _ = writeln!(w.text, "$end");
        w
    }

    /// Writes one instance's scope, recursively.
    fn vcd_scope(&self, w: &mut VcdWriter, inst: InstId) {
        let state = &self.instances[inst.idx()];
        let _ = writeln!(w.text, "$scope module {} $end", state.name);
        for (nid, net) in state.m.nets.iter() {
            let sig = state.nets[nid.index()];
            let s = &self.signals[sig.idx()];
            let width = s.width();
            if width == 0 {
                continue;
            }
            let id = w.ids[sig.idx()]
                .get_or_insert_with(|| identifier(sig.idx()))
                .clone();
            let kind = match &net.ty {
                Type::Real => "real",
                Type::Integer => "integer",
                _ => match s.kind {
                    NetKind::Wire => "wire",
                    NetKind::Reg | NetKind::Variable => "reg",
                },
            };
            if width == 1 || net.ty == Type::Real {
                let _ = writeln!(w.text, "$var {kind} {width} {id} {} $end", net.name);
            } else {
                let _ = writeln!(
                    w.text,
                    "$var {kind} {width} {id} {} [{}:0] $end",
                    net.name,
                    width - 1
                );
            }
        }
        for child in &state.children {
            self.vcd_scope(w, *child);
        }
        let _ = writeln!(w.text, "$upscope $end");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_timescales() {
        assert_eq!(identifier(0), "!");
        assert_eq!(identifier(93), "~");
        assert_eq!(identifier(94), "!\"");
        assert_eq!(timescale(1_000), ("1ps".to_owned(), 1));
        assert_eq!(timescale(10_000_000), ("10ns".to_owned(), 1));
        assert_eq!(timescale(100), ("100fs".to_owned(), 1));
        assert_eq!(timescale(1_000_000_000_000_000), ("1s".to_owned(), 1));
        assert_eq!(timescale(123), ("1fs".to_owned(), 123));
        assert_eq!(value_text("!", &Logic::ones(1), false), "1!\n");
        assert_eq!(
            value_text("!", &Logic::parse_verilog("4'b10xz").unwrap(), false),
            "b10xz !\n"
        );
        assert_eq!(
            value_text("!", &Logic::from_u64(1.5f64.to_bits(), 64), true),
            "r1.5 !\n"
        );
    }
}
