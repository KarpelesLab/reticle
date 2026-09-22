/*
 * demo.c - the whole Reticle C API in one program.
 *
 * It elaborates a Verilog counter, prints whatever the frontend had to
 * say, synthesises the design, writes it out as a Verilog netlist and as
 * the `.rtl` IR text format, then simulates ten clock edges and reads the
 * counter back.
 *
 * Build it against a static library (see docs/ffi.md for the shared one):
 *
 *   cargo rustc --release --features ffi --crate-type staticlib
 *   cc -I src/ffi -o demo examples/ffi/demo.c \
 *      target/release/libreticle.a -lpthread -ldl -lm
 *   ./demo
 *
 * Everything the library hands back through a `char **` is owned by this
 * program and released with reticle_string_free; every handle is released
 * by its _free. Run it under valgrind and it reports no leaks, which is
 * the point of the exercise.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "reticle.h"

static const char *COUNTER =
    "module counter(input clk, input rst, output reg [7:0] q);\n"
    "  always @(posedge clk)\n"
    "    if (rst) q <= 8'd0;\n"
    "    else     q <= q + 8'd1;\n"
    "endmodule\n";

/* Prints a failed call and returns non-zero, so `main` stays readable. */
static int failed(const char *what, int status) {
    fprintf(stderr, "%s: %s (%d)\n", what, reticle_status_message(status), status);
    return 1;
}

/* Prints every diagnostic field by field, then the rendered form. */
static void show(const reticle_diagnostics *diags) {
    size_t count = 0, i, notes, n;
    if (diags == NULL || reticle_diagnostics_count(diags, &count) != RETICLE_OK) {
        return;
    }
    for (i = 0; i < count; i++) {
        char *file = NULL, *message = NULL, *code = NULL, *note = NULL;
        uint32_t line = 0, column = 0;
        int severity = 0;

        reticle_diagnostic_severity(diags, i, &severity);
        reticle_diagnostic_code(diags, i, &code);
        reticle_diagnostic_message(diags, i, &message);
        reticle_diagnostic_file(diags, i, &file);
        reticle_diagnostic_line(diags, i, &line);
        reticle_diagnostic_column(diags, i, &column);

        printf("  [%s] %s:%u:%u: %s%s%s\n",
               severity == RETICLE_SEVERITY_ERROR     ? "error"
               : severity == RETICLE_SEVERITY_WARNING ? "warning"
                                                      : "note",
               file, line, column, message, code[0] ? " " : "", code);

        notes = 0;
        reticle_diagnostic_note_count(diags, i, &notes);
        for (n = 0; n < notes; n++) {
            if (reticle_diagnostic_note(diags, i, n, &note) == RETICLE_OK) {
                printf("    note: %s\n", note);
                reticle_string_free(note);
            }
        }
        reticle_string_free(file);
        reticle_string_free(message);
        reticle_string_free(code);
    }
}

