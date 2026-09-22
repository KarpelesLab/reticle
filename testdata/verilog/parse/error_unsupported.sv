// Constructs outside the supported subset are reported once and skipped
// to their end keyword; everything around them parses.
module m;
    class packet;
        rand bit [7:0] data;
        function new();
        endfunction
    endclass : packet

    import "DPI-C" function int c_add(input int a, b);
    export "DPI-C" function sv_fn;

    covergroup cg @(posedge clk);
        coverpoint data;
    endgroup

    logic ok;
    assign ok = 1'b1;
endmodule
