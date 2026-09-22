//! Discovered inputs: the files a unit of work read while it ran.
//!
//! A key is computed *before* the work runs, from what is known up front:
//! the source text and the options. Some inputs are only known once the
//! work has run. Synthesis turns `$readmemh("prog.hex", rom)` into the
//! memory's initial contents by reading `prog.hex` through
//! [`SynthOptions::files`](crate::synth::SynthOptions), and the name of
//! that file may be computed rather than written literally, so it cannot
//! be read off the source text, and the synthesised-module key is looked
//! up before elaboration, so it cannot be read off the IR either.
//!
//! The technique is the one build systems use for the same problem
//! (ccache's direct mode, Bazel's discovered inputs):
//!
//! 1. On a miss, the work runs against a [`Recorder`] that wraps the
//!    caller's [`FileProvider`] and logs every path read and a digest of
//!    what came back, or that nothing did.
//! 2. That list of [`FileInput`]s is stored in the entry's header, next to
//!    the artefact ([`Entry::inputs`](super::Entry)).
//! 3. A later lookup that finds the entry re-reads every recorded path
//!    through the provider it has *now* and compares ([`still_valid`]).
//!    A changed digest, a file that has gone, or a file that has appeared
//!    where there was none makes the lookup a miss. Only a full match is
//!    a hit.
//!
//! A file the work did not read cannot change its result, so a file that
//! was not recorded is never consulted, and editing it does not invalidate
//! anything.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use super::hash::{Hash128, hash128};
use crate::ir::memfile::FileProvider;

/// One file a unit of work read, and what it found.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileInput {
    /// The path exactly as the work asked for it.
    pub path: String,
    /// The digest of the text the provider returned, or `None` when the
    /// provider had no such file. "Not found" is a record of its own, so a
    /// file that appears later invalidates the entry.
    pub digest: Option<Hash128>,
}

impl FileInput {
    /// What the provider gives for `path` now, as a record.
    pub fn read(files: &dyn FileProvider, path: &str) -> FileInput {
        FileInput {
            path: path.to_owned(),
            digest: digest_of(files.read_file(path).as_deref()),
        }
    }

    /// The record as the rest of an entry-header line: the digest (or
    /// `missing`), a space, and the path with `\`, newline and carriage
    /// return escaped so that the line stays one line.
    pub fn encode(&self) -> String {
        let digest = self
            .digest
            .map_or_else(|| MISSING.to_owned(), |d| d.to_hex());
        format!("{digest} {}", escape(&self.path))
    }

    /// Reads back what [`FileInput::encode`] wrote.
    pub fn decode(text: &str) -> Option<FileInput> {
        let (digest, path) = text.split_once(' ')?;
        let digest = if digest == MISSING {
            None
        } else {
            Some(Hash128::from_hex(digest)?)
        };
        Some(FileInput {
            path: unescape(path)?,
            digest,
        })
    }
}

impl fmt::Display for FileInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

/// The word a header uses for a file that was not found.
const MISSING: &str = "missing";

/// The digest recorded for a file's text; `None` for no file.
fn digest_of(text: Option<&str>) -> Option<Hash128> {
    text.map(|t| hash128(t.as_bytes()))
}

fn escape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            _ => return None,
        }
    }
    Some(out)
}

/// True when every recorded file still reads the same through `files`.
///
/// A digest that differs, a file that was found and is now gone, and a
/// file that was not found and now exists all make it false.
pub fn still_valid(inputs: &[FileInput], files: &dyn FileProvider) -> bool {
    inputs
        .iter()
        .all(|input| digest_of(files.read_file(&input.path).as_deref()) == input.digest)
}

/// A [`FileProvider`] that passes every read through to another one and
/// remembers what it read.
///
/// Hand it to the work as its provider, then take the list with
/// [`Recorder::inputs`]. It is shared through an [`Rc`] because
/// [`SynthOptions::files`](crate::synth::SynthOptions) is one.
pub struct Recorder {
    inner: Rc<dyn FileProvider>,
    log: RefCell<BTreeMap<String, Option<Hash128>>>,
    unstable: Cell<bool>,
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recorder")
            .field("log", &self.log)
            .field("unstable", &self.unstable)
            .finish_non_exhaustive()
    }
}

impl Recorder {
    /// A recorder in front of `inner`.
    pub fn new(inner: Rc<dyn FileProvider>) -> Recorder {
        Recorder {
            inner,
            log: RefCell::new(BTreeMap::new()),
            unstable: Cell::new(false),
        }
    }

