//! Elaboration: from the Verilog AST to the unified IR.
//!
//! [`elaborate`] takes the parsed files of one compilation, resolves every
//! name, evaluates every constant, unrolls every generate region, picks a
//! top module and lowers the whole hierarchy to an [`crate::ir::Design`] that has
//! passed [`crate::ir::validate`].
//!
//! # Order of elaboration
//!
//! 1. **Index.** Every module, interface, program, primitive and package
//!    is collected into a table (`hier::Table`); names declared twice are
//!    reported here.
//! 2. **Compilation unit.** Items outside any module or package (shared
//!    typedefs, parameters, functions) are declared in the `$unit` scope.
//!    Packages are elaborated lazily, on their first `import` or `::` use,
//!    so a package that nothing needs costs nothing and an import cycle is
//!    reported rather than looped on.
//! 3. **Top selection.** The modules that no other module instantiates are
//!    the roots (`hier::Table::roots`); `ElabOptions::top` names the one
//!    that becomes [`crate::ir::Design::top`], and every other root is elaborated
//!    too, so a library of independent modules lowers completely.
//! 4. **Per module, three passes** (see `lower`): subroutines, then
//!    declarations in source order with the instantiation's parameter
//!    overrides applied, then behaviour. Instances recurse into this step;
//!    a module is elaborated once per distinct parameter set and gets a
//!    uniquified name.
//! 5. **Validation.** The finished design is checked with
//!    [`crate::ir::validate`]; a violation is a bug in this frontend and is
//!    reported as an internal error (`V0024`).
//!
//! # Sizing rules
//!
//! Widths and signedness follow IEEE 1364-2005 §5.4 and §5.5, with the
//! details and the standard's table in `width`. The short version:
//! every expression has a self-determined width; the operands of
//! arithmetic, bitwise and conditional operators, and the value of an
//! assignment, are widened to the context's width before the operator
//! runs; comparison, reduction, logical and concatenation operands are
//! self-determined; an expression is signed only when all of its
//! context-determined operands are. The IR requires operands of equal
//! width, so each of those rules becomes an explicit `Resize` node, and a
//! context that truncates its value warns (`V0007`).
//!
//! # Diagnostics
//!
//! Every diagnostic carries a `V0001`-style code; the table is in
//! `codes`.
//!
//! # Example
//!
//! ```
//! use reticle::diag::Diagnostics;
//! use reticle::source::SourceMap;
//! use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};
//!
//! let mut map = SourceMap::new();
//! let id = map
//!     .add(
//!         "counter.v",
//!         "module counter(input clk, output reg [7:0] q);\n\
//!          always @(posedge clk) q <= q + 1;\n\
//!          endmodule\n",
//!     )
//!     .unwrap();
//! let mut diags = Diagnostics::new();
//! let file = parse_source(&mut map, id, Dialect::Verilog2005, &mut NoIncludes, &mut diags);
//! let design = elaborate(&[&file], &ElabOptions::default(), &mut diags).unwrap();
//! assert!(!diags.has_errors());
//! let top = design.top_module().unwrap();
//! assert_eq!(top.name, "counter");
//! assert_eq!(top.processes.len(), 1);
//! ```

pub mod codes;
pub mod constant;
pub(crate) mod decls;
pub(crate) mod env;
pub(crate) mod hier;
pub(crate) mod lower;
pub(crate) mod package;
pub mod scope;
pub mod types;
pub mod width;

use crate::diag::Diagnostics;
use crate::ir::Design;
use crate::verilog::ast::SourceFile;
use crate::verilog::token::Dialect;

/// How a design is elaborated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElabOptions {
    /// The module to use as the top of the hierarchy. When absent, the
    /// first module that no other module instantiates is used.
    pub top: Option<String>,
    /// Parameter overrides for the top module, as `(name, value)` pairs
    /// whose value is parsed as a Verilog literal, or kept as a string
    /// when it is not one.
    pub params: Vec<(String, String)>,
    /// The dialect the sources were parsed in; it selects the rules that
    /// differ between Verilog-2005 and SystemVerilog, such as whether a
    /// variable may be driven by a continuous assignment.
    pub dialect: Dialect,
}

impl ElabOptions {
    /// Options with no top and no overrides, in the given dialect.
    pub fn new(dialect: Dialect) -> Self {
        ElabOptions {
            top: None,
            params: Vec::new(),
            dialect,
        }
    }

