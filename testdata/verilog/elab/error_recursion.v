// A recursive function called at run time with an argument known only
// then: inlining it would never end, and the IR has no calls. With a
// constant argument the call is folded instead (`const_recursion.v`).
module error_recursion(input wire [3:0] n, output wire [7:0] y);
    function automatic integer fact(input integer k);
        if (k <= 1) fact = 1;
        else fact = k * fact(k - 1);
    endfunction

    assign y = fact(n);
endmodule
