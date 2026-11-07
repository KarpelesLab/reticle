//! System tasks and functions against xsim.
//!
//! Each testbench under `testdata/sim/` named here comes with a `.xsim`
//! file: what xsim 2026.1 printed for it, recorded by running it there and
//! keeping only the design's output. A test runs the `reticle` binary on the
//! testbench and wants that output byte for byte, so each line of the
//! testbench is a measured answer rather than a reading of the standard.
//! xsim prints its warnings into the same log; a testbench whose calls
//! would warn keeps them out, since Reticle reports them on standard error.

#![cfg(feature = "cli")]

use std::process::Command;

/// Runs `reticle sim --quiet` on `tb` and returns the exit code, the
/// standard output and the standard error.
fn sim(tb: &str) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["sim", "--quiet", "--top", "tb", tb])
        .output()
        .expect("failed to run the reticle binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Runs `testdata/sim/<name>.<ext>` and compares it with `<name>.xsim`,
/// expecting exit code `code`.
fn matches_xsim(name: &str, ext: &str, code: i32) {
    let tb = format!("testdata/sim/{name}.{ext}");
    let (got_code, stdout, stderr) = sim(&tb);
    let want = std::fs::read_to_string(format!("testdata/sim/{name}.xsim")).unwrap();
    assert_eq!(got_code, code, "{tb}: {stderr}");
    if stdout != want {
        let line = stdout
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(stdout.lines().count().min(want.lines().count()));
        panic!(
            "{tb}: output differs from xsim's at line {}\n  xsim:    {:?}\n  reticle: {:?}\n{stderr}",
            line + 1,
            want.lines().nth(line),
            stdout.lines().nth(line),
        );
    }
}

/// **`$display` pads, fills and spells values as xsim does**
/// (`display_format_tb.v`): `%0h` drops leading zeros and a width
/// zero-fills `%h`/`%o`/`%b`; `%X` prints lower case; a signed `%d` field
/// is the digits of 2^N - 1 and a sign; a negative signed value with an
/// unknown bit prints `x`; an explicit width does not pad an unknown
/// decimal; `%05d` fills with zeros; `%s` prints a NUL byte as a space and
/// `%0s` drops the leading ones; a real ignores `-` and an integer
/// conversion of it rounds to a 32-bit integer; `%l` names the cell;
/// `$strobeh`, `$strobeb` and `$strobeo` keep their radix. Would catch any
/// of those reverting, since every line is xsim's; would not catch `%c` of
/// a NUL byte, where xsim's line ends at the NUL, nor `%5m`, which xsim
/// prints as `5m`; neither is in the testbench.
#[test]
fn display_formats_as_xsim_does() {
    matches_xsim("display_format_tb", "v", 0);
}
