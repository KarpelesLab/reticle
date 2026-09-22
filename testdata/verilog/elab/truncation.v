// An assignment that truncates its value warns with both widths.
module truncation(
    input  wire [15:0] wide,
    output wire [7:0]  narrow,
    output reg  [3:0]  tiny
);
    assign narrow = wide;

    always @* begin
        tiny = 8'hff;
    end
endmodule
