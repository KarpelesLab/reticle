//! Counter-example traces: values per frame, rendered as text or VCD.
//!
//! A [`Trace`] is what a SAT model of an unrolled design looks like to a
//! person: for every frame (clock cycle) the value of every input, state
//! element, output and internal net, as [`Logic`] vectors. Bits the solver
//! left unassigned (because nothing depended on them) read as `x`.
//!
//! Two renderers exist, both sans-I/O: [`Trace::render`] produces a text
//! table with one row per frame, which is what reports embed, and
//! [`Trace::to_vcd`] produces a minimal IEEE 1364 value change dump with
//! one time unit per frame, for waveform viewers. The simulator's own VCD
//! writer lives in `sim::vcd` when that feature is enabled; this one is
//! deliberately independent so the `formal` feature stands alone.

use std::fmt::Write;

use super::blast::{BlastedFrame, Signal};
use super::sat::{Lit, Solver};
use crate::logic::{Bit, Logic};

/// The values of one frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceFrame {
    /// Input ports and free nets, as `(name, value)`.
    pub inputs: Vec<(String, Logic)>,
    /// State elements.
    pub state: Vec<(String, Logic)>,
    /// Output ports.
    pub outputs: Vec<(String, Logic)>,
    /// Every other net.
    pub internal: Vec<(String, Logic)>,
}

/// A counter-example or witness: one [`TraceFrame`] per time step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trace {
    /// The frames, time step 0 first.
    pub frames: Vec<TraceFrame>,
}

/// Reads a vector out of the model; unassigned bits are `x`.
fn read(solver: &Solver, sig: &Signal) -> Logic {
    let bits: Vec<Bit> = sig
        .lits
        .iter()
        .map(|l: &Lit| match solver.value(l.var()) {
            Some(v) => Bit::from_bool(v ^ l.is_neg()),
            None => Bit::X,
        })
        .collect();
    Logic::from_bits(&bits).with_signed(sig.signed)
}

fn read_all(solver: &Solver, signals: &[Signal]) -> Vec<(String, Logic)> {
    signals
        .iter()
        .map(|s| (s.name.clone(), read(solver, s)))
        .collect()
}

impl Trace {
    /// Extracts the values of `frames` from the model of the last
    /// satisfiable `solve` call on `solver`.
    pub fn from_model(frames: &[BlastedFrame], solver: &Solver) -> Trace {
        Trace {
            frames: frames
                .iter()
                .map(|f| TraceFrame {
                    inputs: read_all(solver, &f.inputs),
                    state: read_all(solver, &f.state),
                    outputs: read_all(solver, &f.outputs),
                    internal: read_all(solver, &f.internal),
                })
                .collect(),
        }
    }

    /// Number of frames.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// True when there are no frames.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The columns of the text table: `(name, group)` for inputs, state
    /// and outputs, in that order.
    fn columns(&self) -> Vec<(String, usize)> {
        let Some(first) = self.frames.first() else {
            return Vec::new();
        };
        let mut cols = Vec::new();
        for (group, list) in [&first.inputs, &first.state, &first.outputs]
            .into_iter()
            .enumerate()
        {
            for (name, _) in list {
                cols.push((name.clone(), group));
            }
        }
        cols
    }

    /// Renders one row per frame with a column per input, state element
    /// and output, groups separated by `|`. Values print in binary up to
    /// four bits or when they contain `x`, else in hexadecimal.
    pub fn render(&self) -> String {
        let cols = self.columns();
        let mut out = String::new();
        if cols.is_empty() {
            return out;
        }
        let cells: Vec<Vec<String>> = self
            .frames
            .iter()
            .map(|f| {
                f.inputs
                    .iter()
                    .chain(&f.state)
                    .chain(&f.outputs)
                    .map(|(_, v)| format_value(v))
                    .collect()
            })
            .collect();
        let widths: Vec<usize> = cols
            .iter()
            .enumerate()
            .map(|(i, (name, _))| {
                cells
                    .iter()
                    .map(|row| row[i].len())
                    .max()
                    .unwrap_or(0)
                    .max(name.len())
            })
            .collect();
        let frame_w = "frame".len().max(self.frames.len().to_string().len());
        // Header.
        let _ = write!(out, "{:>frame_w$}", "frame");
        let mut group = None;
        for (i, (name, g)) in cols.iter().enumerate() {
            let sep = if group != Some(*g) { " | " } else { " " };
            group = Some(*g);
            let _ = write!(out, "{sep}{name:>w$}", w = widths[i]);
        }
        out.push('\n');
        // Rule.
        let _ = write!(out, "{}", "-".repeat(frame_w));
        group = None;
        for (i, (_, g)) in cols.iter().enumerate() {
            let sep = if group != Some(*g) { "-+-" } else { "-" };
            group = Some(*g);
            let _ = write!(out, "{sep}{}", "-".repeat(widths[i]));
        }
        out.push('\n');
        // Rows.
        for (k, row) in cells.iter().enumerate() {
            let _ = write!(out, "{k:>frame_w$}");
            group = None;
            for (i, (_, g)) in cols.iter().enumerate() {
                let sep = if group != Some(*g) { " | " } else { " " };
                group = Some(*g);
                let _ = write!(out, "{sep}{:>w$}", row[i], w = widths[i]);
            }
            out.push('\n');
        }
        out
    }

