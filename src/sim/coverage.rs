//! Line and toggle coverage.
//!
//! Collection is behind [`SimOptions::coverage`](super::SimOptions), so a
//! run that does not ask for it pays one `Option` test per statement and
//! one per signal change and nothing else: no tables are allocated and no
//! spans are looked up.
//!
//! # What is measured
//!
//! *Line coverage* counts executions per statement, keyed by the
//! statement's [`Span`]. Every statement of every instantiated module is
//! entered at elaboration with a count of zero, so a report distinguishes
//! "never ran" from "not in the design". A module instantiated twice
//! shares its statements' spans, so its counts accumulate over the
//! instances, which is what a line report is expected to show. Rendering
//! aggregates statements to source lines through a [`SourceMap`], since
//! the simulator holds spans and not line numbers.
//!
//! *Toggle coverage* records, per bit of every net, whether it was ever
//! seen going `0` to `1` and whether it was ever seen going `1` to `0`.
//! Transitions through `x` or `z` are not toggles: `0 -> x -> 1` sets
//! neither flag, which is the conservative reading and the one a synthesis
//! flow cares about. Nets that alias (a port connected to a plain net of
//! the parent) are one signal and so are counted once, under the name the
//! first net gave them.
//!
//! # Output
//!
//! [`CoverageReport::render`] is the human-readable form: percentages per
//! file and per module, then the uncovered lines and the untoggled bits.
//! [`CoverageReport::to_lcov`] writes the `.info` format `lcov`,
//! `genhtml` and every coverage viewer already read: statements become
//! `DA` records and toggles become `BRDA` branch records, two per bit (a
//! rise and a fall) on the line the net is declared on. Both are sorted,
//! so a golden file compares byte for byte.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::ir::walk::walk_block;
use crate::logic::{Bit, Logic};
use crate::source::{SourceMap, Span};

use super::Simulator;
use super::elab::SigId;

/// A fixed set of bits, one per net bit.
#[derive(Clone, Debug, Default)]
struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    /// A set with room for `bits` bits, all clear.
    fn with_bits(bits: u32) -> BitSet {
        let words = usize::try_from(bits.div_ceil(64)).unwrap_or(0);
        BitSet {
            words: vec![0; words],
        }
    }

    /// Adds a bit.
    fn set(&mut self, i: u32) {
        let word = usize::try_from(i / 64).unwrap_or(usize::MAX);
        if let Some(w) = self.words.get_mut(word) {
            *w |= 1u64 << (i % 64);
        }
    }

    /// Whether a bit is in the set.
    fn get(&self, i: u32) -> bool {
        let word = usize::try_from(i / 64).unwrap_or(usize::MAX);
        self.words.get(word).is_some_and(|w| w >> (i % 64) & 1 == 1)
    }
}

/// The rise and fall flags of one signal.
#[derive(Clone, Debug, Default)]
struct Toggle {
    rose: BitSet,
    fell: BitSet,
}

/// One statement's counter.
#[derive(Clone, Debug)]
struct StmtEntry {
    module: String,
    span: Span,
    hits: u64,
}

/// The counters a run fills in, held by the [`Simulator`].
#[derive(Clone, Debug, Default)]
pub(crate) struct Coverage {
    /// Keyed by file, start and end so the order is deterministic.
    stmts: BTreeMap<(u32, u32, u32), StmtEntry>,
    /// Indexed by [`SigId`].
    toggles: Vec<Toggle>,
}

/// The key a span is stored under.
fn key(span: Span) -> (u32, u32, u32) {
    (
        u32::try_from(span.file.index()).unwrap_or(u32::MAX),
        span.start,
        span.end,
    )
}

