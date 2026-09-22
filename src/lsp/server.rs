//! The server loop, without any I/O.
//!
//! [`Server::handle`] takes one decoded JSON-RPC message and returns the
//! messages to send back: a response for a request, zero or more
//! notifications (the diagnostics), and nothing at all for a notification
//! it has no answer to. There is no reader, no writer, no thread and no
//! timer anywhere in it, which means a whole editing session is a `Vec` of
//! messages in and a `Vec` of messages out — exactly what
//! `tests/lsp_session.rs` drives. [`super::stdio`] is the thin adapter
//! that puts pipes around it.
//!
//! The lifecycle follows the specification: nothing but `initialize` is
//! answered before it arrives, `shutdown` stops the server answering
//! anything else, and `exit` sets [`Server::wants_exit`] for the transport
//! to act on. A request the server does not implement gets
//! `MethodNotFound` rather than silence, since a client is entitled to
//! know; an unknown *notification* is ignored, as the specification
//! requires.
//!
//! Diagnostics are published after every change to a document, and after
//! `didOpen` and `didSave`. There is no debouncing here on purpose: the
//! client decides when to send a change, and a server that sleeps is a
//! server that cannot be tested.

use super::analysis::{Document, DocumentStore, Language};
use super::features;
use super::json::Json;
use super::protocol::{
    Diagnostic, ErrorCode, InitializeParams, InitializeResult, Position, Range, TextDocumentItem,
    error_response, notification, response,
};

/// Where the server is in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// No `initialize` yet.
    Starting,
    /// Running normally.
    Running,
    /// `shutdown` received; only `exit` is still accepted.
    ShuttingDown,
    /// `exit` received.
    Exited,
}

/// A language server for Verilog and VHDL, driven one message at a time.
///
/// ```
/// use reticle::lsp::json::Json;
/// use reticle::lsp::protocol::request;
/// use reticle::lsp::server::Server;
///
/// let mut server = Server::new();
/// let out = server.handle(request(Json::Int(1), "initialize", Json::object()));
/// assert!(out[0].get("result").is_some());
/// ```
#[derive(Debug)]
pub struct Server {
    documents: DocumentStore,
    state: State,
    client: Option<String>,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// A server that has not been initialised yet.
    pub fn new() -> Server {
        Server {
            documents: DocumentStore::new(),
            state: State::Starting,
            client: None,
        }
    }

    /// The open documents, for a host that wants to look at them.
    pub fn documents(&self) -> &DocumentStore {
        &self.documents
    }

    /// The client's name, once `initialize` has said it.
    pub fn client_name(&self) -> Option<&str> {
        self.client.as_deref()
    }

    /// True once `exit` has been received; the transport then stops.
    pub fn wants_exit(&self) -> bool {
        self.state == State::Exited
    }

    /// Handles one message and returns what to send back, in order.
    pub fn handle(&mut self, message: Json) -> Vec<Json> {
        let Some(method) = message.get("method").and_then(Json::as_str) else {
            // A response to a request the server never sends; ignore it.
            return Vec::new();
        };
        let method = method.to_string();
        let params = message.get("params").cloned().unwrap_or(Json::Null);
        let id = message.get("id").cloned();

        match id {
            Some(id) if !id.is_null() => self.request(id, &method, &params),
            _ => self.notification(&method, &params),
        }
    }

