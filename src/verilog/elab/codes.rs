//! Diagnostic codes of the Verilog elaborator.
//!
//! Every diagnostic elaboration emits carries one of these stable codes, so
//! tests can assert on them and users can look them up. Codes are never
//! reused for a different meaning.
//!
//! | Code    | Meaning                                                      |
//! |---------|--------------------------------------------------------------|
//! | `V0001` | Identifier not found in scope (with a suggestion)            |
//! | `V0002` | Module, interface or primitive not found                     |
//! | `V0003` | Instance connects a port the target does not have            |
//! | `V0004` | Port left unconnected (warning)                              |
//! | `V0005` | Width mismatch on an instance connection                     |
//! | `V0006` | Override of a parameter the module does not declare          |
//! | `V0007` | Assignment truncates its value (warning)                     |
//! | `V0008` | A net or variable has more than one driver                   |
//! | `V0009` | Procedural assignment to a net                               |
//! | `V0010` | Continuous assignment to a variable                          |
//! | `V0011` | Expression is not constant where a constant is required      |
//! | `V0012` | Recursion in a constant function                             |
//! | `V0013` | Unsupported construct                                        |
//! | `V0014` | Name declared twice in one scope                             |
//! | `V0015` | Constant expression cannot be evaluated                      |
//! | `V0016` | Package problem: missing, cyclic, or missing member          |
//! | `V0017` | Hierarchical reference that cannot be resolved               |
//! | `V0018` | Constant select is outside its operand (warning)             |
//! | `V0019` | Wrong arguments in a call                                    |
//! | `V0020` | Evaluation of a constant function hit its step limit         |
//! | `V0021` | Implicit net created, or forbidden by `` `default_nettype `` |
//! | `V0022` | No top module, or the requested top does not exist           |
//! | `V0023` | Instantiated module is not in the source (warning, blackbox) |
//! | `V0024` | Internal error: the lowered IR failed validation             |
//! | `V0025` | A value has a type the construct does not accept             |

/// `V0001`: identifier not found in scope.
pub const UNDEFINED: &str = "V0001";
/// `V0002`: module, interface or primitive not found.
pub const MODULE_NOT_FOUND: &str = "V0002";
/// `V0003`: instance connects a port the target module does not have.
pub const PORT_NOT_FOUND: &str = "V0003";
/// `V0004`: a port of an instance is left unconnected.
pub const PORT_UNCONNECTED: &str = "V0004";
/// `V0005`: width mismatch on an instance port connection.
pub const PORT_WIDTH: &str = "V0005";
/// `V0006`: override of a parameter the module does not declare.
pub const PARAM_NOT_FOUND: &str = "V0006";
/// `V0007`: an assignment truncates its value.
pub const TRUNCATION: &str = "V0007";
/// `V0008`: a net or variable has more than one driver.
pub const MULTIPLE_DRIVERS: &str = "V0008";
/// `V0009`: procedural assignment to a net.
pub const PROC_ASSIGN_NET: &str = "V0009";
/// `V0010`: continuous assignment to a variable.
pub const CONT_ASSIGN_VAR: &str = "V0010";
/// `V0011`: an expression is not constant where one is required.
pub const NOT_CONSTANT: &str = "V0011";
/// `V0012`: a constant function is recursive.
pub const RECURSION: &str = "V0012";
/// `V0013`: an unsupported construct.
pub const UNSUPPORTED: &str = "V0013";
/// `V0014`: a name is declared twice in one scope.
pub const DUPLICATE: &str = "V0014";
/// `V0015`: a constant expression cannot be evaluated.
pub const CONST_EVAL: &str = "V0015";
/// `V0016`: a package is missing, cyclic, or has no such member.
pub const PACKAGE: &str = "V0016";
/// `V0017`: a hierarchical reference cannot be resolved.
pub const HIERARCHY: &str = "V0017";
/// `V0018`: a constant select lies outside its operand.
pub const OUT_OF_RANGE: &str = "V0018";
/// `V0019`: wrong arguments in a call.
pub const ARGUMENTS: &str = "V0019";
/// `V0020`: constant function evaluation hit its step limit.
pub const LIMIT: &str = "V0020";
/// `V0021`: an implicit net was created, or is forbidden here.
pub const IMPLICIT_NET: &str = "V0021";
/// `V0022`: no top module, or the requested top does not exist.
pub const TOP: &str = "V0022";
/// `V0023`: an instantiated module is not part of the source.
pub const BLACKBOX: &str = "V0023";
/// `V0024`: the lowered IR failed validation, which is a compiler bug.
pub const INTERNAL: &str = "V0024";
/// `V0025`: a value has a type the construct does not accept.
pub const TYPE: &str = "V0025";
