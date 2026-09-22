// A package imported into a module, providing a parameter, a type and a
// function.
package cfg_pkg;
    localparam int WIDTH = 8;
    typedef logic [WIDTH-1:0] word_t;

    function automatic word_t double(input word_t v);
        return v << 1;
    endfunction
endpackage

module sv_package
    import cfg_pkg::*;
(
    input  logic        clk,
    input  word_t       a,
    output word_t       q,
    output logic [31:0] width_out
);
    always_ff @(posedge clk)
        q <= double(a);

    assign width_out = cfg_pkg::WIDTH;
endmodule
