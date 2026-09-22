// An undeclared name in a continuous assignment becomes an implicit
// one-bit net.
module implicit_net(
    input  wire a,
    input  wire b,
    output wire y
);
    assign t = a & b;
    assign y = t;
endmodule
