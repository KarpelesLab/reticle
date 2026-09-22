// A constant function used to compute a localparam.
module constant_fn(
    input  wire [7:0] a,
    output wire [7:0] y
);
    function integer clog2_local;
        input integer value;
        integer i;
        begin
            clog2_local = 0;
            for (i = value - 1; i > 0; i = i >> 1)
                clog2_local = clog2_local + 1;
        end
    endfunction

    localparam DEPTH = 64;
    localparam AW    = clog2_local(DEPTH);

    wire [AW-1:0] addr;
    assign addr = a[AW-1:0];
    assign y    = {{(8 - AW){1'b0}}, addr};
endmodule
