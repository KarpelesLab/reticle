// The iCE40UP5K's PLL, checked by what only a locked 48 MHz clock can do,
// on an iCE40UP5K-SG48 board with an FT2232H (115200 baud, 8N1, pin 10
// out, pin 9 in, the 12 MHz oscillator on pin 20).
//
// SB_PLL40_CORE multiplies the 12 MHz reference by four: DIVR = 0, DIVF =
// 63, DIVQ = 4, so the VCO runs at 12 * 64 = 768 MHz and the output at
// 768 / 16 = 48 MHz, with FILTER_RANGE 1 for a 12 MHz phase detector --
// what Project IceStorm's icepll prints for `-i 12 -o 48`. Its global
// output clocks everything below but the window counter.
//
// The serial transmitter runs on the PLL's clock with 417 clocks a bit,
// which is 115200 baud (0.08 % slow) at 48 MHz and nothing a UART reads
// at any other frequency: at 12 MHz it would be 28 777 baud. So a line
// arriving at all says the PLL runs at 48 MHz, give or take a few
// percent.
//
// And the line says how fast, to the cycle: a counter on the 12 MHz
// reference toggles `win` every 2^20 cycles (87.4 ms); the PLL's domain
// resynchronises it and counts its own cycles between toggles. At exactly
// four times the reference that is 4 * 2^20 = 0x400000, within a cycle or
// two of resynchronisation. Each window it prints
//
//   L<lock> N<count, 6 hex digits>
//
// with <lock> the PLL's LOCK output, resynchronised. A host wants L1 and
// N within 0x400000 +- 2.
//
// Every register starts at zero and is as wide as the values it holds; the
// transmit line idles high, so it is kept inverted.
module pll_uart (
    input  wire clk,
    output wire tx
);
    wire sys;
    wire lock;
    SB_PLL40_CORE #(
        .FEEDBACK_PATH("SIMPLE"),
        .PLLOUT_SELECT("GENCLK"),
        .DIVR(4'd0),
        .DIVF(7'd63),
        .DIVQ(3'd4),
        .FILTER_RANGE(3'd1)
    ) pll (
        .REFERENCECLK(clk),
        .PLLOUTGLOBAL(sys),
        .LOCK(lock),
        .RESETB(1'b1),
        .BYPASS(1'b0)
    );

    // ---- the window, on the reference --------------------------------------
    reg [19:0] ref_n = 20'd0;
    reg        win = 1'b0;
    always @(posedge clk) begin
        ref_n <= ref_n + 20'd1;
        if (ref_n == 20'hfffff) win <= ~win;
    end

    // ---- the PLL's domain ---------------------------------------------------
    reg [2:0]  win_s = 3'd0;
    reg [1:0]  lock_s = 2'd0;
    reg [22:0] count = 23'd0;
    reg [22:0] last = 23'd0;
    reg        fresh = 1'b0;
    always @(posedge sys) begin
        win_s  <= {win_s[1:0], win};
        lock_s <= {lock_s[0], lock};
        fresh  <= 1'b0;
        if (win_s[2] != win_s[1]) begin
            last  <= count;
            count <= 23'd1;
            fresh <= 1'b1;
        end else begin
            count <= count + 23'd1;
        end
    end

    // ---- the transmitter, 417 clocks a bit at 48 MHz -----------------------
    localparam [8:0] DIV = 9'd417;
    reg  [9:0] out_n = 10'd0;
    reg  [3:0] tbits = 4'd0;
    reg  [8:0] tdiv = 9'd0;
    reg        tx_go = 1'b0;
    reg  [7:0] tx_byte = 8'd0;
    wire       tx_busy = tbits != 4'd0;
    assign tx = ~out_n[0];
    always @(posedge sys) begin
        if (tx_go && !tx_busy) begin
            out_n <= ~{1'b1, tx_byte, 1'b0};
            tbits <= 4'd10;
            tdiv  <= DIV - 9'd1;
        end else if (tx_busy) begin
            if (tdiv != 9'd0) begin
                tdiv <= tdiv - 9'd1;
            end else begin
                tdiv  <= DIV - 9'd1;
                out_n <= {1'b0, out_n[9:1]};
                tbits <= tbits - 4'd1;
            end
        end
    end

    function [7:0] hex;
        input [3:0] v;
        hex = v < 4'd10 ? 8'h30 + {4'd0, v} : 8'h57 + {4'd0, v};
    endfunction

    // ---- the printer: "L<lock> N<count>\r\n" each window --------------------
    reg [3:0]  ch = 4'd0;              // 0 idle, 1..12 the characters
    reg [22:0] shown = 23'd0;
    reg        shown_lock = 1'b0;
    always @(posedge sys) begin
        tx_go <= 1'b0;
        if (ch == 4'd0) begin
            if (fresh) begin
                shown      <= last;
                shown_lock <= lock_s[1];
                ch         <= 4'd1;
            end
        end else if (!tx_busy && !tx_go) begin
            tx_go <= 1'b1;
            ch    <= ch == 4'd12 ? 4'd0 : ch + 4'd1;
            case (ch)
                4'd1:  tx_byte <= 8'h4c;                    // L
                4'd2:  tx_byte <= shown_lock ? 8'h31 : 8'h30;
                4'd3:  tx_byte <= 8'h20;
                4'd4:  tx_byte <= 8'h4e;                    // N
                4'd5:  tx_byte <= hex({1'b0, shown[22:20]});
                4'd6:  tx_byte <= hex(shown[19:16]);
                4'd7:  tx_byte <= hex(shown[15:12]);
                4'd8:  tx_byte <= hex(shown[11:8]);
                4'd9:  tx_byte <= hex(shown[7:4]);
                4'd10: tx_byte <= hex(shown[3:0]);
                4'd11: tx_byte <= 8'h0d;
                default: tx_byte <= 8'h0a;
            endcase
        end
    end
endmodule