impl Simulator<'_> {
    /// Builds the coverage tables; called from elaboration when
    /// [`SimOptions::coverage`](super::SimOptions) is on.
    pub(crate) fn init_coverage(&mut self) {
        let mut cov = Coverage {
            stmts: BTreeMap::new(),
            toggles: self
                .signals
                .iter()
                .map(|s| Toggle {
                    rose: BitSet::with_bits(s.value.width()),
                    fell: BitSet::with_bits(s.value.width()),
                })
                .collect(),
        };
        let mut seen: Vec<&str> = Vec::new();
        for inst in &self.instances {
            if seen.contains(&inst.m.name.as_str()) {
                continue;
            }
            seen.push(inst.m.name.as_str());
            let module = inst.m.name.as_str().to_owned();
            for (_, process) in inst.m.processes.iter() {
                walk_block(&process.body, &mut |stmt| {
                    cov.stmts
                        .entry(key(stmt.span))
                        .or_insert_with(|| StmtEntry {
                            module: module.clone(),
                            span: stmt.span,
                            hits: 0,
                        });
                });
            }
        }
        self.coverage = Some(Box::new(cov));
    }

    /// Records one statement execution.
    pub(crate) fn cover_stmt(&mut self, span: Span) {
        let Some(cov) = &mut self.coverage else {
            return;
        };
        match cov.stmts.get_mut(&key(span)) {
            Some(entry) => entry.hits = entry.hits.saturating_add(1),
            None => {
                cov.stmts.insert(
                    key(span),
                    StmtEntry {
                        module: String::new(),
                        span,
                        hits: 1,
                    },
                );
            }
        }
    }

    /// Records the toggles in a signal change.
    pub(crate) fn cover_toggle(&mut self, sig: SigId, old: &Logic, new: &Logic) {
        let Some(cov) = &mut self.coverage else {
            return;
        };
        let Some(toggle) = cov.toggles.get_mut(sig.idx()) else {
            return;
        };
        for i in 0..new.width().min(old.width()) {
            match (old.bit(i), new.bit(i)) {
                (Bit::Zero, Bit::One) => toggle.rose.set(i),
                (Bit::One, Bit::Zero) => toggle.fell.set(i),
                _ => {}
            }
        }
    }

    /// The coverage gathered so far, or `None` when collection is off.
    pub fn coverage(&self) -> Option<CoverageReport> {
        let cov = self.coverage.as_ref()?;
        let lines = cov
            .stmts
            .values()
            .map(|e| LineRecord {
                module: e.module.clone(),
                span: e.span,
                hits: e.hits,
            })
            .collect();
        let mut toggles: Vec<ToggleRecord> = self
            .signals
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let width = s.value.width();
                let empty = Toggle::default();
                let t = cov.toggles.get(i).unwrap_or(&empty);
                ToggleRecord {
                    net: s.name.clone(),
                    module: s.module.as_str().to_owned(),
                    span: s.span,
                    rose: (0..width).map(|b| t.rose.get(b)).collect(),
                    fell: (0..width).map(|b| t.fell.get(b)).collect(),
                }
            })
            .collect();
        toggles.sort_by(|a, b| a.net.cmp(&b.net));
        Some(CoverageReport { lines, toggles })
    }
}

/// One statement's coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRecord {
    /// The module the statement belongs to.
    pub module: String,
    /// Where the statement is.
    pub span: Span,
    /// How often it executed.
    pub hits: u64,
}

/// One net's toggle coverage, one entry per bit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToggleRecord {
    /// The hierarchical net name.
    pub net: String,
    /// The module that declares it.
    pub module: String,
    /// Where it is declared.
    pub span: Span,
    /// Whether each bit was seen going `0` to `1`, least significant
    /// first.
    pub rose: Vec<bool>,
    /// Whether each bit was seen going `1` to `0`.
    pub fell: Vec<bool>,
}

impl ToggleRecord {
    /// The number of bits, and how many toggled both ways.
    pub fn counts(&self) -> (u64, u64) {
        let total = u64::try_from(self.rose.len()).unwrap_or(u64::MAX);
        let covered = self
            .rose
            .iter()
            .zip(&self.fell)
            .filter(|(r, f)| **r && **f)
            .count();
        (u64::try_from(covered).unwrap_or(u64::MAX), total)
    }
}

/// Line and toggle coverage of one run.
///
/// Build it with [`Simulator::coverage`]; render it with
/// [`CoverageReport::render`] or [`CoverageReport::to_lcov`], both of
/// which take the [`SourceMap`] the design was parsed from, since spans
/// are byte offsets and a report wants file names and line numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageReport {
    lines: Vec<LineRecord>,
    toggles: Vec<ToggleRecord>,
}

