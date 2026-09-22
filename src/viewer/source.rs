//! Source listings: one page per file, one anchor per line.
//!
//! Every source location on a schematic or a reference page links here, to
//! `source/<n>-<file>.html#L<line>`, so "where did this flip-flop come
//! from" is one click and not a search through a checkout. The listing is
//! the file's own text, escaped: the viewer never highlights syntax, since
//! that would mean a second lexer to keep in step with the frontends.

use super::html::{Crumb, Html, page};
use crate::source::{SourceId, SourceMap};

/// Renders the listing of one file.
pub fn render(map: &SourceMap, file: SourceId) -> String {
    let source = map.file(file);
    let mut h = Html::new();
    h.open("h1", &[]);
    h.text(source.name());
    h.element(
        "span",
        &[("class", "kind")],
        &format!(" {} lines", source.line_count()),
    );
    h.close();

    h.open("ol", &[("class", "src")]);
    for line in 1..=source.line_count() {
        let id = format!("L{line}");
        h.open("li", &[("id", &id)]);
        h.text(source.line_text(line).unwrap_or(""));
        h.close();
    }
    h.close();

    let crumbs: Vec<Crumb> = vec![
        ("../index.html".to_owned(), "index".to_owned()),
        (String::new(), source.name().to_owned()),
    ];
    // No script: a listing has nothing to pan, zoom or highlight, so the
    // page carries only the style sheet.
    page(source.name(), &crumbs, &h.finish(), false, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_gets_an_anchor() {
        let mut map = SourceMap::new();
        let file = map.add("t.v", "module m;\n  // <b>\nendmodule\n").unwrap();
        let html = render(&map, file);
        assert!(html.contains("<li id=\"L1\">module m;</li>"));
        assert!(html.contains("<li id=\"L2\">  // &lt;b&gt;</li>"));
        assert!(html.contains("<li id=\"L3\">endmodule</li>"));
        assert!(html.contains("href=\"../index.html\""));
    }
}
