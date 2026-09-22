//! The Language Server Protocol wire format and the types Reticle uses.
//!
//! Three layers, from the bottom up.
//!
//! 1. **The base protocol.** Messages are `Content-Length: N\r\n\r\n`
//!    followed by `N` bytes of UTF-8 JSON. [`read_message`] and
//!    [`write_message`] are the whole of it; they take a reader and a
//!    writer so the library itself opens nothing.
//! 2. **JSON-RPC 2.0.** [`request`], [`response`], [`error_response`] and
//!    [`notification`] build the four message shapes, and [`ErrorCode`]
//!    names the codes LSP adds to the standard ones.
//! 3. **The LSP types.** Only the ones the server actually produces or
//!    consumes, as plain structs with `from_json` / `to_json`. Members LSP
//!    marks optional are `Option`, and a member Reticle never sets is
//!    simply absent from the output rather than `null`, since some clients
//!    distinguish the two.
//!
//! # Positions
//!
//! An LSP [`Position`] is a zero-based line and a **UTF-16 code unit**
//! offset within that line; the crate's [`Span`] is a pair of UTF-8 byte
//! offsets. They agree only on ASCII, so every conversion goes through
//! [`LineIndex`], which is built once per document revision and does the
//! arithmetic in both directions. Getting this wrong is the classic
//! language-server bug: it shows up as diagnostics that drift right on
//! lines holding an accented letter and as renames that corrupt them, so
//! the conversion is tested against non-ASCII text in both directions.
//!
//! Offsets that fall outside the text, or inside a character, are clamped
//! to the nearest boundary rather than rejected: a client that is a
//! keystroke ahead of the server must still get a sensible answer.

use std::io::{self, BufRead, Write};

use crate::source::{SourceId, Span};

use super::json::Json;

// ---------------------------------------------------------------------------
// The base protocol
// ---------------------------------------------------------------------------

/// The largest message the server will read, as a guard against a bad
/// `Content-Length` making it allocate wildly. Real LSP traffic is orders
/// of magnitude below this.
const MAX_CONTENT_LENGTH: usize = 64 * 1024 * 1024;

/// Reads one `Content-Length` framed JSON message.
///
/// Returns `Ok(None)` at a clean end of input. A malformed header or body
/// is an [`io::ErrorKind::InvalidData`] error, since the stream cannot be
/// resynchronised once framing is lost.
pub fn read_message(input: &mut impl BufRead) -> io::Result<Option<Json>> {
    let mut length: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        let read = input.read_line(&mut line)?;
        if read == 0 {
            return if length.is_some() {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "end of input inside a message header",
                ))
            } else {
                Ok(None)
            };
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid Content-Length")
            })?);
        }
        // Content-Type and any other header is accepted and ignored: the
        // only encoding LSP allows is UTF-8.
    }
    let Some(length) = length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message header without a Content-Length",
        ));
    };
    if length > MAX_CONTENT_LENGTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message is too large",
        ));
    }
    let mut body = vec![0u8; length];
    input.read_exact(&mut body)?;
    let text = String::from_utf8(body)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message body is not UTF-8"))?;
    Json::parse(&text)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// Writes one `Content-Length` framed JSON message and flushes.
pub fn write_message(out: &mut impl Write, message: &Json) -> io::Result<()> {
    let body = message.to_string();
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(body.as_bytes())?;
    out.flush()
}

// ---------------------------------------------------------------------------
// JSON-RPC
// ---------------------------------------------------------------------------

/// The JSON-RPC and LSP error codes the server can send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// The message was not valid JSON-RPC.
    InvalidRequest,
    /// No handler for the method.
    MethodNotFound,
    /// The parameters did not have the expected shape.
    InvalidParams,
    /// A request arrived before `initialize`.
    ServerNotInitialized,
    /// The request was refused for a reason the message explains, such as
    /// a rename the server cannot carry out safely.
    RequestFailed,
}

impl ErrorCode {
    /// The numeric code sent on the wire.
    pub fn code(self) -> i64 {
        match self {
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::ServerNotInitialized => -32002,
            ErrorCode::RequestFailed => -32803,
        }
    }
}

/// Builds a request.
pub fn request(id: Json, method: &str, params: Json) -> Json {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", id),
        ("method", Json::str(method)),
        ("params", params),
    ])
}

/// Builds a notification.
pub fn notification(method: &str, params: Json) -> Json {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("method", Json::str(method)),
        ("params", params),
    ])
}

/// Builds a successful response.
pub fn response(id: Json, result: Json) -> Json {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", id),
        ("result", result),
    ])
}

