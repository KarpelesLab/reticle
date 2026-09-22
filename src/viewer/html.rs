//! The renderer both views share: escaping, a tag-stack document builder,
//! and the page chrome (style sheet, header, optional script).
//!
//! Everything a page needs is inlined into it. There is no stylesheet
//! link, no font import and no script tag with a `src`, because a page
//! that cannot be opened from `file://` is useless in a bug report. The
//! style sheet and the script below are therefore the only two assets,
//! and they are string constants pasted into every page.
//!
//! [`Html`] keeps a stack of open tags so a document cannot be built with
//! mismatched nesting: [`Html::close`] closes whatever is innermost, and
//! [`Html::finish`] panics if anything is still open, which is an internal
//! invariant rather than a user-facing error. Markup is written in the
//! XHTML-compatible subset — every void element is self-closed and every
//! attribute has a quoted value — so the well-formedness test in
//! `tests/viewer_offline.rs` can check nesting with a plain tag scanner.

use std::fmt::Write as _;

/// Escapes text for an HTML text node or a quoted attribute value.
///
/// The five characters that can end a text node or an attribute value are
/// replaced, so a net called `<script>` renders as text and never as a
/// tag.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A document under construction, with an indentation level and a stack of
/// open tags.
#[derive(Debug, Default)]
pub struct Html {
    buf: String,
    stack: Vec<String>,
    /// True when the last thing written was text, so the closing tag stays
    /// on the same line.
    inline: bool,
}

impl Html {
    /// An empty document.
    pub fn new() -> Self {
        Html::default()
    }

    /// The nesting depth, i.e. the number of tags still open.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    fn newline(&mut self) {
        self.buf.push('\n');
        for _ in 0..self.stack.len() {
            self.buf.push_str("  ");
        }
    }

    fn tag(&mut self, tag: &str, attrs: &[(&str, &str)], close: bool) {
        self.buf.push('<');
        self.buf.push_str(tag);
        for (name, value) in attrs {
            let _ = write!(self.buf, " {}=\"{}\"", name, escape(value));
        }
        if close {
            self.buf.push_str(" /");
        }
        self.buf.push('>');
    }

    /// Opens an element; it stays open until [`Html::close`].
    pub fn open(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        if !self.buf.is_empty() {
            self.newline();
        }
        self.tag(tag, attrs, false);
        self.stack.push(tag.to_owned());
        self.inline = false;
    }

    /// Closes the innermost open element.
    ///
    /// # Panics
    ///
    /// Panics when nothing is open, which is a bug in the caller.
    pub fn close(&mut self) {
        let tag = self.stack.pop().expect("close without an open element");
        if !self.inline {
            self.newline();
        }
        let _ = write!(self.buf, "</{tag}>");
        self.inline = false;
    }

    /// Writes a void (self-closing) element.
    pub fn void(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        self.newline();
        self.tag(tag, attrs, true);
        self.inline = false;
    }

    /// Appends escaped text to the element being built.
    pub fn text(&mut self, text: &str) {
        self.buf.push_str(&escape(text));
        self.inline = true;
    }

    /// Appends already-rendered markup verbatim.
    ///
    /// Only used for fragments this module produced; nothing derived from
    /// a design ever reaches it unescaped.
    pub fn raw(&mut self, markup: &str) {
        self.buf.push_str(markup);
        self.inline = false;
    }

    /// Writes a complete element with a single text child.
    pub fn element(&mut self, tag: &str, attrs: &[(&str, &str)], text: &str) {
        self.open(tag, attrs);
        self.text(text);
        self.close();
    }

    /// The markup built so far.
    ///
    /// # Panics
    ///
    /// Panics when an element is still open.
    pub fn finish(self) -> String {
        assert!(self.stack.is_empty(), "unclosed elements: {:?}", self.stack);
        self.buf
    }
}

/// One entry of a page's breadcrumb trail: a link target and its label.
///
/// An empty target renders as plain text, which is how the current page
/// marks itself.
pub type Crumb = (String, String);

