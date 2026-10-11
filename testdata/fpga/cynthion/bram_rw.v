// What does a block RAM read, on a part, when the same address is written
// in the same cycle?
//
// **Why this exists.** `bram_echo.v`'s header names this as the case it
// does not test, and `usb_inflate.v` needs the answer. On a Cynthion a
// dynamic-Huffman stream decodes correctly only if the decoder has not
// already decoded a *different* stream since configuration: the same
// stream twice is fine, a refused stream before it is fine, and any other
// successful decode before it makes it stall. That is the signature of
// state that should have been re-initialised and was not -- and the only
// thing between a correct simulation and a wrong part is what the fabric
// does that the simulator does not.
//
// Building a Huffman table is counting sorts over small memories, which
// read and write the same array, and a block RAM read in the cycle its
// address is written is the one arrangement whose answer this flow has
// never measured. In Verilog the two statements below are in one `always`
// block with non-blocking assignment, so the read takes the **old**
// contents; `ecppack` configures a `DP16KD` with `WRITEMODE_A/B =
// READBEFOREWRITE`, which says the same. Neither is a measurement.
//
//     host -> device   len_hi len_lo, then that many bytes
//     device -> host   for each address in turn, what a read of it
//                      returned in the cycle it was being overwritten
//
// The expected answer is the bytes the host sent, because the fill pass
// wrote them and the read happens before the write. Getting `addr ^ 8'ha5`
// back instead would mean the block writes first and reads after, which
// no simulation of this flow would show and which would break every
// counting sort built on it.
//
// What this would catch: a read-during-write that returns the new data,
// or nothing, or the wrong address. What it would not: anything about two
// *ports* used at once on different addresses, which is what the window
// does and what `bram_echo.v` already covers.

module bram_rw #(
    parameter integer ADDR_BITS = 11
) (
    input  wire       clk,
    input  wire       rst_n,
    input  wire [7:0] host_data,
    input  wire       host_valid,
    output wire       host_ready,
    output wire [7:0] dev_data,
    output wire       dev_valid,
    input  wire       dev_ready,
    output wire       dev_commit
);
    localparam integer DEPTH = 1 << ADDR_BITS;

    reg [7:0] mem [0:DEPTH-1];
    reg [7:0] rd_q;

    localparam [1:0] S_LEN_HI = 2'd0, S_LEN_LO = 2'd1,
                     S_FILL   = 2'd2, S_RW     = 2'd3;

    reg [1:0]           state = S_LEN_HI;
    reg [15:0]          len   = 16'd0;
    reg [ADDR_BITS-1:0] addr  = {ADDR_BITS{1'b0}};
    reg [15:0]          left  = 16'd0;
    reg                 armed = 1'b0;

    wire want_len = (state == S_LEN_HI) || (state == S_LEN_LO);
    assign host_ready = want_len || (state == S_FILL);
    wire took = host_valid & host_ready;

    // The cycle in which the address is presented to be read and written at
    // once. `armed` is low for exactly that cycle, so the pair below
    // happens together and the byte is sent the cycle after.
    wire issuing = (state == S_RW) && !armed;
    // The read happens in the issuing cycle and in the fill pass, and
    // nowhere else; see the capture note below.
    wire rd_en   = issuing || (state == S_FILL);

    assign dev_data   = rd_q;
    assign dev_valid  = (state == S_RW) && armed;
    assign dev_commit = dev_valid;
    wire gave = dev_valid & dev_ready;

    // **The whole experiment is these three lines.** One `always` block, one
    // address, a write and a read of it in the same cycle.
    //
    // The write is *one* port with its data muxed outside the block, not an
    // `if`/`else if` over two assignments: those become two `$memwr` cells,
    // and with the read that is three ports on a block that has two, which
    // the flow refuses by name (`a memory is left: it did not become block
    // RAM or logic`).
    wire       wr_en = (state == S_FILL && took) || issuing;
    wire [7:0] wr_d  = (state == S_FILL) ? host_data : (addr[7:0] ^ 8'ha5);
    always @(posedge clk) begin
        if (wr_en) mem[addr] <= wr_d;
        // **Captured once, and this is not a detail.** An unconditional
        // `rd_q <= mem[addr]` keeps re-reading while the design waits for
        // the host to take the byte, and by then the write has landed --
        // so the first measurement this design made read the old contents
        // for byte 0, which the host took at once, and the new data for
        // the other fifteen, which waited on USB. That looks exactly like
        // a block that writes before it reads and was entirely this
        // register. Gating the read holds the value the issuing cycle
        // captured, which on a `DP16KD` is its clock enable holding the
        // output.
        if (rd_en) rd_q <= mem[addr];
    end

    always @(posedge clk) begin
        if (!rst_n) begin
            state <= S_LEN_HI;
            len   <= 16'd0;
            addr  <= {ADDR_BITS{1'b0}};
            left  <= 16'd0;
            armed <= 1'b0;
        end else case (state)
            S_LEN_HI:
                if (took) begin
                    len[15:8] <= host_data;
                    state     <= S_LEN_LO;
                end
            S_LEN_LO:
                if (took) begin
                    len[7:0] <= host_data;
                    left     <= {len[15:8], host_data};
                    addr     <= {ADDR_BITS{1'b0}};
                    state    <= ({len[15:8], host_data} == 16'd0)
                                ? S_LEN_HI : S_FILL;
                end
            S_FILL:
                if (took) begin
                    addr <= addr + 1'b1;
                    if (left == 16'd1) begin
                        addr  <= {ADDR_BITS{1'b0}};
                        left  <= len;
                        armed <= 1'b0;
                        state <= S_RW;
                    end else begin
                        left <= left - 16'd1;
                    end
                end
            S_RW: begin
                if (!armed) begin
                    armed <= 1'b1;
                end else if (gave) begin
                    if (left == 16'd1) begin
                        armed <= 1'b0;
                        state <= S_LEN_HI;
                    end else begin
                        left  <= left - 16'd1;
                        addr  <= addr + 1'b1;
                        armed <= 1'b0;
                    end
                end
            end
            default: state <= S_LEN_HI;
        endcase
    end
endmodule