    /// Renders a value change dump with one time unit per frame and every
    /// signal (inputs, state, outputs and internal nets) as a `wire` in a
    /// single scope named `module`.
    pub fn to_vcd(&self, module: &str) -> String {
        let mut out = String::new();
        out.push_str("$timescale 1 ns $end\n");
        let _ = writeln!(out, "$scope module {module} $end");
        let Some(first) = self.frames.first() else {
            out.push_str("$upscope $end\n$enddefinitions $end\n");
            return out;
        };
        let all = |f: &TraceFrame| -> Vec<(String, Logic)> {
            f.inputs
                .iter()
                .chain(&f.state)
                .chain(&f.outputs)
                .chain(&f.internal)
                .cloned()
                .collect()
        };
        let signals = all(first);
        for (i, (name, value)) in signals.iter().enumerate() {
            let _ = writeln!(out, "$var wire {} {} {name} $end", value.width(), vcd_id(i));
        }
        out.push_str("$upscope $end\n$enddefinitions $end\n");
        let mut previous: Vec<Option<Logic>> = vec![None; signals.len()];
        for (k, frame) in self.frames.iter().enumerate() {
            let _ = writeln!(out, "#{k}");
            for (i, (_, value)) in all(frame).iter().enumerate() {
                if previous[i].as_ref() == Some(value) {
                    continue;
                }
                let id = vcd_id(i);
                if value.width() == 1 {
                    let _ = writeln!(out, "{}{id}", value.bit(0).to_char());
                } else {
                    let _ = writeln!(out, "b{} {id}", value.to_binary_string());
                }
                previous[i] = Some(value.clone());
            }
        }
        out
    }
}

/// A short printable identifier for signal `i`, in the VCD style.
fn vcd_id(i: usize) -> String {
    let mut n = i;
    let mut id = String::new();
    loop {
        let digit = u8::try_from(n % 94).expect("digit below 94");
        id.push(char::from(b'!' + digit));
        n /= 94;
        if n == 0 {
            break;
        }
        n -= 1;
    }
    id
}

/// Formats a value for the text table.
fn format_value(v: &Logic) -> String {
    if v.width() <= 4 || v.has_unknown() {
        v.to_binary_string()
    } else {
        let mut hex = String::new();
        let digits = v.width().div_ceil(4);
        for d in (0..digits).rev() {
            let lo = d * 4;
            let hi = (lo + 3).min(v.width() - 1);
            let nibble = v.slice(hi, lo).to_u64().unwrap_or(0);
            let _ = write!(hex, "{nibble:x}");
        }
        hex
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(k: u64) -> TraceFrame {
        TraceFrame {
            inputs: vec![
                ("rst".into(), Logic::from_u64(u64::from(k == 0), 1)),
                ("d".into(), Logic::from_u64(k * 37 % 256, 8)),
            ],
            state: vec![("q".into(), Logic::from_u64(k, 4))],
            outputs: vec![("y".into(), Logic::from_bits(&[Bit::X, Bit::One]))],
            internal: vec![("t".into(), Logic::from_u64(k % 2, 1))],
        }
    }

    #[test]
    fn renders_text_table() {
        let trace = Trace {
            frames: (0..3).map(frame).collect(),
        };
        assert_eq!(trace.len(), 3);
        assert!(!trace.is_empty());
        let expected = "\
frame | rst  d |    q |  y
------+--------+------+---
    0 |   1 00 | 0000 | 1x
    1 |   0 25 | 0001 | 1x
    2 |   0 4a | 0010 | 1x
";
        assert_eq!(trace.render(), expected);
        assert_eq!(Trace::default().render(), "");
    }

    #[test]
    fn renders_vcd() {
        let trace = Trace {
            frames: (0..2).map(frame).collect(),
        };
        let expected = "\
$timescale 1 ns $end
$scope module top $end
$var wire 1 ! rst $end
$var wire 8 \" d $end
$var wire 4 # q $end
$var wire 2 $ y $end
$var wire 1 % t $end
$upscope $end
$enddefinitions $end
#0
1!
b00000000 \"
b0000 #
b1x $
0%
#1
0!
b00100101 \"
b0001 #
1%
";
        assert_eq!(trace.to_vcd("top"), expected);
        assert!(
            Trace::default()
                .to_vcd("e")
                .ends_with("$enddefinitions $end\n")
        );
    }

    #[test]
    fn vcd_ids_are_unique_and_printable() {
        let ids: Vec<String> = (0..200).map(vcd_id).collect();
        for id in &ids {
            assert!(id.chars().all(|c| ('!'..='~').contains(&c)));
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
        assert_eq!(vcd_id(94), "!!");
    }

    #[test]
    fn formats_values() {
        assert_eq!(format_value(&Logic::from_u64(5, 3)), "101");
        assert_eq!(format_value(&Logic::from_u64(0xbeef, 16)), "beef");
        assert_eq!(format_value(&Logic::from_u64(0x1f, 5)), "1f");
        assert_eq!(format_value(&Logic::x(8)), "xxxxxxxx");
    }
}