    /// Handles a request and returns exactly one response, plus any
    /// notifications it triggered.
    fn request(&mut self, id: Json, method: &str, params: &Json) -> Vec<Json> {
        if method == "initialize" {
            if self.state != State::Starting {
                return vec![error_response(
                    id,
                    ErrorCode::InvalidRequest,
                    "the server is already initialized",
                )];
            }
            let params = InitializeParams::from_json(params);
            self.client = params.client_name;
            self.state = State::Running;
            return vec![response(id, self.initialize_result().to_json())];
        }
        if self.state == State::Starting {
            return vec![error_response(
                id,
                ErrorCode::ServerNotInitialized,
                "the server has not been initialized",
            )];
        }
        if self.state != State::Running {
            return vec![error_response(
                id,
                ErrorCode::InvalidRequest,
                "the server is shutting down",
            )];
        }
        match method {
            "shutdown" => {
                self.state = State::ShuttingDown;
                vec![response(id, Json::Null)]
            }
            "textDocument/definition" => {
                let result = self
                    .located(params)
                    .and_then(|(doc, position)| features::definition(doc, position))
                    .map_or(Json::Null, |location| location.to_json());
                vec![response(id, result)]
            }
            "textDocument/references" => {
                let include = params
                    .get("context")
                    .and_then(|c| c.get("includeDeclaration"))
                    .and_then(Json::as_bool)
                    .unwrap_or(true);
                let locations = self
                    .located(params)
                    .map(|(doc, position)| features::references(doc, position, include))
                    .unwrap_or_default();
                vec![response(
                    id,
                    Json::array(locations.iter().map(super::protocol::Location::to_json)),
                )]
            }
            "textDocument/hover" => {
                let result = self
                    .located(params)
                    .and_then(|(doc, position)| features::hover(doc, position))
                    .map_or(Json::Null, |hover| hover.to_json());
                vec![response(id, result)]
            }
            "textDocument/documentSymbol" => {
                let symbols = self
                    .document(params)
                    .map(features::document_symbols)
                    .unwrap_or_default();
                vec![response(
                    id,
                    Json::array(symbols.iter().map(super::protocol::DocumentSymbol::to_json)),
                )]
            }
            "textDocument/completion" => {
                let items = self
                    .located(params)
                    .map(|(doc, position)| features::completion(doc, position))
                    .unwrap_or_default();
                vec![response(
                    id,
                    Json::array(items.iter().map(super::protocol::CompletionItem::to_json)),
                )]
            }
            "textDocument/prepareRename" => {
                let result = self
                    .located(params)
                    .and_then(|(doc, position)| {
                        let offset = doc.offset(position);
                        let index = doc.index();
                        let decl = index.decl_at(offset)?;
                        if index.decls[decl].class.is_file_global() {
                            return None;
                        }
                        index.name_span_at(offset).map(|span| doc.range(span))
                    })
                    .map_or(Json::Null, Range::to_json);
                vec![response(id, result)]
            }
            "textDocument/rename" => vec![self.rename(id, params)],
            "textDocument/formatting" => vec![self.formatting(id, params, None)],
            "textDocument/rangeFormatting" => {
                let range = params.get("range").and_then(Range::from_json);
                vec![self.formatting(id, params, range)]
            }
            _ => vec![error_response(
                id,
                ErrorCode::MethodNotFound,
                format!("no handler for `{method}`"),
            )],
        }
    }

    /// Handles a notification, returning whatever it has to publish.
    fn notification(&mut self, method: &str, params: &Json) -> Vec<Json> {
        match method {
            "exit" => {
                self.state = State::Exited;
                Vec::new()
            }
            // Everything else is ignored until the handshake is done.
            _ if self.state == State::Starting => Vec::new(),
            "initialized" => Vec::new(),
            "textDocument/didOpen" => self.did_open(params),
            "textDocument/didChange" => self.did_change(params),
            "textDocument/didSave" => self.did_save(params),
            "textDocument/didClose" => self.did_close(params),
            // `$/` notifications and anything else the server does not
            // know are dropped, as the specification says to.
            _ => Vec::new(),
        }
    }

    /// What the server can do.
    fn initialize_result(&self) -> InitializeResult {
        InitializeResult {
            capabilities: Json::obj([
                // Spans are bytes inside Reticle and UTF-16 code units on
                // the wire; `super::protocol::LineIndex` converts. Saying
                // so keeps a client from negotiating UTF-8 and getting
                // offsets it cannot use.
                ("positionEncoding", Json::str("utf-16")),
                (
                    "textDocumentSync",
                    Json::obj([
                        ("openClose", Json::Bool(true)),
                        // 2 is incremental sync.
                        ("change", Json::Int(2)),
                        ("save", Json::obj([("includeText", Json::Bool(false))])),
                    ]),
                ),
                (
                    "completionProvider",
                    Json::obj([(
                        "triggerCharacters",
                        Json::array([Json::str("."), Json::str("(")]),
                    )]),
                ),
                ("hoverProvider", Json::Bool(true)),
                ("definitionProvider", Json::Bool(true)),
                ("referencesProvider", Json::Bool(true)),
                ("documentSymbolProvider", Json::Bool(true)),
                (
                    "renameProvider",
                    Json::obj([("prepareProvider", Json::Bool(true))]),
                ),
                ("documentFormattingProvider", Json::Bool(true)),
                ("documentRangeFormattingProvider", Json::Bool(true)),
            ]),
            name: "reticle".to_string(),
            version: crate::VERSION.to_string(),
        }
    }

