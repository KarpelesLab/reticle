//! A small command-line argument parser.
//!
//! Reticle ships no dependencies, so the CLI parses its own arguments. The
//! model is deliberately plain: a command word, then a mix of flags
//! (`--vcd`), options with values (`--top=m` or `--top m`) and positional
//! file names, with `--` ending option parsing.
//!
//! Unknown options are an error rather than a silently ignored typo, since
//! a mistyped `--dpeth` should not quietly run with the default depth.

use std::collections::BTreeMap;
use std::fmt;

/// Everything parsed out of one command's argument list.
#[derive(Debug, Default)]
pub(crate) struct Args {
    flags: Vec<String>,
    options: BTreeMap<String, String>,
    positionals: Vec<String>,
}

/// Why an argument list could not be parsed.
#[derive(Debug)]
pub(crate) enum ArgError {
    /// An option was given that the command does not accept.
    Unknown(String),
    /// An option that needs a value was given without one.
    MissingValue(String),
    /// A value was given for a flag that takes none.
    UnexpectedValue(String),
    /// An option's value is not of the expected shape.
    BadValue {
        /// The option name, without dashes.
        option: String,
        /// The value as written.
        value: String,
        /// What was expected instead.
        expected: &'static str,
    },
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgError::Unknown(name) => write!(f, "unknown option `--{name}`"),
            ArgError::MissingValue(name) => write!(f, "`--{name}` needs a value"),
            ArgError::UnexpectedValue(name) => write!(f, "`--{name}` takes no value"),
            ArgError::BadValue {
                option,
                value,
                expected,
            } => write!(f, "`--{option} {value}` is not {expected}"),
        }
    }
}

/// What a command accepts, used to parse and to reject typos.
pub(crate) struct Spec {
    /// Options that take a value.
    pub(crate) options: &'static [&'static str],
    /// Options that take no value.
    pub(crate) flags: &'static [&'static str],
}

impl Args {
    /// Parses `argv` (without the command word) against `spec`.
    pub(crate) fn parse(argv: &[String], spec: &Spec) -> Result<Args, ArgError> {
        let mut args = Args::default();
        let mut rest_are_positional = false;
        let mut i = 0;
        while i < argv.len() {
            let arg = &argv[i];
            i += 1;
            if rest_are_positional || !arg.starts_with("--") {
                args.positionals.push(arg.clone());
                continue;
            }
            if arg == "--" {
                rest_are_positional = true;
                continue;
            }
            let body = &arg[2..];
            let (name, inline) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (body, None),
            };
            if spec.flags.contains(&name) {
                if inline.is_some() {
                    return Err(ArgError::UnexpectedValue(name.to_string()));
                }
                args.flags.push(name.to_string());
            } else if spec.options.contains(&name) {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        let next = argv
                            .get(i)
                            .ok_or_else(|| ArgError::MissingValue(name.to_string()))?;
                        i += 1;
                        next.clone()
                    }
                };
                args.options.insert(name.to_string(), value);
            } else {
                return Err(ArgError::Unknown(name.to_string()));
            }
        }
        Ok(args)
    }

    /// True when the flag was given.
    pub(crate) fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    /// The option's value, if given.
    pub(crate) fn option(&self, name: &str) -> Option<&str> {
        self.options.get(name).map(String::as_str)
    }

    /// The option parsed as an unsigned integer.
    pub(crate) fn u64_option(&self, name: &str) -> Result<Option<u64>, ArgError> {
        match self.option(name) {
            None => Ok(None),
            Some(value) => value.parse().map(Some).map_err(|_| ArgError::BadValue {
                option: name.to_string(),
                value: value.to_string(),
                expected: "a whole number",
            }),
        }
    }

    /// The option parsed as a `u32`.
    pub(crate) fn u32_option(&self, name: &str) -> Result<Option<u32>, ArgError> {
        match self.u64_option(name)? {
            None => Ok(None),
            Some(v) => u32::try_from(v).map(Some).map_err(|_| ArgError::BadValue {
                option: name.to_string(),
                value: v.to_string(),
                expected: "a number below 2^32",
            }),
        }
    }

    /// The positional arguments, in order.
    pub(crate) fn positionals(&self) -> &[String] {
        &self.positionals
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec = Spec {
        options: &["top", "depth"],
        flags: &["vcd", "quiet"],
    };

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_flags_options_and_positionals() {
        let args = Args::parse(
            &argv(&["a.rtl", "--top", "m", "--vcd", "--depth=12", "b.rtl"]),
            &SPEC,
        )
        .unwrap();
        assert_eq!(args.positionals(), ["a.rtl", "b.rtl"]);
        assert_eq!(args.option("top"), Some("m"));
        assert_eq!(args.u32_option("depth").unwrap(), Some(12));
        assert!(args.flag("vcd"));
        assert!(!args.flag("quiet"));
        assert_eq!(args.u32_option("missing").unwrap(), None);
    }

    #[test]
    fn double_dash_ends_options() {
        let args = Args::parse(&argv(&["--", "--top", "-x"]), &SPEC).unwrap();
        assert_eq!(args.positionals(), ["--top", "-x"]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            Args::parse(&argv(&["--nope"]), &SPEC),
            Err(ArgError::Unknown(n)) if n == "nope"
        ));
        assert!(matches!(
            Args::parse(&argv(&["--top"]), &SPEC),
            Err(ArgError::MissingValue(n)) if n == "top"
        ));
        assert!(matches!(
            Args::parse(&argv(&["--vcd=1"]), &SPEC),
            Err(ArgError::UnexpectedValue(n)) if n == "vcd"
        ));
        let args = Args::parse(&argv(&["--depth", "wide"]), &SPEC).unwrap();
        assert!(matches!(
            args.u32_option("depth"),
            Err(ArgError::BadValue { .. })
        ));
    }
}
