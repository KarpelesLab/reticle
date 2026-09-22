//! The transport: `Content-Length` framed JSON-RPC over a byte stream.
//!
//! This is the only part of the language server that reads or writes
//! anything, and it does not open anything itself either: the caller hands
//! in a reader and a writer, which for the `reticle lsp` command are the
//! process's standard input and output. Everything else — the parsing, the
//! document store, the features — is driven through
//! [`Server::handle`](super::server::Server::handle) and is testable
//! without a pipe in sight.
//!
//! The loop is deliberately unadorned: read a message, hand it to the
//! server, write what comes back, stop at `exit` or at the end of input.
//! There is no concurrency, so a slow request delays the next one; for a
//! parse measured in milliseconds that is the right trade, and it keeps
//! the ordering guarantees the protocol asks for without a scheduler.

use std::io::{self, BufRead, Write};

use super::protocol::{read_message, write_message};
use super::server::Server;

/// Runs a fresh server over `input` and `output` until it exits.
///
/// Returns when the client sends `exit` or closes the stream. A message
/// that is not framed JSON-RPC ends the session with an error, since the
/// stream cannot be resynchronised once framing is lost.
pub fn serve(input: &mut impl BufRead, output: &mut impl Write) -> io::Result<()> {
    serve_with(&mut Server::new(), input, output)
}

/// Runs `server` over `input` and `output` until it exits.
///
/// Takes the server so that a host can inspect it afterwards, or start one
/// that is already configured.
pub fn serve_with(
    server: &mut Server,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<()> {
    while let Some(message) = read_message(input)? {
        for reply in server.handle(message) {
            write_message(output, &reply)?;
        }
        if server.wants_exit() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::json::Json;
    use crate::lsp::protocol::{notification, request};
    use std::io::Cursor;

    /// Frames a sequence of messages the way a client would send them.
    fn framed(messages: &[Json]) -> Vec<u8> {
        let mut buf = Vec::new();
        for message in messages {
            write_message(&mut buf, message).unwrap();
        }
        buf
    }

    /// Reads back every message of a framed stream.
    fn unframe(bytes: Vec<u8>) -> Vec<Json> {
        let mut input = Cursor::new(bytes);
        let mut out = Vec::new();
        while let Some(message) = read_message(&mut input).unwrap() {
            out.push(message);
        }
        out
    }

    #[test]
    fn runs_a_session_over_a_stream() {
        let input = framed(&[
            request(Json::Int(1), "initialize", Json::object()),
            notification("initialized", Json::object()),
            notification(
                "textDocument/didOpen",
                Json::obj([(
                    "textDocument",
                    Json::obj([
                        ("uri", Json::str("file:///a.v")),
                        ("languageId", Json::str("verilog")),
                        ("version", Json::Int(1)),
                        ("text", Json::str("module m;\nendmodule\n")),
                    ]),
                )]),
            ),
            request(Json::Int(2), "shutdown", Json::Null),
            notification("exit", Json::Null),
        ]);
        let mut output = Vec::new();
        let mut server = Server::new();
        serve_with(&mut server, &mut Cursor::new(input), &mut output).unwrap();
        assert!(server.wants_exit());

        let replies = unframe(output);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0].get("id"), Some(&Json::Int(1)));
        assert_eq!(
            replies[1].get("method").and_then(Json::as_str),
            Some("textDocument/publishDiagnostics")
        );
        assert_eq!(replies[2].get("id"), Some(&Json::Int(2)));
    }

    #[test]
    fn stops_at_the_end_of_input() {
        let input = framed(&[request(Json::Int(1), "initialize", Json::object())]);
        let mut output = Vec::new();
        let mut server = Server::new();
        serve_with(&mut server, &mut Cursor::new(input), &mut output).unwrap();
        assert!(!server.wants_exit());
        assert_eq!(unframe(output).len(), 1);
    }

    #[test]
    fn stops_reading_after_exit() {
        // Anything after `exit` is not read, let alone acted on.
        let input = framed(&[
            request(Json::Int(1), "initialize", Json::object()),
            notification("exit", Json::Null),
            request(Json::Int(2), "shutdown", Json::Null),
        ]);
        let mut output = Vec::new();
        serve(&mut Cursor::new(input), &mut output).unwrap();
        let replies = unframe(output);
        assert_eq!(replies.len(), 1);
    }

    #[test]
    fn a_broken_frame_ends_the_session() {
        let mut output = Vec::new();
        let err = serve(
            &mut Cursor::new(b"Content-Length: oops\r\n\r\n{}".to_vec()),
            &mut output,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
