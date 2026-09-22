//! FST waveform tests.
//!
//! Three things are checked here:
//!
//! * **Goldens.** `testdata/sim/fst/<name>.fst` is the byte-for-byte output
//!   of running `testdata/sim/<name>.rtl` with FST capture on. The writer
//!   fixes its version and date strings, so the bytes are reproducible;
//!   `cargo test --all-features -- --ignored update_fst_goldens` rewrites
//!   them after a deliberate format change.
//! * **Round trips.** The in-crate reader decodes the writer's output back
//!   into `(time, handle, value)` tuples, for every combination of chain
//!   compression, hierarchy compression and block count.
//! * **Agreement with VCD.** The same run is captured as VCD and as FST and
//!   the two change lists are compared signal by signal, which is the real
//!   statement of correctness: the binary format must say exactly what the
//!   text one says.
//!
//! Two `#[ignore]`d tests reach outside the crate when the tools exist:
//! one cross-checks the DEFLATE codec against `python3 -c 'import zlib'`,
//! the other feeds a golden to GTKWave's `fst2vcd` / `fstdump`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use reticle::ir::Design;
use reticle::sim::fst::{self, FstCompression, FstValue, lz4, zlib};
use reticle::sim::{SimOptions, Simulator};
use reticle::source::SourceMap;

/// The designs a golden FST exists for. Each runs to completion on its own.
const CASES: &[&str] = &["vcd", "hier", "counter"];

/// `testdata/sim`.
fn sim_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/sim")
}

/// The golden path of a case.
fn golden_path(name: &str) -> PathBuf {
    sim_dir().join("fst").join(format!("{name}.fst"))
}

