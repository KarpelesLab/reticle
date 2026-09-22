//! Every golden design must come out of the default synthesis pipeline as
//! a netlist.
//!
//! That is the whole point of [`reticle::synth::cellify`]: before it, an
//! `add` left in a continuous assignment made `emit --format json` fail
//! with "`add` is not a netlist connection", and BLIF and EDIF failed the
//! same way. So this test runs every `testdata/synth/*.rtl` through
//! [`reticle::synth::run`] with the default options and emits the result
//! in each netlist format.
//!
//! Three things are checked:
//!
//! 1. **No emitter ever fails over an expression.** Whatever a format
//!    cannot do, it is never "this is not a netlist connection".
//! 2. **JSON and EDIF succeed** for every design whose modules are fully
//!    synthesised. Both formats can name every IR cell, so there is no
//!    excuse for a failure. `unsynth.rtl` is the one design that keeps a
//!    process on purpose (it has a `wait`), and the netlist formats
//!    rightly refuse it.
//! 3. **BLIF succeeds once the design is bit-level.** BLIF has no
//!    arithmetic and no memories (see `ir::emit::blif`), so the designs
//!    are LUT-mapped first; what is left of the exceptions is listed in
//!    [`blif_excuse`].

#![cfg(feature = "synth")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::ir::emit::{Format, emit};
use reticle::ir::{CellKind, Design};
use reticle::source::SourceMap;
use reticle::synth::techmap::{MapOptions, map_module};
use reticle::synth::{SynthOptions, run};

/// Every process-form golden design, by name.
fn inputs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/synth");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.ends_with(".rtl") && !name.ends_with(".synth.rtl") && !name.ends_with(".cells.rtl")
        })
        .collect();
    paths.sort();
    paths
}

/// Parses and synthesises one golden design with the default pipeline.
fn synthesise(path: &Path) -> Design {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let text = fs::read_to_string(path).unwrap();
    let mut map = SourceMap::new();
    let file = map.add(name.clone(), text.clone()).unwrap();
    let mut design = Design::parse_text(&text, file)
        .unwrap_or_else(|d| panic!("{name}: parse failed\n{}", d.render(&map)));
    let mut diags = Diagnostics::new();
    run(
        &mut design,
        &SynthOptions {
            validate: true,
            ..SynthOptions::default()
        },
        &mut diags,
    );
    design
}

/// True when the design still holds a process, which every netlist format
/// refuses (and should).
fn has_processes(design: &Design) -> bool {
    design.modules.values().any(|m| !m.processes.is_empty())
}

/// Why BLIF may legitimately refuse a design, or `None` when it must not.
///
/// BLIF is a bit-level format: it has `.names` and `.latch` and nothing
/// else. Memories and asynchronous resets have no rendering at all, and
/// neither does an unsynthesised process.
fn blif_excuse(design: &Design) -> Option<&'static str> {
    if has_processes(design) {
        return Some("an unsynthesisable process");
    }
    for module in design.modules.values() {
        if !module.memories.is_empty() {
            return Some("a memory");
        }
        for (_, cell) in module.cells.iter() {
            if let CellKind::Dff {
                reset: Some(reset), ..
            } = &cell.kind
                && reset.asynchronous
            {
                return Some("an asynchronous reset");
            }
        }
    }
    None
}

#[test]
fn golden_designs_emit_as_netlists() {
    let paths = inputs();
    assert!(!paths.is_empty(), "no golden files found");
    let mut failures = Vec::new();
    let mut emitted = [0usize; 3];

    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let design = synthesise(path);
        let synthesised = !has_processes(&design);

        for (i, format) in [Format::Json, Format::Edif].into_iter().enumerate() {
            match emit(&design, format) {
                Ok(text) => {
                    assert!(!text.is_empty(), "{name}: {format:?} output is empty");
                    emitted[i] += 1;
                }
                Err(e) => {
                    if e.message.contains("is not a netlist connection") {
                        failures.push(format!("{name}: {format:?}: {}", e.message));
                    } else if synthesised {
                        failures.push(format!(
                            "{name}: {format:?} should have emitted, but: {}",
                            e.message
                        ));
                    }
                }
            }
        }

        // BLIF wants gates, so map to LUTs first.
        let mut mapped = design.clone();
        for id in mapped.modules.ids().collect::<Vec<_>>() {
            if mapped.module(id).blackbox || !mapped.module(id).processes.is_empty() {
                continue;
            }
            map_module(mapped.module_mut(id), &MapOptions::lut(4));
        }
        match emit(&mapped, Format::Blif) {
            Ok(text) => {
                assert!(!text.is_empty(), "{name}: BLIF output is empty");
                emitted[2] += 1;
            }
            Err(e) => {
                if e.message.contains("is not a netlist connection") {
                    failures.push(format!("{name}: BLIF: {}", e.message));
                } else if let Some(excuse) = blif_excuse(&design) {
                    assert!(
                        !e.message.is_empty(),
                        "{name}: BLIF refused it over {excuse} without saying why"
                    );
                } else {
                    failures.push(format!(
                        "{name}: BLIF should have emitted, but: {}",
                        e.message
                    ));
                }
            }
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    // `unsynth.rtl` keeps a process on purpose; everything else must
    // emit. BLIF additionally loses the memory and asynchronous-reset
    // designs, so it is two short of the others.
    let expected = paths.len() - 1;
    assert_eq!(emitted[0], expected, "JSON");
    assert_eq!(emitted[1], expected, "EDIF");
    assert_eq!(emitted[2], expected - 2, "BLIF");
}

/// The pass really is what makes the difference: with `cellify` off, the
/// same designs are rejected, and the message is the one the pass exists
/// to remove.
#[test]
fn without_cellify_the_netlist_formats_refuse() {
    let mut refused = 0;
    for path in inputs() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        let mut map = SourceMap::new();
        let file = map.add(name.clone(), text.clone()).unwrap();
        let mut design = Design::parse_text(&text, file).unwrap();
        let mut diags = Diagnostics::new();
        run(
            &mut design,
            &SynthOptions {
                cellify: false,
                ..SynthOptions::default()
            },
            &mut diags,
        );
        if let Err(e) = emit(&design, Format::Json)
            && e.message.contains("is not a netlist connection")
        {
            refused += 1;
        }
    }
    assert!(
        refused >= 8,
        "only {refused} designs needed cellify; the test lost its point"
    );
}
