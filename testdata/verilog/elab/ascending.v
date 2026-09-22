// Ascending ranges and non-zero based vectors exercise index mapping.
module ascending(
    input  wire [0:7]  up,
    input  wire [15:8] high,
    input  wire [2:0]  sel,
    output wire        up_bit,
    output wire        high_bit,
    output wire [0:3]  up_slice,
    output wire [11:8] high_slice
);
    assign up_bit     = up[sel];
    assign high_bit   = high[15];
    assign up_slice   = up[0:3];
    assign high_slice = high[11:8];
endmodule