/// Builds an error response.
pub fn error_response(id: Json, code: ErrorCode, message: impl Into<String>) -> Json {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", id),
        (
            "error",
            Json::obj([
                ("code", Json::Int(code.code())),
                ("message", Json::str(message)),
            ]),
        ),
    ])
}

// ---------------------------------------------------------------------------
// Positions
// ---------------------------------------------------------------------------

/// A zero-based line and UTF-16 code unit offset inside that line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based offset in UTF-16 code units from the start of the line.
    pub character: u32,
}

impl Position {
    /// Builds a position.
    pub fn new(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    /// Reads a position, or `None` when the shape is wrong.
    pub fn from_json(value: &Json) -> Option<Position> {
        Some(Position {
            line: value.get("line")?.as_u32()?,
            character: value.get("character")?.as_u32()?,
        })
    }

    /// Writes the position.
    pub fn to_json(self) -> Json {
        Json::obj([
            ("line", Json::from(self.line)),
            ("character", Json::from(self.character)),
        ])
    }
}

/// A half-open range of [`Position`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    /// The first position covered.
    pub start: Position,
    /// One past the last position covered.
    pub end: Position,
}

impl Range {
    /// Builds a range.
    pub fn new(start: Position, end: Position) -> Range {
        Range { start, end }
    }

    /// Reads a range, or `None` when the shape is wrong.
    pub fn from_json(value: &Json) -> Option<Range> {
        Some(Range {
            start: Position::from_json(value.get("start")?)?,
            end: Position::from_json(value.get("end")?)?,
        })
    }

    /// Writes the range.
    pub fn to_json(self) -> Json {
        Json::obj([("start", self.start.to_json()), ("end", self.end.to_json())])
    }
}

/// The line starts of one document revision, for converting between byte
/// offsets and LSP positions.
///
/// Rebuilt whenever the text changes; building is one pass over the bytes.
#[derive(Clone, Debug)]
pub struct LineIndex {
    /// Byte offset at which each line begins; the first is always 0.
    line_starts: Vec<u32>,
    /// Length of the indexed text in bytes.
    len: u32,
}

impl LineIndex {
    /// Indexes `text`.
    ///
    /// A text longer than `u32::MAX` bytes is truncated to what spans can
    /// address, which is the same limit [`crate::source::SourceMap`] puts
    /// on a file.
    pub fn new(text: &str) -> LineIndex {
        let len = u32::try_from(text.len()).unwrap_or(u32::MAX);
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n'
                && let Ok(start) = u32::try_from(i + 1)
            {
                line_starts.push(start);
            }
        }
        LineIndex { line_starts, len }
    }

    /// Number of lines; always at least one.
    pub fn line_count(&self) -> u32 {
        u32::try_from(self.line_starts.len()).unwrap_or(u32::MAX)
    }

    /// The byte offset at which line `line` starts, clamped to the end of
    /// the text for a line past the last one.
    fn line_start(&self, line: u32) -> u32 {
        match usize::try_from(line) {
            Ok(i) => self.line_starts.get(i).copied().unwrap_or(self.len),
            Err(_) => self.len,
        }
    }

    /// The byte offset one past the end of line `line`, excluding the line
    /// terminator.
    fn line_end(&self, text: &str, line: u32) -> u32 {
        let next = match usize::try_from(line) {
            Ok(i) => self.line_starts.get(i + 1).copied().unwrap_or(self.len),
            Err(_) => self.len,
        };
        let start = self.line_start(line);
        let mut end = next.max(start);
        let bytes = text.as_bytes();
        while end > start {
            let Some(last) = usize::try_from(end - 1).ok().and_then(|i| bytes.get(i)) else {
                break;
            };
            if *last == b'\n' || *last == b'\r' {
                end -= 1;
            } else {
                break;
            }
        }
        end
    }

    /// The zero-based index of the line holding byte `offset`.
    fn line_of(&self, offset: u32) -> u32 {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        u32::try_from(line).unwrap_or(0)
    }

    /// Converts a byte offset into `text` to an LSP position.
    ///
    /// Offsets past the end clamp to the end of the text, and an offset
    /// inside a character clamps to that character's start.
    pub fn position(&self, text: &str, offset: u32) -> Position {
        let offset = offset.min(self.len);
        let line = self.line_of(offset);
        let start = self.line_start(line);
        let (Ok(from), Ok(to)) = (usize::try_from(start), usize::try_from(offset)) else {
            return Position::new(line, 0);
        };
        let slice = slice_on_boundaries(text, from, to);
        Position::new(line, utf16_len(slice))
    }

    /// Converts an LSP position into a byte offset into `text`.
    ///
    /// A line past the last one clamps to the end of the text, a character
    /// past the end of its line clamps to the end of that line (not to the
    /// next one), and a character landing inside a surrogate pair clamps
    /// down to the start of that character, as the specification asks.
    pub fn offset(&self, text: &str, position: Position) -> u32 {
        if position.line >= self.line_count() {
            return self.len;
        }
        let start = self.line_start(position.line);
        let end = self.line_end(text, position.line);
        let (Ok(from), Ok(to)) = (usize::try_from(start), usize::try_from(end)) else {
            return self.len;
        };
        let line_text = slice_on_boundaries(text, from, to);
        let mut units = 0u32;
        for (byte, ch) in line_text.char_indices() {
            let width = u32::try_from(ch.len_utf16()).unwrap_or(1);
            // `>` rather than `>=` so a position landing inside a
            // surrogate pair rounds down to the character that holds it.
            if units + width > position.character {
                return start + u32::try_from(byte).unwrap_or(0);
            }
            units += width;
        }
        end
    }

    /// Converts a byte span to an LSP range.
    pub fn range(&self, text: &str, span: Span) -> Range {
        Range::new(
            self.position(text, span.start),
            self.position(text, span.end),
        )
    }

    /// Converts an LSP range to a byte span in `file`.
    pub fn span(&self, text: &str, file: SourceId, range: Range) -> Span {
        Span::new(
            file,
            self.offset(text, range.start),
            self.offset(text, range.end),
        )
    }
}