    // --- document notifications -------------------------------------------

    fn did_open(&mut self, params: &Json) -> Vec<Json> {
        let Some(item) = params
            .get("textDocument")
            .and_then(TextDocumentItem::from_json)
        else {
            return Vec::new();
        };
        let Some(language) = Language::detect(&item.uri, &item.language_id) else {
            // Not a language this server knows; leave it to another one.
            return Vec::new();
        };
        let uri = item.uri.clone();
        self.documents
            .open(Document::new(item.uri, language, item.version, item.text));
        self.publish(&uri)
    }

    fn did_change(&mut self, params: &Json) -> Vec<Json> {
        let Some(uri) = document_uri(params) else {
            return Vec::new();
        };
        let version = params
            .get("textDocument")
            .and_then(|t| t.get("version"))
            .and_then(Json::as_i64);
        let Some(doc) = self.documents.get_mut(&uri) else {
            return Vec::new();
        };
        if let Some(version) = version {
            doc.version = version;
        }
        let changes = params
            .get("contentChanges")
            .and_then(Json::as_array)
            .map(<[Json]>::to_vec)
            .unwrap_or_default();
        for change in &changes {
            let Some(text) = change.get("text").and_then(Json::as_str) else {
                continue;
            };
            let range = change.get("range").and_then(Range::from_json);
            doc.apply_change(range, text);
        }
        self.publish(&uri)
    }

    fn did_save(&mut self, params: &Json) -> Vec<Json> {
        let Some(uri) = document_uri(params) else {
            return Vec::new();
        };
        // A client that was asked not to send the text still may.
        if let Some(text) = params.get("text").and_then(Json::as_str)
            && let Some(doc) = self.documents.get_mut(&uri)
        {
            doc.set_text(text.to_string());
        }
        self.publish(&uri)
    }

    fn did_close(&mut self, params: &Json) -> Vec<Json> {
        let Some(uri) = document_uri(params) else {
            return Vec::new();
        };
        if self.documents.close(&uri).is_none() {
            return Vec::new();
        }
        // Clearing the diagnostics is the client's cue to drop them; a
        // closed file must not keep squiggles in the problems panel.
        vec![publish_notification(&uri, None, &[])]
    }

    /// The `publishDiagnostics` notification for one document.
    fn publish(&self, uri: &str) -> Vec<Json> {
        let Some(doc) = self.documents.get(uri) else {
            return Vec::new();
        };
        vec![publish_notification(
            uri,
            Some(doc.version),
            doc.diagnostics(),
        )]
    }

    // --- requests that need more than a lookup ----------------------------

    fn rename(&mut self, id: Json, params: &Json) -> Json {
        let Some((doc, position)) = self.located(params) else {
            return error_response(id, ErrorCode::InvalidParams, "no such document is open");
        };
        let new_name = params
            .get("newName")
            .and_then(Json::as_str)
            .unwrap_or_default();
        match features::rename(&self.documents, doc, position, new_name) {
            Ok(edit) => response(id, edit.to_json()),
            Err(err) => error_response(id, ErrorCode::RequestFailed, err.message()),
        }
    }

    fn formatting(&mut self, id: Json, params: &Json, range: Option<Range>) -> Json {
        let Some(doc) = self.document(params) else {
            return error_response(id, ErrorCode::InvalidParams, "no such document is open");
        };
        let options = params.get("options");
        let opts = features::format_options(
            options
                .and_then(|o| o.get("tabSize"))
                .and_then(Json::as_u32),
            options
                .and_then(|o| o.get("insertSpaces"))
                .and_then(Json::as_bool),
        );
        let result = match range {
            None => features::formatting(doc, &opts),
            Some(range) => features::range_formatting(doc, range, &opts),
        };
        match result {
            Ok(edits) => response(
                id,
                Json::array(edits.iter().map(super::protocol::TextEdit::to_json)),
            ),
            Err(diags) => {
                let first = diags.iter().find(|d| d.is_error()).map_or_else(
                    || "the document cannot be formatted".to_string(),
                    |d| d.message.clone(),
                );
                error_response(
                    id,
                    ErrorCode::RequestFailed,
                    format!("not formatted: {first}"),
                )
            }
        }
    }

