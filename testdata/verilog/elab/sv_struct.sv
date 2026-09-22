// A packed struct, accessed by field on both sides of an assignment.
module sv_struct(
    input  logic        clk,
    input  logic [7:0]  data_in,
    input  logic        valid_in,
    output logic [7:0]  data_out,
    output logic        valid_out
);
    typedef struct packed {
        logic       valid;
        logic [2:0] tag;
        logic [7:0] data;
    } packet_t;

    packet_t p;

    always_ff @(posedge clk) begin
        p.valid <= valid_in;
        p.tag   <= 3'd2;
        p.data  <= data_in;
    end

    assign data_out  = p.data;
    assign valid_out = p.valid;
endmodule
