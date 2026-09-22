//! VHDL tokens.
//!
//! A [`Token`] is a [`TokenKind`], a [`Span`] and the token's text. The text
//! is borrowed from the source file whenever the token is spelled exactly as
//! it appears there, and owned only when the lexer had to unescape something
//! (a doubled `""` inside a string literal, a doubled `\\` inside an extended
//! identifier). Keeping text on the token means the parser never needs the
//! `SourceMap` to read an identifier or a literal, and literals stay raw
//! text (`16#FF#`, `8x"F_F"`) so their interpretation, which depends on
//! types the lexer cannot know, happens later.
//!
//! Every reserved word is its own variant so the parser matches on kinds
//! rather than comparing strings. Which words are reserved depends on the
//! [`Standard`]: VHDL-2008 added a dozen words (`context`, `force`,
//! `parameter`, `default`, the PSL ones, ...) that are plain identifiers in
//! VHDL-93 code, and real designs use some of them as names.
//!
//! Identifiers are case-insensitive in VHDL but the token keeps the original
//! spelling: diagnostics should quote what the user wrote, and the
//! case-insensitive interner arrives with the parser.

use std::borrow::Cow;
use std::fmt;

use crate::source::Span;

/// The VHDL language revision to lex against.
///
/// Only the reserved-word set is gated by this today: VHDL-2008 additions
/// are lexed as identifiers in [`Standard::Vhdl93`] mode. Newer syntax such
/// as `/* */` comments, extended bit-string literals and matching operators
/// is still recognised in both modes, with a diagnostic where the standard
/// forbids it, so that a VHDL-93 file using a 2008 feature by mistake gets
/// one precise message rather than a parse-error cascade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Standard {
    /// IEEE 1076-1993 (with the reserved words of 1076-2002, minus `protected`,
    /// which is gated with the 2008 words).
    Vhdl93,
    /// IEEE 1076-2008, the default.
    #[default]
    Vhdl2008,
}

/// Defines [`TokenKind`] and its tables from one list of reserved words and
/// one list of other kinds, so the enum, the golden-file names, the
/// reserved-word lookup and the reverse lookup cannot drift apart.
macro_rules! define_token_kinds {
    (
        reserved {
            $( $kw:ident = $word:literal $(@ $since:ident)? ),* $(,)?
        }
        other {
            $( $(#[$meta:meta])* $other:ident = $text:literal ),* $(,)?
        }
    ) => {
        /// The lexical class of a [`Token`].
        ///
        /// Reserved words come first, in the order of IEEE 1076-2008 clause
        /// 15.10; then identifiers, literals, delimiters and the two
        /// synthetic kinds `Eof` and `Error`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum TokenKind {
            $(
                #[doc = concat!("The reserved word `", $word, "`.")]
                $kw,
            )*
            $(
                $(#[$meta])*
                $other,
            )*
        }

        impl TokenKind {
            /// A stable name for this kind, used in golden files and debug
            /// output. Reserved words use their variant name (`Entity`),
            /// everything else a short label such as `Ident` or `Arrow`.
            pub fn name(self) -> &'static str {
                match self {
                    $( TokenKind::$kw => stringify!($kw), )*
                    $( TokenKind::$other => stringify!($other), )*
                }
            }

            /// The canonical (lowercase) spelling of a reserved word, or of a
            /// delimiter; `None` for kinds whose text varies.
            pub fn fixed_text(self) -> Option<&'static str> {
                match self {
                    $( TokenKind::$kw => Some($word), )*
                    $( TokenKind::$other => {
                        let t: &'static str = $text;
                        if t.is_empty() { None } else { Some(t) }
                    } )*
                }
            }

            /// True for every reserved word, in any standard.
            pub fn is_reserved_word(self) -> bool {
                matches!(self, $( TokenKind::$kw )|*)
            }

            /// Looks up a reserved word by its lowercase spelling, together
            /// with the first standard in which it is reserved.
            fn reserved_entry(lower: &str) -> Option<(TokenKind, Standard)> {
                match lower {
                    $( $word => Some((TokenKind::$kw, define_token_kinds!(@since $($since)?))), )*
                    _ => None,
                }
            }
        }
    };
    (@since) => { Standard::Vhdl93 };
    (@since $s:ident) => { Standard::$s };
}