/// The number of UTF-16 code units `text` encodes to.
pub fn utf16_len(text: &str) -> u32 {
    let mut units = 0u32;
    for ch in text.chars() {
        units += u32::try_from(ch.len_utf16()).unwrap_or(1);
    }
    units
}

/// `text[from..to]`, with both ends moved down to character boundaries.
///
/// Callers hand in offsets derived from the same text, so this only ever
/// matters when a client sent a position inside a character.
fn slice_on_boundaries(text: &str, from: usize, to: usize) -> &str {
    let mut from = from.min(text.len());
    let mut to = to.clamp(from, text.len());
    while from > 0 && !text.is_char_boundary(from) {
        from -= 1;
    }
    while to > from && !text.is_char_boundary(to) {
        to -= 1;
    }
    &text[from..to]
}

// ---------------------------------------------------------------------------
// LSP types
// ---------------------------------------------------------------------------

/// The `initialize` request parameters, reduced to what the server reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InitializeParams {
    /// The client's process id, when it gave one.
    pub process_id: Option<i64>,
    /// The workspace root, when the client is in one.
    pub root_uri: Option<String>,
    /// The client's name, for the log.
    pub client_name: Option<String>,
}

impl InitializeParams {
    /// Reads the parameters; every member is optional, so this cannot fail.
    pub fn from_json(value: &Json) -> InitializeParams {
        InitializeParams {
            process_id: value.get("processId").and_then(Json::as_i64),
            root_uri: value
                .get("rootUri")
                .and_then(Json::as_str)
                .map(str::to_string),
            client_name: value
                .get("clientInfo")
                .and_then(|c| c.get("name"))
                .and_then(Json::as_str)
                .map(str::to_string),
        }
    }
}

/// The `initialize` response: what the server can do, and who it is.
#[derive(Clone, Debug, PartialEq)]
pub struct InitializeResult {
    /// The `capabilities` object.
    pub capabilities: Json,
    /// The server's name.
    pub name: String,
    /// The server's version.
    pub version: String,
}

impl InitializeResult {
    /// Writes the result.
    pub fn to_json(&self) -> Json {
        Json::obj([
            ("capabilities", self.capabilities.clone()),
            (
                "serverInfo",
                Json::obj([
                    ("name", Json::str(self.name.clone())),
                    ("version", Json::str(self.version.clone())),
                ]),
            ),
        ])
    }
}

/// A document the client opened, as sent with `textDocument/didOpen`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDocumentItem {
    /// The document's URI.
    pub uri: String,
    /// The client's language identifier, such as `verilog` or `vhdl`.
    pub language_id: String,
    /// The revision this text belongs to.
    pub version: i64,
    /// The full text.
    pub text: String,
}

impl TextDocumentItem {
    /// Reads the item, or `None` when a required member is missing.
    pub fn from_json(value: &Json) -> Option<TextDocumentItem> {
        Some(TextDocumentItem {
            uri: value.get("uri")?.as_str()?.to_string(),
            language_id: value
                .get("languageId")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string(),
            version: value.get("version").and_then(Json::as_i64).unwrap_or(0),
            text: value.get("text")?.as_str()?.to_string(),
        })
    }

