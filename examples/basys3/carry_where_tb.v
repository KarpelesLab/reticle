`timescale 1ns / 1ps
// `carry_where.v` proved right before it is believed, and in particular
// the arming: a design whose arm can disarm itself freezes both counters
// and lights its latches for no reason, which is how the previous
// attempt failed on the board.
module carry_where_tb;
    reg clk = 1'b0;
    wire free, chain, lo, mid, hi, top, zero;
    carry_where #(.WIDTH(12), .ARM(4)) dut (.clk(clk), .led_free(free), .led_chain(chain),
                     .led_diff_lo(lo), .led_diff_mid(mid),
                     .led_diff_hi(hi), .led_diff_top(top),
                     .led_zero(zero));
    always #5 clk = ~clk;

    integer n;
    integer edges;
    reg prev;

    initial begin
        // Twelve bits and a four-edge arm: the top bit turns over every
        // 2048 cycles, so a few thousand exercise every group.
        for (n = 0; n < 20000; n = n + 1) begin
            @(posedge clk);
            // After the arm, the two counters are equal on every cycle,
            // so their top bits must be equal on every cycle. Checked
            // continuously rather than once at the end, because a
            // disagreement that heals would otherwise pass.
            if (n > 32 && free !== chain) begin
                $display("FAIL: at cycle %0d the two top bits differ (%b vs %b)",
                         n, free, chain);
                $finish;
            end
        end

        if (lo !== 1'b0 || mid !== 1'b0 || hi !== 1'b0 || top !== 1'b0) begin
            $display("FAIL: a difference latched: lo=%b mid=%b hi=%b top=%b",
                     lo, mid, hi, top);
            $finish;
        end
        if (zero !== 1'b0) begin
            $display("FAIL: the control is not dark");
            $finish;
        end

        // And the arm must still be armed. If it could disarm itself —
        // which is exactly how the previous attempt failed on the board —
        // both counters would freeze and the top bit would stop moving.
        edges = 0;
        prev  = free;
        for (n = 0; n < 8000; n = n + 1) begin
            @(posedge clk);
            if (free !== prev) edges = edges + 1;
            prev = free;
        end
        if (edges == 0) begin
            $display("FAIL: the top bit is frozen, so the arm disarmed itself");
            $finish;
        end
        $display("PASS: 20000 cycles with the two counters' top bits equal on every one, no difference latched in any of the four groups, the control dark, and the arm still armed (%0d edges in 8000 cycles)", edges);
        $finish;
    end
endmodule
