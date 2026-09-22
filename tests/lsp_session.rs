//! Golden tests for whole language-server sessions.
//!
//! Each `testdata/lsp/<name>.session` is a script of client messages; the
//! test replays it against a fresh [`Server`], writes down every message
//! the server sent back, and compares the transcript with
//! `testdata/lsp/<name>.expected`. Set `UPDATE_EXPECT=1` to rewrite the
//! expectations after an intended change.
//!
//! The server is sans-I/O, so a session is just a fold over
//! [`Server::handle`]; nothing is framed, no pipe is opened and no time
//! passes. The framing and the stream are covered by the unit tests of
//! `reticle::lsp::stdio`.
//!
//! # The script format
//!
//! One directive per line; `#` starts a comment and blank lines are
//! ignored. Sources live next to the script as ordinary `.v` / `.vhd`
//! files so they can be read (and linted, and formatted) on their own.
//!
//! ```text
//! open    <uri> <languageId> <file>            textDocument/didOpen
//! replace <uri> <version> <file>               didChange, whole text
//! edit    <uri> <version> <l> <c> <l> <c> <json-string>
//!                                              didChange, one range
//! save    <uri>                                textDocument/didSave
//! close   <uri>                                textDocument/didClose
//! ask     <id> <method> <uri> <line> <char> [json]
//!                                              a request with a position
//! doc     <id> <method> <uri> [json]           a request with no position
//! request <id> <method> <json>                 a raw request
//! notify  <method> <json>                      a raw notification
//! ```
//!
//! Lines and characters are zero-based, as they are on the wire.

#![cfg(feature = "lsp")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::lsp::json::Json;
use reticle::lsp::protocol::{notification, request};
use reticle::lsp::server::Server;

/// Line endings are normalised so a CRLF checkout compares equal.
fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/lsp")
}

/// Reads a source file of the corpus.
fn source(dir: &Path, name: &str) -> String {
    normalise(
        &fs::read_to_string(dir.join(name))
            .unwrap_or_else(|e| panic!("reading {}: {e}", dir.join(name).display())),
    )
}

