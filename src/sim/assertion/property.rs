//! The property language: its abstract syntax and a parser for it.
//!
//! The syntax is the SystemVerilog assertion (SVA) one of IEEE 1800 §16,
//! with the PSL spellings that differ only in punctuation accepted as well
//! (`always`, `never`, `->` for boolean implication, and `{a; b}` /
//! `{a : b}` SERE braces for `##1` and `##0` concatenation). What is and is
//! not accepted is listed in the [assertion module docs](super).
//!
//! # Grammar
//!
//! ```text
//! directive  := [ name ":" ] [ "assert" | "assume" | "cover" ]
//!               [ "property" | "sequence" ] [ "(" ] body [ ")" ] [ ";" ]
//! body       := [ clock ] [ "disable" "iff" "(" bool ")" ] property [ clock ]
//! clock      := "@" [ "(" ] [ "posedge" | "negedge" | "edge" ] name [ ")" ]
//!
//! property   := or [ ( "|->" | "|=>" ) property ]
//! or         := and { "or" and }
//! and        := unary { "and" unary }
//! unary      := ( "not" | "always" | "never" ) unary | intersect
//! intersect  := within { "intersect" within }
//! within     := throughout { "within" throughout }
//! throughout := concat [ "throughout" throughout ]
//! concat     := repeat { delay repeat }
//! repeat     := primary { "[*" count "]" | "[->" count "]" }
//! primary    := "(" property ")" | "{" sere "}" | delay repeat | bool
//! sere       := property { ( ";" | ":" ) property }
//!
//! delay      := "##" ( int | "[" bound ":" bound "]" | "[*]" | "[+]" )
//! count      := int [ ":" bound ]
//! bound      := int | "$"
//! ```
//!
//! Boolean expressions are, loosest to tightest: `->`, `||`, `&&`, then a
//! comparison (`==`, `!=`, `===`, `!==`, `<`, `<=`, `>`, `>=`) between two
//! operands, then the unary `!` and `~`, then an operand: a net with an
//! optional bit or part select (`q[3]`, `q[7:4]`), a sized or unsized
//! literal (`1'b1`, `8'hff`, `5`), or one of `$rose`, `$fell` and
//! `$stable`. `!` binds looser than a comparison, so `!a == b` parses as
//! `!(a == b)`; parenthesise when that is not what is meant.
//!
//! Names are net names as [`crate::sim::Simulator::net`] takes them:
//! either a full hierarchical path or a name relative to the scope the
//! directive is added in.

use crate::diag::Diagnostic;
use crate::ir::Polarity;
use crate::logic::Logic;
use crate::source::Span;

// ---------------------------------------------------------------------------
// Abstract syntax
// ---------------------------------------------------------------------------

/// A bit or part select on a net reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    /// `net[index]`.
    Bit(u32),
    /// `net[hi:lo]`.
    Part {
        /// Most significant bit, inclusive.
        hi: u32,
        /// Least significant bit, inclusive.
        lo: u32,
    },
}

/// A net named by a property, with an optional select.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetRef {
    /// The name as written.
    pub name: String,
    /// The bit or part select, if any.
    pub select: Option<Select>,
    /// Where the reference was written.
    pub span: Span,
}

/// A value-producing term of a boolean expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    /// The current sampled value of a net.
    Net(NetRef),
    /// A literal.
    Const(Logic),
}

/// A comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    /// `==`, unknown-propagating.
    Eq,
    /// `!=`, unknown-propagating.
    Ne,
    /// `===`, bit-for-bit including `x` and `z`.
    CaseEq,
    /// `!==`.
    CaseNe,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

impl CmpOp {
    /// The operator as written.
    pub fn as_str(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::CaseEq => "===",
            CmpOp::CaseNe => "!==",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
}

/// A boolean over sampled net values: the letters a sequence is made of.
#[derive(Clone, Debug, PartialEq)]
pub enum BoolExpr {
    /// A literal `true` or `false`.
    Const(bool),
    /// True when the operand is non-zero and fully known.
    Value(Operand),
    /// Logical negation.
    Not(Box<BoolExpr>),
    /// `&&`.
    And(Box<BoolExpr>, Box<BoolExpr>),
    /// `||`.
    Or(Box<BoolExpr>, Box<BoolExpr>),
    /// `->`, boolean implication.
    Implies(Box<BoolExpr>, Box<BoolExpr>),
    /// A comparison of two operands.
    Cmp {
        /// The operator.
        op: CmpOp,
        /// Left operand.
        lhs: Operand,
        /// Right operand.
        rhs: Operand,
    },
    /// `$rose(net)`: the net's least significant bit is 1 now and was not
    /// at the previous clocking event.
    Rose(NetRef),
    /// `$fell(net)`: the mirror of [`BoolExpr::Rose`].
    Fell(NetRef),
    /// `$stable(net)`: the net has the same value as at the previous
    /// clocking event.
    Stable(NetRef),
}

/// A number of cycles: `min` to `max`, with `None` for the unbounded `$`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    /// Lowest count, inclusive.
    pub min: u32,
    /// Highest count, inclusive; `None` is `$`.
    pub max: Option<u32>,
}

impl Range {
    /// The range matching exactly `n`.
    pub fn exact(n: u32) -> Range {
        Range {
            min: n,
            max: Some(n),
        }
    }

    /// True when `max` is below `min`.
    fn is_empty(self) -> bool {
        self.max.is_some_and(|m| m < self.min)
    }
}

/// A sequence: a description of a finite run of clock cycles.
#[derive(Clone, Debug, PartialEq)]
pub enum Sequence {
    /// One cycle in which a boolean holds.
    Bool(BoolExpr),
    /// `lhs ##range rhs`.
    Delay {
        /// The sequence that starts the match.
        lhs: Box<Sequence>,
        /// The cycles between the end of `lhs` and the start of `rhs`.
        range: Range,
        /// The sequence that ends the match.
        rhs: Box<Sequence>,
    },
    /// A leading `##range seq`.
    Lead {
        /// The cycles before `seq` starts.
        range: Range,
        /// The sequence.
        seq: Box<Sequence>,
    },
    /// `seq[*range]`, consecutive repetition.
    Repeat {
        /// The repeated sequence.
        seq: Box<Sequence>,
        /// How many times.
        range: Range,
    },
    /// `expr[->range]`, goto repetition.
    Goto {
        /// The boolean whose occurrences are counted.
        expr: BoolExpr,
        /// How many occurrences.
        range: Range,
    },
    /// `lhs and rhs`.
    And(Box<Sequence>, Box<Sequence>),
    /// `lhs intersect rhs`.
    Intersect(Box<Sequence>, Box<Sequence>),
    /// `lhs or rhs`.
    Or(Box<Sequence>, Box<Sequence>),
    /// `expr throughout seq`.
    Throughout {
        /// The boolean that must hold in every cycle.
        expr: BoolExpr,
        /// The sequence.
        seq: Box<Sequence>,
    },
    /// `lhs within rhs`.
    Within(Box<Sequence>, Box<Sequence>),
}

