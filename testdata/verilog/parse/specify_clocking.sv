// A specify block (skipped), clocking blocks, timeunit / timeprecision
// and pass-through directives inside a module.
timeunit 1ns;
timeprecision 1ps;

module with_specify(input logic clk, a, output logic y);
    timeunit 1ns / 1ps;

    assign y = a;

    specify
        specparam tRise = 1, tFall = 2;
        (a => y) = (tRise, tFall);
        (posedge clk *> y) = (1, 2);
        $setup(a, posedge clk, 1);
        $hold(posedge clk, a, 1);
        if (a) (clk => y) = 3;
    endspecify

    clocking cb @(posedge clk);
        default input #1step output #0;
        input a;
        output y;
    endclocking : cb

    default clocking cb;

    global clocking gcb @(posedge clk); endclocking

    `default_nettype none
    wire logic w;
    `resetall
endmodule