    /// Writes the item.
    pub fn to_json(&self) -> Json {
        Json::obj([
            ("uri", Json::str(self.uri.clone())),
            ("languageId", Json::str(self.language_id.clone())),
            ("version", Json::Int(self.version)),
            ("text", Json::str(self.text.clone())),
        ])
    }
}

/// A range inside a document, identified by URI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// The document.
    pub uri: String,
    /// The range inside it.
    pub range: Range,
}

impl Location {
    /// Builds a location.
    pub fn new(uri: impl Into<String>, range: Range) -> Location {
        Location {
            uri: uri.into(),
            range,
        }
    }

    /// Reads a location, or `None` when the shape is wrong.
    pub fn from_json(value: &Json) -> Option<Location> {
        Some(Location {
            uri: value.get("uri")?.as_str()?.to_string(),
            range: Range::from_json(value.get("range")?)?,
        })
    }

    /// Writes the location.
    pub fn to_json(&self) -> Json {
        Json::obj([
            ("uri", Json::str(self.uri.clone())),
            ("range", self.range.to_json()),
        ])
    }
}

/// How serious a diagnostic is, in LSP's numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    /// The document is invalid.
    Error,
    /// Suspicious, but still valid.
    Warning,
    /// Informational.
    Information,
    /// A hint, usually rendered unobtrusively.
    Hint,
}

impl DiagnosticSeverity {
    /// The number sent on the wire.
    pub fn code(self) -> i64 {
        match self {
            DiagnosticSeverity::Error => 1,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Information => 3,
            DiagnosticSeverity::Hint => 4,
        }
    }
}

/// One problem reported against a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Where the problem is.
    pub range: Range,
    /// How serious it is.
    pub severity: DiagnosticSeverity,
    /// The rule name or diagnostic code, when there is one.
    pub code: Option<String>,
    /// Which part of Reticle reported it.
    pub source: String,
    /// The message, including any notes the renderer would have shown.
    pub message: String,
    /// Secondary locations, shown as related information.
    pub related: Vec<(Location, String)>,
}

impl Diagnostic {
    /// Writes the diagnostic.
    pub fn to_json(&self) -> Json {
        let mut out = Json::obj([
            ("range", self.range.to_json()),
            ("severity", Json::Int(self.severity.code())),
        ]);
        if let Some(code) = &self.code {
            out.set("code", Json::str(code.clone()));
        }
        out.set("source", Json::str(self.source.clone()));
        out.set("message", Json::str(self.message.clone()));
        if !self.related.is_empty() {
            out.set(
                "relatedInformation",
                Json::array(self.related.iter().map(|(loc, message)| {
                    Json::obj([
                        ("location", loc.to_json()),
                        ("message", Json::str(message.clone())),
                    ])
                })),
            );
        }
        out
    }
}

/// The completion item kinds Reticle uses, in LSP's numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompletionItemKind {
    /// A function or a VHDL function.
    Function,
    /// A port of the module or entity being instantiated.
    Field,
    /// A net, signal or variable.
    Variable,
    /// A module, entity or package.
    Module,
    /// A parameter, generic or constant.
    Constant,
    /// A type or subtype.
    Struct,
    /// A reserved word.
    Keyword,
    /// An instance label or a process label.
    Unit,
    /// An enumeration literal.
    EnumMember,
}

impl CompletionItemKind {
    /// The number sent on the wire.
    pub fn code(self) -> i64 {
        match self {
            CompletionItemKind::Function => 3,
            CompletionItemKind::Field => 5,
            CompletionItemKind::Variable => 6,
            CompletionItemKind::Module => 9,
            CompletionItemKind::Constant => 21,
            CompletionItemKind::Struct => 22,
            CompletionItemKind::Keyword => 14,
            CompletionItemKind::Unit => 11,
            CompletionItemKind::EnumMember => 20,
        }
    }
}

/// One completion candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    /// The text shown and, unless `insert_text` says otherwise, inserted.
    pub label: String,
    /// What kind of thing it is.
    pub kind: CompletionItemKind,
    /// A short type or signature, shown next to the label.
    pub detail: Option<String>,
    /// The key the client sorts by, which is how the server keeps its own
    /// grouping (ports before locals before keywords).
    pub sort_text: Option<String>,
}