/// Splits a directive into at most `n` fields plus the rest of the line.
fn fields(line: &str, n: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line.trim();
    for _ in 0..n {
        match rest.split_once(char::is_whitespace) {
            Some((head, tail)) => {
                out.push(head);
                rest = tail.trim_start();
            }
            None => {
                out.push(rest);
                rest = "";
                break;
            }
        }
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

/// Parses the optional trailing JSON object of a directive and merges it
/// into `params`.
fn merge(params: &mut Json, extra: Option<&str>) {
    let Some(extra) = extra else { return };
    let extra = Json::parse(extra).unwrap_or_else(|e| panic!("bad JSON `{extra}`: {e}"));
    for (key, value) in extra.as_object().unwrap_or_default() {
        params.set(key, value.clone());
    }
}

/// A `textDocument` parameter object.
fn text_document(uri: &str) -> Json {
    Json::obj([("textDocument", Json::obj([("uri", Json::str(uri))]))])
}

/// A `position` object.
fn position(line: &str, character: &str) -> Json {
    Json::obj([
        ("line", Json::Int(line.parse().expect("a line number"))),
        (
            "character",
            Json::Int(character.parse().expect("a character offset")),
        ),
    ])
}

/// Turns one directive into the message it stands for.
fn message(dir: &Path, line: &str) -> Json {
    let parts = fields(line, 8);
    match parts.as_slice() {
        ["open", uri, language_id, file] => notification(
            "textDocument/didOpen",
            Json::obj([(
                "textDocument",
                Json::obj([
                    ("uri", Json::str(*uri)),
                    ("languageId", Json::str(*language_id)),
                    ("version", Json::Int(1)),
                    ("text", Json::str(source(dir, file))),
                ]),
            )]),
        ),
        ["replace", uri, version, file] => notification(
            "textDocument/didChange",
            Json::obj([
                (
                    "textDocument",
                    Json::obj([
                        ("uri", Json::str(*uri)),
                        ("version", Json::Int(version.parse().expect("a version"))),
                    ]),
                ),
                (
                    "contentChanges",
                    Json::array([Json::obj([("text", Json::str(source(dir, file)))])]),
                ),
            ]),
        ),
        ["edit", uri, version, sl, sc, el, ec, text] => notification(
            "textDocument/didChange",
            Json::obj([
                (
                    "textDocument",
                    Json::obj([
                        ("uri", Json::str(*uri)),
                        ("version", Json::Int(version.parse().expect("a version"))),
                    ]),
                ),
                (
                    "contentChanges",
                    Json::array([Json::obj([
                        (
                            "range",
                            Json::obj([("start", position(sl, sc)), ("end", position(el, ec))]),
                        ),
                        (
                            "text",
                            Json::parse(text).expect("the replacement is a JSON string"),
                        ),
                    ])]),
                ),
            ]),
        ),
        ["save", uri] => notification("textDocument/didSave", text_document(uri)),
        ["close", uri] => notification("textDocument/didClose", text_document(uri)),
        ["ask", id, method, uri, line, character, rest @ ..] => {
            let mut params = text_document(uri);
            params.set("position", position(line, character));
            merge(&mut params, rest.first().copied());
            request(Json::Int(id.parse().expect("an id")), method, params)
        }
        ["doc", id, method, uri, rest @ ..] => {
            let mut params = text_document(uri);
            merge(&mut params, rest.first().copied());
            request(Json::Int(id.parse().expect("an id")), method, params)
        }
        ["request", id, method, params] => request(
            Json::Int(id.parse().expect("an id")),
            method,
            Json::parse(params).expect("valid JSON params"),
        ),
        ["notify", method, params] => {
            notification(method, Json::parse(params).expect("valid JSON params"))
        }
        ["notify", method] => notification(method, Json::Null),
        other => panic!("unknown directive: {other:?}"),
    }
}

/// Replays one script and returns the transcript.
fn run(path: &Path) -> String {
    let dir = path.parent().expect("a corpus directory");
    let script = normalise(&fs::read_to_string(path).expect("read the script"));
    let mut server = Server::new();
    let mut out = String::new();
    for line in script.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        out.push_str("> ");
        out.push_str(trimmed);
        out.push('\n');
        for reply in server.handle(message(dir, trimmed)) {
            out.push_str("< ");
            out.push_str(&reply.to_pretty_string().replace('\n', "\n  "));
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// Every `.session` file of the corpus, sorted.
fn cases(dir: &Path) -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = fs::read_dir(dir)
        .expect("corpus directory")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("session"))
        .collect();
    cases.sort();
    cases
}

#[test]
fn golden() {
    let dir = corpus();
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let cases = cases(&dir);
    assert!(!cases.is_empty(), "no sessions in {}", dir.display());

    let mut failures = Vec::new();
    for case in &cases {
        let actual = run(case);
        let expect_path = case.with_extension("expected");
        let expected = fs::read_to_string(&expect_path).map(|t| normalise(&t));
        match expected {
            Ok(expected) if expected == actual => {}
            _ if update => {
                fs::write(&expect_path, &actual).expect("write expectation");
                eprintln!("updated {}", expect_path.display());
            }
            Ok(expected) => {
                let line = expected
                    .lines()
                    .zip(actual.lines())
                    .position(|(a, b)| a != b)
                    .map_or(
                        expected.lines().count().min(actual.lines().count()) + 1,
                        |i| i + 1,
                    );
                failures.push(format!(
                    "{}: mismatch at line {line}\n  expected: {}\n  actual:   {}",
                    case.display(),
                    expected.lines().nth(line - 1).unwrap_or("<end>"),
                    actual.lines().nth(line - 1).unwrap_or("<end>"),
                ));
            }
            Err(_) => failures.push(format!(
                "{}: missing expectation {} (run with UPDATE_EXPECT=1)",
                case.display(),
                expect_path.display()
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{} session(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The corpus must exercise both front ends, or a regression in one of
/// them would go unnoticed.
#[test]
fn the_corpus_covers_both_languages() {
    let dir = corpus();
    let scripts: String = cases(&dir)
        .iter()
        .map(|case| fs::read_to_string(case).expect("read the script"))
        .collect();
    assert!(scripts.contains(" verilog "), "no Verilog session");
    assert!(scripts.contains(" vhdl "), "no VHDL session");
    assert!(scripts.contains("textDocument/rename"), "no rename session");
}
