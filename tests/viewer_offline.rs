//! The two properties that make a generated page worth attaching to a bug
//! report: it is well formed, and it fetches nothing.
//!
//! *Well formed* is checked with a small scanner rather than a parser: it
//! walks the markup, treats `<script>` and `<style>` as raw text (which is
//! what HTML does), and checks that every element closes in the order it
//! opened. The viewer writes the XHTML-compatible subset — void elements
//! self-closed, every attribute quoted — so the check is a real one and
//! not a rubber stamp.
//!
//! *Fetches nothing* means no `http:`, no `https:`, no protocol-relative
//! `//host`, no `src=`, no `<link>`, no `@import` and no `url(`. That is
//! the property that makes the page work from `file://` on a machine with
//! no network, which is the whole point of inlining the style sheet and
//! the script.

#![cfg(feature = "viewer")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::ir::Design;
use reticle::source::SourceMap;
use reticle::viewer::{Site, ViewerOptions, render};

fn corpus() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in ["testdata/ir", "testdata/synth"] {
        let dir = root.join(dir);
        for entry in
            fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if path.is_file()
                && (name.ends_with(".cells.rtl") || dir.ends_with("ir"))
                && name.ends_with(".rtl")
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn site_of(path: &Path) -> Site {
    let text = fs::read_to_string(path).unwrap();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let mut map = SourceMap::new();
    let file = map.add(name, text.clone()).unwrap();
    let design = Design::parse_text(&text, file).expect("the corpus parses");
    render(&design, &map, &ViewerOptions::default())
}

#[test]
fn every_page_is_well_formed() {
    let corpus = corpus();
    assert!(!corpus.is_empty());
    for path in corpus {
        let site = site_of(&path);
        for (page, text) in &site.files {
            if let Err(problem) = check_nesting(text) {
                panic!("{}/{page}: {problem}", path.display());
            }
        }
    }
}

#[test]
fn no_page_reaches_outside_itself() {
    for path in corpus() {
        let site = site_of(&path);
        for (page, text) in &site.files {
            for needle in [
                "http://",
                "https://",
                "src=",
                "<link",
                "@import",
                "url(",
                "href=\"//",
                "integrity=",
                "<iframe",
                "<img",
            ] {
                assert!(
                    !text.contains(needle),
                    "{}/{page} contains `{needle}`, so it would not work offline",
                    path.display()
                );
            }
        }
    }
}

/// The elements that hold raw text until their closing tag, exactly as
/// the HTML parser treats them.
const RAW_TEXT: [&str; 2] = ["script", "style"];

/// Elements written self-closed by this generator; anything else must
/// close explicitly.
fn check_nesting(text: &str) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let Some(offset) = text[at..].find('<') else {
            break;
        };
        let start = at + offset;
        let rest = &text[start..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->").ok_or("unterminated comment")?;
            at = start + end + 3;
            continue;
        }
        if rest.starts_with("<!") {
            let end = rest.find('>').ok_or("unterminated declaration")?;
            at = start + end + 1;
            continue;
        }
        let end = rest.find('>').ok_or_else(|| {
            format!(
                "unterminated tag at byte {start}: {}",
                &rest[..rest.len().min(40)]
            )
        })?;
        let inner = &rest[1..end];
        at = start + end + 1;

        if let Some(name) = inner.strip_prefix('/') {
            let name = name.trim();
            match stack.pop() {
                Some((open, _)) if open == name => {}
                Some((open, opened_at)) => {
                    return Err(format!(
                        "`{name}` closes at byte {start} but `{open}`, opened at byte {opened_at}, is still open"
                    ));
                }
                None => return Err(format!("`{name}` closes at byte {start} with nothing open")),
            }
            continue;
        }

        let self_closing = inner.ends_with('/');
        let name: String = inner
            .split([' ', '\t', '\n', '/'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.is_empty() {
            return Err(format!("empty tag name at byte {start}"));
        }
        if !self_closing {
            if RAW_TEXT.contains(&name.as_str()) {
                // Raw text: skip to the matching close, so `<` and `&` in
                // a script or a style sheet are not markup.
                let close = format!("</{name}>");
                let end = text[at..]
                    .find(&close)
                    .ok_or_else(|| format!("unterminated <{name}>"))?;
                at += end + close.len();
                continue;
            }
            stack.push((name, start));
        }
    }
    match stack.pop() {
        None => Ok(()),
        Some((name, opened_at)) => Err(format!(
            "`{name}`, opened at byte {opened_at}, never closes"
        )),
    }
}

#[test]
fn the_nesting_check_catches_real_mistakes() {
    assert!(check_nesting("<p>hi</p>").is_ok());
    assert!(check_nesting("<!DOCTYPE html>\n<p><br />x</p>").is_ok());
    assert!(check_nesting("<!-- <b> -->").is_ok());
    assert!(check_nesting("<style>a > b { }</style>").is_ok());
    assert!(check_nesting("<script>if (a < b) { }</script>").is_ok());
    assert!(check_nesting("<p><b>x</p></b>").is_err());
    assert!(check_nesting("<p>x").is_err());
    assert!(check_nesting("x</p>").is_err());
    assert!(
        check_nesting("<p>x<").is_err(),
        "a stray `<` is an unterminated tag"
    );
    assert!(check_nesting("<p").is_err());
}

/// A net whose name looks like a tag must not become one, and the page
/// must still be well formed with it in.
#[test]
fn a_hostile_name_stays_text() {
    use reticle::ir::builder::ModuleBuilder;
    use reticle::ir::{Design, Type};
    use reticle::source::Span;

    let mut map = SourceMap::new();
    let file = map.add("t.v", "// <script>alert(1)</script>\n").unwrap();
    let span = Span::new(file, 0, 28);
    let mut b = ModuleBuilder::new("</title><script>", span);
    let a = b.input("<script>", Type::bit());
    let y = b.output("y\" onload=\"x", Type::bit());
    let av = b.net(a);
    let not = b.not(av);
    b.assign(y, not);
    let mut design = Design::new();
    let id = design.add_module(b.finish());
    design.top = Some(id);

    let site = render(&design, &map, &ViewerOptions::default());
    assert!(!site.is_empty());
    for (page, text) in &site.files {
        check_nesting(text).unwrap_or_else(|e| panic!("{page}: {e}"));
        assert!(
            !text.contains("<script>alert"),
            "{page} turned a name into a tag"
        );
        assert!(text.contains("&lt;script&gt;"), "{page}");
    }
}
