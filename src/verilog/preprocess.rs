//! The Verilog compiler-directive preprocessor.
//!
//! Verilog's `` ` `` directives work on text, before tokens exist: a macro
//! body may be half a token (`` `define SIZE 8 `` followed by `` `SIZE'hff ``)
//! and `` `ifdef `` can cut a statement in two. The preprocessor therefore
//! runs on the raw text and produces one expanded [`String`] for the lexer.
//!
//! # Spans through expansions
//!
//! Losing positions across expansion would make every later diagnostic
//! useless, so the output is built from *pieces*: each run of output text is
//! recorded in a [`SpanMap`] with its origin. A verbatim piece copies text
//! from some file and maps offsets one-to-one; an expansion piece holds
//! text produced by a macro and maps every offset to the macro *use site*
//! in the original file (the way rustc reports macro spans), remembering
//! the definition site as a secondary "defined here" span. The lexer runs
//! over the concatenated text and calls [`SpanMap::map`] on each token's
//! byte range, which is all it needs to report original locations even for
//! tokens that straddle an expansion boundary.
//!
//! # Directives
//!
//! - `` `define `` (object-like and function-like, defaults for arguments,
//!   `\`-newline continuation, `` `" ``, `` `\`" `` and ``` `` ``` inside
//!   the body), `` `undef ``, `` `undefineall ``.
//! - `` `ifdef `` / `` `ifndef `` / `` `elsif `` / `` `else `` / `` `endif ``,
//!   nested to any depth; inactive branches produce no output.
//! - `` `include `` through the caller's [`IncludeResolver`]; the library
//!   never touches the filesystem. Included text is added to the
//!   [`SourceMap`] as a new file, so its spans stay precise.
//! - `` `__FILE__ `` and `` `__LINE__ `` expand to the use site's file name
//!   (as a string literal) and line number.
//! - `` `timescale `` is recorded in [`Preprocessed::timescales`] and, like
//!   `` `default_nettype ``, `` `resetall ``, `` `begin_keywords `` and
//!   `` `end_keywords ``, passed through to the lexer as a directive token
//!   because its effect belongs to the parser.
//! - `` `celldefine ``, `` `endcelldefine ``, `` `unconnected_drive ``,
//!   `` `nounconnected_drive ``, `` `default_decay_time ``,
//!   `` `default_trireg_strength ``, `` `delay_mode_* ``, `` `pragma ``,
//!   `` `protect `` family and `` `line `` are accepted and dropped.
//!   `` `line `` has nothing to add: the span map already tracks positions.
//!
//! Using an undefined macro is an error and expands to nothing. Recursive
//! expansion is detected by name and reported once per use.
//!
//! Comments and string literals are recognised so directives inside them
//! are left alone; comments are passed through verbatim for the lexer to
//! record.

use std::collections::BTreeMap;

use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{SourceId, SourceMap, Span};

/// Maximum nesting of `` `include `` before the preprocessor gives up.
const MAX_INCLUDE_DEPTH: usize = 32;

/// Maximum nesting of macro expansions before the preprocessor gives up.
///
/// Direct recursion is caught by name; this bounds pathological non-cyclic
/// chains and expansions that grow by other means.
const MAX_EXPANSION_DEPTH: usize = 128;

/// Narrows a byte offset to the `u32` used in spans.
///
/// Every text handled here came from a [`SourceMap`] or was derived from
/// one and is bounded by the same limit, so the conversion cannot fail.
fn offset(n: usize) -> u32 {
    u32::try_from(n).expect("preprocessor offset exceeds u32")
}

/// Supplies the text of `` `include ``d files.
///
/// The preprocessor never reads the filesystem; the caller decides how a
/// path is looked up (search directories, relative to the including file,
/// an in-memory table in tests) and returns the file's display name and
/// text.
pub trait IncludeResolver {
    /// Resolves `path` as written in the directive, requested from the file
    /// `from`. Returns `(name, text)` or `None` when the file is not found.
    fn resolve(&mut self, path: &str, from: SourceId) -> Option<(String, String)>;
}

/// A resolver that finds nothing, for sources without includes.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoIncludes;

impl IncludeResolver for NoIncludes {
    fn resolve(&mut self, _path: &str, _from: SourceId) -> Option<(String, String)> {
        None
    }
}

/// A `` `timescale `` directive that was seen, in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timescale {
    /// The directive, from the backtick to the end of its line.
    pub span: Span,
    /// The trimmed argument text, such as `1ns / 1ps`.
    pub text: String,
}

/// One run of output text and where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Piece {
    /// Byte offset of the first byte of this piece in the output.
    out_start: u32,
    /// Byte offset one past the last byte in the output.
    out_end: u32,
    /// For a verbatim piece, the file range copied; for an expansion, the
    /// macro use site.
    origin: Span,
    /// The definition of the macro that produced an expansion piece.
    defined_at: Option<Span>,
}

impl Piece {
    /// Maps an output offset inside (or at the end of) this piece to the
    /// origin file: one-to-one for verbatim pieces, the use-site start for
    /// expansions.
    fn origin_start(&self, out: u32) -> u32 {
        match self.defined_at {
            Some(_) => self.origin.start,
            None => self.origin.start + (out - self.out_start),
        }
    }

    /// The same for a span end, which for expansions is the use-site end.
    fn origin_end(&self, out: u32) -> u32 {
        match self.defined_at {
            Some(_) => self.origin.end,
            None => self.origin.start + (out - self.out_start),
        }
    }
}

