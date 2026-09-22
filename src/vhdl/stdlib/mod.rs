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
//! # Declarations in VHDL, bodies in Rust
//!
//! Two shapes are used, and which one a package takes is a deliberate
//! choice rather than an accident:
//!
//! - `std_logic_1164` carries a VHDL **body**, because its nine-state
//!   tables *are* the specification and are best read as tables.
//! - every other package declares its subprograms and marks each one
//!   `attribute foreign ... is "reticle: builtin"`, which leaves the
//!   package without a body and hands the implementation to Rust.
//!
//! For the arithmetic packages the second shape is the better design, not
//! merely the cheaper one. `unsigned` and `signed` are arrays of logic
//! values with arithmetic on them, and [`crate::logic::Logic`] already
//! implements every operation they need — add, subtract, multiply,
//! divide, both remainders, comparisons, shifts and resize, signed and
//! unsigned, four-state throughout. Folding a static
//! `to_unsigned(1, 8) + 3` natively is far faster than interpreting a
//! VHDL body, gives exactly the answer the simulator would give because
//! it is the same code, and keeps the bundled text down to the profiles a
//! design has to see.
//!
//! The Rust side lives in two places: [`crate::vhdl::sema::builtin`]
//! folds a call whose arguments are all static, and
//! `crate::vhdl::elab`'s `numeric` module lowers one that is not to the
//! matching IR operator with explicit `Resize` nodes, since the IR
//! requires operands of equal width.
//!
//! # What is bundled
//!
//! | Library | Package | State |
//! |---|---|---|
//! | `std` | `standard` | complete (LRM 16.3) |
//! | `std` | `textio` | declarations complete; every subprogram is `attribute foreign` and implemented by the simulator |
//! | `std` | `env` | complete (LRM 16.5), all `foreign` |
//! | `ieee` | `std_logic_1164` | complete, with a body |
//! | `ieee` | `numeric_std` | declarations complete (LRM 16.9), all `foreign`, bodies native |
//! | `ieee` | `numeric_bit` | declarations complete (LRM 16.10), all `foreign`, sharing `numeric_std`'s core |
//! | `ieee` | `math_real` | constants and functions, functions `foreign` over `f64` |
//! | `ieee` | `std_logic_textio` | declarations complete, all `foreign` |
//! | `ieee` | `std_logic_arith` | the Synopsys interface, all `foreign` |
//! | `ieee` | `std_logic_unsigned` | the Synopsys interface, all `foreign` |
//! | `ieee` | `std_logic_signed` | the Synopsys interface, all `foreign` |
//!
//! # What is not bundled yet
//!
//! The VHDL-2008 packages that build further on `numeric_std`:
//! `ieee.fixed_pkg`, `ieee.float_pkg` and `ieee.numeric_std_unsigned`.
//! They are listed in [`MISSING`], so a design that names one gets a
//! single diagnostic from [`super::sema`] saying the package is not
//! bundled rather than a cascade of unknown-identifier errors; see
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
    Source {
        library: "ieee",
        name: "numeric_std.vhd",
        text: include_str!("ieee/numeric_std.vhd"),
    },
    Source {
        library: "ieee",
        name: "numeric_bit.vhd",
        text: include_str!("ieee/numeric_bit.vhd"),
    },
    Source {
        library: "ieee",
        name: "math_real.vhd",
        text: include_str!("ieee/math_real.vhd"),
    },
    Source {
        library: "ieee",
        name: "std_logic_textio.vhd",
        text: include_str!("ieee/std_logic_textio.vhd"),
    },
    Source {
        library: "ieee",
        name: "std_logic_arith.vhd",
        text: include_str!("ieee/std_logic_arith.vhd"),
    },
    Source {
        library: "ieee",
        name: "std_logic_unsigned.vhd",
        text: include_str!("ieee/std_logic_unsigned.vhd"),
    },
    Source {
        library: "ieee",
        name: "std_logic_signed.vhd",
        text: include_str!("ieee/std_logic_signed.vhd"),
    },
];

/// The packages that a design may reasonably expect in `ieee` (or `std`)
/// but that Reticle does not ship yet, with the note the analyser attaches
/// when one is named.
///
/// Keeping the list here rather than in the checker means the diagnostic
/// and [`SOURCES`] are updated in the same file when a package lands.
///
/// The remaining entries are the VHDL-2008 packages that build further on
/// `numeric_std`: a design naming one gets a single `V0107` rather than
/// an error for every name it uses.
pub const MISSING: &[(&str, &str, &str)] = &[
    (
        "ieee",
        "fixed_pkg",
        "`ieee.fixed_pkg` is not bundled yet; the fixed-point types are unavailable",
    ),
    (
        "ieee",
        "float_pkg",
        "`ieee.float_pkg` is not bundled yet; the floating-point types are unavailable",
    ),
    (
        "ieee",
        "numeric_std_unsigned",
        "`ieee.numeric_std_unsigned` is not bundled yet; `ieee.numeric_std` or `ieee.std_logic_unsigned` covers the same arithmetic",
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

    /// Every bundled file must say, in its own text, that it is an
    /// original implementation rather than a copy of the standard's.
    #[test]
    fn sources_carry_a_provenance_header() {
        for s in SOURCES {
            let head: String = s.text.lines().take(20).collect::<Vec<_>>().join("\n");
            assert!(
                head.to_ascii_lowercase()
                    .contains("clean-room source for reticle"),
                "{} has no provenance header",
                s.name
            );
        }
    }

    #[test]
    fn missing_packages_are_reported_case_insensitively() {
        assert!(missing_package_note("ieee", "fixed_pkg").is_some());
        assert!(missing_package_note("IEEE", "FIXED_PKG").is_some());
        assert!(missing_package_note("ieee", "std_logic_1164").is_none());
        assert!(missing_package_note("work", "fixed_pkg").is_none());
        // The packages that landed are no longer reported as missing.
        for p in [
            "numeric_std",
            "numeric_bit",
            "math_real",
            "std_logic_textio",
            "std_logic_arith",
            "std_logic_unsigned",
            "std_logic_signed",
        ] {
            assert!(missing_package_note("ieee", p).is_none(), "{p}");
            assert!(SOURCES.iter().any(|s| s.name == format!("{p}.vhd")), "{p}");
        }
        // Nothing in MISSING is also in SOURCES.
        for (_, p, _) in MISSING {
            assert!(
                !SOURCES.iter().any(|s| s.name == format!("{p}.vhd")),
                "{p} is both bundled and listed as missing"
            );
        }
    }
}
