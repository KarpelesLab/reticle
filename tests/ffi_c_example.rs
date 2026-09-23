//! Compiles and runs `examples/ffi/demo.c` against a real build of the
//! library.
//!
//! The unit tests in `src/ffi/tests.rs` call the entry points from Rust,
//! which proves the logic but not the ABI: whether `reticle.h` describes
//! the symbols a C compiler actually finds, and whether the archive links.
//! This test answers that by doing what a consumer does.
//!
//! It needs two things that may not be there, and says which is missing
//! rather than failing:
//!
//! - **A C compiler.** `$CC`, then `cc`, `gcc` and `clang`.
//! - **A built library.** Cargo does not build a `staticlib` or `cdylib`
//!   during `cargo test`, since the crate's `[lib]` is a plain `rlib`;
//!   producing one is a separate `cargo rustc` invocation, and running
//!   cargo from inside a test would block on the same lock. `tools/check.sh`
//!   and the CI workflow therefore build it just before the test run, so
//!   the test executes there; a bare `cargo test` skips it with the
//!   command to run.
#![cfg(all(
    feature = "ffi",
    feature = "verilog",
    feature = "sim",
    feature = "synth"
))]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root.
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The first C compiler on this machine, if any.
fn compiler() -> Option<String> {
    let from_env = std::env::var("CC").ok();
    let candidates: Vec<String> = from_env
        .into_iter()
        .chain(["cc", "gcc", "clang"].iter().map(|s| (*s).to_owned()))
        .collect();
    candidates.into_iter().find(|cc| {
        Command::new(cc)
            .arg("--version")
            .output()
            .is_ok_and(|out| out.status.success())
    })
}

/// The static library, if one has been built.
///
/// `cargo test` puts the test binary under `target/<profile>/deps`, so the
/// profile directory is two levels up from it; that is where `cargo rustc
/// --crate-type staticlib` leaves the archive.
fn static_library() -> Option<PathBuf> {
    let name = if cfg!(target_env = "msvc") {
        "reticle.lib"
    } else {
        "libreticle.a"
    };
    let exe = std::env::current_exe().ok()?;
    let profile_dir = exe.parent()?.parent()?.to_path_buf();
    let mut dirs = vec![profile_dir];
    dirs.push(root().join("target/debug"));
    dirs.push(root().join("target/release"));
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// The system libraries a static Rust `std` needs on this platform.
fn system_libraries() -> Vec<&'static str> {
    if cfg!(target_os = "windows") {
        vec![
            "-lkernel32",
            "-luserenv",
            "-lws2_32",
            "-lbcrypt",
            "-lntdll",
            "-ladvapi32",
        ]
    } else if cfg!(target_vendor = "apple") {
        // `IOKit` is macOS's USB stack, which `rawusb` calls and which
        // the `program` feature therefore pulls into the archive. A C
        // program linking a Rust staticlib has to name the native
        // libraries itself; nothing in the archive asks for them. Adding
        // a feature whose platform backend calls a system framework means
        // adding it here, or this test goes red on that platform only.
        vec![
            "-lpthread",
            "-ldl",
            "-lm",
            "-framework",
            "CoreFoundation",
            "-framework",
            "IOKit",
        ]
    } else {
        vec!["-lpthread", "-ldl", "-lm"]
    }
}

#[test]
fn the_c_example_compiles_and_runs() {
    // On the MSVC target the static library is an MSVC archive, and the
    // compilers this test drives with gcc-style flags are GNU toolchains.
    // A MinGW `gcc` found on the path cannot link it: it has no MSVC
    // runtime, so symbols such as `__chkstk` and the C++ `type_info`
    // vtable that panic unwinding needs stay undefined. That is an ABI
    // mismatch between two toolchains, not a fault in the C API, which the
    // Rust-side FFI tests still exercise on Windows. The C example is
    // compiled and run on the GNU targets, where the toolchains agree.
    if cfg!(target_env = "msvc") {
        println!(
            "skipping: the MSVC-built static library cannot be linked by a \
             GNU C compiler; the C example runs on the GNU targets"
        );
        return;
    }
    let Some(cc) = compiler() else {
        println!("skipping: no C compiler found (tried $CC, cc, gcc, clang)");
        return;
    };
    // `examples/` is excluded from the published crate (it is C, not Rust),
    // so a packaged copy has this test but not the program it drives.
    let example = root().join("examples/ffi/demo.c");
    if !example.is_file() {
        println!("skipping: examples/ffi/demo.c is not in this copy of the crate");
        return;
    }
    let Some(library) = static_library() else {
        println!(
            "skipping: no static library to link against. Build one with\n  \
             cargo rustc --lib --features ffi --crate-type staticlib\n\
             (tools/check.sh and CI do this before running the tests)"
        );
        return;
    };

    let out_dir = std::env::temp_dir().join(format!("reticle-ffi-demo-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).expect("a writable temporary directory");
    let binary = out_dir.join(if cfg!(windows) { "demo.exe" } else { "demo" });

    let mut build = Command::new(&cc);
    build
        .arg("-std=c99")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-I")
        .arg(root().join("src/ffi"))
        .arg("-o")
        .arg(&binary)
        .arg(&example)
        .arg(&library)
        .args(system_libraries());

    let built = build.output().expect("the C compiler runs");
    assert!(
        built.status.success(),
        "compiling examples/ffi/demo.c failed:\n{}\n{}",
        String::from_utf8_lossy(&built.stdout),
        String::from_utf8_lossy(&built.stderr),
    );

    let run = Command::new(&binary).output().expect("the demo runs");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        run.status.success(),
        "examples/ffi/demo.c exited with {:?}:\n{stdout}\n{stderr}",
        run.status.code(),
    );

    // The demo counts ten rising edges and prints what it found; if the
    // ABI were wrong this is where a garbled value would show up.
    assert!(
        stdout.contains("simulate: q = 10"),
        "unexpected output:\n{stdout}"
    );
    assert!(
        stdout.contains("emit verilog"),
        "unexpected output:\n{stdout}"
    );
    assert!(stdout.ends_with("done\n"), "unexpected output:\n{stdout}");

    let _ = std::fs::remove_dir_all(&out_dir);
}
