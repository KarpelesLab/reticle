//! A small XML reader, hand written, for reading IP-XACT.
//!
//! IP-XACT is XML and the crate takes no dependencies, so it brings its
//! own reader. This is a **tree** parser: [`Document::parse`] returns the
//! whole document as [`Element`]s and [`Node`]s, because an IP-XACT
//! component is read by looking things up (`model`, then `ports`, then
//! each `port`) rather than streamed, and a tree is what makes
//! [`super::ipxact`] readable.
//!
//! # What it covers
//!
//! | Construct | Handled |
//! |-----------|---------|
//! | elements, nested and self-closing (`<a/>`) | yes |
//! | attributes, single or double quoted | yes |
//! | namespaces: `xmlns` and `xmlns:p`, resolved to URIs | yes |
//! | text content | yes |
//! | `<![CDATA[ ... ]]>` | yes, as text with [`Text::cdata`] set |
//! | `<!-- comments -->` | yes, kept as [`Node::Comment`] |
//! | `<?processing instructions?>`, including the XML declaration | yes, kept as [`Node::Instruction`] |
//! | `&amp; &lt; &gt; &quot; &apos;` | yes |
//! | `&#38;` and `&#x26;` | yes |
//! | any other entity reference | **refused**, see below |
//! | DTDs, `<!DOCTYPE>` internal subsets, schema validation | no, see below |
//!
//! # What it deliberately does not do
//!
//! **No DTDs, no external entities, no schema validation.** An IP-XACT
//! document is validated against its schema by the tool that wrote it;
//! re-implementing XSD here would buy nothing. A `<!DOCTYPE>` with no
//! internal subset is ignored with a warning, and everything else about
//! it is an error:
//!
//! - a `<!DOCTYPE>` that declares entities, or names a `SYSTEM` or
//!   `PUBLIC` identifier, is [`EXTERNAL_ENTITY`];
//! - an entity reference that is not one of the five predefined ones or
//!   a numeric character reference is [`EXTERNAL_ENTITY`] too.
//!
//! That refusal is the point rather than a limitation. Resolving
//! external entities is how XML parsers become file-disclosure and
//! server-side request forgery holes ("XXE"), and a document that needs
//! one can be expanded by whatever wrote it before it reaches Reticle.
//! The library performs no I/O, so there is nowhere for a resolved
//! entity to come from in any case.
//!
//! # Errors
//!
//! A malformed document produces **one** diagnostic, with the span of
//! the construct that is wrong, and [`Document::parse`] returns `None`.
//! Nothing is guessed and no recovery is attempted: half a component
//! description is worse than none, because the half that vanished is
//! exactly what a later reader will not know to look for.
//!
//! ```
//! use reticle::diag::Diagnostics;
//! use reticle::ip::xml::Document;
//! use reticle::source::SourceMap;
//!
//! let text = "<?xml version=\"1.0\"?>\n<a x=\"1\"><b>hi &amp; bye</b><c/></a>\n";
//! let mut map = SourceMap::new();
//! let file = map.add("doc.xml", text).unwrap();
//! let mut diags = Diagnostics::new();
//! let doc = Document::parse(text, file, &mut diags).unwrap();
//! assert_eq!(doc.root.name.local, "a");
//! assert_eq!(doc.root.attr("x"), Some("1"));
//! assert_eq!(doc.root.child_text("b").as_deref(), Some("hi & bye"));
//! assert_eq!(doc.root.elements().count(), 2);
//! ```

use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{SourceId, Span};

/// Diagnostic code for markup that is not well formed.
pub const SYNTAX: &str = "P0501";
/// Diagnostic code for an end tag that does not match its start tag.
pub const MISMATCHED_TAG: &str = "P0502";
/// Diagnostic code for a malformed or out-of-range character reference.
pub const BAD_ENTITY: &str = "P0503";
/// Diagnostic code for an entity reference, or a DTD, that Reticle
/// refuses to resolve.
pub const EXTERNAL_ENTITY: &str = "P0504";
/// Diagnostic code for an attribute given twice on one element.
pub const DUPLICATE_ATTRIBUTE: &str = "P0505";
/// Diagnostic code for a namespace prefix nothing declares.
pub const UNBOUND_PREFIX: &str = "P0506";
/// Diagnostic code for elements nested deeper than [`MAX_DEPTH`].
pub const TOO_DEEP: &str = "P0507";

/// How deeply elements may nest.
///
/// An IP-XACT component is a dozen levels deep at most; the limit exists
/// so that a hostile or corrupt document cannot build a tree whose
/// recursive drop overflows the stack.
pub const MAX_DEPTH: usize = 256;

/// The namespace `xml:` is always bound to, per the namespaces
/// specification.
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

// ---------------------------------------------------------------------------
// The tree
// ---------------------------------------------------------------------------

/// An element or attribute name, with its prefix resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QName {
    /// The prefix as written, without the colon; `None` when there was
    /// none.
    pub prefix: Option<String>,
    /// The part after the colon.
    pub local: String,
    /// The namespace URI the prefix resolved to.
    ///
    /// An element with no prefix takes the default namespace; an
    /// attribute with no prefix is in no namespace at all, which is what
    /// the namespaces specification says and what makes
    /// `spirit:port spirit:id="x"` and `port id="x"` read the same way
    /// here.
    pub namespace: Option<String>,
    /// Where the name was written.
    pub span: Span,
}

impl QName {
    /// The name as it was written, prefix and all.
    pub fn qualified(&self) -> String {
        match &self.prefix {
            Some(prefix) => format!("{prefix}:{}", self.local),
            None => self.local.clone(),
        }
    }
}