/// Maps byte ranges of the expanded text back to original source spans.
#[derive(Clone, Debug, Default)]
pub struct SpanMap {
    /// Pieces in output order, contiguous and non-overlapping.
    pieces: Vec<Piece>,
    /// The root file, used for positions when the output is empty.
    root: Option<SourceId>,
}

impl SpanMap {
    /// The piece containing output offset `out`, or the last piece when
    /// `out` is at the very end.
    fn piece_at(&self, out: u32) -> Option<&Piece> {
        let i = self.pieces.partition_point(|p| p.out_end <= out);
        self.pieces.get(i).or_else(|| self.pieces.last())
    }

    /// Maps the output byte range `[start, end)` to a span in the original
    /// sources.
    ///
    /// Text inside a macro expansion maps to the whole use site. A range
    /// that starts in verbatim text and ends inside an expansion (or the
    /// reverse) gets the union of both, provided they lie in one file; if
    /// not, the span of the piece holding `start` wins.
    ///
    /// # Panics
    ///
    /// On a default-constructed map that no preprocessor run filled in: it
    /// has no root file to attribute an empty output to.
    pub fn map(&self, start: u32, end: u32) -> Span {
        let Some(first) = self.piece_at(start) else {
            let file = self.root.expect("empty span map has no root file");
            return Span::new(file, 0, 0);
        };
        let s = first.origin_start(start.clamp(first.out_start, first.out_end));
        let last = if end > start {
            self.piece_at(end - 1).unwrap_or(first)
        } else {
            first
        };
        if last.origin.file == first.origin.file {
            let e = last.origin_end(end.clamp(last.out_start, last.out_end));
            Span::new(first.origin.file, s, e.max(s))
        } else {
            Span::new(first.origin.file, s, first.origin_end(first.out_end))
        }
    }

    /// The definition site of the macro whose expansion contains output
    /// offset `out`, for a secondary "in this expansion of the macro
    /// defined here" label. `None` for verbatim text.
    pub fn macro_definition(&self, out: u32) -> Option<Span> {
        self.piece_at(out)
            .filter(|p| p.out_start <= out && out < p.out_end)
            .and_then(|p| p.defined_at)
    }

    /// True when output offset `out` lies inside a macro expansion.
    pub fn is_expansion(&self, out: u32) -> bool {
        self.macro_definition(out).is_some()
    }

    fn push(&mut self, piece: Piece) {
        if piece.out_start == piece.out_end {
            return;
        }
        // Coalesce with the previous piece when the two are contiguous in
        // both output and origin; expansions from the same use site merge.
        if let Some(prev) = self.pieces.last_mut()
            && prev.out_end == piece.out_start
            && prev.defined_at == piece.defined_at
            && prev.origin.file == piece.origin.file
        {
            let joinable = match piece.defined_at {
                Some(_) => prev.origin == piece.origin,
                None => prev.origin.end == piece.origin.start,
            };
            if joinable {
                prev.out_end = piece.out_end;
                if piece.defined_at.is_none() {
                    prev.origin.end = piece.origin.end;
                }
                return;
            }
        }
        self.pieces.push(piece);
    }
}

/// The result of preprocessing one root file.
#[derive(Clone, Debug, Default)]
pub struct Preprocessed {
    /// The expanded text, ready for the lexer.
    pub text: String,
    /// Maps offsets in `text` back to the original files.
    pub spans: SpanMap,
    /// Every `` `timescale `` directive seen, in expansion order.
    pub timescales: Vec<Timescale>,
}

/// A macro definition.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Macro {
    /// The formal parameters, `None` for an object-like macro.
    formals: Option<Vec<Formal>>,
    /// The body with continuations resolved and line comments removed.
    body: String,
    /// The `` `define NAME `` part of the definition.
    span: Span,
}

/// One formal parameter of a function-like macro.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Formal {
    name: String,
    default: Option<String>,
}

/// Where the text being scanned came from.
#[derive(Clone, Copy, Debug)]
enum Origin {
    /// The whole text of a file; offset `i` in the text is offset `i` in
    /// the file.
    File(SourceId),
    /// The expansion of a macro; every offset reports the use site.
    Expansion { use_site: Span, defined_at: Span },
}

impl Origin {
    /// The file an offset in this text belongs to.
    fn file(self) -> SourceId {
        match self {
            Origin::File(id) => id,
            Origin::Expansion { use_site, .. } => use_site.file,
        }
    }

    /// The original span of the text range `[start, end)`.
    fn span(self, start: usize, end: usize) -> Span {
        match self {
            Origin::File(id) => Span::new(id, offset(start), offset(end)),
            Origin::Expansion { use_site, .. } => use_site,
        }
    }
}

/// One `` `ifdef `` / `` `ifndef `` awaiting its `` `endif ``.
#[derive(Clone, Copy, Debug)]
struct Conditional {
    /// True once some branch of the conditional has been taken.
    taken: bool,
    /// True once `` `else `` has been seen.
    in_else: bool,
    /// The opening directive.
    span: Span,
}

/// Runs the preprocessor, optionally with predefined macros.
///
/// ```
/// use reticle::diag::Diagnostics;
/// use reticle::source::SourceMap;
/// use reticle::verilog::preprocess::{NoIncludes, Preprocessor};
///
/// let mut map = SourceMap::new();
/// let id = map.add("t.v", "`ifdef SYNTHESIS wire a; `endif").unwrap();
/// let mut diags = Diagnostics::new();
/// let out = Preprocessor::new()
///     .define("SYNTHESIS", "")
///     .run(&mut map, id, &mut NoIncludes, &mut diags);
/// assert_eq!(out.text.trim(), "wire a;");
/// assert!(diags.is_empty());
/// ```
#[derive(Clone, Debug, Default)]
pub struct Preprocessor {
    predefined: Vec<(String, String)>,
}

