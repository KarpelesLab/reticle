// Nets and variables nothing reads, and an input the module ignores.
// lint-config: off:all warn:unused-signal warn:unused-input
module unused (
    input  wire       clk,
    input  wire       enable,
    input  wire [7:0] data,
    output reg  [7:0] q
);
    wire [7:0] scratch;        // never read
    reg  [3:0] counter;        // written, never read
    wire       _spare;         // exempt: a leading underscore says so
    (* keep *) wire kept;      // exempt: the attribute keeps it

    always @(posedge clk) begin
        q       <= data;
        counter <= 4'd0;
    end
endmodule