define_token_kinds! {
    reserved {
        Abs = "abs",
        Access = "access",
        After = "after",
        Alias = "alias",
        All = "all",
        And = "and",
        Architecture = "architecture",
        Array = "array",
        Assert = "assert",
        Assume = "assume" @ Vhdl2008,
        AssumeGuarantee = "assume_guarantee" @ Vhdl2008,
        Attribute = "attribute",
        Begin = "begin",
        Block = "block",
        Body = "body",
        Buffer = "buffer",
        Bus = "bus",
        Case = "case",
        Component = "component",
        Configuration = "configuration",
        Constant = "constant",
        Context = "context" @ Vhdl2008,
        Cover = "cover" @ Vhdl2008,
        Default = "default" @ Vhdl2008,
        Disconnect = "disconnect",
        Downto = "downto",
        Else = "else",
        Elsif = "elsif",
        End = "end",
        Entity = "entity",
        Exit = "exit",
        Fairness = "fairness" @ Vhdl2008,
        File = "file",
        For = "for",
        Force = "force" @ Vhdl2008,
        Function = "function",
        Generate = "generate",
        Generic = "generic",
        Group = "group",
        Guarded = "guarded",
        If = "if",
        Impure = "impure",
        In = "in",
        Inertial = "inertial",
        Inout = "inout",
        Is = "is",
        Label = "label",
        Library = "library",
        Linkage = "linkage",
        Literal = "literal",
        Loop = "loop",
        Map = "map",
        Mod = "mod",
        Nand = "nand",
        New = "new",
        Next = "next",
        Nor = "nor",
        Not = "not",
        Null = "null",
        Of = "of",
        On = "on",
        Open = "open",
        Or = "or",
        Others = "others",
        Out = "out",
        Package = "package",
        Parameter = "parameter" @ Vhdl2008,
        Port = "port",
        Postponed = "postponed",
        Procedure = "procedure",
        Process = "process",
        Property = "property" @ Vhdl2008,
        Protected = "protected" @ Vhdl2008,
        Pure = "pure",
        Range = "range",
        Record = "record",
        Register = "register",
        Reject = "reject",
        Release = "release" @ Vhdl2008,
        Rem = "rem",
        Report = "report",
        Restrict = "restrict" @ Vhdl2008,
        RestrictGuarantee = "restrict_guarantee" @ Vhdl2008,
        Return = "return",
        Rol = "rol",
        Ror = "ror",
        Select = "select",
        Sequence = "sequence" @ Vhdl2008,
        Severity = "severity",
        Shared = "shared",
        Signal = "signal",
        Sla = "sla",
        Sll = "sll",
        Sra = "sra",
        Srl = "srl",
        Strong = "strong" @ Vhdl2008,
        Subtype = "subtype",
        Then = "then",
        To = "to",
        Transport = "transport",
        Type = "type",
        Unaffected = "unaffected",
        Units = "units",
        Until = "until",
        Use = "use",
        Variable = "variable",
        Vmode = "vmode" @ Vhdl2008,
        Vprop = "vprop" @ Vhdl2008,
        Vunit = "vunit" @ Vhdl2008,
        Wait = "wait",
        When = "when",
        While = "while",
        With = "with",
        Xnor = "xnor",
        Xor = "xor",
    }
    other {
        /// A basic identifier, case-insensitive; the text is the original
        /// spelling.
        Ident = "",
        /// An extended identifier `\name\`, case-sensitive; the text is the
        /// content without the backslashes and with `\\` unescaped.
        ExtendedIdent = "",
        /// A character literal `'a'`; the text is the character alone.
        CharLit = "",
        /// A string literal `"..."` (or `%...%`); the text is the content
        /// without delimiters and with doubled delimiters unescaped.
        StringLit = "",
        /// A bit-string literal such as `x"FF"`, `sb"1010"` or `8ux"F_F"`;
        /// the text is the raw literal including length and base specifier.
        BitStringLit = "",
        /// An integer literal, decimal (`42`, `1_000`, `1E3`) or based
        /// (`16#FF#`, `2#1010#E4`); the text is the raw literal.
        Integer = "",
        /// A real literal, decimal (`3.14`, `1.0E-3`) or based (`16#F.8#`);
        /// the text is the raw literal.
        Real = "",
        /// `&`
        Amp = "&",
        /// `'` used as an attribute or qualified-expression tick.
        Tick = "'",
        /// `(`
        LParen = "(",
        /// `)`
        RParen = ")",
        /// `*`
        Star = "*",
        /// `+`
        Plus = "+",
        /// `,`
        Comma = ",",
        /// `-`
        Minus = "-",
        /// `.`
        Dot = ".",
        /// `/`
        Slash = "/",
        /// `:`
        Colon = ":",
        /// `;`
        Semi = ";",
        /// `<`
        Lt = "<",
        /// `=`
        Eq = "=",
        /// `>`
        Gt = ">",
        /// `?` alone, as in the VHDL-2008 `case?` and `select?`.
        Question = "?",
        /// `@`, the VHDL-2008 external-name prefix.
        At = "@",
        /// `[`
        LBracket = "[",
        /// `]`
        RBracket = "]",
        /// `|` (also spelled `!`; the text keeps the spelling used).
        Bar = "|",
        /// `^`, the VHDL-2008 relative-pathname element.
        Caret = "^",
        /// `=>`
        Arrow = "=>",
        /// `**`
        StarStar = "**",
        /// `:=`
        ColonEq = ":=",
        /// `/=`
        Neq = "/=",
        /// `>=`
        Ge = ">=",
        /// `<=`
        Le = "<=",
        /// `<>`
        Box = "<>",
        /// `??`, the VHDL-2008 condition operator.
        QQ = "??",
        /// `?=`
        QEq = "?=",
        /// `?/=`
        QNeq = "?/=",
        /// `?<`
        QLt = "?<",
        /// `?<=`
        QLe = "?<=",
        /// `?>`
        QGt = "?>",
        /// `?>=`
        QGe = "?>=",
        /// `<<`
        LtLt = "<<",
        /// `>>`
        GtGt = ">>",
        /// End of input; always the last token, with an empty span.
        Eof = "",
        /// A character that cannot start any token; a diagnostic was
        /// reported. The text is the offending character.
        Error = "",
    }
}

