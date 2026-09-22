//! `$readmemh`, `$readmemb`, `$writememh` and `$writememb` from Verilog,
//! through the simulator and synthesis.
//!
//! The Verilog frontend lowers the four tasks to `StmtKind::MemFile`,
//! which names the memory itself. These tests check what both consumers
//! make of it: the same words in the same elements, with start and end
//! addresses and `@` addresses in the memory's declared numbering.

#![cfg(all(feature = "verilog", feature = "sim", feature = "synth"))]

use std::rc::Rc;

use reticle::diag::{Diagnostics, Severity};
use reticle::ir::{Design, MemFileOp, StmtKind};
use reticle::logic::Logic;
use reticle::sim::{MemoryFiles, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::synth::{SynthOptions, run as synth_run};
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

/// Elaborates one Verilog source, returning the design (if any) and the
/// rendered diagnostics.
fn elab(text: &str) -> (Option<Design>, Diagnostics, SourceMap) {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let id = map.add("t.v", text).expect("fits");
    let file = parse_source(
        &mut map,
        id,
        Dialect::Verilog2005,
        &mut NoIncludes,
        &mut diags,
    );
    let design = elaborate(
        &[&file],
        &ElabOptions::new(Dialect::Verilog2005),
        &mut diags,
    );
    (design, diags, map)
}

fn verilog(text: &str) -> Design {
    let (design, diags, map) = elab(text);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    design.expect("a design")
}

/// A memory declared from 16, loaded twice: all of it from a hex file
/// with an `@` address, then two words of it from a binary file between
/// a start and an end address, downwards.
const OFFSET: &str = "module t (input wire [4:0] a, output wire [3:0] q);\n\
     reg [3:0] m [16:23];\n\
     assign q = m[a];\n\
     initial begin\n\
     $readmemh(\"m.hex\", m);\n\
     $readmemb(\"m.bin\", m, 22, 21);\n\
     end\n\
     endmodule\n";

fn offset_files() -> MemoryFiles {
    let mut files = MemoryFiles::new();
    files
        .insert("m.hex", "1 2 @13 a b")
        .insert("m.bin", "1110 1101");
    files
}

/// The contents both consumers should agree on, by zero-based element:
/// `1 2` at 16 and 17, `a b` at 19 and 20, then `e` at 22 and `d` at 21.
fn offset_expected() -> Vec<Option<u64>> {
    vec![
        Some(1),
        Some(2),
        None,
        Some(0xa),
        Some(0xb),
        Some(0xd),
        Some(0xe),
        None,
    ]
}

#[test]
fn the_frontend_names_the_memory_and_translates_the_addresses() {
    let design = verilog(OFFSET);
    let top = design.top_module().expect("a top");
    let mut seen = Vec::new();
    top.for_each_stmt(|s| {
        if let StmtKind::MemFile {
            op,
            mem,
            start,
            end,
            base,
            ..
        } = &s.kind
        {
            let value = |e: &Option<_>| {
                e.map(|e| {
                    top.exprs[e]
                        .as_const()
                        .and_then(|c| c.to_u64())
                        .expect("a constant address")
                })
            };
            seen.push((
                *op,
                top.memories[*mem].name.to_string(),
                value(start),
                value(end),
                *base,
            ));
        }
    });
    assert_eq!(
        seen,
        [
            (MemFileOp::ReadHex, "m".to_owned(), None, None, 16),
            (MemFileOp::ReadBin, "m".to_owned(), Some(6), Some(5), 16),
        ]
    );
    // The text format round-trips the statement.
    let text = design.to_text();
    assert!(text.contains("readmemb(\"m.bin\", @m, "), "{text}");
    assert!(text.contains(") base 16\n"), "{text}");
}

#[test]
fn the_simulator_loads_with_declared_addresses() {
    let design = verilog(OFFSET);
    let options = SimOptions {
        files: Some(Box::new(offset_files())),
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, options).expect("simulates");
    sim.run();
    assert!(sim.messages().is_empty(), "{:?}", sim.messages());
    let m = sim.memory("t.m").expect("the memory");
    let got: Vec<Option<u64>> = (0..8)
        .map(|i| sim.get_mem(m, i).and_then(|v| v.to_u64()))
        .collect();
    assert_eq!(got, offset_expected());
}

#[test]
fn synthesis_loads_the_same_words() {
    let mut design = verilog(OFFSET);
    let options = SynthOptions {
        files: Some(Rc::new(offset_files())),
        ..SynthOptions::default()
    };
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &options, &mut diags);
    assert!(diags.is_empty(), "{diags:?}");
    let top = design.top_module().expect("a top");
    let (_, m) = top
        .memories
        .iter()
        .find(|(_, m)| m.name.as_str() == "m")
        .expect("the memory is kept");
    let init = m.init.as_ref().expect("initial contents");
    let got: Vec<Option<u64>> = (0..8)
        .map(|i| init.get(i).and_then(Logic::to_u64))
        .collect();
    assert_eq!(got, offset_expected());
}

#[test]
fn synthesis_names_a_file_it_cannot_load() {
    // A provider without the file: an error naming it.
    let mut design = verilog(OFFSET);
    let mut files = MemoryFiles::new();
    files.insert("m.hex", "@99 1");
    let options = SynthOptions {
        files: Some(Rc::new(files)),
        ..SynthOptions::default()
    };
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &options, &mut diags);
    let errors: Vec<&str> = diags
        .iter()
        .filter(|d| d.code == Some("S0018") && d.severity == Severity::Error)
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        errors,
        [
            "the contents of `m.hex` could not be loaded into `m`: `m.hex`: address @99 is outside the range loaded (@10 to @17)",
            "the contents of `m.bin` could not be loaded into `m`: `m.bin` was not found",
        ]
    );
}

#[test]
fn saving_is_simulation_only() {
    let text = "module t;\n\
         reg [7:0] m [0:1];\n\
         initial begin\n\
         m[0] = 8'h5a;\n\
         m[1] = 8'ha5;\n\
         $writememh(\"out.hex\", m);\n\
         $writememb(\"one.bin\", m, 1);\n\
         end\n\
         endmodule\n";
    let design = verilog(text);
    let mut sim = Simulator::new(&design, SimOptions::default()).expect("simulates");
    sim.run();
    assert_eq!(sim.written_files()["out.hex"], "5a\na5\n");
    assert_eq!(sim.written_files()["one.bin"], "@1\n10100101\n");

    // Synthesis keeps the constant writes and drops the saves with a note.
    let mut design = verilog(text);
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
    let notes: Vec<&str> = diags
        .iter()
        .filter(|d| d.code == Some("S0014"))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        notes,
        ["2 simulation-only statements dropped (memory file task)"]
    );
}

#[test]
fn the_second_argument_must_be_a_memory() {
    for call in [
        "$readmemh(\"f\", v)",
        "$readmemh(\"f\", m[0])",
        "$readmemh(\"f\")",
        "$writememb(\"f\", m, 0, 1, 2)",
    ] {
        let text =
            format!("module t;\nreg [7:0] m [0:1];\nreg [7:0] v;\ninitial {call};\nendmodule\n");
        let (_, diags, map) = elab(&text);
        assert!(diags.has_errors(), "`{call}` is accepted");
        let rendered = diags.render(&map);
        assert!(
            rendered.contains("must be a memory") || rendered.contains("takes a file name"),
            "`{call}`: {rendered}"
        );
    }
}
