//! The language features, written once over the language-neutral
//! [`super::index::Index`].
//!
//! Each function takes a [`Document`] (and, where the answer may reach
//! beyond it, the whole [`DocumentStore`]) and returns an LSP value.
//! Nothing here touches the wire format; [`super::server`] does that.
//!
//! The rule the whole module follows: **where a feature cannot answer
//! precisely, it answers nothing.** A go-to-definition that cannot see the
//! declaration returns no location rather than the nearest name that looks
//! similar; a hover on a name the front end did not resolve says nothing
//! rather than repeating the word under the cursor; a rename that cannot
//! prove it has found every use refuses and says why. An editor recovers
//! from "I don't know" instantly and from a wrong answer not at all.

use std::fmt::Write as _;

use crate::diag::Diagnostics;
use crate::fmt_doc::{Edit, FormatOptions, Indent, diff_lines};
use crate::source::Span;
use crate::vhdl::sema::{Analysis, DeclId, DeclKind};

use super::analysis::{Document, DocumentStore, Language};
use super::index::{DeclClass, DeclInfo, Index, OutlineNode};
use super::protocol::{
    CompletionItem, CompletionItemKind, DocumentSymbol, Hover, Location, Position, Range, TextEdit,
    WorkspaceEdit,
};
use super::text::{char_before_prefix, matches_prefix, prefix_at};
use super::{verilog, vhdl};

// ---------------------------------------------------------------------------
// Go to definition and references
// ---------------------------------------------------------------------------

/// The declaration of the name at `position`, if the document declares it.
///
/// A name that resolves to something in another file — a module defined
/// elsewhere, `std_logic` from `ieee.std_logic_1164` — has no location the
/// server can offer, so it returns nothing.
pub fn definition(doc: &Document, position: Position) -> Option<Location> {
    let offset = doc.offset(position);
    let decl = doc.index().decl_at(offset)?;
    let span = doc.index().decls[decl].name_span;
    Some(Location::new(doc.uri.clone(), doc.range(span)))
}

/// Every use of the name at `position`, optionally including its
/// declaration.
pub fn references(doc: &Document, position: Position, include_declaration: bool) -> Vec<Location> {
    let offset = doc.offset(position);
    let Some(decl) = doc.index().decl_at(offset) else {
        return Vec::new();
    };
    let name_span = doc.index().decls[decl].name_span;
    doc.index()
        .occurrences(decl)
        .into_iter()
        .filter(|span| include_declaration || *span != name_span)
        .map(|span| Location::new(doc.uri.clone(), doc.range(span)))
        .collect()
}

// ---------------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------------

/// What to show for the name at `position`.
///
/// The body is the declaration as it was written, then its resolved type
/// and width where those are known, then the port list for a module,
/// entity or component.
pub fn hover(doc: &Document, position: Position) -> Option<Hover> {
    let offset = doc.offset(position);
    if let Some(decl) = doc.index().decl_at(offset) {
        let range = doc.index().name_span_at(offset).map(|span| doc.range(span));
        return Some(Hover {
            contents: describe(doc.language, &doc.index().decls[decl]),
            range,
        });
    }
    // A VHDL name the index does not hold is still a name the analysis
    // resolved: `std_logic` and the rest of the bundled libraries. There
    // is no location in this document to jump to, but there is plenty to
    // say about it.
    if let Some(hover) = vhdl_hover(doc, offset) {
        return Some(hover);
    }
    // The current revision does not parse well enough to place the cursor;
    // the last clean one may still know the word under it. Its spans
    // belong to a text the client has moved on from, so the hover carries
    // no range.
    let (text, index) = doc.previous()?;
    let word = super::text::word_at(doc.text(), offset);
    if word.is_empty() {
        return None;
    }
    let decl = index.decls.iter().find(|d| {
        index.same_name(&d.name, word) && super::text::slice(text, d.name_span) == d.name
    })?;
    Some(Hover {
        contents: describe(doc.language, decl),
        range: None,
    })
}

