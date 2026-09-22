/*
 * reticle.h - the C API of Reticle, a VHDL / Verilog compiler.
 *
 * Reticle takes a hardware design from Verilog or VHDL source text through
 * a unified IR to simulation, synthesis and netlist emission. This header
 * exposes that pipeline to a program written in any language with a C FFI.
 *
 * The header is written by hand and kept in step with the Rust side by
 * `tests/ffi_header.rs`, which fails if either gains a symbol the other
 * does not have. `docs/ffi.md` has the build commands and a tour, and
 * `examples/ffi/demo.c` is a worked example.
 *
 *
 * RULES THAT HOLD EVERYWHERE
 *
 *   Handles. reticle_design, reticle_sim and reticle_diagnostics are
 *   opaque. Each comes from a named constructor and is released by its
 *   _free, which ignores NULL. Handles are independent: a simulator keeps
 *   its own copy of the design, so they may be freed in any order.
 *
 *   Status and out-parameters. Every fallible call returns one of the
 *   RETICLE_* codes and writes its result through a pointer argument. A
 *   non-zero status leaves every out-parameter untouched, so checking the
 *   status is enough; nothing is half-written.
 *
 *   Strings. A char ** out-parameter receives a NUL-terminated, UTF-8,
 *   heap-allocated string that YOU free with reticle_string_free. The two
 *   const char * returns (reticle_version, reticle_status_message) point
 *   at static storage and must NOT be freed.
 *
 *   Sans-I/O. The library never opens a file. Where an API says "a list of
 *   files" it means a list of (name, text) pairs: you read the bytes, the
 *   name is what diagnostics print. Verilog `include is reported, not
 *   resolved; pass the included text yourself.
 *
 *   No panics. Nothing unwinds across this boundary: an internal panic is
 *   caught and reported as RETICLE_ERR_PANIC. reticle_self_test_panic
 *   exists so you can check that against your own build.
 *
 *   One ABI. Every symbol here exists in every build. A call into a stage
 *   that was not compiled in returns RETICLE_ERR_UNSUPPORTED;
 *   reticle_features says which stages are present.
 *
 *   Threads. A handle carries no locking. Different handles may be driven
 *   from different threads; one handle must not be used from two at once.
 *
 * SPDX-License-Identifier: MIT
 */

#ifndef RETICLE_H
#define RETICLE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------------
 * Status codes
 * ------------------------------------------------------------------ */

/** The call succeeded and every out-parameter was written. */
#define RETICLE_OK 0
/** A NULL pointer, a string that is not UTF-8, or an index out of range. */
#define RETICLE_ERR_INVALID 1
/** The stage ran and reported errors; the diagnostics handle has them. */
#define RETICLE_ERR_DIAGNOSTICS 2
/** A name (a net path, a module, an output format) does not exist. */
#define RETICLE_ERR_NOT_FOUND 3
/** The stage this call needs was not compiled into the library. */
#define RETICLE_ERR_UNSUPPORTED 4
/** The operation failed for a reason that is a bug rather than bad input. */
#define RETICLE_ERR_INTERNAL 5
/** The value has x or z bits and has no integer form. */
#define RETICLE_ERR_UNKNOWN_VALUE 6
/** A panic was caught at the boundary. */
#define RETICLE_ERR_PANIC 7

/** Severities, as reported by reticle_diagnostic_severity. */
#define RETICLE_SEVERITY_HELP 0
#define RETICLE_SEVERITY_NOTE 1
#define RETICLE_SEVERITY_WARNING 2
#define RETICLE_SEVERITY_ERROR 3

/** Simulation states, as reported by reticle_sim_status. */
#define RETICLE_SIM_RUNNING 0
#define RETICLE_SIM_STOPPED 1
#define RETICLE_SIM_FINISHED 2

/* ---------------------------------------------------------------------
 * Opaque handles
 * ------------------------------------------------------------------ */

/** An elaborated design and the sources it came from. */
typedef struct reticle_design reticle_design;
/** A running simulation, with its own copy of the design. */
typedef struct reticle_sim reticle_sim;
/** An immutable snapshot of one stage's diagnostics. */
typedef struct reticle_diagnostics reticle_diagnostics;

/* ---------------------------------------------------------------------
 * Library
 * ------------------------------------------------------------------ */

/** The library version. Static storage: do not free. */
const char *reticle_version(void);

/**
 * A short description of a RETICLE_* code. Static storage: do not free.
 * An unrecognised code yields "unknown status" rather than NULL.
 */
const char *reticle_status_message(int status);

/**
 * Writes the stages this build has, comma separated, drawn from
 * "verilog", "vhdl", "sim" and "synth" in that order.
 */
int reticle_features(char **out);

/** Releases a string from any char ** out-parameter. NULL is ignored. */
void reticle_string_free(char *s);

/**
 * Panics inside the boundary on purpose; always RETICLE_ERR_PANIC.
 * It exists so you can prove, against your own build, that a panic
 * becomes a status rather than an unwind through your stack frames.
 */
int reticle_self_test_panic(void);

