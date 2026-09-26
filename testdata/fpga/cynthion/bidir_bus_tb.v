// What `bidir_bus.v`'s header says a person will see, checked before
// anybody is asked to look.
//
// The bus is left to the design, which is what a loopback through the pads
// is: with no other driver on the net, what the eight pads drive is what
// their input buffers read. So the interesting failures — a pad that never
// drives, one that never reads, an enable the wrong way round, a tristate
// that is never released, a latch that does not latch — all show up here.
// What a person on the board adds is the one thing this cannot have: real
// silicon, real pins, and a real transceiver at the far end deciding whether
// to let go of the bus. What neither can catch is two balls swapped in
// `bidir_bus.rcf`, because a pad reads its own pin whichever ball that is.
//
// Every failing check ends the run, so a passing run prints its verdict and
// nothing else and a failing one prints one line saying which claim broke.
// `tests/fpga_trellis.rs` compares the whole output with the verdict.
//
// `WALK` is 4 rather than the board's 22, so a full sweep of the walking
// pattern is 256 clocks instead of 1.12 s. That is the whole reason it is a
// parameter: the claim worth checking is that **all eight** pins drive a one
// and a zero and read both back, and at 22 that takes 67 million clocks.
//
// Run with: reticle sim --top bidir_bus_tb bidir_bus_tb.v bidir_bus.v
`timescale 1ps/1ps
module bidir_bus_tb;

    reg clk      = 1'b0;
    reg button_n = 1'b1;   // nobody touching the board
    reg dir      = 1'b0;   // the transceiver is not claiming the bus

    wire [7:0] data;
    wire ulpi_clk, ulpi_stp, ulpi_rst_n;
    wire led0_n, led1_n, led2_n, led3_n, led4_n, led5_n;

    bidir_bus #(.WALK(4)) dut (
        .clk        (clk),
        .button_n   (button_n),
        .ulpi_data  (data),
        .ulpi_dir   (dir),
        .ulpi_clk   (ulpi_clk),
        .ulpi_stp   (ulpi_stp),
        .ulpi_rst_n (ulpi_rst_n),
        .led0_n     (led0_n),
        .led1_n     (led1_n),
        .led2_n     (led2_n),
        .led3_n     (led3_n),
        .led4_n     (led4_n),
        .led5_n     (led5_n)
    );

    // 60 MHz, to the nearest picosecond: half a period is 8333 ps, so the
    // timescale is 1 ps and not 1 ns. Nothing here depends on the rate —
    // every delay below is counted in clock edges — but a testbench whose
    // clock is not the board's clock is a testbench that invites the wrong
    // arithmetic later.
    always #8333 clk = ~clk;

    integer i;
    integer bit_i;

    // Which of the eight pins have been seen driven high, and low. The
    // header's claim is that every one of the eight does both.
    reg [7:0] seen_high = 8'h00;
    reg [7:0] seen_low  = 8'h00;

    always @(posedge clk) begin
        if (led0_n === 1'b0) begin
            // `bit_i` and not `i`: the block below uses `i` for its own
            // loops, and one `integer` shared between two processes is one
            // process clobbering the other's counter.
            for (bit_i = 0; bit_i < 8; bit_i = bit_i + 1) begin
                if (data[bit_i] === 1'b1) seen_high[bit_i] <= 1'b1;
                if (data[bit_i] === 1'b0) seen_low[bit_i]  <= 1'b1;
            end
        end
    end

    // One check. `wrong` is the condition that must not hold, and the run
    // ends on the first one that does, so the last line of a failing run
    // names the claim that broke.
    task check;
        input wrong;
        input [64*8-1:0] what;
        begin
            if (wrong === 1'b1) begin
                $display("FAIL: %0s", what);
                $finish;
            end
        end
    endtask

    initial begin
        // ------------------------------------------------------------
        // NOBODY TOUCHING THE BOARD
        // ------------------------------------------------------------
        // All eight pins released, LEDs 0, 1 and 2 dark. The board's resting
        // state has to be this: these pins have another driver on them and
        // the FPGA must not be one of them unasked.
        repeat (40) @(posedge clk);
        check(data !== 8'bzzzz_zzzz, "the released bus is not high impedance");
        check(led0_n !== 1'b1, "LED 0 is lit with nothing pressed");
        check(led1_n !== 1'b1, "LED 1 is lit before anything read back");
        check(led2_n !== 1'b1, "LED 2 is lit before anything went wrong");
        check(led4_n !== 1'b0, "LED 4 is dark though dir has been low");
        check(led5_n !== 1'b1, "LED 5 is lit though dir is low");
        check(ulpi_stp !== 1'b1, "stp is not held high");
        check(ulpi_rst_n !== 1'b1, "the transceiver is held in reset");

        // ------------------------------------------------------------
        // `USER` HELD: THE TURNAROUND
        // ------------------------------------------------------------
        button_n = 1'b0;
        repeat (40) @(posedge clk);
        check(led0_n !== 1'b0, "LED 0 is dark while the bus is driven");
        check(data === 8'bzzzz_zzzz, "the bus is still released");
        check(led1_n !== 1'b0, "LED 1 is dark, so nothing read back");
        check(led2_n !== 1'b1, "LED 2 is lit, so a pin disagreed");

        // ------------------------------------------------------------
        // THE OTHER DRIVER TAKES THE BUS
        // ------------------------------------------------------------
        // `dir` rising must release all eight pins with no clock edge in
        // between — this is the check that says the two drivers cannot meet —
        // and the latches must keep what they had.
        dir = 1'b1;
        #1;
        check(data !== 8'bzzzz_zzzz, "dir rose and the bus is still driven");
        check(led5_n !== 1'b0, "LED 5 is dark while dir is high");
        repeat (20) @(posedge clk);
        check(led0_n !== 1'b1, "LED 0 is lit while the transceiver has the bus");
        check(led1_n !== 1'b0, "LED 1 forgot what it had latched");
        check(led2_n !== 1'b1, "LED 2 came on after the turnaround");

        // ------------------------------------------------------------
        // AND ALL EIGHT PINS, BOTH WAYS
        // ------------------------------------------------------------
        // Two full sweeps of the walking pattern with the bus the FPGA's,
        // watching for a disagreement the whole way.
        dir = 1'b0;
        for (i = 0; i < 600; i = i + 1) begin
            @(posedge clk);
            check(led2_n !== 1'b1, "LED 2 came on during the sweep");
        end
        check(seen_high !== 8'hff, "some pin was never driven high");
        check(seen_low !== 8'hff, "some pin was never driven low");

        // ------------------------------------------------------------
        // AND LET GO
        // ------------------------------------------------------------
        button_n = 1'b1;
        #1;
        check(data !== 8'bzzzz_zzzz, "the button was let go and the bus is still driven");
        check(led1_n !== 1'b0, "LED 1 went out");
        check(led2_n !== 1'b1, "LED 2 came on");

        $display("PASS: eight pads drove a one and a zero and read back all sixteen");
        $finish;
    end
endmodule
