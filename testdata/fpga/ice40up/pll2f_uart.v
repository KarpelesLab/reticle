// The iCE40UP5K's PLL with both outputs, SB_PLL40_2F_CORE, on an
// iCE40UP5K-SG48 board with an FT2232H (115200 baud, 8N1, pin 10 out, the
// 12 MHz oscillator on pin 20). pll_uart.v is the one-output form, and
// this file is that one with a second output.
//
// The same dividers (DIVR 0, DIVF 63, DIVQ 4: a 768 MHz VCO), with port A
// GENCLK, 48 MHz, and port B GENCLK_HALF, half of that: 24 MHz. Both drive
// their global networks. Port A clocks the transmitter, at 417 clocks a
// bit; port B clocks a counter of its own. Each window of 2^20 reference
// cycles it prints
//
//   L<lock> N<A's cycles> H<B's cycles, from the window before>
//
// which at the intended frequencies is L1 N400000 H200000, each within a
// cycle or two of resynchronisation. B's count crosses into A's domain
// half a window after B latched it, when it is long settled.
//
// Every register starts at zero and is as wide as the values it holds; the
// transmit line idles high, so it is kept inverted.
module pll2f_uart (
    input  wire clk,
    output wire tx
);
    wire sys;
    wire half;
    wire lock;
    SB_PLL40_2F_CORE #(
        .FEEDBACK_PATH("SIMPLE"),
        .PLLOUT_SELECT_PORTA("GENCLK"),
        .PLLOUT_SELECT_PORTB("GENCLK_HALF"),
        .DIVR(4'd0),
        .DIVF(7'd63),
        .DIVQ(3'd4),
        .FILTER_RANGE(3'd1)
    ) pll (
        .REFERENCECLK(clk),
        .PLLOUTGLOBALA(sys),
        .PLLOUTGLOBALB(half),
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

    // ---- port B's domain ----------------------------------------------------
    reg [2:0]  win_h = 3'd0;
    reg [21:0] count_h = 22'd0;
    reg [21:0] last_h = 22'd0;
    always @(posedge half) begin
        win_h <= {win_h[1:0], win};
        if (win_h[2] != win_h[1]) begin
            last_h  <= count_h;
            count_h <= 22'd1;
        end else begin
            count_h <= count_h + 22'd1;
        end
    end
    // Into port A's domain half a window later, when it is settled.
    reg [21:0] last_h_a = 22'd0;
    always @(posedge sys) if (count == 23'h200000) last_h_a <= last_h;

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

    // ---- the printer: "L<lock> N<count> H<count>\r\n" each window ----------
    reg [4:0]  ch = 5'd0;              // 0 idle, 1..20 the characters
    reg [22:0] shown = 23'd0;
    reg [21:0] shown_h = 22'd0;
    reg        shown_lock = 1'b0;
    always @(posedge sys) begin
        tx_go <= 1'b0;
        if (ch == 5'd0) begin
            if (fresh) begin
                shown      <= last;
                shown_h    <= last_h_a;
                shown_lock <= lock_s[1];
                ch         <= 5'd1;
            end
        end else if (!tx_busy && !tx_go) begin
            tx_go <= 1'b1;
            ch    <= ch == 5'd20 ? 5'd0 : ch + 5'd1;
            case (ch)
                5'd1:  tx_byte <= 8'h4c;                    // L
                5'd2:  tx_byte <= shown_lock ? 8'h31 : 8'h30;
                5'd3:  tx_byte <= 8'h20;
                5'd4:  tx_byte <= 8'h4e;                    // N
                5'd5:  tx_byte <= hex({1'b0, shown[22:20]});
                5'd6:  tx_byte <= hex(shown[19:16]);
                5'd7:  tx_byte <= hex(shown[15:12]);
                5'd8:  tx_byte <= hex(shown[11:8]);
                5'd9:  tx_byte <= hex(shown[7:4]);
                5'd10: tx_byte <= hex(shown[3:0]);
                5'd11: tx_byte <= 8'h20;
                5'd12: tx_byte <= 8'h48;                    // H
                5'd13: tx_byte <= hex({2'b00, shown_h[21:20]});
                5'd14: tx_byte <= hex(shown_h[19:16]);
                5'd15: tx_byte <= hex(shown_h[15:12]);
                5'd16: tx_byte <= hex(shown_h[11:8]);
                5'd17: tx_byte <= hex(shown_h[7:4]);
                5'd18: tx_byte <= hex(shown_h[3:0]);
                5'd19: tx_byte <= 8'h0d;
                default: tx_byte <= 8'h0a;
            endcase
        end
    end
endmodule