impl Preprocessor {
    /// A preprocessor with no predefined macros.
    pub fn new() -> Self {
        Self::default()
    }

    /// Predefines an object-like macro, as `+define+NAME=body` would.
    pub fn define(mut self, name: impl Into<String>, body: impl Into<String>) -> Self {
        self.predefined.push((name.into(), body.into()));
        self
    }

    /// Preprocesses the file `id` of `map`, adding included files to the
    /// map and reporting problems to `diags`.
    pub fn run(
        &self,
        map: &mut SourceMap,
        id: SourceId,
        resolver: &mut dyn IncludeResolver,
        diags: &mut Diagnostics,
    ) -> Preprocessed {
        let mut state = State {
            map,
            resolver,
            diags,
            out: Preprocessed::default(),
            macros: BTreeMap::new(),
            expanding: Vec::new(),
            include_depth: 0,
            includes: vec![id],
        };
        state.out.spans.root = Some(id);
        let root = Span::new(id, 0, 0);
        for (name, body) in &self.predefined {
            state.macros.insert(
                name.clone(),
                Macro {
                    formals: None,
                    body: body.clone(),
                    span: root,
                },
            );
        }
        // The file's text is cloned so the map can be borrowed mutably for
        // includes while scanning; files are small relative to the work
        // done per byte.
        let text = state.map.file(id).text().to_string();
        state.scan(&text, Origin::File(id), 0);
        state.out
    }
}

/// Preprocesses one file with no predefined macros.
///
/// Shorthand for [`Preprocessor::new().run(..)`](Preprocessor::run).
pub fn preprocess(
    map: &mut SourceMap,
    id: SourceId,
    resolver: &mut dyn IncludeResolver,
    diags: &mut Diagnostics,
) -> Preprocessed {
    Preprocessor::new().run(map, id, resolver, diags)
}

/// Mutable state shared by every recursive scan.
struct State<'a> {
    map: &'a mut SourceMap,
    resolver: &'a mut dyn IncludeResolver,
    diags: &'a mut Diagnostics,
    out: Preprocessed,
    macros: BTreeMap<String, Macro>,
    /// Names of the macros currently being expanded, outermost first.
    expanding: Vec<String>,
    include_depth: usize,
    /// Files currently being scanned, outermost first, to detect cycles.
    includes: Vec<SourceId>,
}

/// True for the characters that may start an identifier or directive name.
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// True for the characters that may continue an identifier.
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// The end of the identifier starting at `i` (which must satisfy
/// [`is_ident_start`]), exclusive.
fn ident_end(text: &str, i: usize) -> usize {
    let bytes = text.as_bytes();
    let mut j = i;
    while j < bytes.len() && is_ident_char(bytes[j]) {
        j += 1;
    }
    j
}

/// Skips spaces and tabs, returning the new offset.
fn skip_blanks(text: &str, mut i: usize) -> usize {
    let bytes = text.as_bytes();
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    i
}

/// Skips all whitespace including newlines.
fn skip_ws(text: &str, mut i: usize) -> usize {
    let bytes = text.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// The offset of the newline ending the line containing `i`, or the text
/// length.
fn line_end(text: &str, i: usize) -> usize {
    text[i..].find('\n').map_or(text.len(), |n| i + n)
}

/// If a line comment, block comment or string starts at `i`, returns the
/// offset just past it (an unterminated one runs to the end of the text).
fn skip_opaque(text: &str, i: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    match bytes[i] {
        b'/' if bytes.get(i + 1) == Some(&b'/') => Some(line_end(text, i)),
        b'/' if bytes.get(i + 1) == Some(&b'*') => Some(
            text[i + 2..]
                .find("*/")
                .map_or(text.len(), |n| i + 2 + n + 2),
        ),
        b'"' => Some(string_end(text, i)),
        _ => None,
    }
}

/// The offset just past the string literal opening at `i`. Stops at an
/// unescaped newline (an unterminated string, which the lexer reports).
fn string_end(text: &str, i: usize) -> usize {
    let bytes = text.as_bytes();
    let mut j = i + 1;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b'"' => return j + 1,
            b'\n' => return j,
            _ => j += 1,
        }
    }
    bytes.len()
}

