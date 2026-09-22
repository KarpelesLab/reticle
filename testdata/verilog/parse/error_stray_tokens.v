// Stray and unexpected tokens at item and statement level, a bad port
// list, and garbage between modules.
module m(input a, output b, 42);
    assign b = a;
    endcase
    wire w;
    initial begin
        end
        a = 1;
    end
    foo bar baz;
    ) ;
    assign b = w;
endmodule
garbage here
module ok(input x, output y);
    assign y = x;
endmodule
