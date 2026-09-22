// spiflash_xip — execute in place from a serial NOR flash.
//
// What it does
//   Turns a word read on a simple memory port into an SPI read command on
//   the flash and hands back the four octets it answers with. That is
//   what "execute in place" means for a small system: the processor
//   fetches straight out of the flash, with no copy to RAM and no boot
//   loader, at the price of a flash access per fetch.
//
//   The memory port is the one `rv32i` puts on its instruction side, so
//   the two connect with no glue at all:
//
//     `mem_req` high with `mem_addr` stable starts a read. The block
//     drives `mem_ready` high for one cycle with the word on
//     `mem_rdata`; the requester takes it at that edge. Reads are always
//     a whole aligned word — the low two bits of `mem_addr` are ignored
//     — and only `mem_addr[23:0]` is used, which is the 16 MiB a
//     three-octet address reaches.
//
//   One read is one transaction: `cs_n` falls, the command octet and
//   three octets of address go out most significant bit first, any
//   configured dummy cycles pass, thirty-two bits come back, `cs_n`
//   rises. SPI mode 0 — `sclk` idles low, the flash samples `mosi` on the
//   rising edge and presents `miso` on the falling one — which is the
//   mode every serial NOR flash supports.
//
//   The four octets arrive in address order and are assembled
//   little-endian: the octet at the lowest address becomes
//   `mem_rdata[7:0]`. That is the order a RISC-V core wants and the order
//   a linker writes.
//
//   Configuration is three fields in two registers, written by holding
//   `cfg_we` with `cfg_addr` and `cfg_wdata`, and read back combinatorially
//   on `cfg_rdata`:
//
//     0   bits 7:0    one less than `sclk`'s half period in clock
//                     cycles, so 0 is the clock divided by two and 1 is
//                     the clock divided by four. Resets to CLK_DIV.
//     1   bits 7:0    the read command octet, resetting to READ_CMD
//         bits 15:8   dummy cycles between address and data, resetting
//                     to DUMMY_CYCLES
//
//   A write is only taken while `busy` is low, so the configuration
//   cannot change under a transaction that is already on the wire.
//
// What it does not do
//   Read only. No page program, no erase, no write enable, no status
//   polling, no protection registers: this block cannot change what is in
//   the flash, which is the useful half of that — an execute-in-place
//   path that can also write is a path that can brick itself.
//
//   No cache and no prefetch. Every word costs a whole transaction of
//   sixty-four `sclk` periods, so even at CLK_DIV = 0 — `sclk` at half
//   the clock — a fetch is about 130 clock cycles, and a processor
//   running from here runs at a small fraction of its clock. A cache in
//   front of this block is the obvious next thing and is deliberately not
//   inside it: a cache has its own correctness argument and belongs in
//   its own tested block.
//
//   Single lane only: no dual or quad IO, no continuous read mode, no
//   XIP mode word that lets the command octet be skipped, and no 32-bit
//   addressing for flashes above 16 MiB. No burst: a read is always
//   exactly four octets, so two consecutive words cost two transactions
//   even though the flash would happily have kept clocking out. No
//   `hold` or `wp` pin, and no octet-enable input, since there is nothing
//   to write.
//
//   `miso` is sampled on `sclk`'s rising edge with no re-timing, which
//   assumes the round trip through the pad and the flash fits in half an
//   `sclk` period. On a fast clock that is what the divisor is for.
module spiflash_xip #(
    // The flash read command. 0x03 is the slow read every device has.
    parameter [7:0] READ_CMD     = 8'h03,
    // Reset value of the divisor: one less than `sclk`'s half period,
    // in clock cycles.
    parameter       CLK_DIV      = 2,
    // Reset value of the dummy cycles between address and data.
    parameter       DUMMY_CYCLES = 0
) (
    input  wire        clk,
    input  wire        rst_n,

    // The read-only memory port.
    input  wire [31:0] mem_addr,
    input  wire        mem_req,
    output wire        mem_ready,
    output wire [31:0] mem_rdata,

    // Configuration.
    input  wire [1:0]  cfg_addr,
    input  wire        cfg_we,
    input  wire [31:0] cfg_wdata,
    output wire [31:0] cfg_rdata,

    output wire        busy,

    // The flash, in SPI mode 0.
    output wire        sclk,
    output wire        cs_n,
    output wire        mosi,
    input  wire        miso
);
    localparam [2:0] S_IDLE  = 3'd0;
    localparam [2:0] S_CMD   = 3'd1;
    localparam [2:0] S_DUMMY = 3'd2;
    localparam [2:0] S_DATA  = 3'd3;
    localparam [2:0] S_DONE  = 3'd4;

    reg [2:0]  state;
    reg [31:0] tx_shift;
    reg [31:0] rx_shift;
    reg [7:0]  bit_cnt;
    reg [7:0]  div_cnt;
    reg        sclk_q;

    reg [7:0]  cfg_div;
    reg [7:0]  cfg_cmd;
    reg [7:0]  cfg_dummy;

    wire clocking = (state == S_CMD) | (state == S_DUMMY) | (state == S_DATA);
    wire tick     = (div_cnt == cfg_div);
    wire rising   = tick & ~sclk_q;
    wire falling  = tick & sclk_q;

    assign sclk      = sclk_q;
    assign cs_n      = ~clocking;
    assign mosi      = tx_shift[31];
    assign mem_ready = (state == S_DONE);
    assign mem_rdata = {rx_shift[7:0], rx_shift[15:8], rx_shift[23:16], rx_shift[31:24]};
    assign busy      = (state != S_IDLE);
    assign cfg_rdata = (cfg_addr == 2'd0) ? {24'd0, cfg_div}
                     : (cfg_addr == 2'd1) ? {16'd0, cfg_dummy, cfg_cmd}
                     :                      32'd0;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state     <= S_IDLE;
            tx_shift  <= 32'd0;
            rx_shift  <= 32'd0;
            bit_cnt   <= 8'd0;
            div_cnt   <= 8'd0;
            sclk_q    <= 1'b0;
            cfg_div   <= CLK_DIV[7:0];
            cfg_cmd   <= READ_CMD;
            cfg_dummy <= DUMMY_CYCLES[7:0];
        end else begin
            if (clocking) begin
                if (tick) div_cnt <= 8'd0;
                else      div_cnt <= div_cnt + 8'd1;
                if (tick) sclk_q <= ~sclk_q;
            end else begin
                div_cnt <= 8'd0;
                sclk_q  <= 1'b0;
            end

            case (state)
                S_IDLE: begin
                    if (cfg_we) begin
                        case (cfg_addr)
                            2'd0: cfg_div <= cfg_wdata[7:0];
                            2'd1: begin
                                cfg_cmd   <= cfg_wdata[7:0];
                                cfg_dummy <= cfg_wdata[15:8];
                            end
                            default: begin end
                        endcase
                    end else if (mem_req) begin
                        // The command octet and a three-octet address,
                        // word aligned, most significant bit first.
                        tx_shift <= {cfg_cmd, mem_addr[23:2], 2'b00};
                        bit_cnt  <= 8'd32;
                        state    <= S_CMD;
                    end
                end
                S_CMD: begin
                    if (falling) begin
                        tx_shift <= {tx_shift[30:0], 1'b0};
                        bit_cnt  <= bit_cnt - 8'd1;
                        if (bit_cnt == 8'd1) begin
                            if (cfg_dummy == 8'd0) begin
                                bit_cnt <= 8'd32;
                                state   <= S_DATA;
                            end else begin
                                bit_cnt <= cfg_dummy;
                                state   <= S_DUMMY;
                            end
                        end
                    end
                end
                S_DUMMY: begin
                    if (falling) begin
                        bit_cnt <= bit_cnt - 8'd1;
                        if (bit_cnt == 8'd1) begin
                            bit_cnt <= 8'd32;
                            state   <= S_DATA;
                        end
                    end
                end
                S_DATA: begin
                    if (rising) rx_shift <= {rx_shift[30:0], miso};
                    if (falling) begin
                        bit_cnt <= bit_cnt - 8'd1;
                        if (bit_cnt == 8'd1) state <= S_DONE;
                    end
                end
                default: state <= S_IDLE;
            endcase
        end
    end
endmodule