/// Longest reserved word (`restrict_guarantee`), the size of the stack
/// buffer used to lowercase candidate identifiers without allocating.
const LONGEST_RESERVED: usize = 18;

impl TokenKind {
    /// Classifies an identifier's text: the reserved word it spells under
    /// `standard`, or [`TokenKind::Ident`] otherwise.
    ///
    /// Reserved words are ASCII, so the comparison lowercases ASCII only.
    /// Basic identifiers with other Latin-1 letters can never be reserved.
    pub fn classify_ident(text: &str, standard: Standard) -> TokenKind {
        if text.len() > LONGEST_RESERVED || !text.is_ascii() {
            return TokenKind::Ident;
        }
        let mut buf = [0u8; LONGEST_RESERVED];
        for (dst, src) in buf.iter_mut().zip(text.bytes()) {
            *dst = src.to_ascii_lowercase();
        }
        // Only ASCII bytes were written, so the slice is valid UTF-8.
        let lower = std::str::from_utf8(&buf[..text.len()]).unwrap_or("");
        match TokenKind::reserved_entry(lower) {
            Some((kind, since)) if since <= standard => kind,
            _ => TokenKind::Ident,
        }
    }

    /// True for the literal kinds: character, string, bit-string, integer
    /// and real.
    pub fn is_literal(self) -> bool {
        matches!(
            self,
            TokenKind::CharLit
                | TokenKind::StringLit
                | TokenKind::BitStringLit
                | TokenKind::Integer
                | TokenKind::Real
        )
    }
}

