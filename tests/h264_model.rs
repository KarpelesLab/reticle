//! The H.264 software model, `tools/h264_model.py`, against ffmpeg.
//!
//! The model is the golden reference of the H.264 decoder IP: the corpora
//! the hardware is checked against (`testdata/ip/h264_*.txt`) are its
//! dumps. These tests hold it to the one claim that makes that worth
//! anything, and keep the corpora honest:
//!
//! | test | what it proves |
//! |---|---|
//! | `the_model_decodes_every_stream_like_ffmpeg` | every frame of every stream in `testdata/ip/h264/` decodes to exactly the bytes ffmpeg's decoder produced (md5 per cropped I420 frame, recorded by `tools/h264_streams.sh`) |
//! | `the_committed_dumps_are_what_the_model_writes` | the corpora are byte for byte what the current model dumps, so a model fix cannot leave stale vectors behind |
//! | `every_stream_has_its_reference` | no stream is committed without its md5 list, so the first test cannot pass by having nothing to check |
//!
//! ffmpeg is not needed here: its output is recorded in the `.md5` files.
//! Python 3 is, and when there is no `python3` these tests say so and
//! pass, as the other tests that need an outside tool do. What they would
//! not catch: anything the streams do not exercise. `tools/h264_model.py
//! stats` lists what each stream covers, and `docs/h264-decoder.md`
//! ("The software model") records that coverage.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The Python interpreter, or `None` (with the reason printed) when there
/// is none or the model is not in this copy of the crate.
fn python() -> Option<&'static str> {
    if !root().join("tools/h264_model.py").exists() {
        println!("skipping: tools/h264_model.py is not in this copy of the crate");
        return None;
    }
    match Command::new("python3").arg("--version").output() {
        Ok(o) if o.status.success() => Some("python3"),
        _ => {
            println!("skipping: python3 is not installed");
            None
        }
    }
}

fn streams() -> Vec<PathBuf> {
    let dir = root().join("testdata/ip/h264");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "264"))
        .collect();
    out.sort();
    out
}

#[test]
fn every_stream_has_its_reference() {
    let all = streams();
    assert!(
        all.len() >= 20,
        "only {} streams in testdata/ip/h264",
        all.len()
    );
    for s in &all {
        let md5 = s.with_extension("md5");
        let text =
            std::fs::read_to_string(&md5).unwrap_or_else(|e| panic!("{}: {e}", md5.display()));
        let frames = text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .count();
        assert!(frames > 0, "{} lists no frame", md5.display());
    }
}

#[test]
fn the_model_decodes_every_stream_like_ffmpeg() {
    let Some(py) = python() else { return };
    let mut failed = Vec::new();
    for s in streams() {
        let out = Command::new(py)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .arg(root().join("tools/h264_model.py"))
            .arg("check")
            .arg(&s)
            .arg(s.with_extension("md5"))
            .output()
            .expect("run the model");
        if !out.status.success() {
            failed.push(format!(
                "{}:\n{}{}",
                s.display(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "the model differs from ffmpeg:\n{}",
        failed.join("\n")
    );
}

/// The committed dumps: (stream, frames). The same list as
/// `tools/h264_dumps.sh`, which regenerates them.
const DUMPS: &[(&str, u32)] = &[
    ("intra_32x32_qp26", 1),
    ("intra_48x32_qp12", 1),
    ("p_32x32_dump", 3),
    ("craft_pcm", 4),
];

#[test]
fn the_committed_dumps_are_what_the_model_writes() {
    let Some(py) = python() else { return };
    let scratch = std::env::temp_dir().join(format!("reticle-h264-dumps-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("scratch directory");
    let script =
        std::fs::read_to_string(root().join("tools/h264_dumps.sh")).expect("h264_dumps.sh");
    let mut stale = Vec::new();
    for (name, frames) in DUMPS {
        // The description is part of each file's header, so take it from
        // the script that wrote the committed files.
        let line = script
            .lines()
            .find(|l| l.starts_with("dump ") && l.split_whitespace().nth(1) == Some(name))
            .unwrap_or_else(|| panic!("{name} is not in tools/h264_dumps.sh"));
        let desc = line.split('"').nth(1).expect("a quoted description");
        assert_eq!(
            line.split_whitespace().nth(2),
            Some(frames.to_string().as_str()),
            "tools/h264_dumps.sh and DUMPS disagree about {name}"
        );
        let stream = root().join(format!("testdata/ip/h264/{name}.264"));
        let out = Command::new(py)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .arg(root().join("tools/h264_model.py"))
            .arg("dump")
            .arg(&stream)
            .arg(&scratch)
            .args(["--frames", &frames.to_string(), "--desc", desc])
            .output()
            .expect("run the model");
        assert!(
            out.status.success(),
            "dump of {name} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        for kind in ["tokens", "recon", "inter"] {
            let file = format!("h264_{kind}_{name}.txt");
            let fresh = scratch.join(&file);
            let committed = root().join("testdata/ip").join(&file);
            match (fresh.exists(), committed.exists()) {
                (false, false) => continue,
                (true, true) => {
                    if read(&fresh) != read(&committed) {
                        stale.push(file);
                    }
                }
                _ => stale.push(format!("{file} (present in only one of the two)")),
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(
        stale.is_empty(),
        "these dumps differ from what the model writes now; run tools/h264_dumps.sh: {stale:?}"
    );
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}
