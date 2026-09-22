`include "defs.vh"
`include "defs.vh"
`include <defs.vh>

module inc(
    `DECL_PORT(a),
    `DECL_PORT(b)
);
    wire [`DATA_W-1:0] sum = a + b;
    localparam HERE = `__LINE__;
    localparam FILE = `__FILE__;
    localparam INNER_OK = `FROM_INNER;
endmodule
