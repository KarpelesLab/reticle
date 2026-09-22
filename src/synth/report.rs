//! Synthesis reports: what came out, and what each pass did.
//!
//! [`Report::of_design`] summarises a design after synthesis: per module,
//! the number of cells of each kind, the nets and assigns, any processes
//! left unsynthesised, and the inferred storage (flip-flops, latches,
//! memory ports) with the source span each was inferred from.
//! [`SynthStats`] adds the per-pass counters collected by [`super::run`]
//! and, with the `formal` feature, the post-synthesis equivalence
//! verdicts.
//! Both render to plain text, with `file:line:col` locations when a
//! [`SourceMap`] is supplied.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::ir::{CellKind, Design, Module};
use crate::source::{SourceMap, Span};
use crate::synth::PassStats;

/// One inferred storage element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageItem {
    /// The cell name.
    pub cell: String,
    /// What was inferred: `flip-flop`, `latch`, `memory read port`,
    /// `memory write port`.
    pub kind: &'static str,
    /// The net driven, or the memory accessed.
    pub target: String,
    /// Width in bits of the stored value (element width for ports).
    pub width: u32,
    /// Features in words: `enable`, `sync reset 8'd0`, `clocked`, ...
    pub features: Vec<String>,
    /// Where the source construct was.
    pub span: Span,
}

/// The summary of one module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleReport {
    /// The module name.
    pub name: String,
    /// Cell counts by kind keyword, sorted by keyword.
    pub cells: Vec<(String, usize)>,
    /// Number of nets.
    pub nets: usize,
    /// Number of continuous assigns.
    pub assigns: usize,
    /// Number of instances.
    pub instances: usize,
    /// Processes left in the module (unsynthesisable ones).
    pub processes: usize,
    /// Inferred storage, in cell order.
    pub storage: Vec<StorageItem>,
}

/// The summary of a design.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// One entry per non-black-box module, in design order.
    pub modules: Vec<ModuleReport>,
}

impl Report {
    /// Summarises every module of `design`.
    pub fn of_design(design: &Design) -> Report {
        Report {
            modules: design
                .modules
                .values()
                .filter(|m| !m.blackbox)
                .map(Self::of_module)
                .collect(),
        }
    }

    /// Summarises one module.
    pub fn of_module(m: &Module) -> ModuleReport {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut storage = Vec::new();
        for (_, cell) in m.cells.iter() {
            *counts.entry(cell.kind.keyword().to_owned()).or_insert(0) += 1;
            let item = match &cell.kind {
                CellKind::Dff {
                    clk_pos,
                    has_enable,
                    reset,
                } => {
                    let q = cell.output("q");
                    let mut features =
                        vec![if *clk_pos { "posedge" } else { "negedge" }.to_owned()];
                    if *has_enable {
                        features.push("enable".to_owned());
                    }
                    if let Some(r) = reset {
                        features.push(format!(
                            "{} reset {} {}",
                            if r.asynchronous { "async" } else { "sync" },
                            if r.active_high {
                                "active-high"
                            } else {
                                "active-low"
                            },
                            crate::synth::util::const_text(&r.value)
                        ));
                    }
                    Some(StorageItem {
                        cell: cell.name.to_string(),
                        kind: "flip-flop",
                        target: q.map_or(String::new(), |n| m.nets[n].name.to_string()),
                        width: q.and_then(|n| m.nets[n].ty.width()).unwrap_or(0),
                        features,
                        span: cell.span,
                    })
                }
                CellKind::Dlatch => {
                    let q = cell.output("q");
                    Some(StorageItem {
                        cell: cell.name.to_string(),
                        kind: "latch",
                        target: q.map_or(String::new(), |n| m.nets[n].name.to_string()),
                        width: q.and_then(|n| m.nets[n].ty.width()).unwrap_or(0),
                        features: Vec::new(),
                        span: cell.span,
                    })
                }
                CellKind::MemRdPort { mem, clocked } => Some(StorageItem {
                    cell: cell.name.to_string(),
                    kind: "memory read port",
                    target: m.memories[*mem].name.to_string(),
                    width: m.memories[*mem].elem.width().unwrap_or(0),
                    features: if *clocked {
                        vec!["clocked".to_owned()]
                    } else {
                        vec!["asynchronous".to_owned()]
                    },
                    span: cell.span,
                }),
                CellKind::MemWrPort { mem, clocked } => Some(StorageItem {
                    cell: cell.name.to_string(),
                    kind: "memory write port",
                    target: m.memories[*mem].name.to_string(),
                    width: m.memories[*mem].elem.width().unwrap_or(0),
                    features: if *clocked {
                        vec!["clocked".to_owned()]
                    } else {
                        vec!["asynchronous".to_owned()]
                    },
                    span: cell.span,
                }),
                _ => None,
            };
            storage.extend(item);
        }
        ModuleReport {
            name: m.name.to_string(),
            cells: counts.into_iter().collect(),
            nets: m.nets.len(),
            assigns: m.assigns.len(),
            instances: m.instances.len(),
            processes: m.processes.len(),
            storage,
        }
    }

