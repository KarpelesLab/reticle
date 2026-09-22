// Shifts, part-selects and indexed part-selects.
module shifts(
    input  wire [15:0] d,
    input  wire [3:0]  amount,
    input  wire [3:0]  index,
    output wire [15:0] left,
    output wire [15:0] right,
    output wire [15:0] arith,
    output wire [3:0]  nibble,
    output wire [3:0]  dynamic_up,
    output wire        bit_sel
);
    assign left         = d << amount;
    assign right        = d >> amount;
    assign arith        = $signed(d) >>> amount;
    assign nibble       = d[7:4];
    assign dynamic_up   = d[index +: 4];
    assign bit_sel      = d[index];
endmodule