impl State<'_> {
    /// Reports an error at a span.
    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.error(span, message);
    }

    /// Appends a verbatim run of `text[start..end)` to the output.
    fn emit_verbatim(&mut self, text: &str, origin: Origin, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let out_start = offset(self.out.text.len());
        self.out.text.push_str(&text[start..end]);
        let (origin_span, defined_at) = match origin {
            Origin::File(_) => (origin.span(start, end), None),
            Origin::Expansion {
                use_site,
                defined_at,
            } => (use_site, Some(defined_at)),
        };
        self.out.spans.push(Piece {
            out_start,
            out_end: offset(self.out.text.len()),
            origin: origin_span,
            defined_at,
        });
    }

    /// Appends generated text attributed to the macro use at `use_site`.
    fn emit_expansion(&mut self, text: &str, use_site: Span, defined_at: Span) {
        let out_start = offset(self.out.text.len());
        self.out.text.push_str(text);
        self.out.spans.push(Piece {
            out_start,
            out_end: offset(self.out.text.len()),
            origin: use_site,
            defined_at: Some(defined_at),
        });
    }

    /// Scans `text`, which came from `origin`, appending its expansion to
    /// the output.
    fn scan(&mut self, text: &str, origin: Origin, depth: usize) {
        let bytes = text.as_bytes();
        let mut conds: Vec<Conditional> = Vec::new();
        let mut run_start = 0;
        let mut i = 0;
        while i < bytes.len() {
            if let Some(end) = skip_opaque(text, i) {
                i = end;
                continue;
            }
            if bytes[i] != b'`' {
                i += 1;
                continue;
            }
            // A directive. Flush the verbatim run before it; the handler
            // says where scanning resumes and whether the directive text
            // itself is kept.
            let name_end = if bytes.get(i + 1).is_some_and(|&b| is_ident_start(b)) {
                ident_end(text, i + 1)
            } else {
                i + 1
            };
            let name = &text[i + 1..name_end];
            match name {
                "" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    let (what, len) = match bytes.get(i + 1) {
                        Some(b'"') => ("`\"` outside a macro body", 2),
                        Some(b'`') => ("``` `` ``` outside a macro body", 2),
                        _ => ("stray backtick", 1),
                    };
                    self.error(origin.span(i, i + len), format!("unexpected {what}"));
                    i += len;
                    run_start = i;
                }
                "timescale" | "default_nettype" | "resetall" | "begin_keywords"
                | "end_keywords" => {
                    if name == "timescale" {
                        let end = line_end(text, name_end);
                        self.out.timescales.push(Timescale {
                            span: origin.span(i, end),
                            text: text[name_end..end].trim().to_string(),
                        });
                    }
                    // Passed through as part of the verbatim run.
                    i = name_end;
                }
                "celldefine"
                | "endcelldefine"
                | "unconnected_drive"
                | "nounconnected_drive"
                | "default_decay_time"
                | "default_trireg_strength"
                | "delay_mode_distributed"
                | "delay_mode_path"
                | "delay_mode_unit"
                | "delay_mode_zero"
                | "pragma"
                | "protect"
                | "endprotect"
                | "protected"
                | "endprotected"
                | "line" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    i = line_end(text, name_end);
                    run_start = i;
                }
                "define" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    i = self.directive_define(text, origin, i, name_end);
                    run_start = i;
                }
                "undef" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    let j = skip_blanks(text, name_end);
                    let (macro_name, after) = self.read_name(text, origin, i, j);
                    if let Some(macro_name) = macro_name
                        && self.macros.remove(&macro_name).is_none()
                    {
                        self.diags.warning(
                            origin.span(i, after),
                            format!("macro `{macro_name}` is not defined"),
                        );
                    }
                    i = after;
                    run_start = i;
                }
                "undefineall" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    self.macros.clear();
                    i = name_end;
                    run_start = i;
                }
                "ifdef" | "ifndef" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    let j = skip_blanks(text, name_end);
                    let (macro_name, after) = self.read_name(text, origin, i, j);
                    let defined = macro_name.is_some_and(|n| self.macros.contains_key(&n));
                    let taken = defined == (name == "ifdef");
                    conds.push(Conditional {
                        taken,
                        in_else: false,
                        span: origin.span(i, after),
                    });
                    i = if taken {
                        after
                    } else {
                        self.skip_inactive(text, origin, after, &mut conds)
                    };
                    run_start = i;
                }
                "elsif" | "else" | "endif" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    if conds.is_empty() {
                        self.error(
                            origin.span(i, name_end),
                            format!("`` `{name} `` without a matching `` `ifdef ``"),
                        );
                        i = if name == "elsif" {
                            self.read_name(text, origin, i, skip_blanks(text, name_end))
                                .1
                        } else {
                            name_end
                        };
                    } else {
                        // The branch that just ended was active, so every
                        // remaining branch is skipped.
                        i = self.skip_inactive(text, origin, i, &mut conds);
                    }
                    run_start = i;
                }
                "include" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    i = self.directive_include(text, origin, i, name_end, depth);
                    run_start = i;
                }
                "__FILE__" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    let use_site = origin.span(i, name_end);
                    let file_name = self.map.file(origin.file()).name().to_string();
                    let literal = format!("{file_name:?}");
                    self.emit_expansion(&literal, use_site, use_site);
                    i = name_end;
                    run_start = i;
                }
                "__LINE__" => {
                    self.emit_verbatim(text, origin, run_start, i);
                    let use_site = origin.span(i, name_end);
                    let line = self.map.file(use_site.file).loc(use_site.start).line;
                    self.emit_expansion(&line.to_string(), use_site, use_site);
                    i = name_end;
                    run_start = i;
                }
                _ => {
                    self.emit_verbatim(text, origin, run_start, i);
                    i = self.expand_macro(text, origin, i, name_end, depth);
                    run_start = i;
                }
            }
        }
        self.emit_verbatim(text, origin, run_start, bytes.len());
        for cond in conds {
            self.error(cond.span, "unterminated conditional directive");
        }
    }

    /// Reads a macro name at `j` for a directive that started at
    /// `directive_start`, reporting an error when there is none. Returns the
    /// name and the offset after it.
    fn read_name(
        &mut self,
        text: &str,
        origin: Origin,
        directive_start: usize,
        j: usize,
    ) -> (Option<String>, usize) {
        if text.as_bytes().get(j).is_some_and(|&b| is_ident_start(b)) {
            let end = ident_end(text, j);
            (Some(text[j..end].to_string()), end)
        } else {
            self.error(
                origin.span(directive_start, j),
                "expected a macro name after the directive",
            );
            (None, j)
        }
    }

    /// Skips inactive conditional text starting at `i`, which is either
    /// just after an `` `ifdef `` / `` `ifndef `` that was not taken, or at
    /// an `` `elsif `` / `` `else `` / `` `endif `` ending an active branch.
    /// Returns where active scanning resumes, with `conds` updated.
    fn skip_inactive(
        &mut self,
        text: &str,
        origin: Origin,
        mut i: usize,
        conds: &mut Vec<Conditional>,
    ) -> usize {
        let bytes = text.as_bytes();
        loop {
            // Find the next branch directive at nesting depth 0.
            let mut nesting = 0usize;
            let found = loop {
                if i >= bytes.len() {
                    break None;
                }
                if let Some(end) = skip_opaque(text, i) {
                    i = end;
                    continue;
                }
                if bytes[i] != b'`' {
                    i += 1;
                    continue;
                }
                if !bytes.get(i + 1).is_some_and(|&b| is_ident_start(b)) {
                    i += 1;
                    continue;
                }
                let name_end = ident_end(text, i + 1);
                let name = &text[i + 1..name_end];
                match name {
                    "ifdef" | "ifndef" => nesting += 1,
                    "endif" if nesting > 0 => nesting -= 1,
                    "elsif" | "else" | "endif" if nesting == 0 => break Some((i, name_end)),
                    _ => {}
                }
                i = name_end;
            };
            let Some((start, name_end)) = found else {
                // Unterminated: reported when the scan finishes.
                return bytes.len();
            };
            let name = &text[start + 1..name_end];
            let cond = conds.last_mut().expect("skip_inactive with no conditional");
            match name {
                "elsif" => {
                    let j = skip_blanks(text, name_end);
                    let (macro_name, after) = self.read_name(text, origin, start, j);
                    i = after;
                    if cond.in_else {
                        self.error(
                            origin.span(start, name_end),
                            "`` `elsif `` after `` `else ``",
                        );
                    }
                    if !cond.taken && macro_name.is_some_and(|n| self.macros.contains_key(&n)) {
                        cond.taken = true;
                        return i;
                    }
                }
                "else" => {
                    i = name_end;
                    if cond.in_else {
                        self.error(origin.span(start, name_end), "duplicate `` `else ``");
                    }
                    cond.in_else = true;
                    if !cond.taken {
                        cond.taken = true;
                        return i;
                    }
                }
                _ => {
                    conds.pop();
                    return name_end;
                }
            }
        }
    }

    /// Handles `` `define `` at `start`; `name_end` is just after the word
    /// `define`. Returns where scanning resumes.
    fn directive_define(
        &mut self,
        text: &str,
        origin: Origin,
        start: usize,
        name_end: usize,
    ) -> usize {
        let bytes = text.as_bytes();
        let j = skip_blanks(text, name_end);
        let (Some(name), mut i) = self.read_name(text, origin, start, j) else {
            return line_end(text, j);
        };
        let span = origin.span(start, i);

        // Formals: only when `(` immediately follows the name.
        let mut formals = None;
        if bytes.get(i) == Some(&b'(') {
            let mut list = Vec::new();
            i += 1;
            loop {
                i = skip_ws(text, i);
                if bytes.get(i) == Some(&b')') {
                    i += 1;
                    break;
                }
                if !bytes.get(i).is_some_and(|&b| is_ident_start(b)) {
                    self.error(origin.span(start, i), "expected a macro parameter name");
                    return line_end(text, i);
                }
                let end = ident_end(text, i);
                let formal_name = text[i..end].to_string();
                i = skip_ws(text, end);
                let mut default = None;
                if bytes.get(i) == Some(&b'=') {
                    let (value, end) = balanced_until(text, i + 1, b')');
                    default = Some(value.trim().to_string());
                    i = end;
                }
                list.push(Formal {
                    name: formal_name,
                    default,
                });
                match bytes.get(i) {
                    Some(b',') => i += 1,
                    Some(b')') => {
                        i += 1;
                        break;
                    }
                    _ => {
                        self.error(
                            origin.span(start, i),
                            "expected `,` or `)` in macro parameter list",
                        );
                        return line_end(text, i);
                    }
                }
            }
            formals = Some(list);
        }

        // Body: the rest of the logical line, continuations joined.
        let (body, end) = read_logical_line(text, skip_blanks(text, i));
        let body = strip_line_comments(&body);
        let def = Macro {
            formals,
            body: body.trim_end().to_string(),
            span,
        };
        if let Some(prev) = self.macros.get(&name)
            && (prev.body != def.body || prev.formals != def.formals)
        {
            self.diags.push(
                Diagnostic::warning(format!("macro `{name}` is redefined"))
                    .with_label(span, "redefined here")
                    .with_secondary(prev.span, "previously defined here"),
            );
        }
        self.macros.insert(name, def);
        end
    }

    /// Handles `` `include `` at `start`. Returns where scanning resumes.
    fn directive_include(
        &mut self,
        text: &str,
        origin: Origin,
        start: usize,
        name_end: usize,
        depth: usize,
    ) -> usize {
        let bytes = text.as_bytes();
        let j = skip_blanks(text, name_end);
        let close = match bytes.get(j) {
            Some(b'"') => b'"',
            Some(b'<') => b'>',
            _ => {
                self.error(
                    origin.span(start, j),
                    "expected a quoted file name after `` `include ``",
                );
                return line_end(text, j);
            }
        };
        let path_start = j + 1;
        let end = line_end(text, path_start);
        let Some(n) = bytes[path_start..end].iter().position(|&b| b == close) else {
            self.error(
                origin.span(start, end),
                "unterminated `` `include `` file name",
            );
            return end;
        };
        let path_end = path_start + n;
        let path = text[path_start..path_end].to_string();
        let after = path_end + 1;
        let span = origin.span(start, after);

        if self.include_depth >= MAX_INCLUDE_DEPTH {
            self.error(span, "`` `include `` nested too deeply");
            return after;
        }
        let from = origin.file();
        let Some((file_name, file_text)) = self.resolver.resolve(&path, from) else {
            self.error(span, format!("cannot find include file `{path}`"));
            return after;
        };
        let id = match self.map.add(file_name, file_text) {
            Ok(id) => id,
            Err(err) => {
                self.error(span, format!("cannot load include file `{path}`: {err}"));
                return after;
            }
        };
        // Cycles are detected by name: the resolver hands the same name
        // back for the same file, and a file including itself is always
        // a mistake.
        let name = self.map.file(id).name().to_string();
        if self
            .includes
            .iter()
            .any(|&inc| self.map.file(inc).name() == name)
        {
            self.error(span, format!("include cycle: `{name}` includes itself"));
            return after;
        }
        let included = self.map.file(id).text().to_string();
        self.include_depth += 1;
        self.includes.push(id);
        self.scan(&included, Origin::File(id), depth);
        self.includes.pop();
        self.include_depth -= 1;
        after
    }

    /// Expands the macro use at `start` whose name ends at `name_end`.
    /// Returns where scanning resumes.
    fn expand_macro(
        &mut self,
        text: &str,
        origin: Origin,
        start: usize,
        name_end: usize,
        depth: usize,
    ) -> usize {
        let bytes = text.as_bytes();
        let name = &text[start + 1..name_end];
        let Some(def) = self.macros.get(name).cloned() else {
            self.error(
                origin.span(start, name_end),
                format!("undefined macro `{name}`"),
            );
            return name_end;
        };

        // Arguments.
        let mut end = name_end;
        let mut actuals: Vec<String> = Vec::new();
        if let Some(formals) = &def.formals {
            let j = skip_blanks(text, name_end);
            if bytes.get(j) != Some(&b'(') {
                self.error(
                    origin.span(start, name_end),
                    format!("macro `{name}` takes arguments but none were given"),
                );
                return name_end;
            }
            let mut i = j + 1;
            loop {
                let (arg, e) = balanced_until(text, i, b')');
                actuals.push(arg.trim().to_string());
                match bytes.get(e) {
                    Some(b',') => i = e + 1,
                    Some(b')') => {
                        end = e + 1;
                        break;
                    }
                    _ => {
                        self.error(
                            origin.span(start, e),
                            format!("unterminated argument list for macro `{name}`"),
                        );
                        return e;
                    }
                }
            }
            // `M()` is one empty argument, which for a macro with no
            // formals at all means no arguments.
            if actuals.len() == 1 && actuals[0].is_empty() && formals.is_empty() {
                actuals.clear();
            }
            if actuals.len() > formals.len() {
                self.error(
                    origin.span(start, end),
                    format!(
                        "macro `{name}` takes {} argument(s) but {} were given",
                        formals.len(),
                        actuals.len()
                    ),
                );
                return end;
            }
            for (k, formal) in formals.iter().enumerate() {
                let missing = actuals.get(k).is_none_or(String::is_empty);
                if missing {
                    match &formal.default {
                        Some(d) => {
                            if k < actuals.len() {
                                actuals[k] = d.clone();
                            } else {
                                actuals.push(d.clone());
                            }
                        }
                        None if k >= actuals.len() => {
                            self.error(
                                origin.span(start, end),
                                format!(
                                    "macro `{name}` takes {} argument(s) but {} were given",
                                    formals.len(),
                                    actuals.len()
                                ),
                            );
                            return end;
                        }
                        None => {}
                    }
                }
            }
        }
        let use_site = origin.span(start, end);

        if self.expanding.iter().any(|n| n == name) {
            self.error(use_site, format!("recursive expansion of macro `{name}`"));
            return end;
        }
        if depth >= MAX_EXPANSION_DEPTH {
            self.error(use_site, "macro expansion nested too deeply");
            return end;
        }

        let formals = def.formals.as_deref().unwrap_or(&[]);
        let expanded = substitute(&def.body, formals, &actuals);
        self.expanding.push(name.to_string());
        self.scan(
            &expanded,
            Origin::Expansion {
                use_site,
                defined_at: def.span,
            },
            depth + 1,
        );
        self.expanding.pop();
        end
    }
}