impl CoverageReport {
    /// Every statement, ordered by file and position.
    pub fn lines(&self) -> &[LineRecord] {
        &self.lines
    }

    /// Every net, ordered by name.
    pub fn toggles(&self) -> &[ToggleRecord] {
        &self.toggles
    }

    /// Statements executed at least once, and statements in total.
    pub fn line_summary(&self) -> (u64, u64) {
        let total = u64::try_from(self.lines.len()).unwrap_or(u64::MAX);
        let hit = self.lines.iter().filter(|l| l.hits > 0).count();
        (u64::try_from(hit).unwrap_or(u64::MAX), total)
    }

    /// Net bits that toggled both ways, and net bits in total.
    pub fn toggle_summary(&self) -> (u64, u64) {
        self.toggles.iter().fold((0, 0), |(c, t), r| {
            let (rc, rt) = r.counts();
            (c + rc, t + rt)
        })
    }

    /// The human-readable report: percentages per file and per module,
    /// then what is not covered.
    pub fn render(&self, map: &SourceMap) -> String {
        let mut out = String::new();
        let (hit, total) = self.line_summary();
        out.push_str("coverage\n========\n\n");
        let _ = writeln!(
            out,
            "line coverage: {hit}/{total} statements ({}%)",
            percent(hit, total)
        );
        self.render_groups(&mut out, "files", &self.line_groups_by_file(map));
        self.render_groups(&mut out, "modules", &self.line_groups_by_module());
        let uncovered: Vec<&LineRecord> = self.lines.iter().filter(|l| l.hits == 0).collect();
        if uncovered.is_empty() {
            out.push_str("\n  every statement ran\n");
        } else {
            out.push_str("\n  uncovered lines\n");
            let mut seen: Vec<(String, u32, String)> = Vec::new();
            for record in uncovered {
                let (file, loc) = map.locate(record.span);
                let entry = (file.to_owned(), loc.line, record.module.clone());
                if !seen.contains(&entry) {
                    seen.push(entry);
                }
            }
            seen.sort();
            for (file, line, module) in seen {
                let _ = writeln!(out, "    {file}:{line} ({module})");
            }
        }
        let (hit, total) = self.toggle_summary();
        let _ = writeln!(
            out,
            "\ntoggle coverage: {hit}/{total} bits ({}%)",
            percent(hit, total)
        );
        self.render_groups(&mut out, "modules", &self.toggle_groups_by_module());
        let mut untoggled = Vec::new();
        for record in &self.toggles {
            for (bit, (rose, fell)) in record.rose.iter().zip(&record.fell).enumerate() {
                if *rose && *fell {
                    continue;
                }
                let why = match (rose, fell) {
                    (false, false) => "never toggled",
                    (false, true) => "never rose",
                    _ => "never fell",
                };
                untoggled.push(format!("    {}[{bit}]  {why}", record.net));
            }
        }
        if untoggled.is_empty() {
            out.push_str("\n  every bit toggled\n");
        } else {
            out.push_str("\n  untoggled bits\n");
            for line in untoggled {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out
    }

    /// Writes one `name` / covered / total table.
    fn render_groups(&self, out: &mut String, title: &str, groups: &[(String, u64, u64)]) {
        if groups.is_empty() {
            return;
        }
        let _ = writeln!(out, "\n  {title}");
        let width = groups.iter().map(|(n, _, _)| n.len()).max().unwrap_or(0);
        for (name, hit, total) in groups {
            let _ = writeln!(
                out,
                "    {name:width$}  {hit:>5}/{total:<5}  {}%",
                percent(*hit, *total)
            );
        }
    }

    /// Statement counts per source file, sorted by name.
    fn line_groups_by_file(&self, map: &SourceMap) -> Vec<(String, u64, u64)> {
        let mut groups: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for record in &self.lines {
            let (file, _) = map.locate(record.span);
            let entry = groups.entry(file.to_owned()).or_insert((0, 0));
            entry.1 += 1;
            if record.hits > 0 {
                entry.0 += 1;
            }
        }
        groups.into_iter().map(|(k, v)| (k, v.0, v.1)).collect()
    }

    /// Statement counts per module, sorted by name.
    fn line_groups_by_module(&self) -> Vec<(String, u64, u64)> {
        let mut groups: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for record in &self.lines {
            let entry = groups.entry(record.module.clone()).or_insert((0, 0));
            entry.1 += 1;
            if record.hits > 0 {
                entry.0 += 1;
            }
        }
        groups.into_iter().map(|(k, v)| (k, v.0, v.1)).collect()
    }

    /// Toggle counts per module, sorted by name.
    fn toggle_groups_by_module(&self) -> Vec<(String, u64, u64)> {
        let mut groups: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for record in &self.toggles {
            let (covered, total) = record.counts();
            let entry = groups.entry(record.module.clone()).or_insert((0, 0));
            entry.0 += covered;
            entry.1 += total;
        }
        groups.into_iter().map(|(k, v)| (k, v.0, v.1)).collect()
    }

    /// The report as an LCOV `.info` file.
    ///
    /// One record per source file: `DA` per line with its execution count,
    /// and two `BRDA` branch records per net bit, branch `2 * bit` for the
    /// rise and `2 * bit + 1` for the fall, on the line the net is
    /// declared on.
    pub fn to_lcov(&self, map: &SourceMap) -> String {
        // file -> line -> hits
        let mut files: BTreeMap<String, BTreeMap<u32, u64>> = BTreeMap::new();
        for record in &self.lines {
            let (file, loc) = map.locate(record.span);
            let entry = files.entry(file.to_owned()).or_default();
            let hits = entry.entry(loc.line).or_insert(0);
            *hits = hits.saturating_add(record.hits);
        }
        // file -> line -> nets
        let mut branches: BTreeMap<String, BTreeMap<u32, Vec<&ToggleRecord>>> = BTreeMap::new();
        for record in &self.toggles {
            if record.rose.is_empty() {
                continue;
            }
            let (file, loc) = map.locate(record.span);
            branches
                .entry(file.to_owned())
                .or_default()
                .entry(loc.line)
                .or_default()
                .push(record);
            files.entry(file.to_owned()).or_default();
        }
        let mut out = String::new();
        for (file, lines) in &files {
            out.push_str("TN:\n");
            let _ = writeln!(out, "SF:{file}");
            let empty = BTreeMap::new();
            let file_branches = branches.get(file).unwrap_or(&empty);
            let (mut found, mut hit) = (0u64, 0u64);
            for (line, nets) in file_branches {
                for (block, record) in nets.iter().enumerate() {
                    for (bit, (rose, fell)) in record.rose.iter().zip(&record.fell).enumerate() {
                        for (offset, taken) in [(0, rose), (1, fell)] {
                            let branch = bit * 2 + offset;
                            let _ = writeln!(
                                out,
                                "BRDA:{line},{block},{branch},{}",
                                if *taken { "1" } else { "0" }
                            );
                            found += 1;
                            if *taken {
                                hit += 1;
                            }
                        }
                    }
                }
            }
            if found > 0 {
                let _ = writeln!(out, "BRF:{found}");
                let _ = writeln!(out, "BRH:{hit}");
            }
            let mut lf = 0u64;
            let mut lh = 0u64;
            for (line, hits) in lines {
                let _ = writeln!(out, "DA:{line},{hits}");
                lf += 1;
                if *hits > 0 {
                    lh += 1;
                }
            }
            let _ = writeln!(out, "LF:{lf}");
            let _ = writeln!(out, "LH:{lh}");
            out.push_str("end_of_record\n");
        }
        out
    }
}

/// A percentage with one decimal, computed without floating point so the
/// text is identical everywhere. An empty population is 100%.
fn percent(covered: u64, total: u64) -> String {
    if total == 0 {
        return "100.0".to_owned();
    }
    let tenths = covered.saturating_mul(1000) / total;
    format!("{}.{}", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Design;
    use crate::sim::{SimOptions, Simulator};
    use crate::source::SourceMap;

    const DESIGN: &str = "\
module cov_tb
  timescale 1 ns / 1 ps
  net %clk u1 reg
  net %q u4 reg
  net %never u2 reg
  process clkgen free
    wait for 8'd5
    %clk = not(%clk)
  end
  process stim initial
    %clk = 1'd0
    %q = 4'd0
    wait for 8'd38
    if %q
      %q = 4'd7
    else
      %q = 4'd8
    end
    finish
  end
  process count seq posedge %clk
    %q <= add(%q, 4'd1)
  end
end
";

    fn run(coverage: bool) -> (SourceMap, Option<CoverageReport>) {
        let mut map = SourceMap::new();
        let file = map.add("cov.rtl", DESIGN).unwrap();
        let design = Design::parse_text(DESIGN, file).unwrap();
        let options = SimOptions {
            coverage,
            ..SimOptions::default()
        };
        let mut sim = Simulator::new(&design, options).unwrap();
        sim.run();
        let report = sim.coverage();
        (map, report)
    }

    #[test]
    fn collection_is_off_by_default() {
        let (_, report) = run(false);
        assert!(report.is_none());
    }

    #[test]
    fn line_and_toggle_accounting() {
        let (map, report) = run(true);
        let report = report.expect("coverage on");
        let (hit, total) = report.line_summary();
        assert!(total > 0);
        assert!(hit > 0 && hit < total, "{hit}/{total}");
        // The `%q = 4'd7` arm never runs: `%q` is non-zero at time 38, so
        // exactly one of the two arms stays uncovered.
        let uncovered: Vec<&LineRecord> = report.lines().iter().filter(|l| l.hits == 0).collect();
        assert_eq!(uncovered.len(), 1);
        assert_eq!(uncovered[0].module, "cov_tb");
        // The clock toggles both ways; `never` never does.
        let clk = report
            .toggles()
            .iter()
            .find(|t| t.net == "cov_tb.clk")
            .expect("clk");
        assert_eq!(clk.counts(), (1, 1));
        let never = report
            .toggles()
            .iter()
            .find(|t| t.net == "cov_tb.never")
            .expect("never");
        assert_eq!(never.counts(), (0, 2));
        assert!(never.rose.iter().all(|b| !b));
        let (bits, total) = report.toggle_summary();
        assert!(bits > 0 && bits < total);
        // The renderer names both the uncovered line and the dead bits.
        let text = report.render(&map);
        assert!(text.contains("line coverage:"), "{text}");
        assert!(text.contains("uncovered lines"), "{text}");
        assert!(text.contains("cov.rtl:"), "{text}");
        assert!(text.contains("cov_tb.never[0]  never toggled"), "{text}");
        assert!(text.contains("cov_tb"), "{text}");
    }

    #[test]
    fn lcov_is_readable() {
        let (map, report) = run(true);
        let lcov = report.expect("coverage on").to_lcov(&map);
        assert!(lcov.starts_with("TN:\nSF:cov.rtl\n"), "{lcov}");
        assert!(lcov.contains("\nDA:"), "{lcov}");
        assert!(lcov.contains("\nBRDA:"), "{lcov}");
        assert!(lcov.contains("\nLF:"), "{lcov}");
        assert!(lcov.contains("\nLH:"), "{lcov}");
        assert!(lcov.ends_with("end_of_record\n"), "{lcov}");
        // Deterministic: the same run renders identically.
        let (map2, report2) = run(true);
        assert_eq!(lcov, report2.expect("coverage on").to_lcov(&map2));
    }

    #[test]
    fn percentages() {
        assert_eq!(percent(0, 0), "100.0");
        assert_eq!(percent(1, 2), "50.0");
        assert_eq!(percent(1, 3), "33.3");
        assert_eq!(percent(3, 3), "100.0");
        assert_eq!(percent(0, 4), "0.0");
    }

    #[test]
    fn bit_sets() {
        let mut set = BitSet::with_bits(70);
        assert!(!set.get(0));
        set.set(0);
        set.set(69);
        assert!(set.get(0));
        assert!(set.get(69));
        assert!(!set.get(1));
        // Out of range is ignored rather than panicking.
        set.set(1000);
        assert!(!set.get(1000));
    }
}
