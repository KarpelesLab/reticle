// Malformed expressions: recovery resumes at the next `;` and the
// surrounding statements survive.
module m(input [3:0] a, b, output reg [3:0] y, z);
    always @(a or b) begin
        y = a + ;
        z = (a + b;
        y = a b;
        z = {a, };
        y = a[3:;
        z = b;
    end
    assign y = * a;
    assign z = a ? b;
    assign y = a[;
    assign z = a;
endmodule