int main(void) {
    reticle_design *design = NULL;
    reticle_diagnostics *diags = NULL;
    reticle_sim *sim = NULL;
    char *text = NULL;
    size_t clk = 0, rst = 0, q = 0, modules = 0;
    uint64_t per_ns = 0, value = 0, i;
    int status;

    printf("reticle %s (stages: ", reticle_version());
    if (reticle_features(&text) == RETICLE_OK) {
        printf("%s", text);
        reticle_string_free(text);
        text = NULL;
    }
    printf(")\n");

    /*
     * A panic inside the library is a status code, never an unwind
     * through these frames. Rust's default hook still prints the panic
     * message to stderr, so the line about a deliberate panic below is
     * this check working, not a failure.
     */
    if (reticle_self_test_panic() != RETICLE_ERR_PANIC) {
        fprintf(stderr, "the panic guard is not working\n");
        return 1;
    }

    /* 1. Parse and elaborate. */
    status = reticle_elaborate_verilog_source("counter.v", COUNTER, "counter", &design, &diags);
    printf("elaborate: %s\n", reticle_status_message(status));
    show(diags);
    reticle_diagnostics_free(diags);
    diags = NULL;
    if (status != RETICLE_OK) {
        return failed("elaborate", status);
    }

    if (reticle_design_module_count(design, &modules) == RETICLE_OK) {
        printf("  %zu module(s)\n", modules);
    }
    if (reticle_design_top(design, &text) == RETICLE_OK) {
        printf("  top: %s\n", text);
        reticle_string_free(text);
        text = NULL;
    }

    /* 2. Save the elaborated design as `.rtl`, which round-trips exactly. */
    if ((status = reticle_design_save_rtl(design, &text)) != RETICLE_OK) {
        reticle_design_free(design);
        return failed("save_rtl", status);
    }
    printf("  %zu bytes of .rtl\n", strlen(text));
    reticle_string_free(text);
    text = NULL;

    /* 3. Simulate before synthesis: ten rising edges after one reset. */
    status = reticle_sim_create(design, NULL, &sim, &diags);
    show(diags);
    reticle_diagnostics_free(diags);
    diags = NULL;
    if (status != RETICLE_OK) {
        reticle_design_free(design);
        return failed("sim_create", status);
    }

    if (reticle_sim_find_net(sim, "counter.clk", &clk) != RETICLE_OK ||
        reticle_sim_find_net(sim, "counter.rst", &rst) != RETICLE_OK ||
        reticle_sim_find_net(sim, "counter.q", &q) != RETICLE_OK) {
        reticle_sim_free(sim);
        reticle_design_free(design);
        return failed("find_net", RETICLE_ERR_NOT_FOUND);
    }
    reticle_sim_ticks_per_ns(sim, &per_ns);

    reticle_sim_set(sim, rst, "1'b1");
    reticle_sim_set(sim, clk, "1'b0");
    reticle_sim_run_for(sim, per_ns);
    reticle_sim_set(sim, clk, "1'b1");
    reticle_sim_run_for(sim, per_ns);
    reticle_sim_set(sim, rst, "1'b0");

    for (i = 0; i < 10; i++) {
        reticle_sim_set(sim, clk, "1'b0");
        reticle_sim_run_for(sim, per_ns);
        reticle_sim_set(sim, clk, "1'b1");
        reticle_sim_run_for(sim, per_ns);
    }

    if ((status = reticle_sim_get_u64(sim, q, &value)) != RETICLE_OK) {
        reticle_sim_free(sim);
        reticle_design_free(design);
        return failed("sim_get_u64", status);
    }
    if (reticle_sim_get(sim, q, &text) == RETICLE_OK) {
        printf("simulate: q = %llu (%s)\n", (unsigned long long)value, text);
        reticle_string_free(text);
        text = NULL;
    }
    if (value != 10) {
        fprintf(stderr, "expected q to be 10 after ten edges, got %llu\n",
                (unsigned long long)value);
        reticle_sim_free(sim);
        reticle_design_free(design);
        return 1;
    }

    /* Whatever $display wrote, plus the simulator's own messages. */
    if (reticle_sim_take_output(sim, &text) == RETICLE_OK) {
        if (text[0] != '\0') {
            printf("  output: %s", text);
        }
        reticle_string_free(text);
        text = NULL;
    }
    if (reticle_sim_take_messages(sim, &diags) == RETICLE_OK) {
        show(diags);
        reticle_diagnostics_free(diags);
        diags = NULL;
    }
    reticle_sim_free(sim);
    sim = NULL;

    /* 4. Synthesise, mapping the leftover logic to 4-input LUTs. */
    status = reticle_design_synth(design, 4, &text, &diags);
    printf("synth: %s\n", reticle_status_message(status));
    show(diags);
    reticle_diagnostics_free(diags);
    diags = NULL;
    if (status != RETICLE_OK) {
        reticle_string_free(text);
        reticle_design_free(design);
        return failed("synth", status);
    }
    reticle_string_free(text);
    text = NULL;

    /* 5. Emit the netlist in every format the build supports. */
    {
        static const char *const formats[] = {"verilog", "vhdl", "json", "blif", "edif"};
        size_t f;
        for (f = 0; f < sizeof formats / sizeof *formats; f++) {
            status = reticle_design_emit(design, formats[f], &text, &diags);
            if (status == RETICLE_OK) {
                printf("emit %-8s %zu bytes\n", formats[f], strlen(text));
                reticle_string_free(text);
                text = NULL;
            } else {
                printf("emit %-8s %s\n", formats[f], reticle_status_message(status));
                show(diags);
            }
            reticle_diagnostics_free(diags);
            diags = NULL;
        }
    }

    reticle_design_free(design);
    printf("done\n");
    return 0;
}