impl CompletionItem {
    /// Builds an item with no detail and no sort key.
    pub fn new(label: impl Into<String>, kind: CompletionItemKind) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            kind,
            detail: None,
            sort_text: None,
        }
    }

    /// Writes the item.
    pub fn to_json(&self) -> Json {
        let mut out = Json::obj([
            ("label", Json::str(self.label.clone())),
            ("kind", Json::Int(self.kind.code())),
        ]);
        if let Some(detail) = &self.detail {
            out.set("detail", Json::str(detail.clone()));
        }
        if let Some(sort) = &self.sort_text {
            out.set("sortText", Json::str(sort.clone()));
        }
        out
    }
}

/// What to show when the pointer rests on a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    /// Markdown contents.
    pub contents: String,
    /// The range the hover describes, so the client can highlight it.
    pub range: Option<Range>,
}

impl Hover {
    /// Writes the hover.
    pub fn to_json(&self) -> Json {
        let mut out = Json::obj([(
            "contents",
            Json::obj([
                ("kind", Json::str("markdown")),
                ("value", Json::str(self.contents.clone())),
            ]),
        )]);
        if let Some(range) = self.range {
            out.set("range", range.to_json());
        }
        out
    }
}

/// A replacement of one range by some text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    /// The range replaced.
    pub range: Range,
    /// What replaces it.
    pub new_text: String,
}

impl TextEdit {
    /// Builds an edit.
    pub fn new(range: Range, new_text: impl Into<String>) -> TextEdit {
        TextEdit {
            range,
            new_text: new_text.into(),
        }
    }

    /// Writes the edit.
    pub fn to_json(&self) -> Json {
        Json::obj([
            ("range", self.range.to_json()),
            ("newText", Json::str(self.new_text.clone())),
        ])
    }
}

/// Edits to one or more documents, as a `rename` returns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceEdit {
    /// The edits per document URI, in the order they were added.
    pub changes: Vec<(String, Vec<TextEdit>)>,
}

impl WorkspaceEdit {
    /// Adds the edits for one document.
    pub fn add(&mut self, uri: impl Into<String>, edits: Vec<TextEdit>) {
        self.changes.push((uri.into(), edits));
    }

    /// Writes the edit.
    pub fn to_json(&self) -> Json {
        Json::obj([(
            "changes",
            Json::obj(self.changes.iter().map(|(uri, edits)| {
                (
                    uri.clone(),
                    Json::array(edits.iter().map(TextEdit::to_json)),
                )
            })),
        )])
    }
}

/// The symbol kinds Reticle reports, in LSP's numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SymbolKind {
    /// A Verilog module or a VHDL entity.
    Module,
    /// A VHDL package.
    Package,
    /// A VHDL architecture or a SystemVerilog interface.
    Namespace,
    /// A port or generic.
    Field,
    /// A function.
    Function,
    /// A net, signal or variable.
    Variable,
    /// A parameter, generic or constant.
    Constant,
    /// A type declaration.
    Struct,
    /// An enumeration literal.
    EnumMember,
    /// An instance.
    Object,
    /// A process, `always` block or generate block.
    Event,
}

impl SymbolKind {
    /// The number sent on the wire.
    pub fn code(self) -> i64 {
        match self {
            SymbolKind::Module => 2,
            SymbolKind::Namespace => 3,
            SymbolKind::Package => 4,
            SymbolKind::Field => 8,
            SymbolKind::Function => 12,
            SymbolKind::Variable => 13,
            SymbolKind::Constant => 14,
            SymbolKind::Object => 19,
            SymbolKind::EnumMember => 22,
            SymbolKind::Struct => 23,
            SymbolKind::Event => 24,
        }
    }
}

/// One node of the document symbol tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentSymbol {
    /// The symbol's name.
    pub name: String,
    /// A short description, such as the declared type.
    pub detail: Option<String>,
    /// What kind of symbol it is.
    pub kind: SymbolKind,
    /// The whole declaration, including its body.
    pub range: Range,
    /// The name alone, which is what the client reveals.
    pub selection_range: Range,
    /// Nested symbols, in source order.
    pub children: Vec<DocumentSymbol>,
}

impl DocumentSymbol {
    /// Writes the symbol and, recursively, its children.
    pub fn to_json(&self) -> Json {
        let mut out = Json::obj([("name", Json::str(self.name.clone()))]);
        if let Some(detail) = &self.detail {
            out.set("detail", Json::str(detail.clone()));
        }
        out.set("kind", Json::Int(self.kind.code()));
        out.set("range", self.range.to_json());
        out.set("selectionRange", self.selection_range.to_json());
        if !self.children.is_empty() {
            out.set(
                "children",
                Json::array(self.children.iter().map(DocumentSymbol::to_json)),
            );
        }
        out
    }

