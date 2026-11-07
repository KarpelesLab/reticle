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