/// Wraps a body fragment in the page chrome.
///
/// `title` becomes the document title, `crumbs` the breadcrumb trail, and
/// `script` decides whether the interaction script is inlined (only the
/// schematic needs it). `wide` drops the reading-width cap, which the
/// schematic also wants.
pub fn page(title: &str, crumbs: &[Crumb], body: &str, script: bool, wide: bool) -> String {
    let mut out = String::with_capacity(body.len() + STYLE.len() + 1024);
    out.push_str("<!DOCTYPE html>\n");
    let mut h = Html::new();
    h.open("html", &[("lang", "en")]);
    h.open("head", &[]);
    h.void("meta", &[("charset", "utf-8")]);
    h.void(
        "meta",
        &[
            ("name", "viewport"),
            ("content", "width=device-width, initial-scale=1"),
        ],
    );
    h.element("title", &[], title);
    h.open("style", &[]);
    h.raw(STYLE);
    h.close();
    h.close();
    h.open("body", &[]);
    h.open("header", &[("class", "top")]);
    h.open("nav", &[("class", "crumbs")]);
    for (i, (href, label)) in crumbs.iter().enumerate() {
        if i > 0 {
            h.element("span", &[("class", "sep")], "/");
        }
        if href.is_empty() {
            h.element("span", &[("class", "here")], label);
        } else {
            h.element("a", &[("href", href)], label);
        }
    }
    h.close();
    h.element("span", &[("class", "brand")], "reticle viewer");
    h.close();
    h.open("main", &[("class", if wide { "wide" } else { "narrow" })]);
    h.raw(body);
    h.close();
    if script {
        h.open("script", &[]);
        h.raw(SCRIPT);
        h.close();
    }
    h.close();
    h.close();
    out.push_str(&h.finish());
    out.push('\n');
    out
}

/// The style sheet inlined into every page.
///
/// Colours are custom properties so the dark variant is one block, and
/// nothing here loads a font: the stacks name families a machine already
/// has.
pub const STYLE: &str = "
:root {
  --bg: #fbfbfa; --fg: #1d2021; --dim: #6b7378; --line: #d6d9dc;
  --panel: #ffffff; --accent: #1d6fb8; --comb: #eef3f8; --comb-line: #7a9cbb;
  --seq: #fdf0e0; --seq-line: #c6852a; --port: #e9f4ec; --port-line: #4c8f63;
  --inst: #f1edf8; --inst-line: #7c6bab; --wire: #7c8790; --hot: #d1440a;
  --mono: ui-monospace, 'DejaVu Sans Mono', Menlo, Consolas, monospace;
  --sans: system-ui, -apple-system, 'Segoe UI', 'DejaVu Sans', sans-serif;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #16191c; --fg: #e4e6e8; --dim: #97a0a6; --line: #333a40;
    --panel: #1d2125; --accent: #6db3f2; --comb: #23303b; --comb-line: #5b87ad;
    --seq: #382b1b; --seq-line: #d29a45; --port: #1e3327; --port-line: #5ea678;
    --inst: #2a2440; --inst-line: #9385c9; --wire: #79858e; --hot: #ff7a45;
  }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg);
  font-family: var(--sans); font-size: 15px; line-height: 1.5; }
header.top { display: flex; justify-content: space-between; align-items: baseline;
  gap: 1rem; padding: 0.6rem 1rem; border-bottom: 1px solid var(--line);
  background: var(--panel); position: sticky; top: 0; z-index: 2; }
