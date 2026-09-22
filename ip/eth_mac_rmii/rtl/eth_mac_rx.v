// eth_mac_rx — the receive half of the Ethernet MAC, DW bits a cycle.
//
// What it does
//   The frame logic `eth_mac_rmii` and `eth_mac_rgmii` share on the
//   receive side, DW bits a cycle, least significant first: 2 for RMII,
//   8 for RGMII once its front end has put the two nibbles of an edge
//   pair together.
//
//   `crs_dv` high starts the search for the end of the preamble: the
//   last DW bits of the 0xD5 delimiter (2'b11 for RMII, the whole octet
//   for RGMII), and everything after them is the frame. Octets come out
//   on `rx_data` with `rx_valid`; the last carries `rx_last` and
//   `rx_crc_ok`. Five octets are held back, so the four of check
//   sequence are never presented as data and the last data octet can be
//   marked: the receiver cannot know which octet was last until `crs_dv`
//   falls.
//
//   `rx_crc_ok` is the residue test: the CRC run over the data and the
//   check sequence leaves 0xDEBB20E3 exactly when the frame arrived
//   intact. A corrupted frame is still delivered, with `rx_crc_ok` low.
//   `rx_error` pulses, and nothing is delivered, for a frame that ended
//   in the middle of an octet, one too short to hold a check sequence,
//   or one the PHY flagged on `rx_er`.
//
// What it does not do
//   Everything `eth_mac_rmii`'s header lists for the receive side: no
//   address filter, no length check, no back-pressure. The first cycle
//   of `crs_dv` is taken as preamble and not examined, so a frame whose
//   delimiter arrives in the very first cycle of carrier is missed; every
//   PHY sends preamble first.
module eth_mac_rx #(
    // Bits a cycle: 2 or 8.
    parameter DW = 2
) (
    input  wire          clk,
    input  wire          rst_n,

    input  wire          crs_dv,
    input  wire [DW-1:0] rxd,
    input  wire          rx_er,

    output wire [7:0]    rx_data,
    output wire          rx_valid,
    output wire          rx_last,
    output wire          rx_crc_ok,
    output wire          rx_error
);
    localparam [31:0]   CRC_RESIDUE = 32'hDEBB_20E3;
    localparam [7:0]    SFD         = 8'hD5;
    localparam [DW-1:0] SFD_TAIL    = SFD >> (8 - DW);
    localparam [1:0]    STEP_LAST   = 8 / DW - 1;

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

    localparam [1:0] R_IDLE = 2'd0;
    localparam [1:0] R_PRE  = 2'd1;
    localparam [1:0] R_DATA = 2'd2;

    reg [1:0]  rx_state;
    reg [7:0]  rx_sr;
    reg [1:0]  rx_step;
    reg [31:0] rx_crc;
    reg [2:0]  rx_fill;
    reg        rx_er_q;
    // The five octets held back, newest first.
    reg [7:0]  hold0;
    reg [7:0]  hold1;
    reg [7:0]  hold2;
    reg [7:0]  hold3;
    reg [7:0]  hold4;

    reg [7:0] rx_data_q;
    reg       rx_valid_q;
    reg       rx_last_q;
    reg       rx_ok_q;
    reg       rx_error_q;

    wire [7:0]    rx_bits     = {{(8 - DW){1'b0}}, rxd};
    wire [31:0]   rx_crc_next = crc_fold(rx_crc, rx_bits);
    wire [DW+7:0] rx_cat      = {rxd, rx_sr};
    wire [7:0]    rx_byte     = rx_cat[DW+7:DW];
    wire          rx_byte_end = (rx_step == STEP_LAST);

    assign rx_data   = rx_data_q;
    assign rx_valid  = rx_valid_q;
    assign rx_last   = rx_last_q;
    assign rx_crc_ok = rx_ok_q;
    assign rx_error  = rx_error_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            rx_state   <= R_IDLE;
            rx_sr      <= 8'd0;
            rx_step    <= 2'd0;
            rx_crc     <= 32'hFFFF_FFFF;
            rx_fill    <= 3'd0;
            rx_er_q    <= 1'b0;
            hold0      <= 8'd0;
            hold1      <= 8'd0;
            hold2      <= 8'd0;
            hold3      <= 8'd0;
            hold4      <= 8'd0;
            rx_data_q  <= 8'd0;
            rx_valid_q <= 1'b0;
            rx_last_q  <= 1'b0;
            rx_ok_q    <= 1'b0;
            rx_error_q <= 1'b0;
        end else begin
            rx_valid_q <= 1'b0;
            rx_last_q  <= 1'b0;
            rx_error_q <= 1'b0;
            case (rx_state)
                R_IDLE: begin
                    rx_er_q <= 1'b0;
                    if (crs_dv) rx_state <= R_PRE;
                end
                R_PRE: begin
                    rx_er_q <= rx_er_q | rx_er;
                    if (!crs_dv) begin
                        // A carrier that never became a frame. Not an
                        // error, just noise on the line.
                        rx_state <= R_IDLE;
                    end else if (rxd == SFD_TAIL) begin
                        rx_state <= R_DATA;
                        rx_crc   <= 32'hFFFF_FFFF;
                        rx_step  <= 2'd0;
                        rx_fill  <= 3'd0;
                    end
                end
                R_DATA: begin
                    if (!crs_dv) begin
                        rx_state <= R_IDLE;
                        if ((rx_step == 2'd0) && (rx_fill == 3'd5) && !rx_er_q) begin
                            // The pipeline holds the last octet of data
                            // and the four of check sequence behind it.
                            rx_data_q  <= hold4;
                            rx_valid_q <= 1'b1;
                            rx_last_q  <= 1'b1;
                            rx_ok_q    <= (rx_crc == CRC_RESIDUE);
                        end else begin
                            rx_error_q <= 1'b1;
                        end
                    end else begin
                        rx_er_q <= rx_er_q | rx_er;
                        rx_crc  <= rx_crc_next;
                        rx_step <= rx_byte_end ? 2'd0 : rx_step + 2'd1;
                        rx_sr   <= rx_byte;
                        if (rx_byte_end) begin
                            hold0 <= rx_byte;
                            hold1 <= hold0;
                            hold2 <= hold1;
                            hold3 <= hold2;
                            hold4 <= hold3;
                            if (rx_fill == 3'd5) begin
                                rx_data_q  <= hold4;
                                rx_valid_q <= 1'b1;
                            end else begin
                                rx_fill <= rx_fill + 3'd1;
                            end
                        end
                    end
                end
                default: rx_state <= R_IDLE;
            endcase
        end
    end
endmodule
