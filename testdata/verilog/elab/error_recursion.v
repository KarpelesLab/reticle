// A recursive constant function.
module error_recursion(output wire [7:0] y);
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