    /// Every path read so far, sorted by path, each once.
    ///
    /// `None` when one path was read twice and came back different, which
    /// means the provider changed under the work: its result reflects no
    /// single state of the files, so it must not be stored.
    pub fn inputs(&self) -> Option<Vec<FileInput>> {
        if self.unstable.get() {
            return None;
        }
        Some(
            self.log
                .borrow()
                .iter()
                .map(|(path, digest)| FileInput {
                    path: path.clone(),
                    digest: *digest,
                })
                .collect(),
        )
    }
}

impl FileProvider for Recorder {
    fn read_file(&self, path: &str) -> Option<String> {
        let text = self.inner.read_file(path);
        let digest = digest_of(text.as_deref());
        let mut log = self.log.borrow_mut();
        match log.get(path) {
            Some(seen) if *seen != digest => self.unstable.set(true),
            Some(_) => {}
            None => {
                log.insert(path.to_owned(), digest);
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::memfile::MemoryFiles;

    fn files(pairs: &[(&str, &str)]) -> MemoryFiles {
        let mut out = MemoryFiles::new();
        for (name, text) in pairs {
            out.insert(*name, *text);
        }
        out
    }

    #[test]
    fn a_recorder_logs_every_read_once_and_in_order() {
        let recorder = Recorder::new(Rc::new(files(&[("b.hex", "01\n"), ("a.hex", "02\n")])));
        assert_eq!(recorder.read_file("b.hex").as_deref(), Some("01\n"));
        assert_eq!(recorder.read_file("gone.hex"), None);
        assert_eq!(recorder.read_file("a.hex").as_deref(), Some("02\n"));
        assert_eq!(recorder.read_file("b.hex").as_deref(), Some("01\n"));
        let inputs = recorder.inputs().expect("stable");
        let paths: Vec<&str> = inputs.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, ["a.hex", "b.hex", "gone.hex"]);
        assert_eq!(inputs[0].digest, Some(hash128(b"02\n")));
        assert_eq!(inputs[2].digest, None, "not found is recorded as such");
    }

    #[test]
    fn a_file_that_changes_during_the_work_is_unstable() {
        struct Flip(Cell<u32>);
        impl FileProvider for Flip {
            fn read_file(&self, _: &str) -> Option<String> {
                self.0.set(self.0.get() + 1);
                Some(self.0.get().to_string())
            }
        }
        let recorder = Recorder::new(Rc::new(Flip(Cell::new(0))));
        recorder.read_file("x");
        recorder.read_file("x");
        assert_eq!(recorder.inputs(), None);
    }

    #[test]
    fn validity_follows_the_recorded_files_only() {
        let before = files(&[("rom.hex", "00\n"), ("other.hex", "11\n")]);
        let recorder = Recorder::new(Rc::new(before.clone()));
        recorder.read_file("rom.hex");
        recorder.read_file("opt.hex");
        let inputs = recorder.inputs().unwrap();
        assert!(still_valid(&inputs, &before));

        // An unrelated file changing does not matter.
        let unrelated = files(&[("rom.hex", "00\n"), ("other.hex", "22\n")]);
        assert!(still_valid(&inputs, &unrelated));
        // The recorded file changing does.
        let edited = files(&[("rom.hex", "01\n"), ("other.hex", "11\n")]);
        assert!(!still_valid(&inputs, &edited));
        // So does it disappearing,
        let deleted = files(&[("other.hex", "11\n")]);
        assert!(!still_valid(&inputs, &deleted));
        // and a file that was missing appearing, even empty.
        let appeared = files(&[("rom.hex", "00\n"), ("other.hex", "11\n"), ("opt.hex", "")]);
        assert!(!still_valid(&inputs, &appeared));
    }

    #[test]
    fn an_empty_file_is_not_a_missing_one() {
        let empty = FileInput::read(&files(&[("e", "")]), "e");
        let missing = FileInput::read(&files(&[]), "e");
        assert_ne!(empty, missing);
    }

    #[test]
    fn a_record_round_trips_through_its_text_form() {
        for input in [
            FileInput {
                path: "prog.hex".to_owned(),
                digest: Some(hash128(b"abc")),
            },
            FileInput {
                path: "odd name\\with\nnewline\r and spaces ".to_owned(),
                digest: None,
            },
            FileInput {
                path: String::new(),
                digest: Some(hash128(b"")),
            },
        ] {
            let text = input.encode();
            assert!(!text.contains('\n'), "{text:?}");
            assert_eq!(FileInput::decode(&text), Some(input));
        }
        assert_eq!(FileInput::decode("nonsense"), None);
        assert_eq!(FileInput::decode("missing bad\\escape"), None);
        assert_eq!(FileInput::decode("zz prog.hex"), None);
    }
}