impl fmt::Display for TokenKind {
    /// Shows the fixed spelling for reserved words and delimiters, and the
    /// kind name in angle brackets otherwise (`<identifier>`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.fixed_text() {
            Some(text) => write!(f, "`{text}`"),
            None => match self {
                TokenKind::Ident => f.write_str("identifier"),
                TokenKind::ExtendedIdent => f.write_str("extended identifier"),
                TokenKind::CharLit => f.write_str("character literal"),
                TokenKind::StringLit => f.write_str("string literal"),
                TokenKind::BitStringLit => f.write_str("bit-string literal"),
                TokenKind::Integer => f.write_str("integer literal"),
                TokenKind::Real => f.write_str("real literal"),
                TokenKind::Eof => f.write_str("end of file"),
                TokenKind::Error => f.write_str("invalid character"),
                _ => f.write_str(self.name()),
            },
        }
    }
}

/// One lexical token with its source range and text.
///
/// `'src` is the lifetime of the source text the token was lexed from; see
/// the module docs for what `text` holds for each kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token<'src> {
    /// The lexical class.
    pub kind: TokenKind,
    /// The bytes of the source covered by the token, delimiters included.
    pub span: Span,
    /// The token's text, borrowed from the source unless unescaping was
    /// needed.
    pub text: Cow<'src, str>,
}

impl<'src> Token<'src> {
    /// Builds a token whose text is a slice of the source.
    pub fn new(kind: TokenKind, span: Span, text: &'src str) -> Self {
        Token {
            kind,
            span,
            text: Cow::Borrowed(text),
        }
    }

    /// The token's text as a string slice.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// True when the token has the given kind.
    pub fn is(&self, kind: TokenKind) -> bool {
        self.kind == kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_words_are_case_insensitive() {
        assert_eq!(
            TokenKind::classify_ident("ENTITY", Standard::Vhdl2008),
            TokenKind::Entity
        );
        assert_eq!(
            TokenKind::classify_ident("Entity", Standard::Vhdl93),
            TokenKind::Entity
        );
        assert_eq!(
            TokenKind::classify_ident("entities", Standard::Vhdl2008),
            TokenKind::Ident
        );
    }

    #[test]
    fn standard_gates_2008_words() {
        for word in ["context", "force", "parameter", "default", "vunit"] {
            let k = TokenKind::classify_ident(word, Standard::Vhdl2008);
            assert!(k.is_reserved_word(), "{word} should be reserved in 2008");
            assert_eq!(
                TokenKind::classify_ident(word, Standard::Vhdl93),
                TokenKind::Ident,
                "{word} is an identifier in VHDL-93"
            );
        }
    }

    #[test]
    fn long_and_non_ascii_names_are_identifiers() {
        assert_eq!(
            TokenKind::classify_ident("restrict_guarantee", Standard::Vhdl2008),
            TokenKind::RestrictGuarantee
        );
        assert_eq!(
            TokenKind::classify_ident("restrict_guarantees", Standard::Vhdl2008),
            TokenKind::Ident
        );
        assert_eq!(
            TokenKind::classify_ident("señal", Standard::Vhdl2008),
            TokenKind::Ident
        );
    }

    #[test]
    fn names_and_fixed_text() {
        assert_eq!(TokenKind::Arrow.name(), "Arrow");
        assert_eq!(TokenKind::Arrow.fixed_text(), Some("=>"));
        assert_eq!(TokenKind::Ident.fixed_text(), None);
        assert_eq!(TokenKind::Entity.fixed_text(), Some("entity"));
        assert_eq!(TokenKind::Entity.to_string(), "`entity`");
        assert_eq!(TokenKind::Ident.to_string(), "identifier");
        assert!(TokenKind::Real.is_literal());
        assert!(!TokenKind::Tick.is_literal());
    }
}
