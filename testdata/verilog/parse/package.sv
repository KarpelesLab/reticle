// A package with parameters, types and functions, and modules importing
// it in the header, the body and by scoped reference.
package math_pkg;
    localparam int WIDTH = 16;
    parameter real PI = 3.14159;
    typedef logic [WIDTH-1:0] word_t;
    typedef enum logic { LOW, HIGH } level_t;

    function automatic word_t saturate(input int x);
        if (x > 2**WIDTH - 1) return '1;
        else if (x < 0)       return '0;
        else                  return word_t'(x);
    endfunction

    function int max2(int a, int b);
        return (a > b) ? a : b;
    endfunction

    task automatic wait_cycles(input int n);
        repeat (n) #1;
    endtask
endpackage : math_pkg

package util_pkg;
    import math_pkg::WIDTH;
    export math_pkg::WIDTH;
    localparam int DOUBLE = WIDTH * 2;
    export *::*;
endpackage

module user
    import math_pkg::*;
    import util_pkg::DOUBLE;
#(
    parameter int N = WIDTH
) (
    input  word_t a, b,
    output word_t y
);
    level_t lvl = HIGH;
    assign y = saturate(max2(a, b) + N);
endmodule

module scoped;
    import math_pkg::word_t;
    math_pkg::word_t w;
    logic [math_pkg::WIDTH-1:0] v;
    initial begin
        w = math_pkg::saturate(-5);
        v = w;
        $display("%0d", math_pkg::PI);
        $display("%0d", $unit::something);
    end
endmodule
