`timescale 1ns / 1ps
// Do the two counters in `carry_carry_probe.v` agree in simulation? If they do, a
// disagreement on the part is the part's answer and not the design's.
module probe_tb;
    reg clk = 1'b0;
    wire one, zero, plain, carry, differed, moved, alive;
    carry_probe dut (.clk(clk), .led_one(one), .led_zero(zero),
               .led_plain(plain), .led_carry(carry),
               .led_differed(differed), .led_moved(moved),
               .led_alive(alive));
    always #5 clk = ~clk;
    integer n;
    initial begin
        // Far more cycles than it takes a disagreement to appear: a
        // broken carry would differ within the first few.
        for (n = 0; n < 5000; n = n + 1) @(posedge clk);
        if (differed !== 1'b0) begin
            $display("FAIL: the two counters disagree in simulation, so the design is wrong");
            $finish;
        end
        if (moved !== 1'b1) begin
            $display("FAIL: the carry counter never left zero in simulation");
            $finish;
        end
        if (one !== 1'b1 || zero !== 1'b0) begin
            $display("FAIL: the constants are not 1 and 0 (%b, %b)", one, zero);
            $finish;
        end
        $display("PASS: after 5000 cycles the carry-chain counter and the carry-free counter agree exactly, and the constants are 1 and 0");
        $finish;
    end
endmodule
