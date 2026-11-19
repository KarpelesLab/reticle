// Every kind of iCE40 logic-cell register, for the parts no board here
// carries: rising and falling edges, an enable, synchronous and
// asynchronous resets and sets, a carry chain, and logic between them.
// `tests/fpga_icestorm.rs` builds it on every part, and
// docs/fpga-icestorm.md says how IceStorm's own decode of each build was
// simulated against this source.
//
// Every register starts at zero, as an iCE40 flip-flop does. Each output
// bit is one register, so a decode that differs anywhere shows on a pin.
module ff_zoo (
    input  wire       clk,
    input  wire       rst,
    input  wire       en,
    input  wire       d,
    output wire [9:0] q
);
    // A counter with an enable: a carry chain and a shared enable.
    reg [3:0] count = 4'd0;
    always @(posedge clk) if (en) count <= count + 4'd1;

    // An asynchronous reset.
    reg ar = 1'b0;
    always @(posedge clk or posedge rst)
        if (rst) ar <= 1'b0;
        else ar <= d ^ count[0];

    // An asynchronous set.
    reg as = 1'b0;
    always @(posedge clk or posedge rst)
        if (rst) as <= 1'b1;
        else as <= d & ~as;

    // A synchronous set gated by the enable, as the cell has it.
    reg ss = 1'b0;
    always @(posedge clk)
        if (en) begin
            if (rst) ss <= 1'b1;
            else ss <= d ^ ar;
        end

    // A synchronous reset with no enable.
    reg sr = 1'b0;
    always @(posedge clk)
        if (rst) sr <= 1'b0;
        else sr <= count[3] | d;

    // The falling edge, in a tile of its own.
    reg fall = 1'b0;
    always @(negedge clk) fall <= ar ^ as ^ ss;

    // A register on the falling edge's output, back on the rising one.
    reg back = 1'b0;
    always @(posedge clk) back <= fall & en;

    assign q = {back, fall, sr, ss, as, ar, count};
endmodule
