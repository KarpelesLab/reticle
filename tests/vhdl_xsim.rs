//! VHDL testbenches simulated by `reticle sim`, held to what Vivado
//! 2026.1's xsim printed for the same file.
//!
//! Every `testdata/sim/vhdl_xsim/<name>_tb.vhd` has a `<name>_tb.xsim`
//! next to it: xsim's own output for the file, every `report` and
//! `assert` line, recorded by running it under `xvhdl --2008` and
//! `xsim -R`. The test runs the same file through the `reticle` binary
//! with `--quiet --top tb` and compares standard output byte for byte, so
//! a fixture is a measurement, not a reading of the standard.
//!
//! Each test says what its fixture would and would not catch.

#![cfg(all(feature = "cli", feature = "vhdl", feature = "sim"))]

use std::process::Command;

/// Runs `reticle sim --quiet --top tb` on one fixture and compares its
/// standard output with the xsim recording.
fn matches_xsim(name: &str) {
    let tb = format!("testdata/sim/vhdl_xsim/{name}_tb.vhd");
    let want = format!("testdata/sim/vhdl_xsim/{name}_tb.xsim");
    let out = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["sim", "--quiet", "--top", "tb", &tb])
        .output()
        .expect("failed to run the reticle binary");
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let want = std::fs::read_to_string(&want)
        .unwrap_or_else(|e| panic!("{want}: {e}"))
        .replace("\r\n", "\n");
    assert!(
        !stderr.contains("error["),
        "{tb} did not elaborate:\n{stderr}"
    );
    assert_eq!(
        stdout, want,
        "{tb} printed something else than xsim\n{stderr}"
    );
}

/// **Report messages built at run time**: `'image` of integers, booleans,
/// times, characters, `std_logic` and a user enumeration; `'succ`, `'pred`,
/// `'pos` and `'val` of variables; the predefined `to_string` of scalars
/// and bit vectors and the `to_hstring`/`to_ostring` of `unsigned`,
/// `signed` and `std_logic_vector`; chains of `&` mixing strings and
/// characters; string comparison; `now`; `minimum`/`maximum` of an
/// enumeration; a `%` in a message; and `Assertion violation` for an
/// assertion without `report`. All of it was refused before (`V0704`,
/// `V0707`, `V0303`) except the last, which printed an empty message.
///
/// Would catch any image written in another form than xsim's (`5000000 fs`
/// for a time, `'a'` for `character'image('A')`, a signed vector
/// zero-extended into its hexadecimal digits), a message whose `%` is
/// read as a format, and a concatenation that drops an operand. Would not
/// catch a nine-state value printed as its four-state collapse: no
/// variable here holds `'U'`, `'H'` or `'-'`.
#[test]
fn report_strings_match_xsim() {
    matches_xsim("report_strings");
}

/// **Operators that answered wrongly**: a bit-string literal compared with
/// an `unsigned` or concatenated to a slice (refused, "this literal cannot
/// be evaluated"); a `std_logic` function folded with constant arguments
/// (it answered `'U'` and `'X'`, its logic read through `bit`'s
/// encoding); `?? 'X'` (true); `'-'` in `?=`, `case?` and `select?`
/// (compared as an unknown); and every shift and rotate of a
/// `std_logic_vector` or `bit_vector` (all zeros, the count replicated
/// across the vector, and the rotates refused).
///
/// Would catch any of those back, `sla` filling with zeros rather than the
/// rightmost bit, a static negative count shifting the wrong way, and a
/// rotate by more than the length. Would not catch a `'-'` held by a
/// signal at run time, which the four-state simulator holds as `x` and
/// which xsim matches: that case is left out because it differs.
#[test]
fn operators_match_xsim() {
    matches_xsim("operators");
}

/// **Slices that move with a loop, or come from a generic, and jumps out
/// of nested loops**: `v(8*i+7 downto 8*i)` read and assigned, on a signal
/// and a variable, and the byte-write loop `mem(a)(8*i+7 downto 8*i) <= d`
/// on a signal memory and a variable one, all refused before ("must have
/// bounds that are static after elaboration") or, as a read, a silent
/// zero; `a(W - 1 downto 0)` in an entity with a generic `W`, which read
/// as zeros; `next outer` and `exit outer`, refused; a loop range that
/// depends on another loop's parameter or on a variable, refused; and a
/// loop over an enumeration, whose parameter printed as a number.
///
/// Would catch a byte written to the wrong lane, the rest of a word lost, a
/// `next` that skips the wrong iteration or an `exit` that leaves the wrong
/// loop, and a loop bound read again after the variable it came from was
/// changed (`n` is decremented inside the loop it bounds). Would not catch
/// a loop with more iterations than the unroll limit, which is refused.
#[test]
fn loop_slices_match_xsim() {
    matches_xsim("loop_slices");
}
