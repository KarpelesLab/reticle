//! Diagnostic codes of the VHDL elaborator.
//!
//! The VHDL frontend numbers its diagnostics `V0xxx`, one hundred per
//! phase: `V01xx` libraries and design units, `V02xx` names and
//! visibility, `V03xx` types, `V04xx` statements, `V05xx` associations and
//! `V06xx` subprograms, all raised by [`crate::vhdl::sema`]. Elaboration
//! and lowering own **`V07xx`**, and the whole range is listed below.
//! Codes are never reused for a different meaning.
//!
//! | Code    | Meaning                                                        |
//! |---------|----------------------------------------------------------------|
//! | `V0700` | No top entity, or the requested top does not exist             |
//! | `V0701` | An unsupported construct, named explicitly                     |
//! | `V0702` | A component instance cannot be bound to an entity              |
//! | `V0703` | A generic has no value and no default                          |
//! | `V0704` | A value must be static here (generate bound, constraint, index)|
//! | `V0705` | Recursion in a subprogram                                      |
//! | `V0706` | A signal has drivers that cannot be resolved                   |
//! | `V0707` | A type has no synthesisable representation (access, file, ...) |
//! | `V0708` | A port map actual cannot be connected                          |
//! | `V0709` | The hierarchy is recursive                                     |
//! | `V0710` | A call into a package Reticle does not bundle yet              |
//! | `V0711` | Internal error: the lowered IR failed validation               |
//! | `V0712` | An unrolled loop or generate exceeded its limit                |
//! | `V0713` | A value does not fit the object it is assigned to              |
//! | `V0714` | An entity has no architecture                                  |

/// `V0700`: no top entity, or the requested top does not exist.
pub const TOP: &str = "V0700";
/// `V0701`: an unsupported construct.
pub const UNSUPPORTED: &str = "V0701";
/// `V0702`: a component instance cannot be bound.
pub const UNBOUND: &str = "V0702";
/// `V0703`: a generic has no value and no default.
pub const GENERIC: &str = "V0703";
/// `V0704`: an expression must be static here.
pub const NOT_STATIC: &str = "V0704";
/// `V0705`: recursion in a subprogram.
pub const RECURSION: &str = "V0705";
/// `V0706`: a signal has drivers that cannot be resolved.
pub const MULTIPLE_DRIVERS: &str = "V0706";
/// `V0707`: a type has no synthesisable representation.
pub const TYPE: &str = "V0707";
/// `V0708`: a port map actual cannot be connected.
pub const PORT_MAP: &str = "V0708";
/// `V0709`: the hierarchy is recursive.
pub const HIERARCHY: &str = "V0709";
/// `V0710`: a call into a package that is not bundled.
pub const NOT_BUNDLED: &str = "V0710";
/// `V0711`: the lowered IR failed validation, which is a compiler bug.
pub const INTERNAL: &str = "V0711";
/// `V0712`: an unrolled loop or generate exceeded its limit.
pub const LIMIT: &str = "V0712";
/// `V0713`: a value does not fit its target.
pub const WIDTH: &str = "V0713";
/// `V0714`: an entity has no architecture.
pub const NO_ARCHITECTURE: &str = "V0714";