/* ---------------------------------------------------------------------
 * Diagnostics
 * ------------------------------------------------------------------ */

/** Releases a diagnostics handle. NULL is ignored. */
void reticle_diagnostics_free(reticle_diagnostics *diags);

/**
 * Writes the whole list as rustc-style text with source excerpts, ready
 * to print to stderr. Empty when there are no diagnostics.
 */
int reticle_diagnostics_render(const reticle_diagnostics *diags, char **out);

/** Writes how many diagnostics the handle holds. */
int reticle_diagnostics_count(const reticle_diagnostics *diags, size_t *out);
/** Writes how many of them are errors. */
int reticle_diagnostics_error_count(const reticle_diagnostics *diags, size_t *out);
/** Writes how many of them are warnings. */
int reticle_diagnostics_warning_count(const reticle_diagnostics *diags, size_t *out);

/** Writes one diagnostic's RETICLE_SEVERITY_* code. */
int reticle_diagnostic_severity(const reticle_diagnostics *diags, size_t index, int *out);
/** Writes one diagnostic's stable code, such as "V0007"; empty if none. */
int reticle_diagnostic_code(const reticle_diagnostics *diags, size_t index, char **out);
/** Writes one diagnostic's one-line headline. */
int reticle_diagnostic_message(const reticle_diagnostics *diags, size_t index, char **out);
/** Writes the file one diagnostic points into; empty if it has no span. */
int reticle_diagnostic_file(const reticle_diagnostics *diags, size_t index, char **out);
/** Writes the 1-based line, or 0 when the diagnostic has no span. */
int reticle_diagnostic_line(const reticle_diagnostics *diags, size_t index, uint32_t *out);
/** Writes the 1-based column in characters, or 0 when there is no span. */
int reticle_diagnostic_column(const reticle_diagnostics *diags, size_t index, uint32_t *out);
/** Writes how many trailing note lines one diagnostic carries. */
int reticle_diagnostic_note_count(const reticle_diagnostics *diags, size_t index, size_t *out);
/** Writes one note line of one diagnostic. */
int reticle_diagnostic_note(const reticle_diagnostics *diags, size_t index, size_t note,
                            char **out);

/* ---------------------------------------------------------------------
 * Designs
 * ------------------------------------------------------------------ */

/**
 * Parses and elaborates a list of Verilog / SystemVerilog sources.
 *
 * `names` and `sources` are parallel arrays of `count` strings forming one
 * compilation, so a module in one may instantiate a module in another. The
 * SystemVerilog rules are selected when the first name ends in .sv or
 * .svh. `top` names the root module, or is NULL to use the module nothing
 * instantiates.
 *
 * `out_diags` may be NULL; when it is not, it receives every diagnostic,
 * warnings included, even on success.
 */
int reticle_elaborate_verilog(const char *const *names, const char *const *sources, size_t count,
                              const char *top, reticle_design **out_design,
                              reticle_diagnostics **out_diags);

/** reticle_elaborate_verilog for a single source held in a string. */
int reticle_elaborate_verilog_source(const char *name, const char *source, const char *top,
                                     reticle_design **out_design,
                                     reticle_diagnostics **out_diags);

/**
 * Parses, analyses and elaborates a list of VHDL sources.
 *
 * Every source is compiled into the `work` library against the bundled std
 * and ieee libraries, under VHDL-2008. Arguments are as
 * reticle_elaborate_verilog.
 */
int reticle_elaborate_vhdl(const char *const *names, const char *const *sources, size_t count,
                           const char *top, reticle_design **out_design,
                           reticle_diagnostics **out_diags);

/** reticle_elaborate_vhdl for a single source held in a string. */
int reticle_elaborate_vhdl_source(const char *name, const char *source, const char *top,
                                  reticle_design **out_design, reticle_diagnostics **out_diags);

/**
 * Loads a design written in the .rtl IR text format. Always available,
 * whatever frontends the build has.
 */
int reticle_design_load_rtl(const char *name, const char *text, reticle_design **out_design,
                            reticle_diagnostics **out_diags);

/**
 * Writes the design back out in the .rtl IR text format. The round trip
 * is exact: loading the result yields the same design.
 */
int reticle_design_save_rtl(const reticle_design *design, char **out);

/**
 * Renders the design in a netlist format: "verilog", "vhdl", "json"
 * (Yosys), "blif" or "edif". An unrecognised name is RETICLE_ERR_NOT_FOUND.
 * A construct the target cannot express is reported through `out_diags`,
 * which may be NULL; the netlist formats generally want a synthesised
 * design.
 */
int reticle_design_emit(const reticle_design *design, const char *format, char **out,
                        reticle_diagnostics **out_diags);

/** Writes the top module's name, or the empty string when there is none. */
int reticle_design_top(const reticle_design *design, char **out);

/** Makes the named module the top; RETICLE_ERR_NOT_FOUND if there is no such module. */
int reticle_design_set_top(reticle_design *design, const char *name);

/** Writes how many modules the design holds. */
int reticle_design_module_count(const reticle_design *design, size_t *out);