/// One attribute of an element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// The attribute's name.
    pub name: QName,
    /// Its value, with entity references resolved and tabs and newlines
    /// normalised to spaces.
    pub value: String,
    /// The span of the value, quotes included.
    pub value_span: Span,
    /// The span of the whole `name="value"`.
    pub span: Span,
}

/// A run of character data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    /// The text, with entity references resolved.
    pub text: String,
    /// True when it came from a `<![CDATA[ ]]>` section, where nothing
    /// was resolved because nothing is markup there.
    pub cdata: bool,
    /// Where it was written.
    pub span: Span,
}

/// A processing instruction, including the `<?xml ... ?>` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    /// The target: the name right after `<?`.
    pub target: String,
    /// Everything between the target and `?>`, trimmed.
    pub data: String,
    /// Where it was written.
    pub span: Span,
}

impl Instruction {
    /// The value of a `name="value"` pair in [`data`](Instruction::data).
    ///
    /// The XML declaration is not made of attributes as far as the
    /// grammar is concerned, but it is written like them, so this reads
    /// `version`, `encoding` and `standalone` out of it without a second
    /// parser.
    pub fn pseudo_attribute(&self, name: &str) -> Option<&str> {
        let mut rest = self.data.as_str();
        while let Some(at) = rest.find('=') {
            let key = rest[..at].trim();
            let after = rest[at + 1..].trim_start();
            let quote = after.chars().next()?;
            if quote != '"' && quote != '\'' {
                return None;
            }
            let body = &after[1..];
            let end = body.find(quote)?;
            if key == name {
                return Some(&body[..end]);
            }
            rest = &body[end + 1..];
        }
        None
    }
}

/// One child of an element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// A nested element.
    Element(Element),
    /// Character data, from text content or a CDATA section.
    Text(Text),
    /// A comment, with the text between `<!--` and `-->`.
    Comment(Text),
    /// A processing instruction.
    Instruction(Instruction),
}

impl Node {
    /// The element, when this node is one.
    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(element) => Some(element),
            _ => None,
        }
    }
}

/// An element: a name, its attributes and its children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    /// The element's name.
    pub name: QName,
    /// Its attributes, in document order, with the namespace
    /// declarations removed.
    pub attributes: Vec<Attribute>,
    /// Its children, in document order.
    pub children: Vec<Node>,
    /// The span from `<` to the end of the element.
    pub span: Span,
}

impl Element {
    /// True when the element's local name is `local`, whatever its
    /// prefix.
    ///
    /// Every lookup here ignores the prefix on purpose: a catalogue is a
    /// mix of `spirit:port` and `ipxact:port`, the namespace URI says
    /// which standard the document follows, and once that has been
    /// checked once at the root there is nothing to gain from checking
    /// it on every element.
    pub fn is(&self, local: &str) -> bool {
        self.name.local == local
    }

    /// The value of the attribute whose local name is `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.name.local == name)
            .map(|a| a.value.as_str())
    }

    /// The attribute whose local name is `name`, for its span.
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.name.local == name)
    }

    /// Every child element, in document order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(Node::as_element)
    }

    /// Every child element whose local name is `local`.
    ///
    /// The name is copied rather than borrowed so that the iterator's
    /// lifetime is the element's alone, which is what lets
    /// [`child`](Element::child) return a reference outliving its
    /// argument.
    pub fn children_named<'a>(
        &'a self,
        local: &str,
    ) -> impl Iterator<Item = &'a Element> + use<'a> {
        let local = local.to_owned();
        self.elements().filter(move |e| e.name.local == local)
    }

    /// The first child element whose local name is `local`.
    pub fn child(&self, local: &str) -> Option<&Element> {
        self.children_named(local).next()
    }

    /// The namespace URI this element's name resolved to.
    pub fn namespace_of(&self) -> Option<&str> {
        self.name.namespace.as_deref()
    }

    /// The first descendant reached by following `path` down from here.
    ///
    /// `component.find(&["model", "ports"])` is the shape every read in
    /// [`super::ipxact`] has.
    pub fn find(&self, path: &[&str]) -> Option<&Element> {
        let mut here = self;
        for step in path {
            here = here.child(step)?;
        }
        Some(here)
    }

    /// Every direct text and CDATA child, concatenated.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            if let Node::Text(text) = child {
                out.push_str(&text.text);
            }
        }
        out
    }

    /// [`text`](Element::text) with leading and trailing whitespace
    /// removed, which is what an IP-XACT value element holds.
    pub fn trimmed_text(&self) -> String {
        self.text().trim().to_owned()
    }

    /// The trimmed text of the first child element named `local`.
    pub fn child_text(&self, local: &str) -> Option<String> {
        Some(self.child(local)?.trimmed_text())
    }
}

/// A whole document: the root element and the markup around it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// The comments and processing instructions before the root.
    pub prolog: Vec<Node>,
    /// The root element.
    pub root: Element,
    /// The comments and processing instructions after the root.
    pub epilog: Vec<Node>,
    /// The span of the whole file.
    pub span: Span,
}

impl Document {
    /// Parses `text`, reporting the first problem and giving up.
    ///
    /// `file` must be the [`SourceId`] `text` was added to the source
    /// map under, so that the one diagnostic this can produce points at
    /// the right place.
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> Option<Document> {
        let mut parser = Parser {
            text,
            bytes: text.as_bytes(),
            pos: 0,
            file,
            diags,
        };
        parser.document().ok()
    }