/// Runs `<name>.rtl` to completion, returning the FST bytes and the VCD text.
fn capture(name: &str, configure: impl FnOnce(&fst::FstCapture)) -> (Vec<u8>, String) {
    let path = sim_dir().join(format!("{name}.rtl"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut map = SourceMap::new();
    let file = map.add(format!("{name}.rtl"), text.clone()).unwrap();
    let design = Design::parse_text(&text, file).unwrap_or_else(|d| panic!("{}", d.render(&map)));
    let mut sim = Simulator::new(&design, SimOptions::default())
        .unwrap_or_else(|d| panic!("{}", d.render(&map)));
    sim.enable_vcd();
    let fst_capture = sim.enable_fst();
    configure(&fst_capture);
    sim.run();
    let mut bytes = Vec::new();
    sim.dump_fst(&fst_capture, &mut bytes).unwrap();
    (bytes, sim.vcd().unwrap_or("").to_owned())
}

/// Every signal's changes, keyed by hierarchical name: the value it started
/// at followed by every value it took, each with its time.
type Timeline = BTreeMap<String, Vec<(u64, String)>>;

/// The timeline an FST file describes.
fn fst_timeline(bytes: &[u8]) -> Timeline {
    let file = fst::read(bytes).expect("the writer's output parses");
    let mut names: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for var in &file.vars {
        let bare = var.name.split(' ').next().unwrap_or(&var.name);
        names
            .entry(var.handle)
            .or_default()
            .push(format!("{}.{bare}", var.scope));
    }
    let mut out: Timeline = BTreeMap::new();
    for (handle, value) in file.frame.iter().enumerate() {
        let handle = u32::try_from(handle + 1).unwrap();
        for name in names.get(&handle).into_iter().flatten() {
            out.entry(name.clone())
                .or_default()
                .push((file.start_time, value.to_string()));
        }
    }
    for change in &file.changes {
        for name in names.get(&change.handle).into_iter().flatten() {
            out.entry(name.clone())
                .or_default()
                .push((change.time, change.value.to_string()));
        }
    }
    out
}

/// The timeline a VCD text describes.
fn vcd_timeline(text: &str) -> Timeline {
    let mut ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut scopes: Vec<String> = Vec::new();
    let mut out: Timeline = BTreeMap::new();
    let mut time = 0u64;
    let mut in_definitions = true;
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        if in_definitions {
            match words[0] {
                "$scope" => scopes.push(words[2].to_owned()),
                "$upscope" => {
                    scopes.pop();
                }
                "$var" => {
                    let id = words[3].to_owned();
                    let name = format!("{}.{}", scopes.join("."), words[4]);
                    ids.entry(id).or_default().push(name);
                }
                "$enddefinitions" => in_definitions = false,
                _ => {}
            }
            continue;
        }
        if let Some(stamp) = words[0].strip_prefix('#') {
            time = stamp.parse().expect("a VCD time stamp");
            continue;
        }
        if words[0].starts_with('$') {
            continue;
        }
        let (value, id) = if words.len() == 2 {
            // `b1010 id` or `r1.5 id`
            (words[0][1..].to_owned(), words[1].to_owned())
        } else {
            (words[0][..1].to_owned(), words[0][1..].to_owned())
        };
        for name in ids.get(&id).into_iter().flatten() {
            out.entry(name.clone())
                .or_default()
                .push((time, value.clone()));
        }
    }
    out
}

/// A one-line summary of where two timelines differ.
fn difference(expected: &Timeline, actual: &Timeline) -> Option<String> {
    for (name, want) in expected {
        match actual.get(name) {
            None => return Some(format!("{name} is missing from the FST")),
            Some(got) if got != want => {
                return Some(format!("{name}:\n  vcd: {want:?}\n  fst: {got:?}"));
            }
            Some(_) => {}
        }
    }
    for name in actual.keys() {
        if !expected.contains_key(name) {
            return Some(format!("{name} is only in the FST"));
        }
    }
    None
}

#[test]
fn fst_goldens_match() {
    let mut failures = Vec::new();
    for name in CASES {
        let (bytes, _) = capture(name, |_| {});
        let path = golden_path(name);
        match fs::read(&path) {
            Ok(golden) if golden == bytes => {}
            Ok(golden) => failures.push(format!(
                "{name}.fst differs: {} golden bytes, {} produced",
                golden.len(),
                bytes.len()
            )),
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
#[ignore = "rewrites the golden files"]
fn update_fst_goldens() {
    fs::create_dir_all(sim_dir().join("fst")).unwrap();
    for name in CASES {
        let (bytes, _) = capture(name, |_| {});
        fs::write(golden_path(name), &bytes).unwrap();
    }
}

#[test]
fn goldens_parse_back() {
    for name in CASES {
        let bytes = fs::read(golden_path(name)).unwrap();
        let file = fst::read(&bytes).unwrap_or_else(|e| panic!("{name}.fst: {e}"));
        assert_eq!(
            file.version,
            format!("Reticle {}", reticle::VERSION),
            "{name}"
        );
        assert_eq!(file.date, "Thu Jan  1 00:00:00 1970", "{name}");
        assert_eq!(file.vars.len() as u64, file.var_count, "{name}");
        assert_eq!(file.lengths.len(), file.frame.len(), "{name}");
        assert!(!file.changes.is_empty(), "{name} recorded no changes");
        assert!(
            file.changes.windows(2).all(|w| w[0].time <= w[1].time),
            "{name} changes are not ordered by time"
        );
    }
}

#[test]
fn fst_agrees_with_vcd() {
    let mut failures = Vec::new();
    for name in CASES {
        let (bytes, vcd) = capture(name, |_| {});
        if let Some(diff) = difference(&vcd_timeline(&vcd), &fst_timeline(&bytes)) {
            failures.push(format!("{name}: {diff}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn every_compression_round_trips() {
    for name in CASES {
        let reference = fst_timeline(&capture(name, |_| {}).0);
        for compression in [
            FstCompression::None,
            FstCompression::Zlib,
            FstCompression::Lz4,
        ] {
            for hierarchy_lz4 in [false, true] {
                let (bytes, _) = capture(name, |c| {
                    c.set_compression(compression);
                    c.set_hierarchy_lz4(hierarchy_lz4);
                });
                assert_eq!(
                    fst_timeline(&bytes),
                    reference,
                    "{name} with {compression:?} chains and lz4 hierarchy {hierarchy_lz4}"
                );
            }
        }
    }
}

#[test]
fn several_blocks_hold_the_same_waveform() {
    for name in CASES {
        let reference = fst_timeline(&capture(name, |_| {}).0);
        for interval in [1usize, 2, 3, 7] {
            let (bytes, _) = capture(name, |c| c.set_flush_interval(interval));
            let file = fst::read(&bytes).unwrap();
            assert!(
                file.block_count > 1,
                "{name} flushing every {interval} changes produced one block"
            );
            assert_eq!(
                fst_timeline(&bytes),
                reference,
                "{name} flushing every {interval} changes"
            );
        }
    }
}

#[test]
fn aliased_ports_share_one_handle() {
    let bytes = fs::read(golden_path("hier")).unwrap();
    let file = fst::read(&bytes).unwrap();
    // `top.x` drives the `a` port of `u0` through a plain net, so both names
    // are the same signal and must resolve to the same FST handle.
    let parent = file.var("top.x").expect("top.x");
    let port = file.var("top.u0.a").expect("top.u0.a");
    assert_eq!(parent.handle, port.handle);
    assert!(
        file.var_count > u64::try_from(file.lengths.len()).unwrap(),
        "the design should have more names than handles"
    );
}

#[test]
fn real_and_unknown_values_survive_a_round_trip() {
    let text = "\
module mix
  net %bit u1 reg
  net %vec u5 reg
  net %r real reg
  process initial
    %bit = 1'bx
    %vec = 5'b10xz1
    %r = 64'd0
    wait for 8'd1
    %bit = 1'd1
    %vec = 5'd7
    %r = 64'd4613937818241073152
    wait for 8'd1
    %bit = 1'bz
  end
end
";
    let mut map = SourceMap::new();
    let file = map.add("mix.rtl", text).unwrap();
    let design = Design::parse_text(text, file).unwrap_or_else(|d| panic!("{}", d.render(&map)));
    let mut sim = Simulator::new(&design, SimOptions::default()).unwrap();
    let capture = sim.enable_fst();
    sim.run();
    let mut bytes = Vec::new();
    sim.dump_fst(&capture, &mut bytes).unwrap();
    let parsed = fst::read(&bytes).unwrap();

    let bit = parsed.var("mix.bit").unwrap().handle;
    let vec = parsed.var("mix.vec").unwrap().handle;
    let real = parsed.var("mix.r").unwrap();
    assert_eq!(real.var_type, fst::VT_VCD_REAL);
    assert_eq!(real.length, 8);
    assert!(parsed.reals[real.handle as usize - 1]);

    // The frame holds the state at enable time; a net assigned the value it
    // already has produces no change, exactly as in VCD.
    assert_eq!(parsed.frame[bit as usize - 1].to_string(), "x");
    let values: Vec<String> = parsed
        .changes_of(bit)
        .iter()
        .map(|c| c.value.to_string())
        .collect();
    assert_eq!(values, ["1", "z"]);
    let values: Vec<String> = parsed
        .changes_of(vec)
        .iter()
        .map(|c| c.value.to_string())
        .collect();
    assert_eq!(values, ["10xz1", "00111"]);
    let reals: Vec<FstValue> = parsed
        .changes_of(real.handle)
        .iter()
        .map(|c| c.value.clone())
        .collect();
    assert_eq!(reals, [FstValue::Real(0.0), FstValue::Real(3.0)]);
}

#[test]
fn a_capture_with_no_changes_is_still_a_valid_file() {
    let text = "\
module quiet
  net %a u3 wire
end
";
    let mut map = SourceMap::new();
    let file = map.add("quiet.rtl", text).unwrap();
    let design = Design::parse_text(text, file).unwrap();
    let mut sim = Simulator::new(&design, SimOptions::default()).unwrap();
    let capture = sim.enable_fst();
    sim.run_for(10);
    let mut bytes = Vec::new();
    sim.dump_fst(&capture, &mut bytes).unwrap();
    let parsed = fst::read(&bytes).unwrap();
    assert_eq!(parsed.block_count, 1);
    assert!(parsed.changes.is_empty());
    assert_eq!(parsed.frame, [FstValue::Bits("zzz".to_owned())]);
    assert_eq!(parsed.end_time, 10);
}

#[test]
fn dumping_twice_gives_the_same_bytes() {
    let (first, _) = capture("counter", |_| {});
    let (second, _) = capture("counter", |_| {});
    assert_eq!(first, second);
}

#[test]
fn compressors_round_trip_large_inputs() {
    let mut state = 0x1234_5678_9abc_def0u64;
    let mut random = Vec::with_capacity(300_000);
    let mut repetitive = Vec::with_capacity(300_000);
    for i in 0..300_000u32 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        random.push(u8::try_from((state >> 33) & 0xff).unwrap());
        repetitive.push(if i % 97 < 60 { b'0' } else { b'x' });
    }
    for data in [&random, &repetitive] {
        let packed = zlib::compress(data);
        assert_eq!(
            zlib::decompress(&packed, Some(data.len())).as_ref(),
            Some(data)
        );
        let gz = zlib::gzip_compress(data);
        assert_eq!(
            zlib::gzip_decompress(&gz, Some(data.len())).as_ref(),
            Some(data)
        );
        let packed = lz4::compress(data);
        assert_eq!(lz4::decompress(&packed, data.len()).as_ref(), Some(data));
    }
    assert!(zlib::compress(&repetitive).len() * 20 < repetitive.len());
    assert!(lz4::compress(&repetitive).len() * 5 < repetitive.len());
}

#[test]
#[ignore = "needs python3"]
fn zlib_interoperates_with_python() {
    if Command::new("which").arg("python3").output().is_err() {
        eprintln!("python3 not installed, skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("reticle-fst-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let mut state = 99u64;
    let mut data = Vec::new();
    for i in 0..200_000u32 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        data.push(if state.is_multiple_of(4) {
            u8::try_from((state >> 40) & 0xff).unwrap()
        } else {
            b'a' + u8::try_from(i % 5).unwrap()
        });
    }
    // Our stream must inflate in Python, and Python's must inflate here.
    let ours = dir.join("ours.z");
    let plain = dir.join("plain.bin");
    fs::write(&ours, zlib::compress(&data)).unwrap();
    fs::write(&plain, &data).unwrap();
    let script = format!(
        "import zlib,sys\n\
         raw=open({plain:?},'rb').read()\n\
         ours=open({ours:?},'rb').read()\n\
         assert zlib.decompress(ours)==raw, 'python cannot read our stream'\n\
         sys.stdout.buffer.write(zlib.compress(raw,9))\n",
        plain = plain.to_string_lossy(),
        ours = ours.to_string_lossy(),
    );
    let output = Command::new("python3")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("python3 runs");
    assert!(
        output.status.success(),
        "python3 failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        zlib::decompress(&output.stdout, Some(data.len())).as_ref(),
        Some(&data),
        "we cannot read python's stream"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "needs gtkwave's command line tools"]
fn gtkwave_tools_accept_the_goldens() {
    let tool = ["fst2vcd", "fstdump", "gtkwave"].into_iter().find(|t| {
        Command::new("which")
            .arg(t)
            .output()
            .is_ok_and(|o| o.status.success())
    });
    let Some(tool) = tool else {
        eprintln!("no gtkwave tools installed, skipping");
        return;
    };
    for name in CASES {
        let path = golden_path(name);
        let output = Command::new(tool)
            .arg(&path)
            .output()
            .unwrap_or_else(|e| panic!("running {tool}: {e}"));
        assert!(
            output.status.success(),
            "{tool} rejected {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("$var") || text.contains("var"),
            "{tool} produced nothing useful for {}",
            path.display()
        );
    }
}
