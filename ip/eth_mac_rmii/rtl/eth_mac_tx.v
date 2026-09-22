// eth_mac_tx — the transmit half of the Ethernet MAC, DW bits a cycle.
//
// What it does
//   The frame logic `eth_mac_rmii` and `eth_mac_rgmii` share: preamble,
//   start-of-frame delimiter, the user's octets, the frame check
//   sequence and the inter-frame gap, sent DW bits a cycle, least
//   significant bit first. DW is 2 for RMII and 8 for RGMII, whose
//   front end turns the octet into two nibbles on the two clock edges.
//
//   The user offers octets on `tx_data` with `tx_valid` and marks the
//   last with `tx_last`; the block takes one by pulsing `tx_ready`, every
//   8 / DW cycles. A frame starts when `tx_valid` first goes high: seven
//   octets of 0x55 and the 0xD5 delimiter, the first octet taken as the
//   delimiter ends, the payload, four octets of check sequence, then
//   `tx_en` low for IFG_CYCLES cycles.
//
//   The frame check sequence is CRC-32, reflected polynomial 0xEDB88320,
//   seeded with all ones and complemented at the end, folded DW bits a
//   cycle as they go out.
//
// What it does not do
//   Everything `eth_mac_rmii`'s header lists for the transmit side: no
//   padding, no pause frames, and on an underrun — `tx_valid` low when
//   an octet is due — it sends 0x00 and raises `tx_underrun` until the
//   next frame, because a wire cannot be stalled.
module eth_mac_tx #(
    // Bits a cycle: 2 or 8.
    parameter DW         = 2,
    // Cycles of inter-frame gap: 96 bit times is 48 at two bits, 12 at
    // eight.
    parameter IFG_CYCLES = 48
) (
    input  wire          clk,
    input  wire          rst_n,

    output wire          tx_en,
    output wire [DW-1:0] txd,

    input  wire [7:0]    tx_data,
    input  wire          tx_valid,
    output wire          tx_ready,
    input  wire          tx_last,
    output wire          tx_busy,
    output wire          tx_underrun
);
    localparam [7:0] IFG_LAST  = IFG_CYCLES - 1;
    localparam [1:0] STEP_LAST = 8 / DW - 1;
    localparam [3:0] FCS_LAST  = 32 / DW - 1;

    // DW bits of the reflected CRC-32, in transmission order.
    function [31:0] crc_fold;
        input [31:0] crc;
        input [7:0]  bits;
        integer      i;
        reg   [31:0] c;
        begin
            c = crc;
            for (i = 0; i < DW; i = i + 1)
                c = (c[0] ^ bits[i]) ? ((c >> 1) ^ 32'hEDB8_8320) : (c >> 1);
            crc_fold = c;
        end
    endfunction

    localparam [2:0] T_IDLE = 3'd0;
    localparam [2:0] T_PRE  = 3'd1;
    localparam [2:0] T_DATA = 3'd2;
    localparam [2:0] T_FCS  = 3'd3;
    localparam [2:0] T_IFG  = 3'd4;

    reg [2:0]  tx_state;
    reg [7:0]  tx_shift;
    reg [1:0]  tx_step;
    reg [2:0]  tx_pre;
    reg [31:0] tx_crc;
    reg [31:0] tx_fcs;
    reg [3:0]  tx_fcs_cnt;
    reg [7:0]  tx_ifg;
    reg        tx_last_q;
    reg        tx_under_q;

    wire tx_byte_end = (tx_step == STEP_LAST);
    wire tx_sending  = (tx_state == T_PRE) | (tx_state == T_DATA) | (tx_state == T_FCS);

    // The bits going out this cycle, folded into the check sequence.
    wire [31:0] tx_crc_next = crc_fold(tx_crc, tx_shift);

    assign tx_en = tx_sending;
    assign txd = ~tx_sending         ? {DW{1'b0}}
               : (tx_state == T_FCS) ? tx_fcs[DW-1:0]
               :                       tx_shift[DW-1:0];
    assign tx_ready = ((tx_state == T_PRE) & (tx_pre == 3'd7) & tx_byte_end)
                    | ((tx_state == T_DATA) & tx_byte_end & ~tx_last_q);
    assign tx_busy = (tx_state != T_IDLE);
    assign tx_underrun = tx_under_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            tx_state   <= T_IDLE;
            tx_shift   <= 8'd0;
            tx_step    <= 2'd0;
            tx_pre     <= 3'd0;
            tx_crc     <= 32'hFFFF_FFFF;
            tx_fcs     <= 32'd0;
            tx_fcs_cnt <= 4'd0;
            tx_ifg     <= 8'd0;
            tx_last_q  <= 1'b0;
            tx_under_q <= 1'b0;
        end else begin
            case (tx_state)
                T_IDLE: begin
                    if (tx_valid) begin
                        tx_state   <= T_PRE;
                        tx_pre     <= 3'd0;
                        tx_step    <= 2'd0;
                        tx_shift   <= 8'h55;
                        tx_crc     <= 32'hFFFF_FFFF;
                        tx_last_q  <= 1'b0;
                        tx_under_q <= 1'b0;
                    end
                end
                T_PRE: begin
                    tx_step  <= tx_byte_end ? 2'd0 : tx_step + 2'd1;
                    tx_shift <= tx_shift >> DW;
                    if (tx_byte_end) begin
                        if (tx_pre == 3'd7) begin
                            // The delimiter has gone out; the first
                            // octet of the frame follows it.
                            tx_state   <= T_DATA;
                            tx_shift   <= tx_valid ? tx_data : 8'h00;
                            tx_last_q  <= tx_valid & tx_last;
                            tx_under_q <= ~tx_valid;
                        end else begin
                            tx_pre   <= tx_pre + 3'd1;
                            tx_shift <= (tx_pre == 3'd6) ? 8'hD5 : 8'h55;
                        end
                    end
                end
                T_DATA: begin
                    tx_step  <= tx_byte_end ? 2'd0 : tx_step + 2'd1;
                    tx_shift <= tx_shift >> DW;
                    tx_crc   <= tx_crc_next;
                    if (tx_byte_end) begin
                        if (tx_last_q) begin
                            tx_state   <= T_FCS;
                            tx_fcs     <= ~tx_crc_next;
                            tx_fcs_cnt <= 4'd0;
                        end else begin
                            tx_shift  <= tx_valid ? tx_data : 8'h00;
                            tx_last_q <= tx_valid & tx_last;
                            if (!tx_valid) tx_under_q <= 1'b1;
                        end
                    end
                end
                T_FCS: begin
                    tx_fcs     <= tx_fcs >> DW;
                    tx_fcs_cnt <= tx_fcs_cnt + 4'd1;
                    if (tx_fcs_cnt == FCS_LAST) begin
                        tx_state <= T_IFG;
                        tx_ifg   <= 8'd0;
                    end
                end
                default: begin
                    tx_ifg <= tx_ifg + 8'd1;
                    if (tx_ifg == IFG_LAST) tx_state <= T_IDLE;
                end
            endcase
        end
    end
endmodule
