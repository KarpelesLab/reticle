// Compilation-unit items outside any module, macromodule, program,
// alias, nested modules, escaped identifiers and end labels.
typedef logic [7:0] u8;
parameter int G = 3;
localparam string V = "1.0";
int unit_var;
import pkg::*;

function automatic int twice(int x);
    return 2 * x;
endfunction

task unit_task;
    $display("unit");
endtask

macromodule mm(input u8 a, output u8 b);
    assign b = a;
endmodule : mm

program automatic test(input logic clk);
    initial begin
        @(posedge clk);
        $display("program");
    end
endprogram : test

module outer;
    wire [7:0] x, y;
    wire [3:0] lo, hi;
    alias x = y;
    alias lo = x[3:0];
    alias hi = x[7:4] = y[7:4];

    module nested(input a, output b);
        assign b = a;
    endmodule

    nested n0 (.a(x[0]), .b(y[0]));
    wire \bus/sel = x[1];
    wire \a+b = \bus/sel ;
    mm \mm.inst (.a(x), .b(y));
    ;
endmodule : outer

module static_mod;
endmodule

module automatic auto_mod;
endmodule

interface automatic ifc;
endinterface

package automatic pk;
endpackage : pk
