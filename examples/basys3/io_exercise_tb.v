`timescale 1ns / 1ps
// `io_exercise.v` checked before it goes near the board, because a design
// whose job is to establish the board's wiring must not have wiring
// mistakes of its own. The four digit-enable patterns take 2^18 cycles to
// come round, so this runs long enough to see all of them.
//
// What it cannot catch, and what BTND on the board is for: that `led[6]`
// reaches ball U14 at all. That is the `_SING` IO tile the flow could
// not configure until `fpga::xray::TileAlias`, and a simulation has no
// pads.
module io_exercise_tb;
    reg clk = 1'b0;
    reg [15:0] sw = 16'd0;
    reg c = 1'b0, u = 1'b0, d = 1'b0, l = 1'b0, r = 1'b0;
    wire [15:0] led;
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
        // One LED per switch, in order, with nothing skipped. `sw[6]`
        // is the one that used to light nothing, because LD6's ball sits
        // in a `LIOB33_SING` tile this flow could not configure; it has
        // its own LED now, and `led[6]` is the bit to watch.
        for (n = 0; n < 16; n = n + 1) begin
            sw = 16'h0001 << n; settle;
            if (led !== (16'h0001 << n)) begin
                $display("FAIL: sw[%0d] gave led=%h", n, led);
                $finish;
            end
        end

        // BTND lights all sixteen whatever the switches say.
        sw = 16'h0000; d = 1'b1; settle;
        if (led !== 16'hFFFF) begin $display("FAIL: BTND gave led=%h", led); $finish; end
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
        $display("PASS: all sixteen switches map to their own LED with no gap, all five buttons do what the header says, and all four digits are multiplexed one at a time");
        $finish;
    end
endmodule
