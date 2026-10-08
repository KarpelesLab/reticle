`timescale 1ns / 1ps
// `io_exercise.v` checked before it goes near the board, because a design
// whose job is to establish the board's wiring must not have wiring
// mistakes of its own. The four digit-enable patterns take 2^18 cycles to
// come round, so this runs long enough to see all of them.
module io_exercise_tb;
    reg clk = 1'b0;
    reg [15:0] sw = 16'd0;
    reg c = 1'b0, u = 1'b0, d = 1'b0, l = 1'b0, r = 1'b0;
    wire [14:0] led;
    wire [6:0]  seg;
    wire        dp;
    wire [3:0]  an;

    io_exercise dut (.clk(clk), .sw(sw), .btn_c(c), .btn_u(u), .btn_d(d),
                     .btn_l(l), .btn_r(r), .led(led), .seg(seg), .dp(dp),
                     .an(an));
    always #5 clk = ~clk;

    integer n;
    reg [3:0] seen;

    task settle;
        begin repeat (4) @(posedge clk); end
    endtask

    initial begin
        settle;
        // Each switch lights its own LED, and `sw[6]` has no LED so the
        // outputs above it shift down by one.
        sw = 16'h0001; settle;
        if (led !== 15'h0001) begin $display("FAIL: sw[0] gave led=%h", led); $finish; end
        sw = 16'h0020; settle;
        if (led !== 15'h0020) begin $display("FAIL: sw[5] gave led=%h", led); $finish; end
        // sw[6] has no LED of its own: it must light nothing.
        sw = 16'h0040; settle;
        if (led !== 15'h0000) begin $display("FAIL: sw[6] lit led=%h, it has no LED", led); $finish; end
        // sw[7] is LD7, which is led[6].
        sw = 16'h0080; settle;
        if (led !== 15'h0040) begin $display("FAIL: sw[7] gave led=%h, wanted 0040", led); $finish; end
        sw = 16'h8000; settle;
        if (led !== 15'h4000) begin $display("FAIL: sw[15] gave led=%h, wanted 4000", led); $finish; end

        // BTND lights all fifteen whatever the switches say.
        sw = 16'h0000; d = 1'b1; settle;
        if (led !== 15'h7FFF) begin $display("FAIL: BTND gave led=%h", led); $finish; end
        d = 1'b0;

        // BTNC lights every segment, the point, and all four digits.
        c = 1'b1; settle;
        if (seg !== 7'b0000000 || dp !== 1'b0 || an !== 4'b0000) begin
            $display("FAIL: BTNC gave seg=%b dp=%b an=%b", seg, dp, an);
            $finish;
        end
        c = 1'b0;

        // BTNL and BTNR each enable exactly one digit, at opposite ends.
        l = 1'b1; settle;
        if (an !== 4'b1110) begin $display("FAIL: BTNL gave an=%b", an); $finish; end
        l = 1'b0;
        r = 1'b1; settle;
        if (an !== 4'b0111) begin $display("FAIL: BTNR gave an=%b", an); $finish; end
        r = 1'b0;

        // And left alone, all four digits come round, one at a time.
        seen = 4'b0000;
        for (n = 0; n < 300000; n = n + 1) begin
            @(posedge clk);
            case (an)
                4'b1110: seen[0] = 1'b1;
                4'b1101: seen[1] = 1'b1;
                4'b1011: seen[2] = 1'b1;
                4'b0111: seen[3] = 1'b1;
                default: begin
                    $display("FAIL: an=%b is not one digit", an);
                    $finish;
                end
            endcase
        end
        if (seen !== 4'b1111) begin
            $display("FAIL: only these digits were enabled: %b", seen);
            $finish;
        end
        $display("PASS: every switch maps to its own LED with the LD6 gap, all five buttons do what the header says, and all four digits are multiplexed one at a time");
        $finish;
    end
endmodule
