// Asynchronous resets that the body does not handle as declared.
// lint-config: off:all warn:reset-style
module resets (
    input  wire clk,
    input  wire rst_n,
    input  wire rst,
    input  wire arst_n,
    input  wire srst,
    input  wire en,
    input  wire d,
    output reg  q1,
    output reg  q2,
    output reg  q3,
    output reg  q4
);
    always @(posedge clk or negedge rst_n)
        if (rst_n) q1 <= 1'b0;           // active high, declared active low
        else       q1 <= d;

    always @(posedge clk or posedge rst)
        if (en)       q2 <= d;           // the reset comes second
        else if (rst) q2 <= 1'b0;

    always @(posedge clk or posedge rst)
        q3 <= d;                         // the reset is never tested

    always @(posedge clk or negedge arst_n)
        if (!arst_n)    q4 <= 1'b0;      // asynchronous and synchronous
        else if (srst)  q4 <= 1'b0;      // resets in one block
        else            q4 <= d;
endmodule
