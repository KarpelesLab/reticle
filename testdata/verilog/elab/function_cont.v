// A function called from a continuous assignment gets a combinational
// helper process.
module function_cont(
    input  wire [3:0] a,
    output wire [3:0] y
);
    function [3:0] reverse;
        input [3:0] v;
        begin
            reverse = {v[0], v[1], v[2], v[3]};
        end
    endfunction

    assign y = reverse(a);
endmodule
