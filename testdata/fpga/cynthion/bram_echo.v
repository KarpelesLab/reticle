// Does a 32768 x 8 block RAM read back what was written to it, on a part?
//
// **Why this exists.** `testdata/fpga/cynthion/usb_inflate.v` decodes the
// first literal byte of a stream correctly and then returns `ff` for every
// byte that comes out of its sliding window -- and an unrouted input on this
// family reads as a one, so `ff` is what a read that never arrives looks
// like. This is that memory on its own, because the way to test a suspicion
// about a memory is to build the memory.
//
// **What it found, which was not what it was built to look for.** The
// suspicion written here first was that 9-bit mode was at fault, sixteen
// blocks of it. The memory was not in 9-bit mode. A 32768 x 8 memory ties
// at sixteen blocks over the 9-, 4-, 2- and 1-bit modes, and
// `Mapper::choose_bram` had just been changed to break such a tie by
// preferring the narrowest, so this was eight blocks **side by side** in
// **1-bit** mode -- one of the three the device file marks MEDIUM
// confidence. In that mode a block's data port is one bit wide, and a
// one-bit port's pin role had its bit index dropped: right for a
// flip-flop's `d`, wrong for `DOB`, whose pin is `p0_dout0`. So the role
// named no wire, the read data net was left with nothing driving it, and
// the router was never asked for it. `docs/fpga-trellis.md` has the
// account, including what each of this flow's checks said while it was
// wrong.
//
// Both are fixed, and this design is the regression test on a part: the
// same memory is sixteen `DP16KD` in 9-bit mode now, and a Cynthion reads
// back what was written to it at 8, 64, 1024, 4096 and 16384 bytes.
// `testdata/fpga/ecp5/block_ram_1bit.v` is the part of it that runs in the
// gate, on the shape that needs no tie-break to reach the mode.
//
// This is that memory and nothing else: the same depth, the same width, the
// same `rd_q <= mem[a]` idiom `inflate_window.v` uses, behind the CDC class
// that `usb_cdc_uart.v` has CHECKED on this board.
//
//     host -> device   len_hi len_lo, then that many bytes
//     device -> host   those same bytes, read back out of the memory
//
// What this would catch, and did: a read port that does not reach the
// fabric. It would equally catch a mode whose data pins are mapped wrongly
// or an address decoded wrongly. What it would not: anything about a read
// and a write in the same cycle at the same address, which is the window's
// other hard case and wants its own test -- and nothing at all about the
// 4-, 2- and 1-bit modes' *contents*, which no test and no part has
// measured.

module bram_echo #(
    parameter integer ADDR_BITS = 15
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

    // The bare idiom this flow needs to see to build a block RAM: no reset,
    // nothing combined into the right-hand side, one consumer.
    reg [7:0] mem [0:DEPTH-1];
    reg [7:0] rd_q;

    localparam [1:0] S_LEN_HI = 2'd0, S_LEN_LO = 2'd1,
                     S_WRITE  = 2'd2, S_READ   = 2'd3;

    reg [1:0]            state = S_LEN_HI;
    reg [15:0]           len   = 16'd0;
    reg [ADDR_BITS-1:0]  addr  = {ADDR_BITS{1'b0}};
    reg [15:0]           left  = 16'd0;
    // One cycle of read latency, so a byte is issued and taken the cycle
    // after: `armed` says the data in `rd_q` is the one to send.
    reg                  armed = 1'b0;

    wire want_len = (state == S_LEN_HI) || (state == S_LEN_LO);
    assign host_ready = want_len || (state == S_WRITE);
    wire took = host_valid & host_ready;

    assign dev_data   = rd_q;
    assign dev_valid  = (state == S_READ) && armed;
    assign dev_commit = dev_valid;
    wire gave = dev_valid & dev_ready;

    always @(posedge clk) begin
        if (state == S_WRITE && took) mem[addr] <= host_data;
        rd_q <= mem[addr];
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
                                ? S_LEN_HI : S_WRITE;
                end
            S_WRITE:
                if (took) begin
                    addr <= addr + 1'b1;
                    if (left == 16'd1) begin
                        addr  <= {ADDR_BITS{1'b0}};
                        left  <= len;
                        armed <= 1'b0;
                        state <= S_READ;
                    end else begin
                        left <= left - 16'd1;
                    end
                end
            S_READ: begin
                // `rd_q` has held `mem[addr]` since the cycle after `addr`
                // was presented, so the first byte needs one cycle of arming.
                if (!armed) begin
                    armed <= 1'b1;
                end else if (gave) begin
                    if (left == 16'd1) begin
                        armed <= 1'b0;
                        state <= S_LEN_HI;
                    end else begin
                        left <= left - 16'd1;
                        addr <= addr + 1'b1;
                        armed <= 1'b0;      // one cycle for the next read
                    end
                end
            end
            default: state <= S_LEN_HI;
        endcase
    end
endmodule