    /// Renders the report as text.
    pub fn render(&self, map: Option<&SourceMap>) -> String {
        let mut out = String::new();
        for module in &self.modules {
            let _ = writeln!(out, "module {}", module.name);
            let _ = writeln!(
                out,
                "  nets: {}  assigns: {}  instances: {}  cells: {}",
                module.nets,
                module.assigns,
                module.instances,
                module.cells.iter().map(|(_, n)| n).sum::<usize>()
            );
            if module.processes > 0 {
                let _ = writeln!(out, "  unsynthesised processes: {}", module.processes);
            }
            for (kind, n) in &module.cells {
                let _ = writeln!(out, "    {kind:<10} {n}");
            }
            if !module.storage.is_empty() {
                let _ = writeln!(out, "  inferred storage:");
                for item in &module.storage {
                    let _ = write!(
                        out,
                        "    {} `{}` -> {} ({} bit{}",
                        item.kind,
                        item.cell,
                        item.target,
                        item.width,
                        if item.width == 1 { "" } else { "s" }
                    );
                    for f in &item.features {
                        let _ = write!(out, ", {f}");
                    }
                    out.push(')');
                    if let Some(map) = map {
                        let (name, loc) = map.locate(item.span);
                        let _ = write!(out, " at {name}:{loc}");
                    }
                    out.push('\n');
                }
            }
        }
        out
    }
}

/// Everything [`super::run`] collected.
#[derive(Clone, Debug, Default)]
pub struct SynthStats {
    /// Every pass run, in order, with its statistics summed over modules.
    pub passes: Vec<(String, PassStats)>,
    /// Number of optimisation-loop iterations executed.
    pub iterations: u32,
    /// The summary of the result.
    pub report: Report,
    /// One entry per module checked by
    /// [`crate::synth::verify::check_synthesis`], empty unless
    /// [`crate::synth::SynthOptions::verify_equivalence`] was set.
    #[cfg(feature = "formal")]
    pub verification: Vec<crate::synth::verify::SynthVerifyReport>,
}

impl SynthStats {
    /// Renders the pass log followed by the report.
    pub fn render(&self, map: Option<&SourceMap>) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "passes ({} optimisation iteration{}):",
            self.iterations,
            if self.iterations == 1 { "" } else { "s" }
        );
        for (name, stats) in &self.passes {
            if stats.counters.is_empty() {
                let _ = writeln!(out, "  {name}: no change");
                continue;
            }
            let items: Vec<String> = stats
                .counters
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect();
            let _ = writeln!(out, "  {name}: {}", items.join(", "));
        }
        #[cfg(feature = "formal")]
        for report in &self.verification {
            out.push_str(&report.render());
        }
        out.push_str(&self.report.render(map));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Design;

    #[test]
    fn renders_netlist_report() {
        let text = include_str!("../../testdata/ir/netlist.rtl");
        let mut map = SourceMap::new();
        let file = map.add("netlist.rtl", text).unwrap();
        let design = Design::parse_text(text, file).unwrap();
        let report = Report::of_design(&design);
        assert_eq!(report.modules.len(), 1);
        let m = &report.modules[0];
        assert_eq!(m.storage.len(), 3);
        assert_eq!(m.storage[0].kind, "flip-flop");
        assert_eq!(
            m.storage[0].features,
            ["posedge", "sync reset active-high 4'd0"]
        );
        let rendered = report.render(Some(&map));
        assert!(rendered.contains("module counter_synth"));
        assert!(rendered.contains("    dff        1"));
        assert!(rendered.contains(
            "flip-flop `ff0` -> q (4 bits, posedge, sync reset active-high 4'd0) at netlist.rtl:"
        ));
        assert!(rendered.contains("memory write port `wr0` -> buf (8 bits, clocked)"));
        let plain = report.render(None);
        assert!(!plain.contains(" at "));
        let mut stats = SynthStats {
            report,
            ..Default::default()
        };
        let mut p = PassStats::default();
        p.bump("things", 2);
        stats.passes.push(("a".into(), PassStats::default()));
        stats.passes.push(("b".into(), p));
        let s = stats.render(None);
        assert!(
            s.starts_with("passes (0 optimisation iterations):\n  a: no change\n  b: things 2\n")
        );
    }
}
