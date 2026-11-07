// A recursive constant function, which a constant expression may call
// (IEEE 1800-2017 §13.4.3); it was refused, as `error_recursion.v` is now
// the call at run time with an argument known only then.
module const_recursion(output wire [7:0] y);
    function integer fact;
        input integer n;
        begin
            if (n <= 1) fact = 1;
            else fact = n * fact(n - 1);
        end
    endfunction

    localparam VALUE = fact(4);
    assign y = VALUE[7:0];
endmodule