    // --- parameter lookup --------------------------------------------------

    /// The document a request names.
    fn document(&self, params: &Json) -> Option<&Document> {
        self.documents.get(&document_uri(params)?)
    }

    /// The document and position a request names.
    fn located(&self, params: &Json) -> Option<(&Document, Position)> {
        let doc = self.document(params)?;
        let position = Position::from_json(params.get("position")?)?;
        Some((doc, position))
    }
}

/// The `textDocument.uri` of a request or notification.
fn document_uri(params: &Json) -> Option<String> {
    Some(
        params
            .get("textDocument")?
            .get("uri")?
            .as_str()?
            .to_string(),
    )
}

/// Builds a `textDocument/publishDiagnostics` notification.
fn publish_notification(uri: &str, version: Option<i64>, diagnostics: &[Diagnostic]) -> Json {
    let mut params = Json::obj([("uri", Json::str(uri))]);
    if let Some(version) = version {
        params.set("version", Json::Int(version));
    }
    params.set(
        "diagnostics",
        Json::array(diagnostics.iter().map(Diagnostic::to_json)),
    );
    notification("textDocument/publishDiagnostics", params)
}

#[cfg(test)]
mod tests {
    use super::super::protocol::request;
    use super::*;

    const VERILOG: &str = "module m(input a, output y);\n  assign y = a;\nendmodule\n";

    fn initialized() -> Server {
        let mut server = Server::new();
        server.handle(request(Json::Int(0), "initialize", Json::object()));
        server.handle(notification("initialized", Json::object()));
        server
    }

    fn open(server: &mut Server, uri: &str, language_id: &str, text: &str) -> Vec<Json> {
        server.handle(notification(
            "textDocument/didOpen",
            Json::obj([(
                "textDocument",
                Json::obj([
                    ("uri", Json::str(uri)),
                    ("languageId", Json::str(language_id)),
                    ("version", Json::Int(1)),
                    ("text", Json::str(text)),
                ]),
            )]),
        ))
    }

    fn at(server: &Server, uri: &str, needle: &str) -> Json {
        let doc = server.documents().get(uri).expect("an open document");
        let offset = u32::try_from(doc.text().find(needle).expect("needle")).unwrap();
        doc.position(offset).to_json()
    }

    fn ask(server: &mut Server, method: &str, params: Json) -> Json {
        let out = server.handle(request(Json::Int(7), method, params));
        assert_eq!(out.len(), 1, "a request gets exactly one response");
        out.into_iter().next().unwrap()
    }

    fn result(message: &Json) -> Json {
        message.get("result").cloned().expect("a result")
    }

    fn text_document(uri: &str) -> Json {
        Json::obj([("textDocument", Json::obj([("uri", Json::str(uri))]))])
    }

    #[test]
    fn initialize_announces_the_capabilities() {
        let mut server = Server::new();
        let out = server.handle(request(
            Json::Int(1),
            "initialize",
            Json::obj([("clientInfo", Json::obj([("name", Json::str("tester"))]))]),
        ));
        assert_eq!(out.len(), 1);
        let caps = result(&out[0]);
        let caps = caps.get("capabilities").unwrap();
        assert_eq!(
            caps.get("positionEncoding").and_then(Json::as_str),
            Some("utf-16")
        );
        assert_eq!(caps.get("hoverProvider"), Some(&Json::Bool(true)));
        assert_eq!(
            caps.get("textDocumentSync")
                .and_then(|s| s.get("change"))
                .and_then(Json::as_i64),
            Some(2)
        );
        assert_eq!(server.client_name(), Some("tester"));
        assert_eq!(out[0].get("id"), Some(&Json::Int(1)));
    }

