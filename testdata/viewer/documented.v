// A small register file wrapper, written to exercise the documentation
// view: every kind of comment the frontends record ends up somewhere on
// the generated page.
//
// The leading block above a module becomes its description; the comment
// after a port becomes that port's note.
module documented #(
    parameter WIDTH = 8,   // width of the stored word
    parameter DEPTH = 4    // number of words
) (
    input  wire              clk,    // rising-edge clock for every port
    input  wire              rst_n,  // active-low asynchronous reset
    input  wire              we,     // write enable, one word per cycle
    input  wire [1:0]        addr,   // word selected for read and write
    input  wire [WIDTH-1:0]  din,    // word written while `we` is high
    output reg  [WIDTH-1:0]  dout,   // registered read port
    output wire              busy    // high on the cycle after a write
);

    // The storage itself; inference turns this into a memory with one
    // read port and one write port.
    reg [WIDTH-1:0] words [0:DEPTH-1];

    // One flag, set by a write and cleared the cycle after.
    reg written;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            dout    <= {WIDTH{1'b0}};
            written <= 1'b0;
        end else begin
            if (we) begin
                words[addr] <= din;
            end
            dout    <= words[addr];
            written <= we;
        end
    end

    assign busy = written;

endmodule

// A plain adder, instantiated by nothing here, so the index shows more
// than one module and the schematic of a leaf.
module documented_adder #(
    parameter W = 4        // operand width
) (
    input  wire [W-1:0] a,  // first operand
    input  wire [W-1:0] b,  // second operand
    output wire [W-1:0] y   // sum, truncated to W bits
);
    assign y = a + b;
endmodule