    /// Flattens the tree into [`SymbolInformation`], for a client that
    /// does not support the hierarchical form.
    pub fn flatten(&self, uri: &str, container: Option<&str>, out: &mut Vec<SymbolInformation>) {
        out.push(SymbolInformation {
            name: self.name.clone(),
            kind: self.kind,
            location: Location::new(uri, self.range),
            container_name: container.map(str::to_string),
        });
        for child in &self.children {
            child.flatten(uri, Some(&self.name), out);
        }
    }
}

/// The flat form of a document symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolInformation {
    /// The symbol's name.
    pub name: String,
    /// What kind of symbol it is.
    pub kind: SymbolKind,
    /// Where it is.
    pub location: Location,
    /// The name of the symbol that encloses it.
    pub container_name: Option<String>,
}

impl SymbolInformation {
    /// Writes the symbol.
    pub fn to_json(&self) -> Json {
        let mut out = Json::obj([
            ("name", Json::str(self.name.clone())),
            ("kind", Json::Int(self.kind.code())),
            ("location", self.location.to_json()),
        ]);
        if let Some(container) = &self.container_name {
            out.set("containerName", Json::str(container.clone()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A text with a 2-byte character (é), a 3-byte one (中) and a 4-byte
    /// one (😀, two UTF-16 code units), on three lines.
    const MIXED: &str = "wire é_a;\nwire 中_b;\n// 😀 tail\n";

    fn index() -> LineIndex {
        LineIndex::new(MIXED)
    }

    #[test]
    fn ascii_positions_are_byte_positions() {
        let idx = index();
        assert_eq!(idx.position(MIXED, 0), Position::new(0, 0));
        assert_eq!(idx.position(MIXED, 4), Position::new(0, 4));
        assert_eq!(idx.offset(MIXED, Position::new(0, 4)), 4);
    }

    #[test]
    fn two_byte_characters_count_as_one_unit() {
        let idx = index();
        let e_acute = u32::try_from(MIXED.find('é').unwrap()).unwrap();
        // "wire " is five characters, so é is at character 5 and the byte
        // after it is at character 6 although it is two bytes further on.
        assert_eq!(idx.position(MIXED, e_acute), Position::new(0, 5));
        assert_eq!(idx.position(MIXED, e_acute + 2), Position::new(0, 6));
        assert_eq!(idx.offset(MIXED, Position::new(0, 5)), e_acute);
        assert_eq!(idx.offset(MIXED, Position::new(0, 6)), e_acute + 2);
    }

    #[test]
    fn three_byte_characters_count_as_one_unit() {
        let idx = index();
        let zhong = u32::try_from(MIXED.find('中').unwrap()).unwrap();
        assert_eq!(idx.position(MIXED, zhong), Position::new(1, 5));
        assert_eq!(idx.position(MIXED, zhong + 3), Position::new(1, 6));
        assert_eq!(idx.offset(MIXED, Position::new(1, 6)), zhong + 3);
    }

    #[test]
    fn astral_characters_count_as_two_units() {
        let idx = index();
        let emoji = u32::try_from(MIXED.find('😀').unwrap()).unwrap();
        // "// " is three characters, then the emoji takes two units.
        assert_eq!(idx.position(MIXED, emoji), Position::new(2, 3));
        assert_eq!(idx.position(MIXED, emoji + 4), Position::new(2, 5));
        assert_eq!(idx.offset(MIXED, Position::new(2, 3)), emoji);
        assert_eq!(idx.offset(MIXED, Position::new(2, 5)), emoji + 4);
        // A position inside the surrogate pair rounds down to its start.
        assert_eq!(idx.offset(MIXED, Position::new(2, 4)), emoji);
    }

    #[test]
    fn round_trips_every_boundary() {
        let idx = index();
        for (byte, _) in MIXED.char_indices() {
            let offset = u32::try_from(byte).unwrap();
            let pos = idx.position(MIXED, offset);
            assert_eq!(idx.offset(MIXED, pos), offset, "at byte {byte}");
        }
    }

    #[test]
    fn clamps_out_of_range_positions() {
        let idx = index();
        let len = u32::try_from(MIXED.len()).unwrap();
        // Past the end of the text.
        assert_eq!(idx.offset(MIXED, Position::new(99, 0)), len);
        assert_eq!(idx.position(MIXED, 9999), idx.position(MIXED, len));
        // Past the end of a line stops at the line end, not the next line.
        let line0_end = u32::try_from(MIXED.find('\n').unwrap()).unwrap();
        assert_eq!(idx.offset(MIXED, Position::new(0, 99)), line0_end);
        // An offset inside a character clamps down.
        let e_acute = u32::try_from(MIXED.find('é').unwrap()).unwrap();
        assert_eq!(idx.position(MIXED, e_acute + 1), Position::new(0, 5));
    }

    #[test]
    fn handles_crlf_and_empty_text() {
        let text = "a\r\nb\r\n";
        let idx = LineIndex::new(text);
        assert_eq!(idx.line_count(), 3);
        assert_eq!(idx.offset(text, Position::new(0, 9)), 1);
        assert_eq!(idx.position(text, 3), Position::new(1, 0));

        let empty = LineIndex::new("");
        assert_eq!(empty.line_count(), 1);
        assert_eq!(empty.offset("", Position::new(0, 0)), 0);
        assert_eq!(empty.position("", 5), Position::new(0, 0));
    }

    #[test]
    fn spans_and_ranges_convert() {
        let mut map = crate::source::SourceMap::new();
        let file = map.add("t.v", MIXED).unwrap();
        let idx = index();
        let start = u32::try_from(MIXED.find('é').unwrap()).unwrap();
        let span = Span::new(file, start, start + 4);
        let range = idx.range(MIXED, span);
        assert_eq!(range, Range::new(Position::new(0, 5), Position::new(0, 8)));
        assert_eq!(idx.span(MIXED, file, range), span);
    }

    #[test]
    fn utf16_len_counts_units() {
        assert_eq!(utf16_len(""), 0);
        assert_eq!(utf16_len("abc"), 3);
        assert_eq!(utf16_len("é中"), 2);
        assert_eq!(utf16_len("😀"), 2);
    }

    #[test]
    fn frames_messages() {
        let message = request(Json::Int(1), "initialize", Json::object());
        let mut buf = Vec::new();
        write_message(&mut buf, &message).unwrap();
        let text = String::from_utf8(buf.clone()).unwrap();
        assert!(text.starts_with("Content-Length: "));
        assert!(text.contains("\r\n\r\n"));

        let mut input = Cursor::new(buf);
        assert_eq!(read_message(&mut input).unwrap(), Some(message));
        assert_eq!(read_message(&mut input).unwrap(), None);
    }

    #[test]
    fn reads_headers_case_insensitively_and_ignores_extras() {
        let body = r#"{"jsonrpc":"2.0","method":"exit"}"#;
        let framed = format!(
            "content-length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{body}",
            body.len()
        );
        let mut input = Cursor::new(framed.into_bytes());
        let message = read_message(&mut input).unwrap().unwrap();
        assert_eq!(message.get("method").and_then(Json::as_str), Some("exit"));
    }

    #[test]
    fn counts_bytes_not_characters() {
        // "é" is two bytes: a length in characters would desynchronise the
        // stream, so the framing must count bytes.
        let message = notification("x", Json::str("é😀"));
        let mut buf = Vec::new();
        write_message(&mut buf, &message).unwrap();
        let text = String::from_utf8(buf.clone()).unwrap();
        let (header, body) = text.split_once("\r\n\r\n").unwrap();
        let length: usize = header.split(':').nth(1).unwrap().trim().parse().unwrap();
        assert_eq!(length, body.len());
        assert_eq!(read_message(&mut Cursor::new(buf)).unwrap(), Some(message));
    }

    #[test]
    fn rejects_broken_framing() {
        for text in [
            "Content-Length: nope\r\n\r\n{}",
            "X-Other: 1\r\n\r\n{}",
            "Content-Length: 5\r\n\r\n{}",
            "Content-Length: 2\r\n\r\n{,",
        ] {
            let mut input = Cursor::new(text.as_bytes().to_vec());
            assert!(read_message(&mut input).is_err(), "{text:?}");
        }
    }

    #[test]
    fn lsp_types_round_trip_through_json() {
        let pos = Position::new(3, 7);
        assert_eq!(Position::from_json(&pos.to_json()), Some(pos));
        let range = Range::new(pos, Position::new(4, 0));
        assert_eq!(Range::from_json(&range.to_json()), Some(range));
        let loc = Location::new("file:///a.v", range);
        assert_eq!(Location::from_json(&loc.to_json()), Some(loc.clone()));
        let item = TextDocumentItem {
            uri: "file:///a.v".to_string(),
            language_id: "verilog".to_string(),
            version: 2,
            text: "module m; endmodule".to_string(),
        };
        assert_eq!(TextDocumentItem::from_json(&item.to_json()), Some(item));
        assert_eq!(Position::from_json(&Json::Null), None);
        assert_eq!(Range::from_json(&Json::object()), None);
    }

    #[test]
    fn writes_optional_members_only_when_set() {
        let plain = CompletionItem::new("wire", CompletionItemKind::Keyword);
        assert_eq!(plain.to_json().to_string(), r#"{"label":"wire","kind":14}"#);
        let detailed = CompletionItem {
            detail: Some("logic [7:0]".to_string()),
            sort_text: Some("1wire".to_string()),
            ..CompletionItem::new("wire", CompletionItemKind::Variable)
        };
        assert_eq!(
            detailed.to_json().to_string(),
            r#"{"label":"wire","kind":6,"detail":"logic [7:0]","sortText":"1wire"}"#
        );
        let hover = Hover {
            contents: "**x**".to_string(),
            range: None,
        };
        assert_eq!(
            hover.to_json().to_string(),
            r#"{"contents":{"kind":"markdown","value":"**x**"}}"#
        );
    }

    #[test]
    fn diagnostics_and_edits_serialise() {
        let range = Range::new(Position::new(0, 0), Position::new(0, 4));
        let diag = Diagnostic {
            range,
            severity: DiagnosticSeverity::Warning,
            code: Some("unused-signal".to_string()),
            source: "reticle".to_string(),
            message: "unused".to_string(),
            related: vec![(Location::new("file:///a.v", range), "declared here".into())],
        };
        let json = diag.to_json();
        assert_eq!(json.get("severity"), Some(&Json::Int(2)));
        assert_eq!(
            json.get("code").and_then(Json::as_str),
            Some("unused-signal")
        );
        assert!(json.get("relatedInformation").is_some());

        let mut edit = WorkspaceEdit::default();
        edit.add("file:///a.v", vec![TextEdit::new(range, "y")]);
        assert_eq!(
            edit.to_json().to_string(),
            r#"{"changes":{"file:///a.v":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}},"newText":"y"}]}}"#
        );
    }

    #[test]
    fn document_symbols_nest_and_flatten() {
        let range = Range::new(Position::new(0, 0), Position::new(5, 0));
        let child = DocumentSymbol {
            name: "clk".to_string(),
            detail: Some("input".to_string()),
            kind: SymbolKind::Field,
            range,
            selection_range: range,
            children: Vec::new(),
        };
        let root = DocumentSymbol {
            name: "top".to_string(),
            detail: None,
            kind: SymbolKind::Module,
            range,
            selection_range: range,
            children: vec![child],
        };
        let json = root.to_json();
        assert_eq!(json.get("kind"), Some(&Json::Int(2)));
        assert_eq!(
            json.get("children").and_then(Json::as_array).unwrap().len(),
            1
        );
        assert!(json.get("detail").is_none());

        let mut flat = Vec::new();
        root.flatten("file:///a.v", None, &mut flat);
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[1].container_name.as_deref(), Some("top"));
        assert_eq!(
            flat[1]
                .to_json()
                .get("containerName")
                .and_then(Json::as_str),
            Some("top")
        );
    }

    #[test]
    fn jsonrpc_shapes() {
        assert_eq!(
            response(Json::Int(3), Json::Null).to_string(),
            r#"{"jsonrpc":"2.0","id":3,"result":null}"#
        );
        assert_eq!(
            error_response(Json::str("a"), ErrorCode::MethodNotFound, "no").to_string(),
            r#"{"jsonrpc":"2.0","id":"a","error":{"code":-32601,"message":"no"}}"#
        );
        assert_eq!(
            notification("x", Json::Null).to_string(),
            r#"{"jsonrpc":"2.0","method":"x","params":null}"#
        );
        assert_eq!(ErrorCode::ServerNotInitialized.code(), -32002);
        assert_eq!(ErrorCode::InvalidRequest.code(), -32600);
        assert_eq!(ErrorCode::InvalidParams.code(), -32602);
        assert_eq!(ErrorCode::RequestFailed.code(), -32803);
    }

    #[test]
    fn initialize_params_and_result() {
        let params = InitializeParams::from_json(
            &Json::parse(
                r#"{"processId":42,"rootUri":"file:///w","clientInfo":{"name":"tester"}}"#,
            )
            .unwrap(),
        );
        assert_eq!(params.process_id, Some(42));
        assert_eq!(params.root_uri.as_deref(), Some("file:///w"));
        assert_eq!(params.client_name.as_deref(), Some("tester"));
        assert_eq!(
            InitializeParams::from_json(&Json::Null),
            InitializeParams::default()
        );

        let result = InitializeResult {
            capabilities: Json::obj([("hoverProvider", Json::Bool(true))]),
            name: "reticle".to_string(),
            version: "0".to_string(),
        };
        assert_eq!(
            result.to_json().to_string(),
            r#"{"capabilities":{"hoverProvider":true},"serverInfo":{"name":"reticle","version":"0"}}"#
        );
    }
}
