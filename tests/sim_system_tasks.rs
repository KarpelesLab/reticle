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

/// **`$countones`, `$countbits`, `$onehot`, `$onehot0` and `$isunknown`
/// compute on run-time values** (`bitcount_tb.sv`). They were unknown
/// functions to the simulator, warned about and `x`, and only folded when
/// their operand was constant. Would catch any of the five returning `x`
/// again or counting an unknown bit as a one; `bit_counts_are_xsims` in
/// `sim::sys` pins the same values without the frontend.
#[test]
fn bit_counts_compute_as_xsim_does() {
    matches_xsim("bitcount_tb", "sv", 0);
}

/// **An immediate assertion runs its pass action or its fail action, never
/// both, and the severity tasks print xsim's words** (`assert_tb.sv`). A
/// pass action ran unconditionally after the check; a fail action other
/// than one severity task (`else $display(..)`, `else begin .. end`) was
/// dropped; a failure with no action printed `Error: ` with nothing after;
/// `$info` printed `Note:` and `$fatal` `Failure:` with its finish number
/// in the text. Would catch any of those back, and an unknown condition
/// passing. The run exits 1, since errors were reported, where xsim exits
/// 0; that is deliberate, and this pins it.
#[test]
fn assertions_and_reports_as_xsim_prints_them() {
    matches_xsim("assert_tb", "sv", 1);
}

/// **`unique`, `unique0` and `priority` take xsim's branches and warn
/// where xsim warns** (`unique_tb.sv`): the output without xsim's warning
/// lines is xsim's, and Reticle's warnings name the same lines in the same
/// order. `unique if` and `priority if` never warned, and `unique0 case`
/// warned when nothing matched. Would catch any of those back, a warning
/// for a chain with an `else`, or an unknown condition counted as true;
/// would not catch the warnings' wording, which differs on purpose (xsim's
/// carries the file's path), nor that they go to standard error.
#[test]
fn unique_and_priority_warn_where_xsim_does() {
    let tb = "testdata/sim/unique_tb.sv";
    let (code, stdout, stderr) = sim(tb);
    assert_eq!(code, 0, "{stderr}");
    let xsim = std::fs::read_to_string("testdata/sim/unique_tb.xsim").unwrap();
    let want: String = xsim
        .lines()
        .filter(|l| !l.starts_with("WARNING:") && !l.starts_with('\t'))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(stdout, want, "{stderr}");
    // xsim ends each warning with `from File:unique_tb.sv:<line>`.
    let xsim_lines: Vec<&str> = xsim
        .lines()
        .filter_map(|l| l.split_once("from File:unique_tb.sv:"))
        .map(|(_, n)| n)
        .collect();
    // Reticle points at the statement: `--> testdata/sim/unique_tb.sv:<line>:<col>`.
    let ours: Vec<&str> = stderr
        .lines()
        .filter_map(|l| {
            l.trim_start()
                .strip_prefix("--> testdata/sim/unique_tb.sv:")
        })
        .filter_map(|rest| rest.split(':').next())
        .collect();
    assert_eq!(xsim_lines.len(), 8, "{xsim}");
    assert_eq!(ours, xsim_lines, "{stderr}");
}

/// **`$typename` spells a type as xsim does** (`typename_tb.sv`): the
/// integer atoms with `unsigned` where it is not their default, vectors
/// with their keyword and ranges, `$` before unpacked ranges, `wire` for a
/// net, an enum with its base and the typedef's scope, a struct by its
/// typedef, another typedef by its type. It was an unknown function, so
/// `%s` printed `????`. Would catch any of those spellings changing, an
/// enum without a base type that is not `int`, or a package typedef named
/// by the module; would not catch a type the frontend refuses, such as a
/// two-dimensional unpacked array, nor `$typename` of a part select, where
/// xsim prints the whole vector's type and this prints the part's.
#[test]
fn typename_spells_types_as_xsim_does() {
    matches_xsim("typename_tb", "sv", 0);
}

/// **`%p` prints a value's structure as xsim does** (`pattern_tb.sv`):
/// structs with member names (none under `%0p`), nested, enums by name and
/// by number for a value they do not name, arrays of vectors and of enums
/// element by element, a member that is itself a struct, and the plain
/// values' spellings. `%p` was printed as the two characters `%p`. Would
/// catch any of those spellings changing, the members out of order, an
/// array read from its wrong end, or a signed member printed unsigned;
/// would not catch `%p` of a string variable, a dynamic array or a queue,
/// which are not in it, nor `%p` in `$sformatf`, which prints the plain
/// value of a struct.
#[test]
fn p_prints_structure_as_xsim_does() {
    matches_xsim("pattern_tb", "sv", 0);
}

/// **A recursive function called with an argument known only at run time
/// computes what xsim computes, as deep as Reticle inlines it, and stops
/// the simulation past that** (`recursion_tb.v`): `sum` calls itself from
/// one place and `fib` from two, inside a `?:`; both were refused. xsim's
/// output is the testbench's but for its last line, `sum(100)`, which
/// needs 101 frames where Reticle inlines 64: there Reticle prints a
/// failure naming the function and the depth, and exits 1. Would catch the
/// recursion refused again, a `?:` running both branches (every `fib` call
/// would then reach the end of the inlining and fail), and a call past the
/// depth computing a value instead of stopping; would not catch the depth
/// changing, which only moves where it stops.
#[test]
fn recursion_runs_as_deep_as_it_is_inlined() {
    let tb = "testdata/sim/recursion_tb.v";
    let (code, stdout, stderr) = sim(tb);
    assert_eq!(code, 1, "{stderr}");
    let xsim = std::fs::read_to_string("testdata/sim/recursion_tb.xsim").unwrap();
    assert!(xsim.ends_with("sum(100)=5050\n"), "{xsim}");
    let mut want = xsim.trim_end_matches("sum(100)=5050\n").to_owned();
    want.push_str(
        "Failure: `sum` recursed deeper than Reticle inlined it \
         (64 calls deep, or 2048 recursive calls in all)\n",
    );
    assert_eq!(stdout, want, "{stderr}");
}