/// A property: what a directive checks.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyExpr {
    /// A sequence, which holds when it matches.
    Seq(Sequence),
    /// `not p`, and the PSL `never`.
    Not(Box<PropertyExpr>),
    /// `p1 and p2`.
    And(Box<PropertyExpr>, Box<PropertyExpr>),
    /// `p1 or p2`.
    Or(Box<PropertyExpr>, Box<PropertyExpr>),
    /// `seq |-> p` and `seq |=> p`.
    Implies {
        /// The antecedent sequence.
        ante: Sequence,
        /// True for `|->` (the consequent starts in the cycle the
        /// antecedent ends), false for `|=>` (it starts the cycle after).
        overlapped: bool,
        /// The consequent.
        conseq: Box<PropertyExpr>,
    },
}

/// Which directive a property carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectiveKind {
    /// The property must hold; a failure is an error.
    Assert,
    /// The property is taken as given; a failure is a warning.
    Assume,
    /// Count the attempts in which the property holds.
    Cover,
}

impl DirectiveKind {
    /// The keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            DirectiveKind::Assert => "assert",
            DirectiveKind::Assume => "assume",
            DirectiveKind::Cover => "cover",
        }
    }
}

/// The clocking event a property is evaluated on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockSpec {
    /// Which edge starts a cycle; [`Polarity::Any`] for `@(net)`.
    pub polarity: Polarity,
    /// The clock net.
    pub net: NetRef,
}

/// One `assert` / `assume` / `cover` directive over a property.
#[derive(Clone, Debug, PartialEq)]
pub struct Directive {
    /// The `label:` before the directive, used in reports.
    pub name: Option<String>,
    /// Which directive.
    pub kind: DirectiveKind,
    /// The clocking event; required by
    /// [`Simulator::add_assertion`](crate::sim::Simulator::add_assertion).
    pub clock: Option<ClockSpec>,
    /// `disable iff (expr)`: when the expression holds at a clocking
    /// event, every live attempt is abandoned and none is started.
    pub disable: Option<BoolExpr>,
    /// The property checked.
    pub property: PropertyExpr,
    /// The source text the directive came from.
    pub span: Span,
}

impl Directive {
    /// A directive with no clock, no reset and no name.
    pub fn new(kind: DirectiveKind, property: PropertyExpr, span: Span) -> Directive {
        Directive {
            name: None,
            kind,
            clock: None,
            disable: None,
            property,
            span,
        }
    }

    /// Sets the name used in reports.
    pub fn with_name(mut self, name: impl Into<String>) -> Directive {
        self.name = Some(name.into());
        self
    }

    /// Sets the clocking event.
    pub fn with_clock(mut self, polarity: Polarity, net: &str) -> Directive {
        self.clock = Some(ClockSpec {
            polarity,
            net: NetRef {
                name: net.to_owned(),
                select: None,
                span: self.span,
            },
        });
        self
    }

