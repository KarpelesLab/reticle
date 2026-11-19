// Two block RAMs, 256 x 16 and 512 x 8, each with contents from an
// `initial` block and written at run time, for the parts no board here
// carries. One address counter drives both ports: each RAM is written at
// `a` and read at `a` with its top bit flipped, so a read never meets the
// write of the same cycle and what it returns does not depend on how the
// silicon orders the two. Every word read is on the pins.
//
// `tests/fpga_icestorm.rs` builds it on every part with block RAM, and
// docs/fpga-icestorm.md says how IceStorm's own decode of each build was
// simulated against this source.
module bram_check (
    input  wire        clk,
    input  wire        we,
    input  wire [3:0]  din,
    output wire [15:0] wide,
    output wire [7:0]  narrow
);
    reg [15:0] w16 [0:255];
    reg [7:0]  w8  [0:511];
    integer i;
    initial begin
        for (i = 0; i < 256; i = i + 1) w16[i] = {i[7:0], i[7:0]} ^ 16'h5a3c;
        for (i = 0; i < 512; i = i + 1) w8[i] = i[7:0] ^ {i[8], i[8:2]} ^ 8'hc3;
    end

    reg [8:0]  a = 9'd0;
    reg [15:0] r16 = 16'd0;
    reg [7:0]  r8 = 8'd0;
    always @(posedge clk) begin
        a <= a + 9'd1;
        if (we) begin
            w16[a[7:0]] <= {din, ~din, din ^ a[3:0], a[7:4]};
            w8[a] <= {din ^ a[7:4], a[3:0]};
        end
        r16 <= w16[a[7:0] ^ 8'h80];
        r8 <= w8[a ^ 9'h100];
    end

    assign wide = r16;
    assign narrow = r8;
endmodule