/** Writes the name of the index'th module, in elaboration order. */
int reticle_design_module_name(const reticle_design *design, size_t index, char **out);

/**
 * Synthesises the design in place.
 *
 * Generic synthesis (process lowering, flip-flop / memory / FSM inference,
 * optimisation, cellification) always runs. `lut_inputs` then selects
 * technology mapping: 0 leaves the netlist generic, and 2 to 8 map what is
 * left onto k-input LUTs. Any other value is RETICLE_ERR_INVALID and the
 * design is untouched.
 *
 * `out_report` may be NULL; when it is not, it receives the pass log and
 * the cell report. `out_diags` may be NULL.
 */
int reticle_design_synth(reticle_design *design, uint32_t lut_inputs, char **out_report,
                         reticle_diagnostics **out_diags);

/** Releases a design handle. NULL is ignored. */
void reticle_design_free(reticle_design *design);

/* ---------------------------------------------------------------------
 * Simulation
 * ------------------------------------------------------------------ */

/**
 * Elaborates a design for simulation.
 *
 * `top` names the root module, or is NULL to use the design's own top. The
 * simulator takes its own copy, so `design` may be freed straight after.
 * Elaboration failures go to `out_diags`, which may be NULL; warnings
 * raised while running are collected by reticle_sim_take_messages.
 */
int reticle_sim_create(const reticle_design *design, const char *top, reticle_sim **out_sim,
                       reticle_diagnostics **out_diags);

/** Releases a simulation handle. NULL is ignored. */
void reticle_sim_free(reticle_sim *sim);

/**
 * Writes how many nets the simulation has. Net indices run from 0 to this
 * count and stay valid for the life of the handle.
 */
int reticle_sim_net_count(const reticle_sim *sim, size_t *out);

/** Writes the hierarchical path of one net, such as "counter.q". */
int reticle_sim_net_name(const reticle_sim *sim, size_t index, char **out);

/** Writes the index of the net at a path; RETICLE_ERR_NOT_FOUND if absent. */
int reticle_sim_find_net(const reticle_sim *sim, const char *path, size_t *out);

/** Writes one net's width in bits. */
int reticle_sim_net_width(const reticle_sim *sim, size_t index, uint32_t *out);

/**
 * Writes one net's value as a Verilog literal, such as "8'h2a" or
 * "4'bx0z1". Every value is representable this way, unknown bits included.
 */
int reticle_sim_get(const reticle_sim *sim, size_t index, char **out);

/**
 * Writes one net's value as an unsigned integer.
 * RETICLE_ERR_UNKNOWN_VALUE when it has x or z bits, RETICLE_ERR_INVALID
 * when it is wider than 64 bits; use reticle_sim_get for those.
 */
int reticle_sim_get_u64(const reticle_sim *sim, size_t index, uint64_t *out);

/**
 * Writes a net from a Verilog literal such as "1'b1", resized to the net's
 * width. The write takes effect immediately; its fanout is evaluated by
 * the next run call.
 */
int reticle_sim_set(reticle_sim *sim, size_t index, const char *literal);

/** Writes a net from an unsigned integer, truncated to the net's width. */
int reticle_sim_set_u64(reticle_sim *sim, size_t index, uint64_t value);

/** Advances the simulation by `ticks`; see reticle_sim_ticks_per_ns. */
int reticle_sim_run_for(reticle_sim *sim, uint64_t ticks);

/** Advances the simulation to absolute time `time`, in ticks. */
int reticle_sim_run_until(reticle_sim *sim, uint64_t time);

/**
 * Runs until the event queue drains, $finish runs or $stop pauses. A
 * design with a free-running clock and no $finish would never return;
 * drive those with reticle_sim_run_for.
 */
int reticle_sim_run(reticle_sim *sim);

/** Writes the current simulation time, in ticks. */
int reticle_sim_time(const reticle_sim *sim, uint64_t *out);

/** Writes the current RETICLE_SIM_* status. */
int reticle_sim_status(const reticle_sim *sim, int *out);

/** Writes how many ticks make one nanosecond in this design's timescale. */
int reticle_sim_ticks_per_ns(const reticle_sim *sim, uint64_t *out);

/**
 * Takes the $display and $write output produced since the last call; the
 * buffer is cleared, so a long run can be drained incrementally.
 */
int reticle_sim_take_output(reticle_sim *sim, char **out);

/**
 * Starts VCD capture: a header and a snapshot of every net now, then every
 * later change. Calling it again restarts the capture.
 */
int reticle_sim_enable_vcd(reticle_sim *sim);

/**
 * Writes the VCD text captured so far, or the empty string when capture
 * was never enabled. The capture keeps running.
 */
int reticle_sim_vcd(const reticle_sim *sim, char **out);

/**
 * Takes the simulator's own diagnostics: unresolved instances, port width
 * mismatches, failing assertions and $error calls. The simulator's list is
 * cleared, so each call reports only what is new.
 */
int reticle_sim_take_messages(reticle_sim *sim, reticle_diagnostics **out);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* RETICLE_H */
