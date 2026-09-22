`define FEATURE_A
`define LEVEL 2

module cond;
`ifdef FEATURE_A
    wire a_on;
    `ifdef FEATURE_B
        wire b_on;
    `elsif FEATURE_A
        wire b_off_a_on;
        `ifndef FEATURE_C
            wire c_off;
        `else
            wire c_on;
        `endif
    `else
        wire neither;
    `endif
`else
    wire a_off;
    `ifdef FEATURE_A
        wire unreachable;
    `endif
`endif

`ifndef FEATURE_A
    wire skipped;
`elsif NOPE
    wire also_skipped;
`elsif LEVEL
    wire level;
`else
    wire not_level;
`endif

`undef FEATURE_A
`ifdef FEATURE_A
    wire gone; `undefined_inside_inactive_branch
`else
    wire after_undef;
`endif

`ifdef LEVEL `ifdef FEATURE_A wire x; `else wire y; `endif `endif
    // Inactive text may contain anything: "`endif" /* `endif */ `endif-ish
`ifdef NOPE
    "`endif" /* `endif */ garbage ` `` `"
`endif
endmodule
