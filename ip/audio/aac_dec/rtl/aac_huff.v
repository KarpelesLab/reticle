// aac_huff -- decodes one codeword of one of the twelve AAC codebooks.
//
// A bit-serial walk down the binary tree `aac_huff_rom` holds: one bit and
// one block RAM read a cycle. `start` with `book` begins a codeword; `hit`
// is high for one cycle, combinationally, when the leaf is read, with the
// leaf's `payload` beside it (README, "The Huffman tables").
//
// The address of the next read is formed from the RAM's own output, so a
// codeword of n bits takes n + 1 cycles from `start`. Every codebook of
// the standard is a complete prefix code, so every path ends in a leaf and
// there is no invalid codeword to report.

module aac_huff (
    input  wire       clk,
    input  wire       rst_n,
    input  wire       start,
    input  wire       abort,
    input  wire [3:0] book,
    input  wire       bit_avail,
    input  wire       bit_val,
    output wire       bit_take,
    output wire       hit,
    output wire [9:0] payload
);
    wire [10:0] root;
    aac_huff_root u_root (.book(book), .root(root));

    reg         busy;
    reg         pend;    // a read was issued last cycle
    reg  [10:0] node;
    wire [11:0] q;

    wire        leaf     = pend && q[11];
    wire [10:0] cur_node = pend ? q[10:0] : node;
    wire        issue    = busy && !leaf && bit_avail;

    assign bit_take = issue;
    assign hit      = leaf;
    assign payload  = q[9:0];

    aac_huff_rom u_rom (.clk(clk), .addr({cur_node, bit_val}), .q(q));

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            busy <= 1'b0;
            pend <= 1'b0;
            node <= 11'd0;
        end else if (abort) begin
            busy <= 1'b0;
            pend <= 1'b0;
        end else if (start) begin
            busy <= 1'b1;
            pend <= 1'b0;
            node <= root;
        end else if (busy) begin
            if (leaf) begin
                busy <= 1'b0;
                pend <= 1'b0;
            end else begin
                if (pend)
                    node <= q[10:0];
                pend <= issue;
            end
        end
    end
endmodule