    #[test]
    fn requests_before_initialize_are_refused() {
        let mut server = Server::new();
        let out = server.handle(request(Json::Int(1), "textDocument/hover", Json::object()));
        assert_eq!(
            out[0].get("error").and_then(|e| e.get("code")),
            Some(&Json::Int(-32002))
        );
        // A notification before initialize is simply dropped.
        assert!(open(&mut server, "file:///a.v", "verilog", VERILOG).is_empty());
    }

    #[test]
    fn the_lifecycle_runs_to_exit() {
        let mut server = initialized();
        // A second initialize is an error.
        let out = server.handle(request(Json::Int(1), "initialize", Json::object()));
        assert!(out[0].get("error").is_some());

        let out = ask(&mut server, "shutdown", Json::Null);
        assert_eq!(result(&out), Json::Null);
        // After shutdown nothing else is answered.
        let out = ask(&mut server, "textDocument/hover", Json::object());
        assert_eq!(
            out.get("error").and_then(|e| e.get("code")),
            Some(&Json::Int(-32600))
        );
        assert!(!server.wants_exit());
        assert!(server.handle(notification("exit", Json::Null)).is_empty());
        assert!(server.wants_exit());
    }

    #[test]
    fn unknown_methods_and_malformed_messages() {
        let mut server = initialized();
        let out = ask(&mut server, "textDocument/codeLens", Json::object());
        assert_eq!(
            out.get("error").and_then(|e| e.get("code")),
            Some(&Json::Int(-32601))
        );
        // Unknown notifications are dropped.
        assert!(
            server
                .handle(notification("$/setTrace", Json::object()))
                .is_empty()
        );
        // So is a message with no method at all (a response to us).
        assert!(server.handle(Json::obj([("id", Json::Int(1))])).is_empty());
        assert!(server.handle(Json::Null).is_empty());
    }