/// Reads text from `i` up to a top-level `,` or `close`, respecting nested
/// brackets and strings. Returns the text and the offset of the terminator
/// (or the text length when unterminated).
fn balanced_until(text: &str, i: usize, close: u8) -> (String, usize) {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut j = i;
    while j < bytes.len() {
        if bytes[j] == b'"' {
            j = string_end(text, j);
            continue;
        }
        match bytes[j] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth > 0 => depth -= 1,
            b',' if depth == 0 => break,
            b if b == close && depth == 0 => break,
            _ => {}
        }
        j += 1;
    }
    (text[i..j].to_string(), j)
}

/// Reads the logical line starting at `i`: up to the newline, with
/// `\`-newline sequences replaced by a newline. Returns the joined text and
/// the offset of the terminating newline (or the text length).
fn read_logical_line(text: &str, i: usize) -> (String, usize) {
    let bytes = text.as_bytes();
    let mut body = String::new();
    let mut j = i;
    let mut seg = i;
    while j < bytes.len() {
        match bytes[j] {
            b'\n' => break,
            b'\\' if bytes.get(j + 1) == Some(&b'\n') => {
                body.push_str(&text[seg..j]);
                body.push('\n');
                j += 2;
                seg = j;
            }
            b'\\' if bytes.get(j + 1) == Some(&b'\r') && bytes.get(j + 2) == Some(&b'\n') => {
                body.push_str(&text[seg..j]);
                body.push('\n');
                j += 3;
                seg = j;
            }
            _ => j += 1,
        }
    }
    body.push_str(&text[seg..j]);
    (body, j)
}

