// aac_bits -- bytes in, one bit at a time out, most significant first.
//
// Everything in an AAC bitstream is read a bit at a time here: the parser
// takes at most one bit a cycle, and a frame is a few thousand bits, so a
// wider reader would buy nothing the budget needs (README, "Cycles").
//
// Two byte slots: `cur` is being read, `nxt` waits. `in_ready` is `nxt`
// being empty, a register, so it does not depend on `bit_take`.
//
// `bit_eob` says the bit on offer is the last of its byte, and `bit_last`
// that its byte came with `in_last`: the parser counts bytes and finds the
// end of a raw access unit from those two.

module aac_bits (
    input  wire       clk,
    input  wire       rst_n,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    input  wire       in_last,
    output wire       in_ready,
    output wire       bit_avail,
    output wire       bit_val,
    output wire       bit_eob,
    output wire       bit_last,
    input  wire       bit_take
);
    reg [7:0] cur;
    reg [3:0] cnt;       // bits of cur not yet taken, 0..8
    reg       cur_last;
    reg [7:0] nxt;
    reg       nxt_valid;
    reg       nxt_last;

    assign in_ready  = !nxt_valid;
    assign bit_avail = cnt != 4'd0;
    assign bit_val   = cur[7];
    assign bit_eob   = cnt == 4'd1;
    assign bit_last  = cur_last;

    wire take = bit_take && bit_avail;
    wire empty_next = (cnt == 4'd0) || (take && cnt == 4'd1);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            cur       <= 8'd0;
            cnt       <= 4'd0;
            cur_last  <= 1'b0;
            nxt       <= 8'd0;
            nxt_valid <= 1'b0;
            nxt_last  <= 1'b0;
        end else begin
            if (empty_next && nxt_valid) begin
                cur       <= nxt;
                cnt       <= 4'd8;
                cur_last  <= nxt_last;
                nxt_valid <= 1'b0;
            end else if (take) begin
                cur <= {cur[6:0], 1'b0};
                cnt <= cnt - 4'd1;
            end
            if (in_valid && !nxt_valid) begin
                nxt       <= in_data;
                nxt_last  <= in_last;
                nxt_valid <= 1'b1;
            end
        end
    end
endmodule
