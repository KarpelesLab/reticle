// Instance connection style.
// lint-config: off:all note:port-connection
module leaf (input wire a, input wire b, input wire c, input wire d, output wire y);
    assign y = a & b & c & d;
endmodule

module top (input wire w, output wire y);
    wire y0, y1;
    leaf u0 (w, w, w, w, y0);                            // five by position
    leaf u1 (.a(w), .b(w), .c(w), .d(w), .y());          // output left open
    leaf u2 (.*);                                        // connected by name
    leaf u3 (w, w, w, , y1);                             // an empty slot
    assign y = y0 & y1;
endmodule
