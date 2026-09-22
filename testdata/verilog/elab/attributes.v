// Attributes are carried to the IR objects they annotate.
(* keep_hierarchy = "yes" *)
module attributed(
    input  wire clk,
    input  wire d,
    output reg  q
);
    (* ram_style = "block" *)
    reg [7:0] mem [0:3];
    (* keep *) wire internal = d;

    (* fsm_encoding = "one-hot" *)
    always @(posedge clk)
        q <= internal;
endmodule