    #[test]
    fn opening_a_document_publishes_diagnostics() {
        let mut server = initialized();
        let out = open(&mut server, "file:///a.v", "verilog", VERILOG);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].get("method").and_then(Json::as_str),
            Some("textDocument/publishDiagnostics")
        );
        let params = out[0].get("params").unwrap();
        assert_eq!(
            params.get("uri").and_then(Json::as_str),
            Some("file:///a.v")
        );
        assert_eq!(params.get("version").and_then(Json::as_i64), Some(1));
        assert_eq!(
            params
                .get("diagnostics")
                .and_then(Json::as_array)
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn a_document_of_another_language_is_left_alone() {
        let mut server = initialized();
        assert!(open(&mut server, "file:///a.py", "python", "print(1)").is_empty());
        assert_eq!(server.documents().len(), 0);
    }

    #[test]
    fn an_edit_introduces_an_error_and_then_fixes_it() {
        let mut server = initialized();
        open(&mut server, "file:///a.v", "verilog", VERILOG);

        // Break line 1 by replacing `a;` with `;`.
        let broken = "module m(input a, output y);\n  assign y = ;\nendmodule\n";
        let out = server.handle(notification(
            "textDocument/didChange",
            Json::obj([
                (
                    "textDocument",
                    Json::obj([("uri", Json::str("file:///a.v")), ("version", Json::Int(2))]),
                ),
                (
                    "contentChanges",
                    Json::array([Json::obj([("text", Json::str(broken))])]),
                ),
            ]),
        ));
        let diagnostics = out[0]
            .get("params")
            .and_then(|p| p.get("diagnostics"))
            .and_then(Json::as_array)
            .unwrap();
        assert!(!diagnostics.is_empty());
        assert_eq!(diagnostics[0].get("severity"), Some(&Json::Int(1)));
        assert_eq!(
            out[0]
                .get("params")
                .and_then(|p| p.get("version"))
                .and_then(Json::as_i64),
            Some(2)
        );

        // Put `a` back with an incremental change.
        let out = server.handle(notification(
            "textDocument/didChange",
            Json::obj([
                (
                    "textDocument",
                    Json::obj([("uri", Json::str("file:///a.v")), ("version", Json::Int(3))]),
                ),
                (
                    "contentChanges",
                    Json::array([Json::obj([
                        (
                            "range",
                            Range::new(Position::new(1, 13), Position::new(1, 13)).to_json(),
                        ),
                        ("text", Json::str("a")),
                    ])]),
                ),
            ]),
        ));
        let diagnostics = out[0]
            .get("params")
            .and_then(|p| p.get("diagnostics"))
            .and_then(Json::as_array)
            .unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            server.documents().get("file:///a.v").unwrap().text(),
            VERILOG
        );
    }

    #[test]
    fn closing_clears_the_diagnostics() {
        let mut server = initialized();
        open(
            &mut server,
            "file:///a.v",
            "verilog",
            "module m(;\nendmodule\n",
        );
        let out = server.handle(notification(
            "textDocument/didClose",
            text_document("file:///a.v"),
        ));
        assert_eq!(out.len(), 1);
        let params = out[0].get("params").unwrap();
        assert_eq!(
            params
                .get("diagnostics")
                .and_then(Json::as_array)
                .unwrap()
                .len(),
            0
        );
        assert!(params.get("version").is_none());
        assert_eq!(server.documents().len(), 0);
        // Closing again says nothing.
        assert!(
            server
                .handle(notification(
                    "textDocument/didClose",
                    text_document("file:///a.v")
                ))
                .is_empty()
        );
    }

    #[test]
    fn saving_republishes() {
        let mut server = initialized();
        open(&mut server, "file:///a.v", "verilog", VERILOG);
        let out = server.handle(notification(
            "textDocument/didSave",
            text_document("file:///a.v"),
        ));
        assert_eq!(out.len(), 1);
        // With the text included, the document is replaced.
        let mut params = text_document("file:///a.v");
        params.set("text", Json::str("module other; endmodule\n"));
        server.handle(notification("textDocument/didSave", params));
        assert!(
            server
                .documents()
                .get("file:///a.v")
                .unwrap()
                .text()
                .contains("other")
        );
    }

    #[test]
    fn the_features_answer_over_the_wire() {
        let mut server = initialized();
        open(&mut server, "file:///a.v", "verilog", VERILOG);
        let mut params = text_document("file:///a.v");
        params.set("position", at(&server, "file:///a.v", "a;"));

        let definition = result(&ask(&mut server, "textDocument/definition", params.clone()));
        assert_eq!(
            definition.get("uri").and_then(Json::as_str),
            Some("file:///a.v")
        );

        let hover = result(&ask(&mut server, "textDocument/hover", params.clone()));
        assert!(
            hover
                .get("contents")
                .and_then(|c| c.get("value"))
                .and_then(Json::as_str)
                .unwrap()
                .contains("input"),
        );

        let references = result(&ask(&mut server, "textDocument/references", params.clone()));
        assert_eq!(references.as_array().unwrap().len(), 2);

        let symbols = result(&ask(
            &mut server,
            "textDocument/documentSymbol",
            text_document("file:///a.v"),
        ));
        assert_eq!(symbols.as_array().unwrap().len(), 1);

        let completion = result(&ask(&mut server, "textDocument/completion", params.clone()));
        assert!(!completion.as_array().unwrap().is_empty());

        let prepare = result(&ask(
            &mut server,
            "textDocument/prepareRename",
            params.clone(),
        ));
        assert!(prepare.get("start").is_some());

        params.set("newName", Json::str("b"));
        let rename = result(&ask(&mut server, "textDocument/rename", params.clone()));
        let edits = rename
            .get("changes")
            .and_then(|c| c.get("file:///a.v"))
            .and_then(Json::as_array)
            .unwrap();
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn references_can_exclude_the_declaration() {
        let mut server = initialized();
        open(&mut server, "file:///a.v", "verilog", VERILOG);
        let mut params = text_document("file:///a.v");
        params.set("position", at(&server, "file:///a.v", "a;"));
        params.set(
            "context",
            Json::obj([("includeDeclaration", Json::Bool(false))]),
        );
        let references = result(&ask(&mut server, "textDocument/references", params));
        assert_eq!(references.as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_refused_rename_comes_back_as_an_error() {
        let mut server = initialized();
        open(&mut server, "file:///a.v", "verilog", VERILOG);
        let mut params = text_document("file:///a.v");
        params.set("position", at(&server, "file:///a.v", "m("));
        params.set("newName", Json::str("n"));
        let out = ask(&mut server, "textDocument/rename", params);
        let error = out.get("error").expect("an error");
        assert_eq!(error.get("code"), Some(&Json::Int(-32803)));
        assert!(
            error
                .get("message")
                .and_then(Json::as_str)
                .unwrap()
                .contains("module")
        );
        // `prepareRename` says so in advance.
        let mut params = text_document("file:///a.v");
        params.set("position", at(&server, "file:///a.v", "m("));
        assert_eq!(
            result(&ask(&mut server, "textDocument/prepareRename", params)),
            Json::Null
        );
    }

    #[test]
    fn formatting_and_range_formatting() {
        let mut server = initialized();
        // Line 1 is already formatted, so lines 0 and 2 are two hunks.
        open(
            &mut server,
            "file:///a.v",
            "verilog",
            "module   m ;\n  wire a;\n  wire   b;\nendmodule\n",
        );
        let mut params = text_document("file:///a.v");
        params.set(
            "options",
            Json::obj([
                ("tabSize", Json::Int(2)),
                ("insertSpaces", Json::Bool(true)),
            ]),
        );
        let edits = result(&ask(&mut server, "textDocument/formatting", params.clone()));
        let edits = edits.as_array().unwrap();
        assert_eq!(edits.len(), 2, "{edits:?}");

        // The range picks out the first of them.
        let mut ranged = params.clone();
        ranged.set(
            "range",
            Range::new(Position::new(0, 0), Position::new(0, 5)).to_json(),
        );
        let edits = result(&ask(&mut server, "textDocument/rangeFormatting", ranged));
        let edits = edits.as_array().unwrap();
        assert_eq!(edits.len(), 1);

        // The client's indentation is honoured.
        params.set(
            "options",
            Json::obj([
                ("tabSize", Json::Int(4)),
                ("insertSpaces", Json::Bool(true)),
            ]),
        );
        let edits = result(&ask(&mut server, "textDocument/formatting", params));
        assert!(edits.to_string().contains("    wire a;"), "{edits}");
    }

    #[test]
    fn formatting_a_broken_document_is_an_error() {
        let mut server = initialized();
        open(
            &mut server,
            "file:///a.v",
            "verilog",
            "module m(;\nendmodule\n",
        );
        let out = ask(
            &mut server,
            "textDocument/formatting",
            text_document("file:///a.v"),
        );
        assert!(
            out.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Json::as_str)
                .unwrap()
                .starts_with("not formatted:")
        );
    }

    #[test]
    fn requests_for_unopened_documents_are_harmless() {
        let mut server = initialized();
        let mut params = text_document("file:///missing.v");
        params.set("position", Position::new(0, 0).to_json());
        assert_eq!(
            result(&ask(&mut server, "textDocument/definition", params.clone())),
            Json::Null
        );
        assert_eq!(
            result(&ask(&mut server, "textDocument/hover", params.clone())),
            Json::Null
        );
        assert_eq!(
            result(&ask(&mut server, "textDocument/references", params.clone())),
            Json::Array(Vec::new())
        );
        params.set("newName", Json::str("x"));
        assert!(
            ask(&mut server, "textDocument/rename", params)
                .get("error")
                .is_some()
        );
        // A change to a document that was never opened is ignored.
        assert!(
            server
                .handle(notification(
                    "textDocument/didChange",
                    text_document("file:///missing.v")
                ))
                .is_empty()
        );
    }

    #[test]
    fn vhdl_goes_through_the_same_path() {
        let mut server = initialized();
        let src = "entity e is port (a : in bit; y : out bit); end entity;\narchitecture r of e is\nbegin\n  y <= a;\nend architecture;\n";
        let out = open(&mut server, "file:///a.vhd", "vhdl", src);
        assert_eq!(
            out[0]
                .get("params")
                .and_then(|p| p.get("diagnostics"))
                .and_then(Json::as_array)
                .unwrap()
                .len(),
            0
        );
        let mut params = text_document("file:///a.vhd");
        params.set("position", at(&server, "file:///a.vhd", "a;"));
        let hover = result(&ask(&mut server, "textDocument/hover", params));
        assert!(
            hover
                .get("contents")
                .and_then(|c| c.get("value"))
                .and_then(Json::as_str)
                .unwrap()
                .contains("bit")
        );
    }
}
