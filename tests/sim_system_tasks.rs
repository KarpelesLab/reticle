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

/// **`$printtimescale` prints xsim's line** (`printtimescale_tb.v`): the
/// calling module by its module name, or the instance an argument names by
/// its path, each with its own `` `timescale ``. It printed nothing.
/// Would catch the task ignored again, the wrong module's timescale (the
/// testbench's two modules differ), or a relative instance name not found;
/// would not catch an absolute path as an argument, which xsim prints with
/// the caller's scope in front (`tb.tb.u1`) and Reticle does not copy.
#[test]
fn printtimescale_prints_as_xsim_does() {
    matches_xsim("printtimescale_tb", "v", 0);
}

/// **`$random` and the `$dist_*` functions compute what xsim computes**
/// (`random_tb.v`), which is IEEE 1364-2005's own code: their values, the
/// seed each leaves in its variable, and the sequence of `$random` with no
/// seed, which goes on from the seed the last `$random(seed)` or
/// `$dist_uniform` left. Reticle used a xorshift generator and never
/// wrote a seed back. Would catch any of the seven functions computing
/// differently, a seed not written back, or the seedless sequence starting
/// or continuing elsewhere; would not catch the warnings for a mean that is
/// not positive or an empty `$dist_uniform` range, which the testbench
/// leaves out because xsim prints them into its output.
#[test]
fn random_computes_what_xsim_computes() {
    matches_xsim("random_tb", "v", 0);
}

/// **`$monitor` prints when a monitored value changes, and only then**
/// (`monitor_tb.v`). It compared the text it would print, which holds
/// `$time`, so it printed at every step with any activity: an unmonitored
/// variable written, a monitored one rewritten with its own value. Would
/// catch a print for a change of the time alone, of an operand that leaves
/// the expression's value, of a value changed back within the step, or of
/// an unmonitored variable, and `$monitorh` printing in decimal; would not
/// catch a second `$monitor`, which xsim runs beside the first where the
/// standard has it replace it, and which the testbench therefore leaves
/// out.
#[test]
fn monitor_prints_only_on_a_change() {
    matches_xsim("monitor_tb", "v", 0);
}

/// **`%m` names the scope the statement is written in** (`scope_tb.sv`):
/// a named block, a nested one and a loop's named body, a task and a
/// function, in the top and in an instance, and in `$sformatf`. It printed
/// the module instance's path for all of them. And `$test$plusargs` and
/// `$value$plusargs` find nothing, where they returned `x`. Would catch the
/// scope dropped, a task named by its caller's scope rather than its own,
/// a scope Reticle makes for itself showing up in the name, or `%%m`
/// read as a conversion; would not catch an unnamed generate block, which
/// xsim calls `genblk1` and Reticle `genblk`, and which is not in it.
#[test]
fn m_names_the_scope_as_xsim_does() {
    matches_xsim("scope_tb", "sv", 0);
}
