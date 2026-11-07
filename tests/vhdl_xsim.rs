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

/// **Waits, signal history and initial values**: `wait for 0 ns`
/// (resumed before the delta's update), `wait until c` when `c` already
/// holds (resumed at once), `wait until c for t` and `wait on s for t`
/// (never timed out); `rising_edge`/`falling_edge` outside a clock
/// condition, `wait until rising_edge(clk)` in a process that is not a
/// clocked one, `'event`, `'active`, `'last_value`, `'last_event`,
/// `'stable` and `'delayed` (all refused); an edge process on a signal
/// initialised to its edge's level (fired at time zero); and
/// `std.env.finish` (refused, "no body to inline").
///
/// Would catch a wait resuming a delta early or late, a timeout that fires
/// when the condition came first, an event that outlives its delta, and
/// an initial value that makes an event. Would not catch the
/// one-delta-per-statement chain of concurrent assignments, which this
/// simulator settles in one delta and xsim in several; nothing here
/// depends on it.
#[test]
fn waits_and_events_match_xsim() {
    matches_xsim("waits_events");
}

/// **Values that came out wrong**: the initial value of an array of
/// integers indexed `(7 downto 4)` (reversed), an aggregate as the target
/// of a variable and of a signal assignment (assigned nothing), an alias
/// whose subtype runs the other way from its target's (indexed as the
/// target), an alias of a constant (refused), and a `case` choice
/// `integer'low to -1` (the analysis never finished, recording two
/// billion values one at a time).
///
/// Would catch any of those back, and a wide range lowered to a chain of
/// tests that takes the wrong alternative. Would not catch an alias of a
/// slice of a memory element, which is not here.
#[test]
fn values_match_xsim() {
    matches_xsim("values");
}

/// **`real` objects**: variables and signals of type `real` ("has no
/// representation in the IR"), their arithmetic, comparisons, `**`,
/// `mod`, `abs` and negation, `ieee.math_real` called with values known
/// only at run time, `integer(r)` and `real(i)`, a time scaled by a real,
/// and a `wait for` computed from a real constant ("this literal cannot be
/// evaluated").
///
/// Would catch `integer(r)` truncating rather than rounding halves away
/// from zero, `mod` taking the dividend's sign, and a real image in any
/// other form than xsim's. Would not catch a `real` that has no exact
/// binary value printed differently in its last digit, since every value
/// here is exact or rounds alike.
#[test]
fn reals_match_xsim() {
    matches_xsim("reals");
}

/// **`to_string` with a second argument, and the rest of `std.standard`'s
/// predefined operations**: a time in a unit, a real with a number of
/// digits or a C format; `minimum` and `maximum` of a vector; a unit name
/// alone as a value (`ps`, "has no value at elaboration"); and a loop over
/// an enumeration range written with attributes, whose parameter printed
/// as a number.
///
/// Would catch a time in a unit written as a real (`1.5 ns`, which LRM 5.7
/// reads it as and xsim does not), a unit finer than a picosecond kept
/// (xsim writes `0 ps` for `to_string(now, fs)`), and `maximum` of vectors
/// ordered as signed numbers. Would not catch a format with a width
/// (`"%8.3f"`), which is not here.
#[test]
fn to_string_overloads_match_xsim() {
    matches_xsim("to_string");
}

/// **Objects whose subtype leaves the bounds open**: `'length`, `'left`
/// and `'right` of a string constant and generic ("cannot be lowered"); a
/// port sized by an unconstrained generic's `'range` ("not static after
/// elaboration"); an unconstrained parameter given a literal, a signal or
/// a slice, looped over with `'range` and copied into a variable of its
/// `'reverse_range`; an unconstrained result compared with a literal; and
/// an unconstrained `signal` parameter of a procedure. All were refused.
///
/// Would catch a parameter taking the length of the context rather than
/// the actual's, a `'reverse_range` constraint keeping the original
/// direction, and a result whose width is not the returned value's. Would
/// not catch a function that returns values of different lengths from
/// different `return`s, which takes the first one's.
#[test]
fn unconstrained_objects_match_xsim() {
    matches_xsim("unconstrained");
}

/// **Arrays of wide elements whole, and string objects**: an aggregate
/// assigned to an array of integers, one array copied to another and
/// compared with it, a slice of one assigned from a slice of another
/// (signal and variable), and a `string(1 to 5)` variable assigned a
/// constant, indexed, modified, printed and compared. All were refused (a
/// whole memory "cannot be assigned at once", "can only be read one
/// element at a time", and the string an internal error).
///
/// Would catch elements written to the wrong addresses or in the reverse
/// order, and a string's characters reversed. Would not catch an array of
/// more than 4096 elements used whole, which is refused.
#[test]
fn whole_arrays_match_xsim() {
    matches_xsim("whole_arrays");
}

/// **Nested targets**: a field of a record field (`o.p.x := 7`), an
/// element and a slice of a record field, at a static and a run-time
/// index, and a field of an element of an array of records, on a variable
/// and a signal (`r(1).a <= 9`), with part of that field (`v(2).b(7
/// downto 4)`). All but the first were "a nested record target" or "a
/// nested array target".
///
/// Would catch a field written at the offset of the record rather than of
/// the field it is in, and a write that clears the rest of the element.
/// Would not catch a target whose index is not static inside an array of
/// records, which is still refused.
#[test]
fn nested_targets_match_xsim() {
    matches_xsim("nested_targets");
}

/// **Inertial delay**: a 2 ns pulse through a 3 ns inertial concurrent
/// assignment (passed, every delay being transport) and through a
/// `transport` one (passes), a pulse longer than the delay (passes), a
/// pending process update cancelled by the next one of another value (was
/// not), and a waveform of several elements, whose later elements must not
/// cancel the first.
///
/// Would catch inertial delay applied to `transport`, to the later
/// elements of a waveform, or not at all. Would not catch a `reject` limit
/// shorter than the delay, which is lowered as transport, nor an
/// assignment without `after` cancelling pending updates, which is not
/// done; neither is here.
#[test]
fn inertial_delay_matches_xsim() {
    matches_xsim("inertial");
}