    /// Sets the `disable iff` expression.
    pub fn with_disable(mut self, expr: BoolExpr) -> Directive {
        self.disable = Some(expr);
        self
    }
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

/// One lexical token of the property language.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    /// An identifier or keyword, possibly a dotted path.
    Ident(String),
    /// A `$name` system function, without the `$`.
    System(String),
    /// A numeric literal, as written.
    Number(String),
    /// A bare `$`, the unbounded range marker.
    Dollar,
    /// Punctuation, from [`PUNCT`].
    Punct(&'static str),
}

/// Punctuation, longest first so the scan is greedy.
const PUNCT: &[&str] = &[
    "|->", "|=>", "===", "!==", "[->", "##", "||", "&&", "==", "!=", "<=", ">=", "->", "[*", "[=",
    "(", ")", "[", "]", "{", "}", ":", ";", ",", "!", "~", "|", "&", "<", ">", "@", "+", "-", "*",
    "=",
];

/// A token and where it came from, as byte offsets into the source text.
#[derive(Clone, Debug)]
struct Token {
    kind: Tok,
    start: usize,
    end: usize,
}

/// Splits `text` into tokens, or reports the first character it cannot
/// read.
fn lex(text: &str, base: Span) -> Result<Vec<Token>, Diagnostic> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        let start = i;
        if c.is_ascii_alphabetic() || c == '_' {
            while i < bytes.len() {
                let b = bytes[i] as char;
                if b.is_ascii_alphanumeric() || b == '_' || b == '.' {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Token {
                kind: Tok::Ident(text[start..i].to_owned()),
                start,
                end: i,
            });
            continue;
        }
        if c == '$' {
            i += 1;
            let name_start = i;
            while i < bytes.len() {
                let b = bytes[i] as char;
                if b.is_ascii_alphanumeric() || b == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let kind = if i == name_start {
                Tok::Dollar
            } else {
                Tok::System(text[name_start..i].to_owned())
            };
            out.push(Token {
                kind,
                start,
                end: i,
            });
            continue;
        }
        if c.is_ascii_digit() || c == '\'' {
            i = scan_number(bytes, i);
            out.push(Token {
                kind: Tok::Number(text[start..i].to_owned()),
                start,
                end: i,
            });
            continue;
        }
        match PUNCT.iter().find(|p| text[i..].starts_with(**p)) {
            Some(p) => {
                i += p.len();
                out.push(Token {
                    kind: Tok::Punct(p),
                    start,
                    end: i,
                });
            }
            None => {
                return Err(Diagnostic::error(format!(
                    "unexpected character `{c}` in a property expression"
                ))
                .with_span(sub_span(base, start, i + 1)));
            }
        }
    }
    Ok(out)
}

/// The end offset of the numeric literal starting at `i`.
fn scan_number(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && ((bytes[i] as char).is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'\'' {
        i += 1;
        if i < bytes.len() && matches!(bytes[i], b's' | b'S') {
            i += 1;
        }
        if i < bytes.len() && (bytes[i] as char).is_ascii_alphabetic() {
            i += 1;
        }
        while i < bytes.len() {
            let b = bytes[i] as char;
            if b.is_ascii_alphanumeric() || b == '_' || b == '?' {
                i += 1;
            } else {
                break;
            }
        }
    }
    i
}

/// A span inside `base` for the byte range `[start, end)` of the text.
///
/// The offsets are clamped to `base`, so a property whose text was
/// normalised (a Verilog `RawTokens`, whose spelling is the tokens joined
/// by single spaces) still reports inside the original source range rather
/// than past it.
fn sub_span(base: Span, start: usize, end: usize) -> Span {
    let at = |off: usize| -> u32 {
        u32::try_from(off)
            .ok()
            .and_then(|o| base.start.checked_add(o))
            .unwrap_or(base.end)
            .min(base.end)
    };
    let s = at(start);
    let e = at(end).max(s);
    Span::new(base.file, s, e)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parses a directive, or the body of one.
///
/// The text may be a whole directive (`assert property (@(posedge clk) a
/// |-> b);`) or just its body (`@(posedge clk) a |-> b`), which is what a
/// Verilog front end keeps for `assert property (...)`. A body alone gets
/// [`DirectiveKind::Assert`].
///
/// `span` is the source range `text` came from; sub-spans of the returned
/// nodes are offsets inside it, clamped to its end.
///
/// # Errors
///
/// Returns the first syntax error as a [`Diagnostic`] with a span.
pub fn parse_directive(text: &str, span: Span) -> Result<Directive, Diagnostic> {
    let toks = lex(text, span)?;
    let mut p = Parser {
        toks,
        pos: 0,
        base: span,
    };
    let d = p.directive()?;
    if p.pos < p.toks.len() {
        return Err(p.error("unexpected trailing text in a property expression"));
    }
    Ok(d)
}

/// Parses a bare property expression: no directive keyword, no clock and
/// no `disable iff`.
///
/// # Errors
///
/// Returns the first syntax error as a [`Diagnostic`] with a span.
pub fn parse_property(text: &str, span: Span) -> Result<PropertyExpr, Diagnostic> {
    let toks = lex(text, span)?;
    let mut p = Parser {
        toks,
        pos: 0,
        base: span,
    };
    let e = p.property()?;
    if p.pos < p.toks.len() {
        return Err(p.error("unexpected trailing text in a property expression"));
    }
    Ok(e)
}

/// Parses the property text a Verilog `assert property (...)` kept
/// verbatim.
///
/// The front end stores property expressions as
/// [`RawTokens`](crate::verilog::ast::RawTokens) rather than parsing them,
/// so this is the bridge from the Verilog AST into the simulator's
/// property language; the front end itself needs no change.
///
/// # Errors
///
/// Returns the first syntax error as a [`Diagnostic`] with a span.
#[cfg(feature = "verilog")]
pub fn parse_raw_tokens(
    kind: DirectiveKind,
    raw: &crate::verilog::ast::RawTokens,
) -> Result<Directive, Diagnostic> {
    let mut d = parse_directive(&raw.text, raw.span)?;
    d.kind = kind;
    Ok(d)
}

/// The directive for a parsed Verilog assertion, or `None` when it is not
/// a concurrent `assert` / `assume` / `cover property` (an immediate
/// assertion already lowers to
/// [`StmtKind::Assert`](crate::ir::StmtKind::Assert), and `restrict` has
/// no simulation meaning).
///
/// # Errors
///
/// Returns the first syntax error as a [`Diagnostic`] with a span.
#[cfg(feature = "verilog")]
pub fn from_assertion(a: &crate::verilog::ast::Assertion) -> Option<Result<Directive, Diagnostic>> {
    use crate::verilog::ast::{AssertKind, AssertSpec};
    let kind = match a.kind {
        AssertKind::Assert => DirectiveKind::Assert,
        AssertKind::Assume => DirectiveKind::Assume,
        AssertKind::Cover => DirectiveKind::Cover,
        AssertKind::Restrict => return None,
    };
    let AssertSpec::Property(raw) = &a.spec else {
        return None;
    };
    let mut parsed = parse_raw_tokens(kind, raw);
    if let (Ok(d), Some(label)) = (&mut parsed, &a.label) {
        d.name = Some(label.name.clone());
    }
    Some(parsed)
}

/// The recursive-descent parser over the token list.
struct Parser {
    toks: Vec<Token>,
    pos: usize,
    base: Span,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.kind)
    }

    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.pos + n).map(|t| &t.kind)
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    /// The span of the token under the cursor, or the end of the text.
    fn span(&self) -> Span {
        match self.toks.get(self.pos) {
            Some(t) => sub_span(self.base, t.start, t.end),
            None => self.base,
        }
    }

    fn error(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::error(message).with_span(self.span())
    }