.crumbs { font-family: var(--mono); font-size: 13px; }
.crumbs .sep { color: var(--dim); margin: 0 0.35rem; }
.crumbs .here { font-weight: 600; }
.brand { color: var(--dim); font-size: 12px; letter-spacing: 0.08em; text-transform: uppercase; }
main { padding: 1rem; margin: 0 auto; }
main.narrow { max-width: 70rem; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
h1 { font-size: 1.5rem; margin: 0.2rem 0 0.4rem; }
h2 { font-size: 1.1rem; margin: 1.6rem 0 0.4rem; border-bottom: 1px solid var(--line);
  padding-bottom: 0.2rem; }
h1 .kind, .loc { color: var(--dim); font-weight: 400; font-size: 0.8rem; font-family: var(--mono); }
p.desc { margin: 0.4rem 0 1rem; max-width: 46rem; white-space: pre-wrap; }
p.none, td.none { color: var(--dim); font-style: italic; }
table { border-collapse: collapse; width: 100%; font-size: 14px; }
th, td { text-align: left; padding: 0.3rem 0.6rem; border-bottom: 1px solid var(--line);
  vertical-align: top; }
th { color: var(--dim); font-weight: 600; font-size: 12px; text-transform: uppercase;
  letter-spacing: 0.04em; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
code, td.mono { font-family: var(--mono); font-size: 13px; }
ul.tree { list-style: none; padding-left: 1.1rem; margin: 0.2rem 0; border-left: 1px solid var(--line); }
ul.tree li { padding: 0.1rem 0; font-family: var(--mono); font-size: 13px; }
.toolbar { display: flex; gap: 0.5rem; align-items: center; flex-wrap: wrap; margin-bottom: 0.5rem; }
.toolbar input { font: inherit; font-size: 13px; padding: 0.25rem 0.5rem;
  border: 1px solid var(--line); border-radius: 4px; background: var(--panel);
  color: var(--fg); min-width: 14rem; }
.toolbar button { font: inherit; font-size: 13px; padding: 0.25rem 0.6rem;
  border: 1px solid var(--line); border-radius: 4px; background: var(--panel);
  color: var(--fg); cursor: pointer; }
.toolbar .hint { color: var(--dim); font-size: 12px; }
.canvas { display: flex; gap: 0.75rem; align-items: stretch; flex-wrap: wrap; }
.frame { flex: 1 1 28rem; border: 1px solid var(--line); border-radius: 6px;
  background: var(--panel); overflow: hidden; }
svg.schematic { display: block; width: 100%; height: 72vh; touch-action: none; cursor: grab; }
aside.info { flex: 0 1 17rem; border: 1px solid var(--line); border-radius: 6px;
  background: var(--panel); padding: 0.6rem 0.8rem; font-size: 13px; overflow: auto;
  max-height: 72vh; }
aside.info h3 { margin: 0 0 0.4rem; font-size: 0.95rem; font-family: var(--mono); }
aside.info dl { margin: 0; display: grid; grid-template-columns: auto 1fr; gap: 0.15rem 0.6rem; }
aside.info dt { color: var(--dim); font-size: 12px; }
aside.info dd { margin: 0; font-family: var(--mono); font-size: 12px; overflow-wrap: anywhere; }
.legend { display: flex; gap: 0.9rem; flex-wrap: wrap; color: var(--dim); font-size: 12px;
  margin-top: 0.5rem; }
.legend span::before { content: ''; display: inline-block; width: 0.7rem; height: 0.7rem;
  margin-right: 0.3rem; border: 1px solid var(--line); vertical-align: -1px; }
.legend .l-comb::before { background: var(--comb); border-color: var(--comb-line); }
.legend .l-seq::before { background: var(--seq); border-color: var(--seq-line); }
.legend .l-port::before { background: var(--port); border-color: var(--port-line); }
.legend .l-inst::before { background: var(--inst); border-color: var(--inst-line); }
.box { fill: var(--comb); stroke: var(--comb-line); stroke-width: 1; }
.seq .box { fill: var(--seq); stroke: var(--seq-line); stroke-width: 2; }
.port .box { fill: var(--port); stroke: var(--port-line); }
.inst .box { fill: var(--inst); stroke: var(--inst-line); stroke-width: 1.5; }
.kind { fill: var(--fg); font-family: var(--mono); font-size: 12px; font-weight: 600; }
.label { fill: var(--dim); font-family: var(--mono); font-size: 10px; }
.pinname { fill: var(--dim); font-family: var(--mono); font-size: 9px; }
.pin { fill: var(--wire); stroke: none; }
.wire { fill: none; stroke: var(--wire); stroke-width: 1.4; stroke-linejoin: round; }
.wire.bus { stroke-width: 2.6; }
.clkmark { fill: none; stroke: var(--seq-line); stroke-width: 1.4; }
.node { cursor: pointer; }
polyline.hl { stroke: var(--hot); }
circle.hl { fill: var(--hot); }
text.hl { fill: var(--hot); }
.hl-node .box { stroke: var(--hot); }
.found .box { stroke: var(--hot); stroke-dasharray: 4 2; }
.sel .box { stroke: var(--accent); stroke-width: 2.5; }
ol.src { font-family: var(--mono); font-size: 12px; background: var(--panel);
  border: 1px solid var(--line); border-radius: 6px; padding: 0.6rem 0.6rem 0.6rem 4rem;
  overflow-x: auto; }
ol.src li { white-space: pre; padding: 0 0.3rem; }
ol.src li:target { background: var(--comb); outline: 1px solid var(--comb-line); }
";

/// The interaction script inlined into schematic pages.
///
/// Hand-written and dependency-free: a transform for pan and zoom,
/// attribute-driven highlighting so no net name is ever interpolated into
/// a selector, and numeric ids for the detail panels for the same reason.
pub const SCRIPT: &str = r#"
(function () {
  var svg = document.getElementById('schematic');
  if (!svg) { return; }
  var view = document.getElementById('view');
  var info = document.getElementById('info');
  var tx = 0, ty = 0, scale = 1;
  function apply() {
    view.setAttribute('transform', 'translate(' + tx.toFixed(2) + ' ' + ty.toFixed(2) +
      ') scale(' + scale.toFixed(4) + ')');
  }
  function zoom(factor, cx, cy) {
    var next = Math.min(8, Math.max(0.05, scale * factor));
    factor = next / scale;
    tx = cx - (cx - tx) * factor;
    ty = cy - (cy - ty) * factor;
    scale = next;
    apply();
  }
  function centre() {
    var r = svg.getBoundingClientRect();
    return [r.width / 2, r.height / 2];
  }
  var down = false, lastX = 0, lastY = 0, moved = 0;
  svg.addEventListener('pointerdown', function (e) {
    down = true; moved = 0; lastX = e.clientX; lastY = e.clientY;
    svg.setPointerCapture(e.pointerId); svg.style.cursor = 'grabbing';
  });
  svg.addEventListener('pointermove', function (e) {
    if (!down) { return; }
    var dx = e.clientX - lastX, dy = e.clientY - lastY;
    moved += Math.abs(dx) + Math.abs(dy);
    tx += dx; ty += dy; lastX = e.clientX; lastY = e.clientY;
    apply();
  });
  function release(e) {
    down = false; svg.style.cursor = 'grab';
    if (svg.hasPointerCapture(e.pointerId)) { svg.releasePointerCapture(e.pointerId); }
  }
  svg.addEventListener('pointerup', release);
  svg.addEventListener('pointercancel', release);
  svg.addEventListener('wheel', function (e) {
    e.preventDefault();
    var r = svg.getBoundingClientRect();
    zoom(e.deltaY > 0 ? 1 / 1.12 : 1.12, e.clientX - r.left, e.clientY - r.top);
  }, { passive: false });

  var netted = svg.querySelectorAll('[data-net]');
  var nodes = svg.querySelectorAll('.node');
  for (var i = 0; i !== netted.length; i++) { netted[i].nodeOwner = netted[i].closest('.node'); }

  function highlight(net) {
    for (var j = 0; j !== nodes.length; j++) { nodes[j].classList.remove('hl-node'); }
    for (var k = 0; k !== netted.length; k++) {
      var el = netted[k];
      if (net !== null && el.getAttribute('data-net') === net) {
        el.classList.add('hl');
        if (el.nodeOwner) { el.nodeOwner.classList.add('hl-node'); }
      } else {
        el.classList.remove('hl');
      }
    }
  }
  svg.addEventListener('pointerover', function (e) {
    var t = e.target.closest ? e.target.closest('[data-net]') : null;
    highlight(t ? t.getAttribute('data-net') : null);
  });
  svg.addEventListener('pointerleave', function () { highlight(null); });

  var selected = null;
  function select(node) {
    if (selected) { selected.classList.remove('sel'); }
    selected = node;
    var panes = info ? info.querySelectorAll('.detail') : [];
    for (var j = 0; j !== panes.length; j++) { panes[j].hidden = true; }
    var empty = document.getElementById('info-empty');
    if (!node) { if (empty) { empty.hidden = false; } return; }
    node.classList.add('sel');
    if (empty) { empty.hidden = true; }
    var pane = document.getElementById('d' + node.getAttribute('data-detail'));
    if (pane) { pane.hidden = false; }
  }
  svg.addEventListener('click', function (e) {
    if (moved > 4) { return; }
    select(e.target.closest ? e.target.closest('.node') : null);
  });

  var search = document.getElementById('search');
  if (search) {
    search.addEventListener('input', function () {
      var q = search.value.trim().toLowerCase();
      var first = null;
      for (var j = 0; j !== nodes.length; j++) {
        var hay = nodes[j].getAttribute('data-search') || '';
        var hit = q !== '' && hay.indexOf(q) !== -1;
        nodes[j].classList.toggle('found', hit);
        if (hit && first === null) { first = nodes[j]; }
      }
      if (first) {
        var c = centre();
        tx = c[0] - parseFloat(first.getAttribute('data-cx')) * scale;
        ty = c[1] - parseFloat(first.getAttribute('data-cy')) * scale;
        apply();
      }
    });
  }
  function reset() { tx = 0; ty = 0; scale = 1; apply(); }
  var zin = document.getElementById('zoom-in');
  var zout = document.getElementById('zoom-out');
  var zfit = document.getElementById('zoom-reset');
  if (zin) { zin.addEventListener('click', function () { var c = centre(); zoom(1.25, c[0], c[1]); }); }
  if (zout) { zout.addEventListener('click', function () { var c = centre(); zoom(0.8, c[0], c[1]); }); }
  if (zfit) { zfit.addEventListener('click', reset); }
  document.addEventListener('keydown', function (e) {
    if (e.target && e.target.tagName === 'INPUT') { return; }
    var c = centre();
    if (e.key === '+' || e.key === '=') { zoom(1.25, c[0], c[1]); }
    else if (e.key === '-') { zoom(0.8, c[0], c[1]); }
    else if (e.key === '0') { reset(); }
    else if (e.key === 'Escape') { select(null); highlight(null); }
  });
  apply();
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_every_dangerous_character() {
        assert_eq!(escape("<script>"), "&lt;script&gt;");
        assert_eq!(escape("a & b"), "a &amp; b");
        assert_eq!(escape("\"x'y\""), "&quot;x&#39;y&quot;");
        assert_eq!(escape("plain"), "plain");
    }

    #[test]
    fn builds_nested_markup() {
        let mut h = Html::new();
        h.open("div", &[("class", "a")]);
        h.element("p", &[], "hi <there>");
        h.void("br", &[]);
        assert_eq!(h.depth(), 1);
        h.close();
        assert_eq!(
            h.finish(),
            "<div class=\"a\">\n  <p>hi &lt;there&gt;</p>\n  <br />\n</div>"
        );
    }

    #[test]
    #[should_panic(expected = "unclosed elements")]
    fn finish_rejects_an_open_element() {
        let mut h = Html::new();
        h.open("div", &[]);
        let _ = h.finish();
    }

    #[test]
    fn page_inlines_its_assets() {
        let body = Html::new().finish();
        let out = page(
            "<b>t</b>",
            &[(String::new(), "here".into())],
            &body,
            false,
            false,
        );
        assert!(out.starts_with("<!DOCTYPE html>\n<html lang=\"en\">"));
        assert!(out.contains("<title>&lt;b&gt;t&lt;/b&gt;</title>"));
        assert!(out.contains("<style>"));
        assert!(!out.contains("<script>"));
        assert!(!out.contains("http"));
        let with = page("t", &[], &body, true, true);
        assert!(with.contains("<script>"));
        assert!(with.contains("<main class=\"wide\">"));
    }
}
