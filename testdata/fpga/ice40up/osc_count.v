// The iCE40UP5K's two internal oscillators, SB_HFOSC and SB_LFOSC,
// counted against the 12 MHz crystal oscillator of an iCE40UP5K-SG48
// board with an FT2232H (115200 baud, 8N1, pin 10 out, the 12 MHz
// oscillator on pin 20).
//
// SB_HFOSC runs at a nominal 48 MHz, divided by CLKHF_DIV: "0b10" asks
// for 12 MHz. SB_LFOSC runs at a nominal 10 kHz. Each drives a global
// network of its own, and each clocks a counter. A counter on the crystal
// toggles `win` every 2^20 cycles (87.4 ms), and each oscillator's domain
// counts its own cycles between toggles. So each window it prints
//
//   H<HFOSC's cycles, 6 hex digits> L<LFOSC's cycles, 4 hex digits>
//
// which at the nominal frequencies is H100000 L0369 (1 048 576 and 874).
// The data sheet allows the high-frequency one 10 % and the low-frequency
// one more, so a host checks the order of magnitude and that both run, and
// reports the frequencies measured.
//
// Every register starts at zero and is as wide as the values it holds; the
// transmit line idles high, so it is kept inverted.
module osc_count (
    input  wire clk,
    output wire tx
);
    wire hf, lf;
    SB_HFOSC #(.CLKHF_DIV("0b10")) hfosc (.CLKHFEN(1'b1), .CLKHFPU(1'b1), .CLKHF(hf));
    SB_LFOSC lfosc (.CLKLFEN(1'b1), .CLKLFPU(1'b1), .CLKLF(lf));

    // ---- the window, on the crystal -----------------------------------------
    reg [19:0] ref_n = 20'd0;
    reg        win = 1'b0;
    always @(posedge clk) begin
        ref_n <= ref_n + 20'd1;
        if (ref_n == 20'hfffff) win <= ~win;
    end

    // ---- each oscillator's count -------------------------------------------
    reg [2:0]  win_h = 3'd0;
    reg [22:0] count_h = 23'd0;
    reg [22:0] last_h = 23'd0;
    always @(posedge hf) begin
        win_h <= {win_h[1:0], win};
        if (win_h[2] != win_h[1]) begin
            last_h  <= count_h;
            count_h <= 23'd1;
        end else begin
            count_h <= count_h + 23'd1;
        end
    end
    reg [2:0]  win_l = 3'd0;
    reg [15:0] count_l = 16'd0;
    reg [15:0] last_l = 16'd0;
    always @(posedge lf) begin
        win_l <= {win_l[1:0], win};
        if (win_l[2] != win_l[1]) begin
            last_l  <= count_l;
            count_l <= 16'd1;
        end else begin
            count_l <= count_l + 16'd1;
        end
    end
    // Both counts into the crystal's domain half a window after they were
    // latched, when they are long settled.
    reg [22:0] seen_h = 23'd0;
    reg [15:0] seen_l = 16'd0;
    always @(posedge clk) begin
        if (ref_n == 20'h80000) begin
            seen_h <= last_h;
            seen_l <= last_l;
        end
    end

    // ---- the transmitter, 104 clocks a bit at 12 MHz -----------------------
    localparam [6:0] DIV = 7'd104;
    reg  [9:0] out_n = 10'd0;
    reg  [3:0] tbits = 4'd0;
    reg  [6:0] tdiv = 7'd0;
    reg        tx_go = 1'b0;
    reg  [7:0] tx_byte = 8'd0;
    wire       tx_busy = tbits != 4'd0;
    assign tx = ~out_n[0];
    always @(posedge clk) begin
        if (tx_go && !tx_busy) begin
            out_n <= ~{1'b1, tx_byte, 1'b0};
            tbits <= 4'd10;
            tdiv  <= DIV - 7'd1;
        end else if (tx_busy) begin
            if (tdiv != 7'd0) begin
                tdiv <= tdiv - 7'd1;
            end else begin
                tdiv  <= DIV - 7'd1;
                out_n <= {1'b0, out_n[9:1]};
                tbits <= tbits - 4'd1;
            end
        end
    end

    function [7:0] hex;
        input [3:0] v;
        hex = v < 4'd10 ? 8'h30 + {4'd0, v} : 8'h57 + {4'd0, v};
    endfunction

    // ---- the printer: "H<count> L<count>\r\n" each window -------------------
    reg [3:0]  ch = 4'd0;              // 0 idle, 1..15 the characters
    reg [22:0] shown_h = 23'd0;
    reg [15:0] shown_l = 16'd0;
    always @(posedge clk) begin
        tx_go <= 1'b0;
        if (ch == 4'd0) begin
            if (ref_n == 20'h00000) begin
                shown_h <= seen_h;
                shown_l <= seen_l;
                ch      <= 4'd1;
            end
        end else if (!tx_busy && !tx_go) begin
            tx_go <= 1'b1;
            ch    <= ch == 4'd15 ? 4'd0 : ch + 4'd1;
            case (ch)
                4'd1:  tx_byte <= 8'h48;                    // H
                4'd2:  tx_byte <= hex({1'b0, shown_h[22:20]});
                4'd3:  tx_byte <= hex(shown_h[19:16]);
                4'd4:  tx_byte <= hex(shown_h[15:12]);
                4'd5:  tx_byte <= hex(shown_h[11:8]);
                4'd6:  tx_byte <= hex(shown_h[7:4]);
                4'd7:  tx_byte <= hex(shown_h[3:0]);
                4'd8:  tx_byte <= 8'h20;
                4'd9:  tx_byte <= 8'h4c;                    // L
                4'd10: tx_byte <= hex(shown_l[15:12]);
                4'd11: tx_byte <= hex(shown_l[11:8]);
                4'd12: tx_byte <= hex(shown_l[7:4]);
                4'd13: tx_byte <= hex(shown_l[3:0]);
                4'd14: tx_byte <= 8'h0d;
                default: tx_byte <= 8'h0a;
            endcase
        end
    end
endmodule