    fn at_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(q)) if *q == p)
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.at_punct(p) {
            self.bump();
            return true;
        }
        false
    }

    fn expect_punct(&mut self, p: &str) -> Result<(), Diagnostic> {
        if self.eat_punct(p) {
            return Ok(());
        }
        Err(self.error(format!("expected `{p}`")))
    }

    fn at_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(n)) if n == kw)
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.bump();
            return true;
        }
        false
    }

    /// A plain integer under the cursor.
    fn count(&mut self) -> Result<u32, Diagnostic> {
        let Some(Tok::Number(text)) = self.peek() else {
            return Err(self.error("expected a cycle count"));
        };
        let text = text.clone();
        let value = text
            .parse::<u32>()
            .map_err(|_| self.error(format!("`{text}` is not a plain cycle count")))?;
        self.bump();
        Ok(value)
    }

    /// An upper bound: an integer or `$`.
    fn bound(&mut self) -> Result<Option<u32>, Diagnostic> {
        if matches!(self.peek(), Some(Tok::Dollar)) {
            self.bump();
            return Ok(None);
        }
        Ok(Some(self.count()?))
    }

    // ---- directives ----

    fn directive(&mut self) -> Result<Directive, Diagnostic> {
        let span = self.base;
        let mut name = None;
        if let (Some(Tok::Ident(n)), Some(Tok::Punct(":"))) = (self.peek(), self.peek_at(1))
            && !is_keyword(n)
        {
            name = Some(n.clone());
            self.bump();
            self.bump();
        }
        let mut kind = DirectiveKind::Assert;
        let mut explicit = false;
        for k in [
            DirectiveKind::Assert,
            DirectiveKind::Assume,
            DirectiveKind::Cover,
        ] {
            if self.at_kw(k.as_str()) {
                kind = k;
                explicit = true;
                self.bump();
                break;
            }
        }
        if self.at_kw("restrict") {
            return Err(self.error("`restrict` has no simulation meaning"));
        }
        let mut closed = false;
        if explicit {
            let _ = self.eat_kw("property") || self.eat_kw("sequence");
            closed = self.eat_punct("(");
        }
        let (clock, disable, property) = self.body()?;
        if closed {
            self.expect_punct(")")?;
        }
        let _ = self.eat_punct(";");
        Ok(Directive {
            name,
            kind,
            clock,
            disable,
            property,
            span,
        })
    }

    #[allow(clippy::type_complexity)]
    fn body(&mut self) -> Result<(Option<ClockSpec>, Option<BoolExpr>, PropertyExpr), Diagnostic> {
        let mut clock = None;
        if self.at_punct("@") {
            clock = Some(self.clock()?);
        }
        let mut disable = None;
        if self.eat_kw("disable") {
            if !self.eat_kw("iff") {
                return Err(self.error("expected `iff` after `disable`"));
            }
            let parens = self.eat_punct("(");
            disable = Some(self.bool_expr()?);
            if parens {
                self.expect_punct(")")?;
            }
        }
        let property = self.property()?;
        if clock.is_none() && self.at_punct("@") {
            clock = Some(self.clock()?);
        }
        Ok((clock, disable, property))
    }

    fn clock(&mut self) -> Result<ClockSpec, Diagnostic> {
        self.expect_punct("@")?;
        let parens = self.eat_punct("(");
        let polarity = if self.eat_kw("posedge") {
            Polarity::Pos
        } else if self.eat_kw("negedge") {
            Polarity::Neg
        } else {
            let _ = self.eat_kw("edge");
            Polarity::Any
        };
        let net = self.net_ref()?;
        if parens {
            self.expect_punct(")")?;
        }
        Ok(ClockSpec { polarity, net })
    }

    // ---- properties ----

    fn property(&mut self) -> Result<PropertyExpr, Diagnostic> {
        let start = self.span();
        let lhs = self.prop_or()?;
        let overlapped = if self.at_punct("|->") {
            true
        } else if self.at_punct("|=>") {
            false
        } else {
            return Ok(lhs);
        };
        self.bump();
        let conseq = self.property()?;
        let ante = as_sequence(lhs, start)?;
        Ok(PropertyExpr::Implies {
            ante,
            overlapped,
            conseq: Box::new(conseq),
        })
    }

    fn prop_or(&mut self) -> Result<PropertyExpr, Diagnostic> {
        let mut lhs = self.prop_and()?;
        while self.eat_kw("or") {
            let rhs = self.prop_and()?;
            lhs = match (lhs, rhs) {
                (PropertyExpr::Seq(a), PropertyExpr::Seq(b)) => {
                    PropertyExpr::Seq(Sequence::Or(Box::new(a), Box::new(b)))
                }
                (a, b) => PropertyExpr::Or(Box::new(a), Box::new(b)),
            };
        }
        Ok(lhs)
    }

    fn prop_and(&mut self) -> Result<PropertyExpr, Diagnostic> {
        let mut lhs = self.prop_unary()?;
        while self.at_kw("and") {
            self.bump();
            let rhs = self.prop_unary()?;
            lhs = match (lhs, rhs) {
                (PropertyExpr::Seq(a), PropertyExpr::Seq(b)) => {
                    PropertyExpr::Seq(Sequence::And(Box::new(a), Box::new(b)))
                }
                (a, b) => PropertyExpr::And(Box::new(a), Box::new(b)),
            };
        }
        Ok(lhs)
    }

    fn prop_unary(&mut self) -> Result<PropertyExpr, Diagnostic> {
        if self.eat_kw("not") || self.eat_kw("never") {
            let inner = self.prop_unary()?;
            return Ok(PropertyExpr::Not(Box::new(inner)));
        }
        if self.eat_kw("always") {
            // Every clock tick starts an attempt already, so `always p` is
            // `p`; see the module docs.
            return self.prop_unary();
        }
        for unsupported in [
            "s_always",
            "s_eventually",
            "eventually",
            "until",
            "nexttime",
        ] {
            if self.at_kw(unsupported) {
                return Err(self.error(format!(
                    "`{unsupported}` is not one of the supported property operators"
                )));
            }
        }
        self.seq_intersect()
    }

    // ---- sequences ----

    fn seq_intersect(&mut self) -> Result<PropertyExpr, Diagnostic> {
        let start = self.span();
        let mut lhs = self.seq_within()?;
        while self.eat_kw("intersect") {
            let at = self.span();
            let rhs = self.seq_within()?;
            lhs = PropertyExpr::Seq(Sequence::Intersect(
                Box::new(as_sequence(lhs, start)?),
                Box::new(as_sequence(rhs, at)?),
            ));
        }
        Ok(lhs)
    }

    fn seq_within(&mut self) -> Result<PropertyExpr, Diagnostic> {
        let start = self.span();
        let mut lhs = self.seq_throughout()?;
        while self.eat_kw("within") {
            let at = self.span();
            let rhs = self.seq_throughout()?;
            lhs = PropertyExpr::Seq(Sequence::Within(
                Box::new(as_sequence(lhs, start)?),
                Box::new(as_sequence(rhs, at)?),
            ));
        }
        Ok(lhs)
    }

    fn seq_throughout(&mut self) -> Result<PropertyExpr, Diagnostic> {
        let start = self.span();
        let lhs = self.seq_concat()?;
        if !self.eat_kw("throughout") {
            return Ok(lhs);
        }
        let at = self.span();
        let rhs = self.seq_throughout()?;
        let expr = as_bool(as_sequence(lhs, start)?, start)?;
        Ok(PropertyExpr::Seq(Sequence::Throughout {
            expr,
            seq: Box::new(as_sequence(rhs, at)?),
        }))
    }

    fn seq_concat(&mut self) -> Result<PropertyExpr, Diagnostic> {
        let start = self.span();
        let mut lhs = self.seq_repeat()?;
        while self.at_punct("##") {
            let range = self.delay_range()?;
            let at = self.span();
            let rhs = self.seq_repeat()?;
            lhs = PropertyExpr::Seq(Sequence::Delay {
                lhs: Box::new(as_sequence(lhs, start)?),
                range,
                rhs: Box::new(as_sequence(rhs, at)?),
            });
        }
        Ok(lhs)
    }

    fn seq_repeat(&mut self) -> Result<PropertyExpr, Diagnostic> {
        let start = self.span();
        let mut lhs = self.seq_primary()?;
        loop {
            if self.at_punct("[=") {
                return Err(self.error(
                    "non-consecutive repetition `[=n]` is not supported; use `[->n]` or a \
                     sequence",
                ));
            }
            if self.at_punct("[*") {
                self.bump();
                let range = self.count_range()?;
                self.expect_punct("]")?;
                if range.min == 0 {
                    return Err(Diagnostic::error(
                        "a repetition of zero would match no cycles, which is not supported",
                    )
                    .with_span(start));
                }
                lhs = PropertyExpr::Seq(Sequence::Repeat {
                    seq: Box::new(as_sequence(lhs, start)?),
                    range,
                });
                continue;
            }
            if self.at_punct("[->") {
                self.bump();
                let range = self.count_range()?;
                self.expect_punct("]")?;
                if range.min == 0 {
                    return Err(
                        Diagnostic::error("goto repetition needs at least one occurrence")
                            .with_span(start),
                    );
                }
                let expr = as_bool(as_sequence(lhs, start)?, start)?;
                lhs = PropertyExpr::Seq(Sequence::Goto { expr, range });
                continue;
            }
            return Ok(lhs);
        }
    }

    fn seq_primary(&mut self) -> Result<PropertyExpr, Diagnostic> {
        if self.at_punct("##") {
            let range = self.delay_range()?;
            let at = self.span();
            let seq = self.seq_repeat()?;
            return Ok(PropertyExpr::Seq(Sequence::Lead {
                range,
                seq: Box::new(as_sequence(seq, at)?),
            }));
        }
        if self.eat_punct("{") {
            let seq = self.sere()?;
            self.expect_punct("}")?;
            return Ok(PropertyExpr::Seq(seq));
        }
        if self.at_punct("(") && self.paren_holds_property() {
            self.bump();
            let inner = self.property()?;
            self.expect_punct(")")?;
            return Ok(inner);
        }
        Ok(PropertyExpr::Seq(Sequence::Bool(self.bool_expr()?)))
    }

    /// Whether the parenthesised group under the cursor holds sequence or
    /// property syntax rather than a plain boolean, which decides whether
    /// [`Parser::seq_primary`] recurses or hands it to the boolean parser.
    fn paren_holds_property(&self) -> bool {
        let mut depth = 0usize;
        let mut i = self.pos;
        while let Some(t) = self.toks.get(i) {
            match &t.kind {
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                Tok::Punct("##" | "|->" | "|=>" | "[*" | "[->" | "{") => return true,
                Tok::Ident(n)
                    if matches!(
                        n.as_str(),
                        "and"
                            | "or"
                            | "not"
                            | "throughout"
                            | "within"
                            | "intersect"
                            | "always"
                            | "never"
                    ) =>
                {
                    return true;
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// The PSL SERE body of `{ ... }`: `;` is `##1` and `:` is `##0`.
    fn sere(&mut self) -> Result<Sequence, Diagnostic> {
        let start = self.span();
        let mut lhs = as_sequence(self.property()?, start)?;
        loop {
            let range = if self.eat_punct(";") {
                Range::exact(1)
            } else if self.eat_punct(":") {
                Range::exact(0)
            } else {
                return Ok(lhs);
            };
            let at = self.span();
            let rhs = as_sequence(self.property()?, at)?;
            lhs = Sequence::Delay {
                lhs: Box::new(lhs),
                range,
                rhs: Box::new(rhs),
            };
        }
    }

    /// `## n`, `## [m:n]`, `## [*]`, `## [+]`.
    fn delay_range(&mut self) -> Result<Range, Diagnostic> {
        let at = self.span();
        self.expect_punct("##")?;
        let range = if self.eat_punct("[*") {
            self.expect_punct("]")?;
            Range { min: 0, max: None }
        } else if self.eat_punct("[") {
            if self.eat_punct("+") {
                self.expect_punct("]")?;
                Range { min: 1, max: None }
            } else if self.eat_punct("*") {
                self.expect_punct("]")?;
                Range { min: 0, max: None }
            } else {
                let min = self.count()?;
                self.expect_punct(":")?;
                let max = self.bound()?;
                self.expect_punct("]")?;
                Range { min, max }
            }
        } else {
            Range::exact(self.count()?)
        };
        if range.is_empty() {
            return Err(Diagnostic::error("the cycle range is empty").with_span(at));
        }
        Ok(range)
    }

    /// `n` or `m:n` inside a repetition bracket.
    fn count_range(&mut self) -> Result<Range, Diagnostic> {
        let at = self.span();
        let min = self.count()?;
        let max = if self.eat_punct(":") {
            self.bound()?
        } else {
            Some(min)
        };
        let range = Range { min, max };
        if range.is_empty() {
            return Err(Diagnostic::error("the repetition range is empty").with_span(at));
        }
        Ok(range)
    }

    // ---- booleans ----

    fn bool_expr(&mut self) -> Result<BoolExpr, Diagnostic> {
        let lhs = self.bool_or()?;
        if self.eat_punct("->") {
            let rhs = self.bool_expr()?;
            return Ok(BoolExpr::Implies(Box::new(lhs), Box::new(rhs)));
        }
        Ok(lhs)
    }

    fn bool_or(&mut self) -> Result<BoolExpr, Diagnostic> {
        let mut lhs = self.bool_and()?;
        while self.eat_punct("||") {
            let rhs = self.bool_and()?;
            lhs = BoolExpr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn bool_and(&mut self) -> Result<BoolExpr, Diagnostic> {
        let mut lhs = self.bool_unary()?;
        while self.eat_punct("&&") {
            let rhs = self.bool_unary()?;
            lhs = BoolExpr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn bool_unary(&mut self) -> Result<BoolExpr, Diagnostic> {
        if self.eat_punct("!") || self.eat_punct("~") {
            let inner = self.bool_unary()?;
            return Ok(BoolExpr::Not(Box::new(inner)));
        }
        self.bool_primary()
    }

    fn bool_primary(&mut self) -> Result<BoolExpr, Diagnostic> {
        if self.eat_punct("(") {
            let inner = self.bool_expr()?;
            self.expect_punct(")")?;
            return Ok(inner);
        }
        if let Some(Tok::System(name)) = self.peek() {
            let name = name.clone();
            self.bump();
            let parens = self.eat_punct("(");
            let net = self.net_ref()?;
            if parens {
                self.expect_punct(")")?;
            }
            return match name.as_str() {
                "rose" => Ok(BoolExpr::Rose(net)),
                "fell" => Ok(BoolExpr::Fell(net)),
                "stable" => Ok(BoolExpr::Stable(net)),
                other => Err(self.error(format!(
                    "`${other}` is not one of the supported sampled value functions \
                     (`$rose`, `$fell`, `$stable`)"
                ))),
            };
        }
        let lhs = self.operand()?;
        let op = match self.peek() {
            Some(Tok::Punct("==")) => Some(CmpOp::Eq),
            Some(Tok::Punct("!=")) => Some(CmpOp::Ne),
            Some(Tok::Punct("===")) => Some(CmpOp::CaseEq),
            Some(Tok::Punct("!==")) => Some(CmpOp::CaseNe),
            Some(Tok::Punct("<")) => Some(CmpOp::Lt),
            Some(Tok::Punct("<=")) => Some(CmpOp::Le),
            Some(Tok::Punct(">")) => Some(CmpOp::Gt),
            Some(Tok::Punct(">=")) => Some(CmpOp::Ge),
            _ => None,
        };
        let Some(op) = op else {
            return Ok(BoolExpr::Value(lhs));
        };
        self.bump();
        let rhs = self.operand()?;
        Ok(BoolExpr::Cmp { op, lhs, rhs })
    }

    fn operand(&mut self) -> Result<Operand, Diagnostic> {
        if let Some(Tok::Number(text)) = self.peek() {
            let text = text.clone();
            let value = Logic::parse_verilog(&text)
                .map_err(|e| self.error(format!("bad literal `{text}`: {e}")))?;
            self.bump();
            return Ok(Operand::Const(value));
        }
        Ok(Operand::Net(self.net_ref()?))
    }

    fn net_ref(&mut self) -> Result<NetRef, Diagnostic> {
        let Some(Tok::Ident(name)) = self.peek() else {
            return Err(self.error("expected a net name"));
        };
        if is_keyword(name) {
            return Err(self.error(format!("expected a net name, found the keyword `{name}`")));
        }
        let name = name.clone();
        let span = self.span();
        self.bump();
        let mut select = None;
        if self.at_punct("[") {
            self.bump();
            let hi = self.count()?;
            select = Some(if self.eat_punct(":") {
                let lo = self.count()?;
                if lo > hi {
                    return Err(self.error("a part select runs from the high bit to the low one"));
                }
                Select::Part { hi, lo }
            } else {
                Select::Bit(hi)
            });
            self.expect_punct("]")?;
        }
        let end = self
            .toks
            .get(self.pos.saturating_sub(1))
            .map_or(span, |t| sub_span(self.base, t.start, t.end));
        Ok(NetRef {
            name,
            select,
            span: span.to(end),
        })
    }
}

/// Every word the grammar treats as a keyword rather than a net name.
fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "assert"
            | "assume"
            | "cover"
            | "restrict"
            | "property"
            | "sequence"
            | "disable"
            | "iff"
            | "posedge"
            | "negedge"
            | "edge"
            | "and"
            | "or"
            | "not"
            | "throughout"
            | "within"
            | "intersect"
            | "always"
            | "never"
    )
}

/// The sequence a property expression is, or an error when it uses an
/// operator that only properties have.
fn as_sequence(p: PropertyExpr, span: Span) -> Result<Sequence, Diagnostic> {
    match p {
        PropertyExpr::Seq(s) => Ok(s),
        _ => Err(
            Diagnostic::error("a property is not allowed here; a sequence is")
                .with_span(span)
                .with_note("`not`, `|->` and `|=>` make a property, which cannot be delayed, repeated or intersected"),
        ),
    }
}

/// The boolean a sequence is, or an error: `throughout` and `[->n]` take a
/// boolean on the left.
fn as_bool(s: Sequence, span: Span) -> Result<BoolExpr, Diagnostic> {
    match s {
        Sequence::Bool(b) => Ok(b),
        _ => {
            Err(Diagnostic::error("expected a boolean expression, not a sequence").with_span(span))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("property", "").unwrap();
        Span::new(id, 0, 200)
    }

    fn parse(text: &str) -> Directive {
        parse_directive(text, span()).unwrap_or_else(|e| panic!("{text}: {}", e.message))
    }

    fn fails(text: &str) -> String {
        parse_directive(text, span())
            .err()
            .unwrap_or_else(|| panic!("`{text}` parsed"))
            .message
    }

    /// Renders an operand without its span.
    fn shape_operand(o: &Operand) -> String {
        match o {
            Operand::Const(l) => l.to_string(),
            Operand::Net(r) => match r.select {
                None => r.name.clone(),
                Some(Select::Bit(i)) => format!("{}[{i}]", r.name),
                Some(Select::Part { hi, lo }) => format!("{}[{hi}:{lo}]", r.name),
            },
        }
    }

    /// Renders a boolean without its spans.
    fn shape_bool(b: &BoolExpr) -> String {
        match b {
            BoolExpr::Const(v) => v.to_string(),
            BoolExpr::Value(o) => shape_operand(o),
            BoolExpr::Not(i) => format!("!{}", shape_bool(i)),
            BoolExpr::And(a, b) => format!("({} && {})", shape_bool(a), shape_bool(b)),
            BoolExpr::Or(a, b) => format!("({} || {})", shape_bool(a), shape_bool(b)),
            BoolExpr::Implies(a, b) => format!("({} -> {})", shape_bool(a), shape_bool(b)),
            BoolExpr::Cmp { op, lhs, rhs } => format!(
                "({} {} {})",
                shape_operand(lhs),
                op.as_str(),
                shape_operand(rhs)
            ),
            BoolExpr::Rose(r) => format!("$rose({})", r.name),
            BoolExpr::Fell(r) => format!("$fell({})", r.name),
            BoolExpr::Stable(r) => format!("$stable({})", r.name),
        }
    }

    fn shape_range(r: Range) -> String {
        match r.max {
            Some(m) if m == r.min => r.min.to_string(),
            Some(m) => format!("{}:{m}", r.min),
            None => format!("{}:$", r.min),
        }
    }

    /// Renders a sequence without its spans.
    fn shape_seq(s: &Sequence) -> String {
        match s {
            Sequence::Bool(b) => shape_bool(b),
            Sequence::Delay { lhs, range, rhs } => format!(
                "({} ##[{}] {})",
                shape_seq(lhs),
                shape_range(*range),
                shape_seq(rhs)
            ),
            Sequence::Lead { range, seq } => {
                format!("(##[{}] {})", shape_range(*range), shape_seq(seq))
            }
            Sequence::Repeat { seq, range } => {
                format!("({}[*{}])", shape_seq(seq), shape_range(*range))
            }
            Sequence::Goto { expr, range } => {
                format!("({}[->{}])", shape_bool(expr), shape_range(*range))
            }
            Sequence::And(a, b) => format!("({} and {})", shape_seq(a), shape_seq(b)),
            Sequence::Intersect(a, b) => {
                format!("({} intersect {})", shape_seq(a), shape_seq(b))
            }
            Sequence::Or(a, b) => format!("({} or {})", shape_seq(a), shape_seq(b)),
            Sequence::Throughout { expr, seq } => {
                format!("({} throughout {})", shape_bool(expr), shape_seq(seq))
            }
            Sequence::Within(a, b) => format!("({} within {})", shape_seq(a), shape_seq(b)),
        }
    }

    /// Renders a property without its spans, so two parses compare.
    fn shape(p: &PropertyExpr) -> String {
        match p {
            PropertyExpr::Seq(s) => shape_seq(s),
            PropertyExpr::Not(i) => format!("(not {})", shape(i)),
            PropertyExpr::And(a, b) => format!("({} AND {})", shape(a), shape(b)),
            PropertyExpr::Or(a, b) => format!("({} OR {})", shape(a), shape(b)),
            PropertyExpr::Implies {
                ante,
                overlapped,
                conseq,
            } => format!(
                "({} {} {})",
                shape_seq(ante),
                if *overlapped { "|->" } else { "|=>" },
                shape(conseq)
            ),
        }
    }

    /// True when two texts parse to the same tree, spans aside.
    fn same_shape(a: &str, b: &str) -> bool {
        shape(&parse(a).property) == shape(&parse(b).property)
    }

    #[test]
    fn directive_shapes() {
        let d = parse("assert property (@(posedge clk) full |-> !push);");
        assert_eq!(d.kind, DirectiveKind::Assert);
        let clock = d.clock.expect("clock");
        assert_eq!(clock.polarity, Polarity::Pos);
        assert_eq!(clock.net.name, "clk");
        assert!(matches!(d.property, PropertyExpr::Implies { .. }));
        let d = parse("cover property (@(negedge clk) a ##1 b);");
        assert_eq!(d.kind, DirectiveKind::Cover);
        assert_eq!(d.clock.expect("clock").polarity, Polarity::Neg);
        let d = parse("name: assume property (@(clk) disable iff (rst) a |=> b);");
        assert_eq!(d.name.as_deref(), Some("name"));
        assert_eq!(d.kind, DirectiveKind::Assume);
        assert_eq!(d.clock.expect("clock").polarity, Polarity::Any);
        assert!(d.disable.is_some());
        // A bare body, as a Verilog `RawTokens` holds it.
        let d = parse("@(posedge clk) a |-> b");
        assert_eq!(d.kind, DirectiveKind::Assert);
        assert!(d.clock.is_some());
        // A trailing PSL-style clock.
        let d = parse("assert always (a -> b) @(posedge clk);");
        assert!(d.clock.is_some());
    }

    #[test]
    fn implication_kinds() {
        let d = parse("a |-> b");
        let PropertyExpr::Implies {
            ante, overlapped, ..
        } = &d.property
        else {
            panic!("not an implication");
        };
        assert!(*overlapped);
        assert_eq!(shape_seq(ante), "a");
        let d = parse("a |=> b");
        let PropertyExpr::Implies { overlapped, .. } = &d.property else {
            panic!("not an implication");
        };
        assert!(!*overlapped);
    }

    #[test]
    fn delays_and_repetitions() {
        let d = parse("a ##2 b");
        let PropertyExpr::Seq(Sequence::Delay { range, .. }) = &d.property else {
            panic!("not a delay");
        };
        assert_eq!(*range, Range::exact(2));
        let d = parse("a ##[1:3] b");
        let PropertyExpr::Seq(Sequence::Delay { range, .. }) = &d.property else {
            panic!("not a delay");
        };
        assert_eq!(
            *range,
            Range {
                min: 1,
                max: Some(3)
            }
        );
        let d = parse("a ##[2:$] b");
        let PropertyExpr::Seq(Sequence::Delay { range, .. }) = &d.property else {
            panic!("not a delay");
        };
        assert_eq!(range.max, None);
        let d = parse("##3 b");
        assert!(matches!(
            d.property,
            PropertyExpr::Seq(Sequence::Lead { .. })
        ));
        let d = parse("a[*3]");
        let PropertyExpr::Seq(Sequence::Repeat { range, .. }) = &d.property else {
            panic!("not a repetition");
        };
        assert_eq!(*range, Range::exact(3));
        let d = parse("a[*2:$]");
        let PropertyExpr::Seq(Sequence::Repeat { range, .. }) = &d.property else {
            panic!("not a repetition");
        };
        assert_eq!(range.max, None);
        let d = parse("b[->2]");
        assert!(matches!(
            d.property,
            PropertyExpr::Seq(Sequence::Goto { .. })
        ));
        // `##[*]` and `##[+]`
        let d = parse("a ##[*] b");
        let PropertyExpr::Seq(Sequence::Delay { range, .. }) = &d.property else {
            panic!("not a delay");
        };
        assert_eq!(*range, Range { min: 0, max: None });
        let d = parse("a ##[+] b");
        let PropertyExpr::Seq(Sequence::Delay { range, .. }) = &d.property else {
            panic!("not a delay");
        };
        assert_eq!(*range, Range { min: 1, max: None });
    }

    #[test]
    fn sequence_operators() {
        assert!(matches!(
            parse("a and b").property,
            PropertyExpr::Seq(Sequence::And(..))
        ));
        assert!(matches!(
            parse("a or b").property,
            PropertyExpr::Seq(Sequence::Or(..))
        ));
        assert!(matches!(
            parse("a intersect b").property,
            PropertyExpr::Seq(Sequence::Intersect(..))
        ));
        assert!(matches!(
            parse("a within (b ##1 c)").property,
            PropertyExpr::Seq(Sequence::Within(..))
        ));
        assert!(matches!(
            parse("a throughout (b ##1 c)").property,
            PropertyExpr::Seq(Sequence::Throughout { .. })
        ));
        assert!(matches!(parse("not a").property, PropertyExpr::Not(_)));
        assert!(matches!(parse("never a").property, PropertyExpr::Not(_)));
        // A property-level `and` when one side is a property.
        assert!(matches!(
            parse("(a |-> b) and c").property,
            PropertyExpr::And(..)
        ));
    }

    #[test]
    fn precedence() {
        // `throughout` takes the whole concatenation on its right.
        assert!(same_shape("a throughout b ##1 c", "a throughout (b ##1 c)"));
        // Repetition binds tighter than concatenation.
        assert!(same_shape("a ##1 b[*2]", "a ##1 (b[*2])"));
        // `and` binds tighter than `or`.
        assert!(same_shape("a or b and c", "a or (b and c)"));
        // Implication is the loosest and right-associative.
        assert!(same_shape("a |-> b |-> c", "a |-> (b |-> c)"));
        assert!(same_shape("a or b |-> c", "(a or b) |-> c"));
        // `always` is the identity under a per-cycle attempt model.
        assert!(same_shape("always a |-> b", "a |-> b"));
    }

    #[test]
    fn sere_braces() {
        // `;` is `##1`, `:` is `##0`.
        assert!(same_shape("{a; b}", "a ##1 b"));
        assert!(same_shape("{a : b}", "a ##0 b"));
        assert!(same_shape("{a; b; c}", "(a ##1 b) ##1 c"));
    }

    #[test]
    fn booleans() {
        let d = parse("q[3] == 1'b1 && !full");
        let PropertyExpr::Seq(Sequence::Bool(BoolExpr::And(lhs, rhs))) = &d.property else {
            panic!("not a conjunction: {:?}", d.property);
        };
        let BoolExpr::Cmp { op, lhs, .. } = lhs.as_ref() else {
            panic!("not a comparison");
        };
        assert_eq!(*op, CmpOp::Eq);
        assert!(matches!(
            lhs,
            Operand::Net(NetRef {
                select: Some(Select::Bit(3)),
                ..
            })
        ));
        assert!(matches!(rhs.as_ref(), BoolExpr::Not(_)));
        let d = parse("data[7:4] != 4'h0");
        let PropertyExpr::Seq(Sequence::Bool(BoolExpr::Cmp { lhs, .. })) = &d.property else {
            panic!("not a comparison");
        };
        assert!(matches!(
            lhs,
            Operand::Net(NetRef {
                select: Some(Select::Part { hi: 7, lo: 4 }),
                ..
            })
        ));
        assert!(matches!(
            parse("$rose(req)").property,
            PropertyExpr::Seq(Sequence::Bool(BoolExpr::Rose(_)))
        ));
        assert!(matches!(
            parse("$fell(req)").property,
            PropertyExpr::Seq(Sequence::Bool(BoolExpr::Fell(_)))
        ));
        assert!(matches!(
            parse("$stable(req)").property,
            PropertyExpr::Seq(Sequence::Bool(BoolExpr::Stable(_)))
        ));
        assert!(matches!(
            parse("a -> b").property,
            PropertyExpr::Seq(Sequence::Bool(BoolExpr::Implies(..)))
        ));
        // Hierarchical names keep their dots.
        let d = parse("top.fifo.full");
        let PropertyExpr::Seq(Sequence::Bool(BoolExpr::Value(Operand::Net(r)))) = &d.property
        else {
            panic!("not a net");
        };
        assert_eq!(r.name, "top.fifo.full");
    }

    #[test]
    fn errors_are_diagnostics() {
        assert!(fails("a ##").contains("cycle count"));
        assert!(fails("a[*0]").contains("zero"));
        assert!(fails("a[=2]").contains("not supported"));
        assert!(fails("a ##[3:1] b").contains("empty"));
        assert!(fails("(a |-> b) ##1 c").contains("sequence"));
        assert!(fails("(a ##1 b) throughout c").contains("boolean"));
        assert!(fails("a |-> b eventually c").contains("trailing"));
        assert!(fails("s_eventually a").contains("supported"));
        assert!(fails("$past(a)").contains("sampled value"));
        assert!(fails("a #").contains("unexpected character"));
        assert!(fails("restrict property (a)").contains("restrict"));
        assert!(fails("a[3:5]").contains("part select"));
        assert!(fails("disable a").contains("iff"));
        assert!(fails("and").contains("keyword"));
    }

    #[test]
    fn builders() {
        let d = Directive::new(
            DirectiveKind::Cover,
            PropertyExpr::Seq(Sequence::Bool(BoolExpr::Const(true))),
            span(),
        )
        .with_name("c1")
        .with_clock(Polarity::Pos, "clk")
        .with_disable(BoolExpr::Const(false));
        assert_eq!(d.name.as_deref(), Some("c1"));
        assert_eq!(d.kind.as_str(), "cover");
        assert_eq!(d.clock.expect("clock").net.name, "clk");
        assert_eq!(d.disable, Some(BoolExpr::Const(false)));
        assert_eq!(CmpOp::CaseNe.as_str(), "!==");
        assert_eq!(DirectiveKind::Assume.as_str(), "assume");
        assert_eq!(
            parse_property("a ##1 b", span()).map(|p| matches!(p, PropertyExpr::Seq(_))),
            Ok(true)
        );
    }
}
