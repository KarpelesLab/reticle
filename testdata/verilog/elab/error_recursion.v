// A recursive function in a constant expression that never reaches a base
// case: the evaluator stops 200 calls deep and says so. This file was a
// call at run time with an argument known only then, refused because
// inlining would not end; recursive calls are inlined to a fixed depth now
// (testdata/sim/recursion_tb.v), and with a constant argument the call is
// folded (`const_recursion.v`).
module error_recursion(output wire [7:0] y);
    function automatic integer down(input integer k);
        down = down(k - 1);
    endfunction

    localparam P = down(4);
    assign y = P[7:0];
endmodule