    /// The same options with `top` as the top module.
    pub fn with_top(mut self, top: impl Into<String>) -> Self {
        self.top = Some(top.into());
        self
    }

    /// The same options with one more parameter override.
    pub fn with_param(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((name.into(), value.into()));
        self
    }
}

/// Elaborates parsed source files into a design.
///
/// Every file is treated as part of one compilation: they share the
/// compilation-unit scope, and a module in one file may instantiate a
/// module in another. Diagnostics are appended to `diags`; `None` means at
/// least one error was reported and no usable design was produced.
///
/// The returned design has a `top` when one could be selected, holds every
/// root module of the sources, and satisfies [`crate::ir::validate`].
pub fn elaborate(
    files: &[&SourceFile],
    opts: &ElabOptions,
    diags: &mut Diagnostics,
) -> Option<Design> {
    let with_dialect: Vec<(&SourceFile, Dialect)> =
        files.iter().map(|f| (*f, opts.dialect)).collect();
    lower::lower_all(&with_dialect, opts, diags)
}

/// Elaborates one parsed file, the common case.
///
/// See [`elaborate`] for the details.
pub fn elaborate_file(
    file: &SourceFile,
    opts: &ElabOptions,
    diags: &mut Diagnostics,
) -> Option<Design> {
    elaborate(&[file], opts, diags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;
    use crate::verilog::{NoIncludes, parse_source};

    /// Parses and elaborates one snippet, returning the design (if any)
    /// and the rendered diagnostics.
    pub(crate) fn elab(text: &str, dialect: Dialect) -> (Option<Design>, String) {
        elab_with(text, &ElabOptions::new(dialect))
    }

    /// Like [`elab`] with explicit options.
    pub(crate) fn elab_with(text: &str, opts: &ElabOptions) -> (Option<Design>, String) {
        let mut map = SourceMap::new();
        let name = if opts.dialect == Dialect::SystemVerilog {
            "t.sv"
        } else {
            "t.v"
        };
        let id = map.add(name, text).unwrap();
        let mut diags = Diagnostics::new();
        let file = parse_source(&mut map, id, opts.dialect, &mut NoIncludes, &mut diags);
        assert!(
            !diags.has_errors(),
            "the snippet must parse:\n{}",
            diags.render(&map)
        );
        let design = elaborate(&[&file], opts, &mut diags);
        diags.sort();
        (design, diags.render(&map))
    }

    /// Elaborates `module t;` with `decls` and `localparam R = <expr>;`
    /// and returns the text of the resulting parameter value.
    ///
    /// This is how the constant-evaluation tests check the sizing rules:
    /// the value keeps its width and signedness, so `8'd255 + 8'd1`
    /// renders as `8'd0` and `-1` as `32'sd-1`.
    pub(crate) fn param_text(decls: &str, expr: &str) -> String {
        let text = format!("module t;\n{decls}\nlocalparam R = {expr};\nendmodule\n");
        let (design, diags) = elab(&text, Dialect::SystemVerilog);
        let design = design.unwrap_or_else(|| panic!("`{expr}` should elaborate:\n{diags}"));
        let m = design.module_by_name("t").expect("module t");
        let value = &design
            .module(m)
            .param("R")
            .unwrap_or_else(|| panic!("no parameter R in:\n{}", design.to_text()))
            .value;
        // Known integral values render in decimal, which is what the
        // sizing rules are easiest to read in; everything else keeps the
        // canonical form.
        match value {
            crate::ir::AttrValue::Const(c) if c.is_fully_known() && c.width() <= 64 => {
                let digits = if c.is_signed() {
                    c.to_i64().map(|v| v.to_string())
                } else {
                    c.to_u64().map(|v| v.to_string())
                };
                match digits {
                    Some(d) => format!(
                        "{}'{}{d}",
                        c.width(),
                        if c.is_signed() { "sd" } else { "d" }
                    ),
                    None => c.to_string(),
                }
            }
            other => other.to_string(),
        }
    }

    /// The `.rtl` text of an elaborated snippet.
    pub(crate) fn rtl(text: &str, dialect: Dialect) -> String {
        let (design, diags) = elab(text, dialect);
        assert!(design.is_some(), "elaboration failed:\n{diags}");
        design.unwrap().to_text()
    }

    #[test]
    fn elaborates_a_counter() {
        let text = "module counter(input clk, input rst, output reg [7:0] q);\n\
                    always @(posedge clk) if (rst) q <= 8'd0; else q <= q + 1;\n\
                    endmodule\n";
        let out = rtl(text, Dialect::Verilog2005);
        assert!(out.contains("top counter"), "{out}");
        assert!(out.contains("net %q u8 reg"), "{out}");
        assert!(out.contains("process seq posedge %clk"), "{out}");
        assert!(out.contains("%q <= add(%q, 8'd1)"), "{out}");
    }

    #[test]
    fn options_build() {
        let o = ElabOptions::new(Dialect::SystemVerilog)
            .with_top("top")
            .with_param("W", "8");
        assert_eq!(o.top.as_deref(), Some("top"));
        assert_eq!(o.params, [("W".to_owned(), "8".to_owned())]);
        assert_eq!(ElabOptions::default().dialect, Dialect::SystemVerilog);
    }

    #[test]
    fn reports_a_missing_top() {
        let opts = ElabOptions::new(Dialect::Verilog2005).with_top("nope");
        let (design, diags) = elab_with("module m; endmodule\n", &opts);
        assert!(design.is_none());
        assert!(diags.contains("V0022"), "{diags}");
    }

    #[test]
    fn elaborate_file_matches_elaborate() {
        let mut map = SourceMap::new();
        let id = map
            .add(
                "t.v",
                "module m(input a, output y); assign y = ~a; endmodule\n",
            )
            .unwrap();
        let mut diags = Diagnostics::new();
        let file = parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        );
        let opts = ElabOptions::new(Dialect::Verilog2005);
        let one = elaborate_file(&file, &opts, &mut diags).unwrap();
        let two = elaborate(&[&file], &opts, &mut diags).unwrap();
        assert_eq!(one.to_text(), two.to_text());
    }

    /// A string literal is an unsigned integer constant, so it may be
    /// passed where a vector is wanted.
    ///
    /// IEEE 1364-2005 makes `"T"` mean `8'h54`. Assigning one to a
    /// register always worked, because the assignment says how wide the
    /// value must be before it is folded. A task argument says nothing,
    /// so the literal stayed a string until the argument was coerced,
    /// and coercion refused it: `sendchar("T")` was rejected while
    /// `c = "T";` right beside it was accepted. Two of this repository's
    /// own testbenches carry a comment about the workaround.
    #[test]
    fn a_string_literal_passes_as_a_vector_argument() {
        let text = "module t;\n\
             \x20   reg [7:0] last;\n\
             \x20   reg [15:0] pair;\n\
             \x20   task sendchar(input [7:0] c);\n\
             \x20       last = c;\n\
             \x20   endtask\n\
             \x20   task sendpair(input [15:0] p);\n\
             \x20       pair = p;\n\
             \x20   endtask\n\
             \x20   initial begin\n\
             \x20       sendchar(\"T\");\n\
             \x20       sendpair(\"Hi\");\n\
             \x20   end\n\
             endmodule\n";
        let (design, diags) = elab(text, Dialect::Verilog2005);
        let design = design.unwrap_or_else(|| panic!("should elaborate:\n{diags}"));
        let rtl = design.to_text();
        // `T` is 0x54, which the IR prints in decimal as 84, and `Hi`
        // is 0x4869, which is 18537: the literal is folded to bits at
        // the width the argument asks for, rather than reaching the IR
        // as a string.
        assert!(rtl.contains("8'd84"), "`T` is not 84 in:\n{rtl}");
        assert!(rtl.contains("16'd18537"), "`Hi` is not 18537 in:\n{rtl}");
        assert!(
            !rtl.contains("\"T\""),
            "the literal reached the IR as a string:\n{rtl}"
        );
    }

    /// A real is still refused where a vector is wanted: the fix above
    /// is about strings, and must not have opened the other door.
    #[test]
    fn a_real_net_is_still_not_a_vector() {
        let text = "module t;\n\
             \x20   real r;\n\
             \x20   reg [7:0] q;\n\
             \x20   task take(input [7:0] c);\n\
             \x20       q = c;\n\
             \x20   endtask\n\
             \x20   initial take(r);\n\
             endmodule\n";
        let (_, diags) = elab(text, Dialect::Verilog2005);
        assert!(
            diags.contains("cannot be used as a"),
            "a real argument should still be refused:\n{diags}"
        );
    }
}