    /// The XML declaration, when the document opens with one.
    pub fn declaration(&self) -> Option<&Instruction> {
        match self.prolog.first() {
            Some(Node::Instruction(pi)) if pi.target == "xml" => Some(pi),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// The parser
// ---------------------------------------------------------------------------

/// One namespace scope: the declarations one element introduced.
type Scope = Vec<(Option<String>, String)>;

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    file: SourceId,
    diags: &'a mut Diagnostics,
}

/// Every failure path pushes exactly one diagnostic and returns this.
type Fail = ();

impl Parser<'_> {
    // -- positions and errors ------------------------------------------

    fn span(&self, start: usize, end: usize) -> Span {
        let to_u32 = |n: usize| u32::try_from(n).expect("source offset exceeds u32");
        Span::new(self.file, to_u32(start), to_u32(end))
    }

    /// The span of one byte, or of the end of the file.
    fn here(&self) -> Span {
        let end = (self.pos + 1).min(self.text.len());
        self.span(self.pos.min(end), end)
    }

    fn fail<T>(
        &mut self,
        code: &'static str,
        span: Span,
        message: impl Into<String>,
    ) -> Result<T, Fail> {
        self.diags
            .push(Diagnostic::error(message).with_code(code).with_span(span));
        Err(())
    }

    fn fail_with<T>(
        &mut self,
        code: &'static str,
        span: Span,
        message: impl Into<String>,
        note: impl Into<String>,
    ) -> Result<T, Fail> {
        self.diags.push(
            Diagnostic::error(message)
                .with_code(code)
                .with_span(span)
                .with_note(note),
        );
        Err(())
    }

    // -- low-level scanning --------------------------------------------

    fn at(&self, needle: &str) -> bool {
        self.text[self.pos..].starts_with(needle)
    }

    fn eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// True when `b` may appear in an XML name.
    ///
    /// Permissive on purpose: every byte of a non-ASCII character is
    /// accepted, so a name in any script stays one token and the slice
    /// boundaries stay on character boundaries.
    fn is_name_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':') || b >= 0x80
    }

    /// Reads a name, or reports that there is not one here.
    fn name(&mut self, what: &str) -> Result<(String, Span), Fail> {
        let start = self.pos;
        while self.peek().is_some_and(Self::is_name_byte) {
            self.pos += 1;
        }
        if self.pos == start {
            let span = self.here();
            return self.fail(SYNTAX, span, format!("expected {what} here"));
        }
        Ok((
            self.text[start..self.pos].to_owned(),
            self.span(start, self.pos),
        ))
    }

    /// Splits a name into a prefix and a local part.
    ///
    /// A name with more than one colon is not a qualified name; it is
    /// reported rather than split arbitrarily.
    fn split_name(&mut self, raw: &str, span: Span) -> Result<(Option<String>, String), Fail> {
        match raw.split_once(':') {
            None => Ok((None, raw.to_owned())),
            Some((prefix, local)) => {
                if prefix.is_empty() || local.is_empty() || local.contains(':') {
                    return self.fail_with(
                        SYNTAX,
                        span,
                        format!("`{raw}` is not a qualified name"),
                        "a qualified name is `prefix:local`, with one colon",
                    );
                }
                Ok((Some(prefix.to_owned()), local.to_owned()))
            }
        }
    }

    // -- entity references ---------------------------------------------

    /// Resolves the entity references in `raw`, which starts at `base`.
    ///
    /// The five predefined entities and numeric character references are
    /// resolved; anything else is refused, because resolving it would
    /// mean reading a DTD, and an external one would mean I/O. See the
    /// module documentation.
    fn decode(&mut self, raw: &str, base: usize, attribute: bool) -> Result<String, Fail> {
        if !raw.contains('&') && !(attribute && raw.contains(['\t', '\n', '\r'])) {
            return Ok(raw.to_owned());
        }
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        let mut at = base;
        while let Some(amp) = rest.find('&') {
            out.push_str(&Self::normalise(&rest[..amp], attribute));
            let after = &rest[amp + 1..];
            let start = at + amp;
            let Some(semi) = after.find(';') else {
                let span = self.span(start, (start + 1).min(self.text.len()));
                return self.fail_with(
                    BAD_ENTITY,
                    span,
                    "an entity reference has no `;`",
                    "write `&amp;` for a literal ampersand",
                );
            };
            let body = &after[..semi];
            let span = self.span(start, start + semi + 2);
            match body {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ if body.starts_with('#') => out.push(self.character_reference(body, span)?),
                _ => {
                    return self.fail_with(
                        EXTERNAL_ENTITY,
                        span,
                        format!("Reticle will not resolve the entity `&{body};`"),
                        "only `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` and numeric \
                         references are read: Reticle parses no DTD and never fetches an \
                         external entity, so expand the document before importing it",
                    );
                }
            }
            rest = &after[semi + 1..];
            at = start + semi + 2;
        }
        out.push_str(&Self::normalise(rest, attribute));
        Ok(out)
    }

    /// Attribute-value normalisation: a tab, newline or carriage return
    /// in an attribute value stands for a space.
    fn normalise(text: &str, attribute: bool) -> String {
        if attribute && text.contains(['\t', '\n', '\r']) {
            text.replace(['\t', '\n', '\r'], " ")
        } else {
            text.to_owned()
        }
    }

    /// `#38` or `#x26`, already known to start with `#`.
    fn character_reference(&mut self, body: &str, span: Span) -> Result<char, Fail> {
        let digits = &body[1..];
        let value = match digits.strip_prefix(['x', 'X']) {
            Some(hex) if !hex.is_empty() => u32::from_str_radix(hex, 16).ok(),
            Some(_) => None,
            None if !digits.is_empty() => digits.parse::<u32>().ok(),
            None => None,
        };
        let Some(value) = value else {
            return self.fail_with(
                BAD_ENTITY,
                span,
                format!("`&{body};` is not a character reference"),
                "write `&#38;` in decimal or `&#x26;` in hexadecimal",
            );
        };
        // The XML character range: no surrogates, no C0 controls other
        // than tab, newline and carriage return, no U+FFFE or U+FFFF.
        let ok = matches!(value, 0x9 | 0xA | 0xD)
            || (0x20..=0xD7FF).contains(&value)
            || (0xE000..=0xFFFD).contains(&value)
            || (0x1_0000..=0x10_FFFF).contains(&value);
        match char::from_u32(value).filter(|_| ok) {
            Some(c) => Ok(c),
            None => self.fail_with(
                BAD_ENTITY,
                span,
                format!("`&{body};` is not a character XML allows"),
                "a character reference must name a valid XML character",
            ),
        }
    }

    // -- markup --------------------------------------------------------

    fn document(&mut self) -> Result<Document, Fail> {
        let whole = self.span(0, self.text.len());
        // A byte-order mark is not part of the document.
        if self.at("\u{feff}") {
            self.pos += "\u{feff}".len();
        }
        let mut prolog = Vec::new();
        let root = loop {
            self.skip_whitespace();
            if self.eof() {
                return self.fail_with(
                    SYNTAX,
                    whole,
                    "the document has no root element",
                    "an XML document is one element, with anything else around it",
                );
            }
            if !self.at("<") {
                let span = self.here();
                return self.fail(SYNTAX, span, "text outside the root element");
            }
            if self.at("<?") {
                let pi = self.instruction()?;
                self.check_declaration(&pi, prolog.is_empty());
                prolog.push(Node::Instruction(pi));
            } else if self.at("<!--") {
                prolog.push(Node::Comment(self.comment()?));
            } else if self.at("<!DOCTYPE") {
                self.doctype()?;
            } else if self.at("</") {
                let span = self.here();
                return self.fail(SYNTAX, span, "an end tag before any start tag");
            } else {
                break self.element()?;
            }
        };
        let mut epilog = Vec::new();
        loop {
            self.skip_whitespace();
            if self.eof() {
                break;
            }
            if self.at("<?") {
                epilog.push(Node::Instruction(self.instruction()?));
            } else if self.at("<!--") {
                epilog.push(Node::Comment(self.comment()?));
            } else {
                let span = self.here();
                return self.fail_with(
                    SYNTAX,
                    span,
                    "content after the root element",
                    "an XML document has exactly one root element",
                );
            }
        }
        Ok(Document {
            prolog,
            root,
            epilog,
            span: whole,
        })
    }

    /// Warns about an XML declaration that is in the wrong place or
    /// claims an encoding this reader cannot honour.
    ///
    /// Neither is a reason to reject the document: the text has already
    /// been decoded to UTF-8 by whoever read the file, so a declaration
    /// that disagrees is a stale label rather than a broken document.
    fn check_declaration(&mut self, pi: &Instruction, first: bool) {
        if pi.target != "xml" {
            return;
        }
        if !first {
            self.diags.push(
                Diagnostic::warning("the XML declaration is not the first thing in the document")
                    .with_code(SYNTAX)
                    .with_span(pi.span),
            );
        }
        if let Some(encoding) = pi.pseudo_attribute("encoding") {
            let lower = encoding.to_ascii_lowercase();
            if !matches!(lower.as_str(), "utf-8" | "utf8" | "us-ascii" | "ascii") {
                self.diags.push(
                    Diagnostic::warning(format!(
                        "the document declares the encoding `{encoding}`, which Reticle cannot \
                         transcode"
                    ))
                    .with_code(SYNTAX)
                    .with_span(pi.span)
                    .with_note("the text was read as UTF-8; convert the file if it is not"),
                );
            }
        }
    }

    /// `<!DOCTYPE ...>`, which is skipped, refused or warned about.
    fn doctype(&mut self) -> Result<(), Fail> {
        let start = self.pos;
        self.pos += "<!DOCTYPE".len();
        let mut depth = 0usize;
        let mut internal = false;
        loop {
            let Some(b) = self.peek() else {
                let span = self.span(start, self.text.len());
                return self.fail(SYNTAX, span, "the document type declaration has no `>`");
            };
            match b {
                b'[' => {
                    internal = true;
                    depth += 1;
                }
                b']' => depth = depth.saturating_sub(1),
                b'>' if depth == 0 => {
                    self.pos += 1;
                    break;
                }
                _ => {}
            }
            self.pos += 1;
        }
        let span = self.span(start, self.pos);
        let body = &self.text[start..self.pos];
        if internal || body.contains("SYSTEM") || body.contains("PUBLIC") {
            return self.fail_with(
                EXTERNAL_ENTITY,
                span,
                "Reticle will not read a document type declaration with a subset or an \
                 external identifier",
                "DTDs are not parsed and external entities are never fetched; remove the \
                 `<!DOCTYPE>` or expand the document before importing it",
            );
        }
        self.diags.push(
            Diagnostic::warning("the document type declaration is ignored")
                .with_code(EXTERNAL_ENTITY)
                .with_span(span)
                .with_note("Reticle validates IP-XACT against its own reader, not against a DTD"),
        );
        Ok(())
    }

    /// `<?target data?>`.
    fn instruction(&mut self) -> Result<Instruction, Fail> {
        let start = self.pos;
        self.pos += 2;
        let (target, _) = self.name("a processing instruction target")?;
        let body = self.pos;
        let Some(end) = self.text[body..].find("?>") else {
            let span = self.span(start, self.text.len());
            return self.fail(SYNTAX, span, "a processing instruction has no `?>`");
        };
        let data = self.text[body..body + end].trim().to_owned();
        self.pos = body + end + 2;
        Ok(Instruction {
            target,
            data,
            span: self.span(start, self.pos),
        })
    }

    /// `<!-- text -->`.
    fn comment(&mut self) -> Result<Text, Fail> {
        let start = self.pos;
        let body = start + "<!--".len();
        let Some(end) = self.text[body..].find("-->") else {
            let span = self.span(start, self.text.len());
            return self.fail(SYNTAX, span, "a comment has no `-->`");
        };
        let text = self.text[body..body + end].to_owned();
        if text.contains("--") {
            let span = self.span(start, body + end + 3);
            return self.fail_with(
                SYNTAX,
                span,
                "a comment contains `--`",
                "XML does not allow `--` inside a comment",
            );
        }
        self.pos = body + end + 3;
        Ok(Text {
            text,
            cdata: false,
            span: self.span(start, self.pos),
        })
    }

    /// `<![CDATA[ text ]]>`.
    fn cdata(&mut self) -> Result<Text, Fail> {
        let start = self.pos;
        let body = start + "<![CDATA[".len();
        let Some(end) = self.text[body..].find("]]>") else {
            let span = self.span(start, self.text.len());
            return self.fail(SYNTAX, span, "a CDATA section has no `]]>`");
        };
        let text = self.text[body..body + end].to_owned();
        self.pos = body + end + 3;
        Ok(Text {
            text,
            cdata: true,
            span: self.span(start, self.pos),
        })
    }

    /// The whole root element, iteratively: nesting is a stack rather
    /// than recursion, so a deep document cannot overflow ours.
    fn element(&mut self) -> Result<Element, Fail> {
        let mut stack: Vec<(Element, Scope)> = Vec::new();
        let mut scopes: Vec<Scope> = Vec::new();

        loop {
            if stack.is_empty() {
                // Only reachable for the very first start tag; every
                // other iteration ends with something on the stack or
                // returns.
                let (element, scope, closed) = self.start_tag(&scopes)?;
                if closed {
                    return Ok(element);
                }
                scopes.push(scope.clone());
                stack.push((element, scope));
                continue;
            }

            if self.eof() {
                let open = &stack.last().expect("the stack is not empty").0;
                let span = open.span;
                let name = open.name.qualified();
                return self.fail(SYNTAX, span, format!("`<{name}>` is never closed"));
            }

            if self.at("</") {
                self.pos += 2;
                let (raw, name_span) = self.name("an element name")?;
                self.skip_whitespace();
                if self.peek() != Some(b'>') {
                    let span = self.here();
                    return self.fail(SYNTAX, span, "an end tag has no `>`");
                }
                self.pos += 1;
                let (mut element, _) = stack.pop().expect("the stack is not empty");
                scopes.pop();
                if raw != element.name.qualified() {
                    let opened = element.name.qualified();
                    return self.fail_with(
                        MISMATCHED_TAG,
                        name_span,
                        format!("`</{raw}>` closes `<{opened}>`"),
                        format!("the element opened at this document's `<{opened}>` is still open"),
                    );
                }
                let opened_at = usize::try_from(element.span.start).expect("span fits usize");
                element.span = self.span(opened_at, self.pos);
                match stack.last_mut() {
                    Some((parent, _)) => parent.children.push(Node::Element(element)),
                    None => return Ok(element),
                }
                continue;
            }

            if self.at("<!--") {
                let comment = self.comment()?;
                stack
                    .last_mut()
                    .expect("the stack is not empty")
                    .0
                    .children
                    .push(Node::Comment(comment));
                continue;
            }
            if self.at("<![CDATA[") {
                let text = self.cdata()?;
                stack
                    .last_mut()
                    .expect("the stack is not empty")
                    .0
                    .children
                    .push(Node::Text(text));
                continue;
            }
            if self.at("<?") {
                let pi = self.instruction()?;
                stack
                    .last_mut()
                    .expect("the stack is not empty")
                    .0
                    .children
                    .push(Node::Instruction(pi));
                continue;
            }
            if self.at("<!DOCTYPE") {
                let span = self.here();
                return self.fail_with(
                    SYNTAX,
                    span,
                    "a document type declaration inside an element",
                    "a `<!DOCTYPE>` may only appear before the root element",
                );
            }
            if self.at("<!") {
                let span = self.here();
                return self.fail(SYNTAX, span, "markup Reticle does not recognise");
            }
            if self.at("<") {
                if stack.len() >= MAX_DEPTH {
                    let span = self.here();
                    return self.fail(
                        TOO_DEEP,
                        span,
                        format!("elements are nested more than {MAX_DEPTH} deep"),
                    );
                }
                let (element, scope, closed) = self.start_tag(&scopes)?;
                if closed {
                    stack
                        .last_mut()
                        .expect("the stack is not empty")
                        .0
                        .children
                        .push(Node::Element(element));
                } else {
                    scopes.push(scope.clone());
                    stack.push((element, scope));
                }
                continue;
            }

            // Character data up to the next `<`.
            let text = self.text;
            let start = self.pos;
            let end = text[start..].find('<').map_or(text.len(), |at| start + at);
            let decoded = self.decode(&text[start..end], start, false)?;
            self.pos = end;
            stack
                .last_mut()
                .expect("the stack is not empty")
                .0
                .children
                .push(Node::Text(Text {
                    text: decoded,
                    cdata: false,
                    span: self.span(start, end),
                }));
        }
    }

    /// One start tag, with its namespace declarations applied.
    ///
    /// Returns the element, the scope it declared and whether it was
    /// self-closing.
    fn start_tag(&mut self, scopes: &[Scope]) -> Result<(Element, Scope, bool), Fail> {
        let start = self.pos;
        self.pos += 1;
        let (raw_name, name_span) = self.name("an element name")?;

        // Attributes first: a namespace declaration on this element is
        // in scope for its own name.
        let mut raw_attributes: Vec<(String, Span, String, Span, Span)> = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => {
                    let span = self.span(start, self.text.len());
                    return self.fail(SYNTAX, span, format!("`<{raw_name}` has no `>`"));
                }
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                Some(b'/') => {
                    if !self.at("/>") {
                        let span = self.here();
                        return self.fail(SYNTAX, span, "expected `/>` here");
                    }
                    self.pos += 2;
                    break;
                }
                Some(_) => {}
            }
            let attr_start = self.pos;
            let (attr_name, attr_span) = self.name("an attribute name")?;
            self.skip_whitespace();
            if self.peek() != Some(b'=') {
                let span = self.here();
                return self.fail_with(
                    SYNTAX,
                    span,
                    format!("the attribute `{attr_name}` has no value"),
                    "every XML attribute is written `name=\"value\"`",
                );
            }
            self.pos += 1;
            self.skip_whitespace();
            let quote = match self.peek() {
                Some(q @ (b'"' | b'\'')) => q,
                _ => {
                    let span = self.here();
                    return self.fail_with(
                        SYNTAX,
                        span,
                        format!("the value of `{attr_name}` is not quoted"),
                        "an attribute value is in single or double quotes",
                    );
                }
            };
            let value_start = self.pos;
            self.pos += 1;
            let body = self.pos;
            let Some(end) = self.text[body..].find(char::from(quote)) else {
                let span = self.span(value_start, self.text.len());
                return self.fail(
                    SYNTAX,
                    span,
                    format!("the value of `{attr_name}` is never closed"),
                );
            };
            let raw_value: &str = &self.text[body..body + end];
            if raw_value.contains('<') {
                let span = self.span(body, body + end);
                return self.fail_with(
                    SYNTAX,
                    span,
                    format!("the value of `{attr_name}` contains `<`"),
                    "write `&lt;` instead",
                );
            }
            let value = self.decode(raw_value, body, true)?;
            self.pos = body + end + 1;
            raw_attributes.push((
                attr_name,
                attr_span,
                value,
                self.span(value_start, self.pos),
                self.span(attr_start, self.pos),
            ));
        }
        let self_closing = self.text[..self.pos].ends_with("/>");

        // Namespace declarations, then the names they apply to.
        let mut scope: Scope = Vec::new();
        let mut attributes = Vec::new();
        for (name, name_at, value, value_span, span) in raw_attributes {
            if name == "xmlns" {
                scope.push((None, value));
                continue;
            }
            if let Some(prefix) = name.strip_prefix("xmlns:") {
                if prefix.is_empty() || prefix.contains(':') {
                    return self.fail(
                        SYNTAX,
                        name_at,
                        format!("`{name}` does not declare a namespace prefix"),
                    );
                }
                if value.is_empty() {
                    return self.fail_with(
                        SYNTAX,
                        value_span,
                        format!("the prefix `{prefix}` is bound to nothing"),
                        "only the default namespace may be undeclared with an empty value",
                    );
                }
                scope.push((Some(prefix.to_owned()), value));
                continue;
            }
            attributes.push((name, name_at, value, value_span, span));
        }

        let mut all: Vec<&Scope> = scopes.iter().collect();
        all.push(&scope);

        let (prefix, local) = self.split_name(&raw_name, name_span)?;
        let namespace = self.lookup(&all, prefix.as_deref(), name_span, true)?;
        let name = QName {
            prefix,
            local,
            namespace,
            span: name_span,
        };

        let mut resolved: Vec<Attribute> = Vec::new();
        for (raw, name_at, value, value_span, span) in attributes {
            let (prefix, local) = self.split_name(&raw, name_at)?;
            let namespace = self.lookup(&all, prefix.as_deref(), name_at, false)?;
            let qname = QName {
                prefix,
                local,
                namespace,
                span: name_at,
            };
            if resolved
                .iter()
                .any(|a| a.name.local == qname.local && a.name.namespace == qname.namespace)
            {
                return self.fail(
                    DUPLICATE_ATTRIBUTE,
                    name_at,
                    format!("the attribute `{raw}` is given twice on `<{raw_name}>`"),
                );
            }
            resolved.push(Attribute {
                name: qname,
                value,
                value_span,
                span,
            });
        }

        Ok((
            Element {
                name,
                attributes: resolved,
                children: Vec::new(),
                span: self.span(start, self.pos),
            },
            scope,
            self_closing,
        ))
    }

    /// The URI a prefix is bound to, innermost scope first.
    ///
    /// `element` says whether the name is an element's: an unprefixed
    /// element takes the default namespace, an unprefixed attribute is
    /// in none.
    fn lookup(
        &mut self,
        scopes: &[&Scope],
        prefix: Option<&str>,
        span: Span,
        element: bool,
    ) -> Result<Option<String>, Fail> {
        if prefix == Some("xml") {
            return Ok(Some(XML_NAMESPACE.to_owned()));
        }
        if prefix.is_none() && !element {
            return Ok(None);
        }
        for scope in scopes.iter().rev() {
            for (declared, uri) in scope.iter().rev() {
                if declared.as_deref() == prefix {
                    return Ok(if uri.is_empty() {
                        None
                    } else {
                        Some(uri.clone())
                    });
                }
            }
        }
        match prefix {
            None => Ok(None),
            Some(prefix) => self.fail_with(
                UNBOUND_PREFIX,
                span,
                format!("the namespace prefix `{prefix}` is not declared"),
                format!("add an `xmlns:{prefix}=\"...\"` attribute to an enclosing element"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    /// Parses `text`, returning the document and everything reported.
    fn parse(text: &str) -> (Option<Document>, String) {
        let mut map = SourceMap::new();
        let file = map.add("doc.xml", text).unwrap();
        let mut diags = Diagnostics::new();
        let doc = Document::parse(text, file, &mut diags);
        (doc, diags.render(&map))
    }

    /// Parses `text`, expecting it to be well formed.
    fn ok(text: &str) -> Document {
        let (doc, reported) = parse(text);
        assert_eq!(reported, "", "unexpected diagnostics for {text:?}");
        doc.expect("well formed")
    }

    /// Parses `text`, expecting exactly one error naming `code`.
    fn bad(text: &str, code: &str) -> String {
        let mut map = SourceMap::new();
        let file = map.add("doc.xml", text).unwrap();
        let mut diags = Diagnostics::new();
        let doc = Document::parse(text, file, &mut diags);
        assert!(doc.is_none(), "{text:?} should not parse");
        assert_eq!(
            diags.len(),
            1,
            "expected one diagnostic: {}",
            diags.render(&map)
        );
        let diag = diags.iter().next().expect("one diagnostic");
        assert_eq!(diag.code, Some(code), "{}", diags.render(&map));
        assert!(diag.primary_span().is_some(), "the diagnostic has no span");
        diags.render(&map)
    }

    #[test]
    fn reads_elements_attributes_and_text() {
        let doc = ok("<a x='1' y=\"2\">text<b/></a>");
        assert_eq!(doc.root.name.local, "a");
        assert_eq!(doc.root.attr("x"), Some("1"));
        assert_eq!(doc.root.attr("y"), Some("2"));
        assert_eq!(doc.root.attr("z"), None);
        assert_eq!(doc.root.text(), "text");
        assert_eq!(doc.root.elements().count(), 1);
        assert!(doc.root.child("b").expect("b").children.is_empty());
    }

    #[test]
    fn self_closing_and_nested_elements() {
        let doc = ok("<a><b><c/><c/></b><d/></a>");
        assert_eq!(
            doc.root.child("b").expect("b").children_named("c").count(),
            2
        );
        assert_eq!(doc.root.elements().count(), 2);
        assert_eq!(doc.root.find(&["b", "c"]).expect("c").name.local, "c");
        assert!(doc.root.find(&["b", "z"]).is_none());
    }

    #[test]
    fn resolves_namespaces_and_prefixes() {
        let text = "<s:component xmlns:s=\"urn:spirit\" xmlns=\"urn:default\" s:id=\"7\" id=\"8\">\
                    <name>x</name><s:name>y</s:name></s:component>";
        let doc = ok(text);
        assert_eq!(doc.root.name.prefix.as_deref(), Some("s"));
        assert_eq!(doc.root.name.local, "component");
        assert_eq!(doc.root.name.namespace.as_deref(), Some("urn:spirit"));
        assert_eq!(doc.root.name.qualified(), "s:component");
        // An unprefixed attribute is in no namespace; a prefixed one is.
        let ids: Vec<Option<&str>> = doc
            .root
            .attributes
            .iter()
            .map(|a| a.name.namespace.as_deref())
            .collect();
        assert_eq!(ids, vec![Some("urn:spirit"), None]);
        // An unprefixed element takes the default namespace.
        let plain = doc.root.child("name").expect("name");
        assert_eq!(plain.namespace_of(), Some("urn:default"));
        assert_eq!(doc.root.children_named("name").count(), 2);
    }

    #[test]
    fn an_inner_declaration_wins() {
        let doc = ok("<a xmlns=\"urn:one\"><b xmlns=\"urn:two\"><c/></b></a>");
        let b = doc.root.child("b").expect("b");
        assert_eq!(b.namespace_of(), Some("urn:two"));
        assert_eq!(b.child("c").expect("c").namespace_of(), Some("urn:two"));
        assert_eq!(doc.root.namespace_of(), Some("urn:one"));
    }

    #[test]
    fn xml_prefix_is_always_bound() {
        let doc = ok("<a xml:lang=\"en\"/>");
        assert_eq!(doc.root.attr("lang"), Some("en"));
        assert_eq!(
            doc.root.attributes[0].name.namespace.as_deref(),
            Some(XML_NAMESPACE)
        );
    }

    #[test]
    fn reads_cdata_comments_and_instructions() {
        let text = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                    <!-- before -->\n\
                    <a><![CDATA[ <not markup> & ]]><!-- in --><?php echo?>tail</a>\n\
                    <!-- after -->";
        let doc = ok(text);
        assert_eq!(doc.root.text(), " <not markup> & tail");
        let cdata = doc
            .root
            .children
            .iter()
            .filter_map(|n| match n {
                Node::Text(t) if t.cdata => Some(t),
                _ => None,
            })
            .count();
        assert_eq!(cdata, 1);
        assert_eq!(
            doc.root
                .children
                .iter()
                .filter(|n| matches!(n, Node::Comment(_)))
                .count(),
            1
        );
        let pi = doc
            .root
            .children
            .iter()
            .find_map(|n| match n {
                Node::Instruction(pi) => Some(pi),
                _ => None,
            })
            .expect("a processing instruction");
        assert_eq!(pi.target, "php");
        assert_eq!(pi.data, "echo");
        let declaration = doc.declaration().expect("an XML declaration");
        assert_eq!(declaration.pseudo_attribute("version"), Some("1.0"));
        assert_eq!(declaration.pseudo_attribute("encoding"), Some("UTF-8"));
        assert_eq!(declaration.pseudo_attribute("standalone"), None);
        assert_eq!(doc.prolog.len(), 2);
        assert_eq!(doc.epilog.len(), 1);
    }

    #[test]
    fn resolves_predefined_and_numeric_entities() {
        let doc = ok("<a t=\"&lt;&amp;&gt;&quot;&apos;\">&#65;&#x42;&#x1F600;&amp;</a>");
        assert_eq!(doc.root.attr("t"), Some("<&>\"'"));
        assert_eq!(doc.root.text(), "AB\u{1F600}&");
    }

    #[test]
    fn normalises_whitespace_in_attribute_values() {
        let doc = ok("<a t=\"one\ttwo\nthree\"/>");
        assert_eq!(doc.root.attr("t"), Some("one two three"));
        // Text content keeps its newlines.
        let doc = ok("<a>one\ntwo</a>");
        assert_eq!(doc.root.text(), "one\ntwo");
        assert_eq!(doc.root.trimmed_text(), "one\ntwo");
    }

    #[test]
    fn strips_a_byte_order_mark() {
        let doc = ok("\u{feff}<a/>");
        assert_eq!(doc.root.name.local, "a");
    }

    #[test]
    fn refuses_an_external_entity_reference() {
        let rendered = bad("<a>&xxe;</a>", EXTERNAL_ENTITY);
        assert!(
            rendered.contains("will not resolve the entity `&xxe;`"),
            "{rendered}"
        );
        assert!(
            rendered.contains("never fetches an external entity"),
            "{rendered}"
        );
        // The classic shape, with the entity declared in an internal subset.
        let rendered = bad(
            "<!DOCTYPE a [<!ENTITY xxe SYSTEM \"file:///etc/passwd\">]>\n<a>&xxe;</a>",
            EXTERNAL_ENTITY,
        );
        assert!(rendered.contains("document type declaration"), "{rendered}");
        // And an external identifier with no subset at all.
        bad("<!DOCTYPE a SYSTEM \"a.dtd\">\n<a/>", EXTERNAL_ENTITY);
    }

    #[test]
    fn a_plain_doctype_is_ignored_with_a_warning() {
        let (doc, rendered) = parse("<!DOCTYPE a>\n<a/>");
        assert_eq!(doc.expect("parses").root.name.local, "a");
        assert!(rendered.contains("ignored"), "{rendered}");
    }

    #[test]
    fn an_odd_encoding_declaration_is_a_warning_not_an_error() {
        let (doc, rendered) = parse("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a/>");
        assert!(doc.is_some());
        assert!(rendered.contains("cannot transcode"), "{rendered}");
    }

    #[test]
    fn every_malformed_document_gives_one_diagnostic() {
        bad("", SYNTAX);
        bad("   \n", SYNTAX);
        bad("not markup", SYNTAX);
        bad("<a>", SYNTAX);
        bad("<a><b></a></b>", MISMATCHED_TAG);
        bad("</a>", SYNTAX);
        bad("<a/><b/>", SYNTAX);
        bad("<a", SYNTAX);
        bad("<a x>", SYNTAX);
        bad("<a x=1>", SYNTAX);
        bad("<a x=\"1>", SYNTAX);
        bad("<a x=\"<\"/>", SYNTAX);
        bad("<a x=\"1\" x=\"2\"/>", DUPLICATE_ATTRIBUTE);
        bad("<a><!-- unterminated </a>", SYNTAX);
        bad("<a><!-- a -- b --></a>", SYNTAX);
        bad("<a><![CDATA[x</a>", SYNTAX);
        bad("<a><?pi ></a>", SYNTAX);
        bad("<a>&#xZZ;</a>", BAD_ENTITY);
        bad("<a>&#0;</a>", BAD_ENTITY);
        bad("<a>&amp</a>", BAD_ENTITY);
        bad("<s:a/>", UNBOUND_PREFIX);
        bad("<a b:c=\"1\"/>", UNBOUND_PREFIX);
        bad("<a:b:c/>", SYNTAX);
        bad("<a xmlns:=\"urn:x\"/>", SYNTAX);
        bad("<a xmlns:p=\"\"/>", SYNTAX);
        bad("<a><!DOCTYPE b></a>", SYNTAX);
        bad("<a><!ENTITY b></a>", SYNTAX);
        bad("<a></b>", MISMATCHED_TAG);
        bad("<!DOCTYPE a", SYNTAX);
    }

    #[test]
    fn refuses_a_document_nested_too_deep() {
        let mut text = String::new();
        for _ in 0..=MAX_DEPTH {
            text.push_str("<a>");
        }
        text.push('x');
        for _ in 0..=MAX_DEPTH {
            text.push_str("</a>");
        }
        bad(&text, TOO_DEEP);
    }

    #[test]
    fn spans_point_at_the_construct() {
        let text = "<a>\n  <b>value</b>\n</a>\n";
        let doc = ok(text);
        let b = doc.root.child("b").expect("b");
        let span = b.span;
        assert_eq!(
            &text[span.start as usize..span.end as usize],
            "<b>value</b>"
        );
        let name = b.name.span;
        assert_eq!(&text[name.start as usize..name.end as usize], "b");
    }

    #[test]
    fn helpers_read_what_ipxact_needs() {
        let text = "<component><name> uart </name><model><ports><port><name>clk</name></port>\
                    </ports></model></component>";
        let doc = ok(text);
        assert_eq!(doc.root.child_text("name").as_deref(), Some("uart"));
        assert_eq!(doc.root.child_text("nope"), None);
        let ports = doc.root.find(&["model", "ports"]).expect("ports");
        assert_eq!(ports.children_named("port").count(), 1);
        assert!(doc.root.attribute("x").is_none());
        assert!(matches!(doc.root.children[0], Node::Element(_)));
        assert!(doc.root.children[0].as_element().is_some());
    }
}