/// Removes `//` comments from a macro body, leaving strings and block
/// comments alone.
fn strip_line_comments(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    let mut seg = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => i = string_end(body, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = body[i + 2..].find("*/").map_or(bytes.len(), |n| i + 4 + n);
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                out.push_str(&body[seg..i]);
                i = line_end(body, i);
                seg = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&body[seg..]);
    out
}

/// Substitutes actual arguments for formals in a macro body and resolves
/// `` `" ``, `` `\`" `` and ``` `` ```.
///
/// Formals inside ordinary string literals are not substituted; inside
/// `` `" `` strings they are, which is the whole point of that form.
fn substitute(body: &str, formals: &[Formal], actuals: &[String]) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut in_macro_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'`' {
            match bytes.get(i + 1) {
                Some(b'"') => {
                    out.push('"');
                    in_macro_string = !in_macro_string;
                    i += 2;
                }
                Some(b'\\')
                    if bytes.get(i + 2) == Some(&b'`') && bytes.get(i + 3) == Some(&b'"') =>
                {
                    out.push_str("\\\"");
                    i += 4;
                }
                Some(b'`') => i += 2,
                _ => {
                    out.push('`');
                    i += 1;
                }
            }
            continue;
        }
        if b == b'"' && !in_macro_string {
            let end = string_end(body, i);
            out.push_str(&body[i..end]);
            i = end;
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let end = body[i + 2..].find("*/").map_or(bytes.len(), |n| i + 4 + n);
            out.push_str(&body[i..end]);
            i = end;
            continue;
        }
        if is_ident_start(b) {
            let end = ident_end(body, i);
            let word = &body[i..end];
            match formals.iter().position(|f| f.name == word) {
                Some(k) => out.push_str(actuals.get(k).map_or("", String::as_str)),
                None => out.push_str(word),
            }
            i = end;
            continue;
        }
        // Any other byte, including the leading byte of a multi-byte
        // character: copy the whole character.
        let ch = body[i..].chars().next().expect("in bounds");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Table(Vec<(&'static str, &'static str)>);

    impl IncludeResolver for Table {
        fn resolve(&mut self, path: &str, _from: SourceId) -> Option<(String, String)> {
            self.0
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(p, t)| (p.to_string(), t.to_string()))
        }
    }

    fn run(text: &str) -> (SourceMap, SourceId, Preprocessed, Diagnostics) {
        run_with(text, Table(Vec::new()))
    }

    fn run_with(text: &str, mut table: Table) -> (SourceMap, SourceId, Preprocessed, Diagnostics) {
        let mut map = SourceMap::new();
        let id = map.add("t.v", text).unwrap();
        let mut diags = Diagnostics::new();
        let out = preprocess(&mut map, id, &mut table, &mut diags);
        (map, id, out, diags)
    }

    fn messages(diags: &Diagnostics) -> Vec<String> {
        diags.iter().map(|d| d.message.clone()).collect()
    }

    #[test]
    fn object_macro_maps_to_use_site() {
        let src = "`define W 8\nwire [`W-1:0] a;\n";
        let (map, id, out, diags) = run(src);
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(out.text, "\nwire [8-1:0] a;\n");
        // The `8` in the output maps to the whole `` `W `` use.
        let eight = offset(out.text.find('8').unwrap());
        let span = out.spans.map(eight, eight + 1);
        let use_start = offset(src.find("`W").unwrap());
        assert_eq!(span, Span::new(id, use_start, use_start + 2));
        assert_eq!(map.file(id).loc(span.start).line, 2);
        assert!(out.spans.is_expansion(eight));
        assert_eq!(out.spans.macro_definition(eight), Some(Span::new(id, 0, 9)));
        // Verbatim text maps one-to-one.
        let wire = offset(out.text.find("wire").unwrap());
        assert_eq!(
            out.spans.map(wire, wire + 4),
            Span::new(
                id,
                offset(src.find("wire").unwrap()),
                offset(src.find("wire").unwrap()) + 4
            )
        );
        assert!(!out.spans.is_expansion(wire));
    }

    #[test]
    fn token_straddling_expansion_gets_union_span() {
        let src = "`define SIZE 8\nx = `SIZE'hff;";
        let (_, id, out, diags) = run(src);
        assert!(diags.is_empty());
        assert_eq!(out.text, "\nx = 8'hff;");
        let start = offset(out.text.find('8').unwrap());
        let span = out.spans.map(start, start + 5);
        let use_start = offset(src.find("`SIZE").unwrap());
        assert_eq!(span, Span::new(id, use_start, use_start + 5 + 4));
    }

    #[test]
    fn function_macro_with_defaults_and_paste() {
        let src = "`define CAT(a, b = bar) a``b\n`define STR(x) `\"x`\"\n`CAT(foo,) `CAT(x, y) `STR(hi there)";
        let (_, _, out, diags) = run(src);
        assert!(diags.is_empty(), "{:?}", messages(&diags));
        assert_eq!(out.text, "\n\nfoobar xy \"hi there\"");
    }

    #[test]
    fn nested_macros_and_arguments_with_brackets() {
        let src = "`define A(x) (x + `B)\n`define B 1\n`A({2, 3})\n";
        let (_, _, out, diags) = run(src);
        assert!(diags.is_empty(), "{:?}", messages(&diags));
        assert_eq!(out.text, "\n\n({2, 3} + 1)\n");
    }

    #[test]
    fn multiline_define_and_comments() {
        let src = "`define M a \\\n  b // note\n`M\n";
        let (_, _, out, diags) = run(src);
        assert!(diags.is_empty(), "{:?}", messages(&diags));
        assert_eq!(out.text, "\na \n  b\n");
    }

    #[test]
    fn conditionals_nest() {
        let src = "`define A\n`ifdef A\n1\n`ifndef A\n2\n`elsif A\n3\n`else\n4\n`endif\n`else\n5\n`endif\n`ifdef Z\n6\n`elsif Y\n7\n`else\n8\n`endif\n";
        let (_, _, out, diags) = run(src);
        assert!(diags.is_empty(), "{:?}", messages(&diags));
        let kept: Vec<&str> = out.text.split_whitespace().collect();
        assert_eq!(kept, ["1", "3", "8"]);
    }

    #[test]
    fn conditional_errors() {
        let (_, _, _, diags) = run("`endif\n`ifdef X\n");
        assert_eq!(
            messages(&diags),
            [
                "`` `endif `` without a matching `` `ifdef ``",
                "unterminated conditional directive"
            ]
        );
        let (_, _, _, diags) = run("`ifdef X\n`else\n`else\n`endif\n");
        assert_eq!(messages(&diags), ["duplicate `` `else ``"]);
    }

    #[test]
    fn undefined_and_recursive_macros() {
        let (_, _, out, diags) = run("`NOPE x\n`define R `R\n`R\n");
        assert_eq!(out.text, " x\n\n\n");
        assert_eq!(
            messages(&diags),
            ["undefined macro `NOPE`", "recursive expansion of macro `R`"]
        );
    }

    #[test]
    fn argument_count_errors() {
        let (_, _, _, diags) = run("`define F(a,b) a b\n`F(1)\n`F(1,2,3)\n`F\n");
        assert_eq!(
            messages(&diags),
            [
                "macro `F` takes 2 argument(s) but 1 were given",
                "macro `F` takes 2 argument(s) but 3 were given",
                "macro `F` takes arguments but none were given",
            ]
        );
    }

    #[test]
    fn includes_add_files_and_detect_cycles() {
        let table = Table(vec![
            ("a.vh", "`define FROM_A 1\n`include \"b.vh\"\n"),
            ("b.vh", "b_text `FROM_A\n"),
        ]);
        let (map, _, out, diags) = run_with("`include \"a.vh\"\n`FROM_A\n", table);
        assert!(diags.is_empty(), "{:?}", messages(&diags));
        assert_eq!(map.len(), 3);
        assert_eq!(out.text, "\nb_text 1\n\n\n1\n");
        let b = offset(out.text.find("b_text").unwrap());
        let span = out.spans.map(b, b + 6);
        assert_eq!(map.file(span.file).name(), "b.vh");
        assert_eq!(span.start, 0);

        let table = Table(vec![("c.vh", "`include \"c.vh\"\n")]);
        let (_, _, _, diags) = run_with("`include \"c.vh\"\n", table);
        assert_eq!(messages(&diags), ["include cycle: `c.vh` includes itself"]);

        let (_, _, _, diags) = run("`include \"missing.vh\"\n`include x\n");
        assert_eq!(
            messages(&diags),
            [
                "cannot find include file `missing.vh`",
                "expected a quoted file name after `` `include ``"
            ]
        );
    }

    #[test]
    fn builtin_macros_and_passthrough() {
        let src = "`timescale 1ns / 1ps\n`default_nettype none\n`celldefine\n`__FILE__ `__LINE__\n";
        let (_, _, out, diags) = run(src);
        assert!(diags.is_empty(), "{:?}", messages(&diags));
        assert_eq!(
            out.text,
            "`timescale 1ns / 1ps\n`default_nettype none\n\n\"t.v\" 4\n"
        );
        assert_eq!(out.timescales.len(), 1);
        assert_eq!(out.timescales[0].text, "1ns / 1ps");
    }

    #[test]
    fn directives_in_comments_and_strings_are_untouched() {
        let src = "// `define X\n/* `X */ \"`X\" `undef Y\n";
        let (_, _, out, diags) = run(src);
        assert_eq!(out.text, "// `define X\n/* `X */ \"`X\" \n");
        assert_eq!(messages(&diags), ["macro `Y` is not defined"]);
    }

    #[test]
    fn redefinition_warns_only_when_different() {
        let (_, _, _, diags) = run("`define A 1\n`define A 1\n`define A 2\n");
        assert_eq!(messages(&diags), ["macro `A` is redefined"]);
    }

    #[test]
    fn predefined_macros() {
        let mut map = SourceMap::new();
        let id = map.add("t.v", "`X").unwrap();
        let mut diags = Diagnostics::new();
        let out =
            Preprocessor::new()
                .define("X", "42")
                .run(&mut map, id, &mut NoIncludes, &mut diags);
        assert_eq!(out.text, "42");
        assert!(diags.is_empty());
    }

    #[test]
    fn unicode_is_preserved() {
        let src = "`define G λ\nwire `G; // ünïcode\n";
        let (_, _, out, diags) = run(src);
        assert!(diags.is_empty());
        assert_eq!(out.text, "\nwire λ; // ünïcode\n");
    }

    #[test]
    fn stray_backtick_forms() {
        let (_, _, _, diags) = run("`\"x `` y ` z");
        assert_eq!(
            messages(&diags),
            [
                "unexpected `\"` outside a macro body",
                "unexpected ``` `` ``` outside a macro body",
                "unexpected stray backtick",
            ]
        );
    }
}