/// A hover for a VHDL name that the semantic analysis resolved but this
/// document does not declare.
///
/// The declaration is somewhere the client cannot be sent — a bundled
/// `std` or `ieee` source compiled into the crate — so the hover says what
/// it is, what its type is and where it came from, and offers no range
/// beyond the name itself.
fn vhdl_hover(doc: &Document, offset: u32) -> Option<Hover> {
    let analysis = doc.vhdl_analysis()?;
    let (start, end) = super::text::word_span_at(doc.text(), offset)?;
    let span = Span::new(doc.source_id(), start, end);
    let range = Some(doc.range(span));
    let map = doc.source_map();

    if let Some(id) = analysis.decl_of(span) {
        let decl = analysis.decl(id);
        let mut out = format!(
            "```vhdl
{}
```
",
            decl.spelling
        );
        let kind = vhdl_decl_kind(analysis, id);
        match analysis.decl_type(id) {
            Some(ty) if !analysis.is_error(ty) => {
                let _ = writeln!(
                    out,
                    "{kind} `{}` — `{}`",
                    decl.spelling,
                    analysis.describe_type(ty, Some(map))
                );
            }
            _ => {
                let _ = writeln!(out, "{kind} `{}`", decl.spelling);
            }
        }
        let (file, loc) = map.locate(decl.span);
        let _ = writeln!(out, "\ndeclared in `{file}` at {loc}");
        return Some(Hover {
            contents: out.trim_end().to_string(),
            range,
        });
    }

    // Not a name, but an expression the analysis typed.
    let ty = analysis.type_of(span)?;
    if analysis.is_error(ty) {
        return None;
    }
    Some(Hover {
        contents: format!(
            "```vhdl
{}
```
expression — `{}`",
            super::text::slice(doc.text(), span),
            analysis.describe_type(ty, Some(map))
        ),
        range,
    })
}

/// The word for what a VHDL declaration is.
fn vhdl_decl_kind(analysis: &Analysis, id: DeclId) -> &'static str {
    match &analysis.decl(id).kind {
        DeclKind::Library => "library",
        DeclKind::Unit { .. } => "design unit",
        DeclKind::Type(_) => "type",
        DeclKind::Subtype(_) => "subtype",
        DeclKind::Object { class, .. } => class.as_str(),
        DeclKind::Subprogram { sig, .. } => match sig.kind {
            crate::vhdl::ast::SubprogramKind::Function => "function",
            crate::vhdl::ast::SubprogramKind::Procedure => "procedure",
        },
        DeclKind::EnumLiteral { .. } => "enumeration literal",
        DeclKind::PhysicalUnit { .. } => "physical unit",
        DeclKind::Component { .. } => "component",
        DeclKind::Alias(_) => "alias",
        DeclKind::Attribute(_) => "attribute",
        DeclKind::Label(_) => "label",
        DeclKind::Group => "group",
        DeclKind::RecordElement(_) => "record element",
        DeclKind::Error => "declaration",
    }
}

/// The markdown body of a hover.
fn describe(language: Language, decl: &DeclInfo) -> String {
    let fence = if language.is_verilog() {
        "verilog"
    } else {
        "vhdl"
    };
    let mut out = String::new();
    if !decl.text.is_empty() {
        let _ = writeln!(out, "```{fence}\n{}\n```", decl.text);
    }
    let mut facts = vec![decl.class.describe().to_string()];
    if !decl.detail.is_empty() && decl.detail != decl.class.describe() {
        facts.push(format!("`{}`", decl.detail));
    }
    if let Some(width) = decl.width {
        facts.push(format!("{width} bit{}", if width == 1 { "" } else { "s" }));
    }
    if facts.len() == 1 {
        let _ = writeln!(out, "{} `{}`", facts[0], decl.name);
    } else {
        let _ = writeln!(
            out,
            "{} `{}` — {}",
            facts[0],
            decl.name,
            facts[1..].join(", ")
        );
    }
    if !decl.ports.is_empty() {
        out.push_str("\nports:\n");
        for port in &decl.ports {
            if port.detail.is_empty() {
                let _ = writeln!(out, "- `{}`", port.name);
            } else {
                let _ = writeln!(out, "- `{}`: {}", port.name, port.detail);
            }
        }
    }
    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Document symbols
// ---------------------------------------------------------------------------

/// The document outline: modules and entities, their ports, signals,
/// processes and instances, nested as they are in the source.
pub fn document_symbols(doc: &Document) -> Vec<DocumentSymbol> {
    doc.index()
        .outline()
        .iter()
        .map(|node| symbol(doc, node))
        .collect()
}

/// One outline node as an LSP symbol.
fn symbol(doc: &Document, node: &OutlineNode) -> DocumentSymbol {
    let decl = &doc.index().decls[node.decl];
    let detail = (!decl.detail.is_empty()).then(|| decl.detail.clone());
    DocumentSymbol {
        name: decl.name.clone(),
        detail,
        kind: decl.class.symbol_kind(),
        range: doc.range(decl.full_span),
        selection_range: doc.range(decl.name_span),
        children: node.children.iter().map(|c| symbol(doc, c)).collect(),
    }
}

// ---------------------------------------------------------------------------
// Completion
// ---------------------------------------------------------------------------

/// The completion sort groups, which the client honours through
/// `sortText`: ports of the unit being instantiated first, then names in
/// scope, then reserved words.
const GROUP_PORT: u8 = 0;
const GROUP_NAME: u8 = 1;
const GROUP_KEYWORD: u8 = 2;

/// What can be typed at `position`.
///
/// Three sources, in this order of usefulness: the port names of the unit
/// an instantiation is connecting (the one completion that saves real
/// typing, since those names live in another declaration), the identifiers
/// in scope, and the reserved words that can start something here.
pub fn completion(doc: &Document, position: Position) -> Vec<CompletionItem> {
    let offset = doc.offset(position);
    let prefix = prefix_at(doc.text(), offset);
    let index = doc.index();
    let mut items: Vec<CompletionItem> = Vec::new();

    // Inside an instantiation's connection list, the port names of the
    // instantiated unit are what is wanted.
    let inside_instance = index.instance_at(offset);
    if let Some(site) = inside_instance
        && let Some(target) = index.find(
            &site.target,
            &[
                DeclClass::Module,
                DeclClass::Interface,
                DeclClass::Entity,
                DeclClass::Component,
            ],
        )
    {
        for port in &index.decls[target].ports {
            // A port already connected is not offered again — except the
            // one being typed, which the parser has already seen.
            if site.taken(&port.name, offset, index.case_insensitive)
                || !matches_prefix(&port.name, prefix)
            {
                continue;
            }
            items.push(CompletionItem {
                detail: (!port.detail.is_empty()).then(|| port.detail.clone()),
                sort_text: Some(sort_key(GROUP_PORT, &port.name)),
                ..CompletionItem::new(port.name.clone(), CompletionItemKind::Field)
            });
        }
    }

    // After a `.` only a port name makes sense, so nothing else is added.
    if inside_instance.is_some() && char_before_prefix(doc.text(), offset) == Some('.') {
        items.sort_by(|a, b| a.sort_text.cmp(&b.sort_text));
        return items;
    }

    for decl in index.visible(offset) {
        let decl = &index.decls[decl];
        if decl.name.is_empty() || !matches_prefix(&decl.name, prefix) {
            continue;
        }
        items.push(CompletionItem {
            detail: (!decl.detail.is_empty()).then(|| decl.detail.clone()),
            sort_text: Some(sort_key(GROUP_NAME, &decl.name)),
            ..CompletionItem::new(decl.name.clone(), decl.class.completion_kind())
        });
    }

    for word in keywords(doc, offset) {
        if !matches_prefix(word, prefix) {
            continue;
        }
        items.push(CompletionItem {
            sort_text: Some(sort_key(GROUP_KEYWORD, word)),
            ..CompletionItem::new(word, CompletionItemKind::Keyword)
        });
    }

    items.sort_by(|a, b| {
        a.sort_text
            .cmp(&b.sort_text)
            .then_with(|| a.label.cmp(&b.label))
    });
    items.dedup_by(|a, b| a.label == b.label && a.kind == b.kind);
    items
}

/// The `sortText` that puts `label` in `group`.
fn sort_key(group: u8, label: &str) -> String {
    format!("{group}{label}")
}

/// The reserved words that may start something at `offset`.
fn keywords(doc: &Document, offset: u32) -> Vec<&'static str> {
    let classes = enclosing_classes(doc.index(), offset);
    match doc.language {
        Language::Verilog(dialect) => {
            let context = if classes.iter().any(|c| {
                matches!(
                    c,
                    DeclClass::Process | DeclClass::Function | DeclClass::Procedure
                )
            }) {
                verilog::Context::Statement
            } else if classes.iter().any(|c| {
                matches!(
                    c,
                    DeclClass::Module | DeclClass::Interface | DeclClass::Package
                )
            }) {
                verilog::Context::Item
            } else {
                verilog::Context::File
            };
            verilog::keywords(context, dialect)
        }
        Language::Vhdl(_) => {
            let context = if classes.iter().any(|c| {
                matches!(
                    c,
                    DeclClass::Process | DeclClass::Function | DeclClass::Procedure
                )
            }) {
                vhdl::Context::Sequential
            } else if classes.is_empty() {
                vhdl::Context::File
            } else {
                vhdl::Context::Unit
            };
            vhdl::keywords(context)
        }
    }
}

/// The classes of every declaration whose body contains `offset`,
/// innermost first.
fn enclosing_classes(index: &Index, offset: u32) -> Vec<DeclClass> {
    let mut out = Vec::new();
    let mut current = index.enclosing(offset);
    for _ in 0..=index.decls.len() {
        let Some(id) = current else { break };
        let Some(decl) = index.decls.get(id) else {
            break;
        };
        out.push(decl.class);
        current = decl.parent;
    }
    out
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

/// Why a rename was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameError {
    /// There is no name at the position, or none this document declares.
    NoSymbol,
    /// The new name is not a legal identifier.
    BadName(String),
    /// Some uses of the name are outside the open document, so renaming
    /// here would leave the design inconsistent.
    NotLocal(String),
}

impl RenameError {
    /// The message to show the user.
    pub fn message(&self) -> String {
        match self {
            RenameError::NoSymbol => {
                "there is nothing here that this document declares, so it cannot be renamed"
                    .to_string()
            }
            RenameError::BadName(name) => format!("`{name}` is not a valid identifier"),
            RenameError::NotLocal(message) => message.clone(),
        }
    }
}

/// Renames the symbol at `position` throughout the document.
///
/// Refuses, rather than producing a half-edit, when the name can be seen
/// from outside this document: a module, entity, architecture, package or
/// component name is visible to every other file of the design, and the
/// server only ever sees the files the client has opened. The message says
/// so, which is what the editor shows.
pub fn rename(
    store: &DocumentStore,
    doc: &Document,
    position: Position,
    new_name: &str,
) -> Result<WorkspaceEdit, RenameError> {
    if !is_identifier(new_name) {
        return Err(RenameError::BadName(new_name.to_string()));
    }
    let offset = doc.offset(position);
    let index = doc.index();
    let decl = index.decl_at(offset).ok_or(RenameError::NoSymbol)?;
    let info = &index.decls[decl];

    if info.class.is_file_global() {
        let kind = info.class.describe();
        return Err(RenameError::NotLocal(format!(
            "`{}` is {} {kind} name, which other files of the design may use; \
             the server only sees the documents you have open, so it cannot \
             rename it everywhere",
            info.name,
            article(kind),
        )));
    }
    // A name this document keeps to itself may still be spelled in another
    // open document through a hierarchical reference the server does not
    // model, so say so rather than silently renaming half of it.
    for other in store.iter() {
        if other.uri == doc.uri {
            continue;
        }
        if other
            .index()
            .decls
            .iter()
            .any(|d| d.class.is_file_global() && other.index().same_name(&d.name, &info.name))
        {
            let kind = other
                .index()
                .decls
                .iter()
                .find(|d| d.class.is_file_global() && other.index().same_name(&d.name, &info.name))
                .map_or("declaration", |d| d.class.describe());
            return Err(RenameError::NotLocal(format!(
                "`{}` also names {} {kind} in {}, so renaming it here would not be consistent",
                info.name,
                article(kind),
                other.uri
            )));
        }
    }

    let edits: Vec<TextEdit> = index
        .occurrences(decl)
        .into_iter()
        .map(|span| TextEdit::new(doc.range(span), new_name))
        .collect();
    if edits.is_empty() {
        return Err(RenameError::NoSymbol);
    }
    let mut out = WorkspaceEdit::default();
    out.add(doc.uri.clone(), edits);
    Ok(out)
}

/// `a` or `an`, for a word put in front of a noun.
fn article(word: &str) -> &'static str {
    if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

/// True for a name both languages would lex as one identifier.
///
/// The intersection of the two rules: a letter or underscore, then letters,
/// digits and underscores. Escaped and extended identifiers are not
/// offered, since inserting one would need the escape syntax of the
/// language and the user can type that themselves.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// The formatter options a client's `FormattingOptions` ask for.
///
/// Only indentation is taken from the client; everything else is the house
/// style, which is the point of having one.
pub fn format_options(tab_size: Option<u32>, insert_spaces: Option<bool>) -> FormatOptions {
    let mut opts = FormatOptions::default();
    opts.indent = match (insert_spaces, tab_size) {
        (Some(false), _) => Indent::Tabs,
        (_, Some(size)) if size > 0 => Indent::Spaces(usize::try_from(size).unwrap_or(2)),
        _ => opts.indent,
    };
    opts
}

/// Formats the whole document, as a minimal set of line edits.
///
/// Returns the front end's diagnostics when the document does not parse,
/// which is how the formatters refuse: a file that does not parse cannot
/// be laid out again without losing something.
pub fn formatting(doc: &Document, opts: &FormatOptions) -> Result<Vec<TextEdit>, Diagnostics> {
    let formatted = match doc.language {
        Language::Verilog(dialect) => {
            crate::verilog::format::format_source(doc.text(), dialect, opts)?
        }
        Language::Vhdl(standard) => crate::vhdl::format::format_source(doc.text(), standard, opts)?,
    };
    Ok(line_edits(doc, &formatted, None))
}

/// Formats the document but keeps only the edits that touch `range`.
///
/// The formatters lay out a whole file at a time — a port list's alignment
/// depends on every port, and an expression's breaks on the indentation
/// around it — so range formatting formats everything and then reports
/// only what changed inside the range. That keeps the result identical to
/// what full formatting would have produced there.
pub fn range_formatting(
    doc: &Document,
    range: Range,
    opts: &FormatOptions,
) -> Result<Vec<TextEdit>, Diagnostics> {
    let formatted = match doc.language {
        Language::Verilog(dialect) => {
            crate::verilog::format::format_source(doc.text(), dialect, opts)?
        }
        Language::Vhdl(standard) => crate::vhdl::format::format_source(doc.text(), standard, opts)?,
    };
    Ok(line_edits(doc, &formatted, Some(range)))
}

/// The minimal line-level edits turning the document into `formatted`,
/// optionally restricted to the hunks that overlap `limit`.
fn line_edits(doc: &Document, formatted: &str, limit: Option<Range>) -> Vec<TextEdit> {
    let old: Vec<&str> = doc.text().split('\n').collect();
    let new: Vec<&str> = formatted.split('\n').collect();
    if old == new {
        return Vec::new();
    }
    let script = diff_lines(&old, &new, 100_000);

    let mut edits = Vec::new();
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let mut i = 0;
    while i < script.len() {
        match script[i] {
            Edit::Keep(..) => {
                old_line += 1;
                new_line += 1;
                i += 1;
            }
            _ => {
                // One hunk: the run of deletions and insertions here.
                let old_start = old_line;
                let new_start = new_line;
                while i < script.len() && !matches!(script[i], Edit::Keep(..)) {
                    match script[i] {
                        Edit::Delete(_) => old_line += 1,
                        Edit::Insert(_) => new_line += 1,
                        Edit::Keep(..) => unreachable!("the loop stops on a keep"),
                    }
                    i += 1;
                }
                if let Some(edit) = hunk_edit(doc, &new, old_start, old_line, new_start, new_line)
                    && limit.is_none_or(|limit| overlaps(edit.range, limit))
                {
                    edits.push(edit);
                }
            }
        }
    }
    edits
}

/// The edit replacing old lines `[old_start, old_end)` with new lines
/// `[new_start, new_end)`.
fn hunk_edit(
    doc: &Document,
    new: &[&str],
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
) -> Option<TextEdit> {
    let (Ok(start_line), Ok(end_line)) = (u32::try_from(old_start), u32::try_from(old_end)) else {
        return None;
    };
    let range = Range::new(Position::new(start_line, 0), Position::new(end_line, 0));
    let lines = new.get(new_start..new_end)?;
    // A hunk reaching past the last line has no newline to replace after
    // it, so its replacement must not end with one either.
    let text = if old_end >= new.len().max(1) && old_end >= doc.lines().line_count() as usize {
        lines.join("\n")
    } else {
        lines.iter().map(|line| format!("{line}\n")).collect()
    };
    Some(TextEdit::new(range, text))
}

/// True when two ranges share at least one line.
fn overlaps(a: Range, b: Range) -> bool {
    a.start.line <= b.end.line && b.start.line <= a.end.line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verilog::Dialect;
    use crate::vhdl::Standard;

    const COUNTER_V: &str = "\
module counter #(parameter W = 8) (
  input wire clk,
  output reg [7:0] q
);
  wire [7:0] next;
  assign next = q + 1'b1;
  always @(posedge clk) q <= next;
endmodule

module top;
  wire c;
  wire [7:0] out;
  counter u0 (.clk(c), .q(out));
endmodule
";

    const COUNTER_VHD: &str = "\
entity counter is
  port (clk : in bit;
        q   : out bit_vector(7 downto 0));
end entity counter;

architecture rtl of counter is
  signal count : bit_vector(7 downto 0);
begin
  tick : process (clk) is
  begin
    count <= count;
  end process tick;
  q <= count;
end architecture rtl;
";

    fn verilog(text: &str) -> Document {
        Document::new(
            "file:///t.v",
            Language::Verilog(Dialect::SystemVerilog),
            1,
            text.to_string(),
        )
    }

    fn vhdl(text: &str) -> Document {
        Document::new(
            "file:///t.vhd",
            Language::Vhdl(Standard::Vhdl2008),
            1,
            text.to_string(),
        )
    }

    /// The position of the `n`-th occurrence of `needle`, at its start.
    fn pos(doc: &Document, needle: &str, n: usize) -> Position {
        let offset = doc
            .text()
            .match_indices(needle)
            .nth(n)
            .unwrap_or_else(|| panic!("occurrence {n} of {needle:?}"))
            .0;
        doc.position(u32::try_from(offset).unwrap())
    }

    #[test]
    fn definition_jumps_to_the_declaration() {
        let doc = verilog(COUNTER_V);
        // The `next` in `assign next = ...` goes to `wire [7:0] next;`.
        let location = definition(&doc, pos(&doc, "next", 1)).expect("a definition");
        assert_eq!(location.uri, "file:///t.v");
        assert_eq!(location.range.start, pos(&doc, "next", 0));
        // A module instantiation goes to the module.
        let location = definition(&doc, pos(&doc, "counter u0", 0)).expect("a definition");
        assert_eq!(location.range.start, pos(&doc, "counter #", 0));
    }

    #[test]
    fn definition_works_for_vhdl() {
        let doc = vhdl(COUNTER_VHD);
        let location = definition(&doc, pos(&doc, "count <=", 0)).expect("a definition");
        assert_eq!(location.range.start, pos(&doc, "count :", 0));
        // `q <= count` goes to the port.
        let location = definition(&doc, pos(&doc, "q <= count", 0)).expect("a definition");
        assert_eq!(location.range.start, pos(&doc, "q   :", 0));
    }

    #[test]
    fn definition_says_nothing_about_names_it_cannot_place() {
        let doc = vhdl(COUNTER_VHD);
        // `bit_vector` comes from `std.standard`, which is not this file.
        assert!(definition(&doc, pos(&doc, "bit_vector", 0)).is_none());
        // Whitespace is not a name.
        assert!(definition(&doc, Position::new(4, 0)).is_none());
    }

    #[test]
    fn references_lists_every_use() {
        let doc = verilog(COUNTER_V);
        let all = references(&doc, pos(&doc, "next", 1), true);
        assert_eq!(all.len(), 3);
        let without = references(&doc, pos(&doc, "next", 1), false);
        assert_eq!(without.len(), 2);
        assert!(
            without
                .iter()
                .all(|l| l.range.start != pos(&doc, "next", 0))
        );

        let doc = vhdl(COUNTER_VHD);
        assert_eq!(references(&doc, pos(&doc, "count :", 0), true).len(), 4);
    }

    #[test]
    fn hover_shows_the_type_and_width() {
        let doc = verilog(COUNTER_V);
        let shown = hover(&doc, pos(&doc, "q <= next", 0)).expect("a hover");
        assert!(
            shown.contents.contains("output reg [7:0]"),
            "{}",
            shown.contents
        );
        assert!(shown.contents.contains("8 bits"), "{}", shown.contents);
        assert!(shown.range.is_some());

        let doc = vhdl(COUNTER_VHD);
        let shown = hover(&doc, pos(&doc, "count <=", 0)).expect("a hover");
        assert!(
            shown.contents.contains("bit_vector(7 downto 0)"),
            "{}",
            shown.contents
        );
        assert!(shown.contents.contains("8 bits"), "{}", shown.contents);
    }

    #[test]
    fn hover_on_a_module_lists_its_ports() {
        let doc = verilog(COUNTER_V);
        let shown = hover(&doc, pos(&doc, "counter u0", 0)).expect("a hover");
        assert!(shown.contents.contains("ports:"), "{}", shown.contents);
        assert!(
            shown.contents.contains("- `clk`: input wire"),
            "{}",
            shown.contents
        );
        assert!(shown.contents.contains("- `q`: output reg [7:0]"));

        let doc = vhdl(COUNTER_VHD);
        let shown = hover(&doc, pos(&doc, "counter is", 0)).expect("a hover");
        assert!(
            shown.contents.contains("- `clk`: in bit"),
            "{}",
            shown.contents
        );
    }

    #[test]
    fn hover_describes_a_name_from_a_bundled_library() {
        let doc = vhdl(
            "library ieee;\nuse ieee.std_logic_1164.all;\nentity e is port (a : in std_logic); end entity;\n",
        );
        let shown = hover(&doc, pos(&doc, "std_logic)", 0)).expect("a hover");
        assert!(
            shown.contents.contains("subtype `std_logic`"),
            "{}",
            shown.contents
        );
        assert!(
            shown.contents.contains("declared in `<reticle>/ieee/"),
            "{}",
            shown.contents
        );
        // The range is the name itself, which is in this document.
        assert!(shown.range.is_some());
    }

    #[test]
    fn hover_falls_back_to_the_last_clean_parse_without_a_range() {
        let mut doc = verilog(COUNTER_V);
        doc.apply_change(None, "module counter #(parameter W = 8) (\n  input wire clk\n);\n  wire [7:0] next;\n  assign next = ;\n");
        assert!(!doc.is_healthy());
        let shown = hover(&doc, pos(&doc, "next = ", 0));
        let shown = shown.expect("the previous parse still knows `next`");
        assert!(shown.contents.contains("wire [7:0]"));
        // No range: the spans belong to a revision the client has replaced.
        assert!(shown.range.is_none());
    }

    #[test]
    fn document_symbols_nest() {
        let doc = verilog(COUNTER_V);
        let symbols = document_symbols(&doc);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["counter", "top"]);
        let children: Vec<&str> = symbols[0]
            .children
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(children, ["W", "clk", "q", "next", "always"]);
        assert_eq!(symbols[0].kind, super::super::protocol::SymbolKind::Module);
        // The instance shows up under `top`.
        assert_eq!(symbols[1].children.last().unwrap().name, "u0");

        let doc = vhdl(COUNTER_VHD);
        let symbols = document_symbols(&doc);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "counter");
        assert_eq!(symbols[1].name, "rtl");
        assert_eq!(symbols[1].children[1].name, "tick");
    }

    #[test]
    fn completion_offers_port_names_inside_a_connection_list() {
        let doc = verilog(COUNTER_V);
        // Just after the `.` of `.q(out)`.
        let offset = u32::try_from(doc.text().find(".q(out)").unwrap() + 1).unwrap();
        let items = completion(&doc, doc.position(offset));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // `clk` is already connected, `q` is the one being typed.
        assert_eq!(labels, ["q"]);
        assert_eq!(items[0].kind, CompletionItemKind::Field);
        assert_eq!(items[0].detail.as_deref(), Some("output reg [7:0]"));
    }

    #[test]
    fn completion_offers_ports_in_a_vhdl_port_map() {
        let src = "\
entity sub is port (clk : in bit; rst : in bit); end entity;
architecture s of sub is begin end architecture;

entity top is end entity;
architecture t of top is
  component sub is port (clk : in bit; rst : in bit); end component;
  signal c : bit;
begin
  u0 : sub port map (clk => c, rst => c);
end architecture;
";
        let doc = vhdl(src);
        let offset = u32::try_from(doc.text().find("rst => c").unwrap()).unwrap();
        let items = completion(&doc, doc.position(offset));
        assert!(
            items
                .iter()
                .any(|i| i.label == "rst" && i.kind == CompletionItemKind::Field),
            "{items:?}"
        );
    }

    #[test]
    fn completion_offers_names_in_scope_and_keywords() {
        let doc = verilog(COUNTER_V);
        let offset = u32::try_from(doc.text().find("q + 1'b1").unwrap()).unwrap();
        let items = completion(&doc, doc.position(offset));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"q"), "{labels:?}");
        assert!(labels.contains(&"next"));
        assert!(labels.contains(&"W"));
        // Names sort before keywords.
        let first_keyword = items
            .iter()
            .position(|i| i.kind == CompletionItemKind::Keyword)
            .unwrap();
        let last_name = items
            .iter()
            .rposition(|i| i.kind != CompletionItemKind::Keyword)
            .unwrap();
        assert!(last_name < first_keyword);
        // The list is sorted and free of duplicates.
        let keys: Vec<&String> = items.iter().filter_map(|i| i.sort_text.as_ref()).collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn completion_filters_by_what_is_typed() {
        let doc = verilog("module m;\n  wire alpha, beta;\n  assign alpha = al\nendmodule\n");
        let offset = u32::try_from(doc.text().find("= al").unwrap() + 4).unwrap();
        let labels: Vec<String> = completion(&doc, doc.position(offset))
            .into_iter()
            .map(|i| i.label)
            .collect();
        assert!(labels.contains(&"alpha".to_string()));
        assert!(!labels.contains(&"beta".to_string()));
    }

    #[test]
    fn rename_rewrites_every_occurrence() {
        let doc = verilog(COUNTER_V);
        let store = DocumentStore::new();
        let edit = rename(&store, &doc, pos(&doc, "next", 1), "counted").expect("renames");
        assert_eq!(edit.changes.len(), 1);
        let (uri, edits) = &edit.changes[0];
        assert_eq!(uri, "file:///t.v");
        assert_eq!(edits.len(), 3);
        assert!(edits.iter().all(|e| e.new_text == "counted"));

        let doc = vhdl(COUNTER_VHD);
        let edit = rename(&store, &doc, pos(&doc, "count :", 0), "value").expect("renames");
        assert_eq!(edit.changes[0].1.len(), 4);
    }

    #[test]
    fn rename_refuses_what_it_cannot_see_all_of() {
        let doc = verilog(COUNTER_V);
        let store = DocumentStore::new();
        // A module name reaches every other file.
        let err = rename(&store, &doc, pos(&doc, "counter #", 0), "ctr").unwrap_err();
        assert!(matches!(err, RenameError::NotLocal(_)));
        assert!(err.message().contains("module"));
        assert!(err.message().contains("open"));

        // Nothing under the cursor.
        assert_eq!(
            rename(&store, &doc, Position::new(3, 0), "x"),
            Err(RenameError::NoSymbol)
        );
        // An illegal new name.
        assert!(matches!(
            rename(&store, &doc, pos(&doc, "next", 1), "2bad"),
            Err(RenameError::BadName(_))
        ));
        assert!(matches!(
            rename(&store, &doc, pos(&doc, "next", 1), ""),
            Err(RenameError::BadName(_))
        ));
    }

    #[test]
    fn rename_refuses_when_another_open_document_declares_the_name() {
        let mut store = DocumentStore::new();
        store.open(Document::new(
            "file:///other.v",
            Language::Verilog(Dialect::Verilog2005),
            1,
            "module next_stage; endmodule\n".to_string(),
        ));
        let doc =
            verilog("module m;\n  wire next_stage;\n  assign next_stage = 1'b0;\nendmodule\n");
        let err = rename(&store, &doc, pos(&doc, "next_stage", 0), "x").unwrap_err();
        assert!(
            err.message().contains("file:///other.v"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn formatting_returns_minimal_edits() {
        let doc = verilog("module   m ;\n  wire a;\nendmodule\n");
        let opts = FormatOptions::default();
        let edits = formatting(&doc, &opts).expect("formats");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "module m;\n");
        assert_eq!(
            edits[0].range,
            Range::new(Position::new(0, 0), Position::new(1, 0))
        );
        // An already formatted document needs no edits at all.
        let doc = verilog("module m;\n  wire a;\nendmodule\n");
        assert!(formatting(&doc, &opts).expect("formats").is_empty());
    }

    #[test]
    fn formatting_refuses_a_document_that_does_not_parse() {
        let doc = verilog("module m(;\nendmodule\n");
        assert!(formatting(&doc, &FormatOptions::default()).is_err());
    }

    #[test]
    fn range_formatting_keeps_only_the_hunks_asked_for() {
        // Two hunks with an untouched line between them, so the range can
        // pick out one of them.
        let doc = verilog("module m;\n  wire   a;\n  wire b;\n  wire   c;\nendmodule\n");
        let opts = FormatOptions::default();
        let all = formatting(&doc, &opts).expect("formats");
        assert_eq!(all.len(), 2);
        let limited = range_formatting(
            &doc,
            Range::new(Position::new(3, 0), Position::new(3, 11)),
            &opts,
        )
        .expect("formats");
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].new_text, "  wire c;\n");
    }

    #[test]
    fn formatting_vhdl_goes_through_its_own_formatter() {
        let doc = vhdl("entity   e is port(a:in bit); end;\n");
        let edits = formatting(&doc, &FormatOptions::default()).expect("formats");
        assert!(!edits.is_empty());
        assert!(edits[0].new_text.starts_with("entity e is"), "{edits:?}");
    }

    #[test]
    fn formatting_options_follow_the_client() {
        assert_eq!(
            format_options(Some(4), Some(true)).indent,
            Indent::Spaces(4)
        );
        assert_eq!(format_options(Some(4), Some(false)).indent, Indent::Tabs);
        assert_eq!(format_options(None, None).indent, Indent::Spaces(2));
        assert_eq!(
            format_options(Some(0), Some(true)).indent,
            Indent::Spaces(2)
        );
    }

    #[test]
    fn a_document_without_a_trailing_newline_formats() {
        let doc = verilog("module   m ;\n  wire a;\nendmodule");
        let edits = formatting(&doc, &FormatOptions::default()).expect("formats");
        // Applying the edits must reproduce the formatted text exactly.
        let mut text = doc.text().to_string();
        for edit in edits.iter().rev() {
            let start = doc.offset(edit.range.start) as usize;
            let end = doc.offset(edit.range.end) as usize;
            text.replace_range(start..end, &edit.new_text);
        }
        assert_eq!(text, "module m;\n  wire a;\nendmodule\n");
    }
}
