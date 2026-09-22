//! The bundled `std` and `ieee` libraries, shipped as VHDL source.
//!
//! VHDL's standard libraries are ordinary VHDL: they are analysed by the
//! same front end as user code, and their declarations end up in the same
//! arenas. Reticle therefore embeds them as source text with
//! [`include_str!`] rather than modelling them in Rust, which keeps one
//! code path for everything and makes the libraries auditable and fixable
//! in the language they are written in.
//!
//! Every file here is an **original implementation** written for Reticle.
//! The *declarations* match the package interfaces defined by the relevant
//! IEEE standard, because a design that says `use ieee.std_logic_1164.all;`
//! must see exactly those names and profiles; the *bodies* are Reticle's
//! own code and are not derived from the IEEE source distribution.
//!
//! # What is bundled
//!
//! | Library | Package | State |
//! |---|---|---|
//! | `std` | `standard` | complete (LRM 16.3) |
//! | `std` | `textio` | declarations complete; every subprogram is `attribute foreign` and implemented by the simulator |
//! | `std` | `env` | complete (LRM 16.5), all `foreign` |
//! | `ieee` | `std_logic_1164` | complete, with a body |
//!
//! # What is not bundled yet
//!
//! `ieee.numeric_std`, `ieee.numeric_bit`, `ieee.math_real`,
//! `ieee.std_logic_textio` and the Synopsys legacy packages
//! (`std_logic_arith`, `std_logic_unsigned`, `std_logic_signed`) are not
//! shipped yet. A design that names one gets a single diagnostic from
//! [`super::sema`] saying the package is not bundled, rather than a
//! cascade of unknown-identifier errors; see
//! [`missing_package_note`].
//!
//! # Adding a package
//!
//! Add the `.vhd` file next to its siblings and one [`Source`] entry to
//! [`SOURCES`], in dependency order (a package before anything that uses
//! it, a declaration before its body). `tests/vhdl_sema.rs` analyses every
//! entry and requires zero diagnostics.

/// One bundled source file.
#[derive(Clone, Copy, Debug)]
pub struct Source {
    /// The logical library the file is compiled into (`std` or `ieee`).
    pub library: &'static str,
    /// The file's name, used in diagnostics as `<reticle>/<library>/<name>`.
    pub name: &'static str,
    /// The VHDL source.
    pub text: &'static str,
}

/// Every bundled source, in analysis order.
pub const SOURCES: &[Source] = &[
    Source {
        library: "std",
        name: "standard.vhd",
        text: include_str!("std/standard.vhd"),
    },
    Source {
        library: "std",
        name: "textio.vhd",
        text: include_str!("std/textio.vhd"),
    },
    Source {
        library: "std",
        name: "env.vhd",
        text: include_str!("std/env.vhd"),
    },
    Source {
        library: "ieee",
        name: "std_logic_1164.vhd",
        text: include_str!("ieee/std_logic_1164.vhd"),
    },
    Source {
        library: "ieee",
        name: "std_logic_1164_body.vhd",
        text: include_str!("ieee/std_logic_1164_body.vhd"),
    },
];

/// The packages that a design may reasonably expect in `ieee` (or `std`)
/// but that Reticle does not ship yet, with the note the analyser attaches
/// when one is named.
///
/// Keeping the list here rather than in the checker means the diagnostic
/// and [`SOURCES`] are updated in the same file when a package lands.
pub const MISSING: &[(&str, &str, &str)] = &[
    (
        "ieee",
        "numeric_std",
        "`ieee.numeric_std` is not bundled yet; `unsigned`, `signed` and their arithmetic are unavailable",
    ),
    (
        "ieee",
        "numeric_bit",
        "`ieee.numeric_bit` is not bundled yet",
    ),
    (
        "ieee",
        "math_real",
        "`ieee.math_real` is not bundled yet; the real-valued maths functions are unavailable",
    ),
    (
        "ieee",
        "std_logic_textio",
        "`ieee.std_logic_textio` is not bundled yet; `std.textio` covers the predefined types",
    ),
    (
        "ieee",
        "std_logic_arith",
        "`ieee.std_logic_arith` is a Synopsys package and is not bundled",
    ),
    (
        "ieee",
        "std_logic_unsigned",
        "`ieee.std_logic_unsigned` is a Synopsys package and is not bundled",
    ),
    (
        "ieee",
        "std_logic_signed",
        "`ieee.std_logic_signed` is a Synopsys package and is not bundled",
    ),
];

/// The explanation for a package that is known to be missing, or `None`
/// when the name is not one Reticle recognises.
///
/// Both arguments are compared case-insensitively, as VHDL basic
/// identifiers are.
pub fn missing_package_note(library: &str, package: &str) -> Option<&'static str> {
    MISSING.iter().find_map(|(l, p, note)| {
        (l.eq_ignore_ascii_case(library) && p.eq_ignore_ascii_case(package)).then_some(*note)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_non_empty_and_named() {
        assert!(SOURCES.len() >= 5);
        for s in SOURCES {
            assert!(matches!(s.library, "std" | "ieee"), "{}", s.library);
            assert!(s.name.ends_with(".vhd"), "{}", s.name);
            assert!(!s.text.is_empty(), "{}", s.name);
        }
        // `standard` must come first: every other unit depends on it.
        assert_eq!(SOURCES[0].name, "standard.vhd");
    }

    #[test]
    fn missing_packages_are_reported_case_insensitively() {
        assert!(missing_package_note("ieee", "numeric_std").is_some());
        assert!(missing_package_note("IEEE", "NUMERIC_STD").is_some());
        assert!(missing_package_note("ieee", "std_logic_1164").is_none());
        assert!(missing_package_note("work", "numeric_std").is_none());
        // Nothing in MISSING is also in SOURCES.
        for (_, p, _) in MISSING {
            assert!(
                !SOURCES.iter().any(|s| s.name == format!("{p}.vhd")),
                "{p} is both bundled and listed as missing"
            );
        }
    }
}
