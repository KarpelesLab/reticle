// Both kinds of RAM of an iCE40UP5K, written, read and reported over the
// serial line of an iCE40UP5K-SG48 board with an FT2232H (115200 baud, 8N1,
// pin 10 out, pin 9 in, the 12 MHz oscillator on pin 20).
//
// At power-up, and again whenever a byte arrives, it:
//
//   1. fills all four SB_SPRAM256KA (16K x 16 each) with a pattern of the
//      address, rewrites half of every word's nibbles through MASKWREN =
//      4'b0101 with the complement, and reads all 65536 words back;
//   2. writes and reads back four inferred block RAMs, one in each of the
//      SB_RAM40_4K's widths: 256 x 16, 512 x 8, 1024 x 4 and 2048 x 2;
//   3. prints one line of verdicts, P or F per RAM, then dumps four
//      read-only block RAMs whose contents come from `initial` blocks, one
//      in each width, as hex, so that a host can compare every bit with the
//      formula below and see the contents' bit order on silicon:
//
//        SP:PPPP BR:PPPP
//        R16:<256 words, 4 digits each>
//        R08:<512 words, 2 digits each>
//        R04:<1024 words, 1 digit each>
//        R02:<2048 words, 1 digit each>
//        END
//
// The RGB LED is blue while it runs, then green if every RAM passed and red
// if one did not.
//
// Every register starts at zero (an iCE40 flip-flop does) and is as wide as
// the values it holds; the transmit line idles high, so it is kept inverted.
module ram_selftest (
    input  wire clk,
    input  wire rx,
    output wire tx,
    output wire led_r,
    output wire led_g,
    output wire led_b
);
    // ---- the read-only memories, contents from `initial` ---------------
    reg [15:0] rom16 [0:255];
    reg [7:0]  rom8  [0:511];
    reg [3:0]  rom4  [0:1023];
    reg [1:0]  rom2  [0:2047];
    integer i;
    initial begin
        for (i = 0; i < 256; i = i + 1)
            rom16[i] = {i[7:0] ^ 8'h5a, ~i[7:0]};
        for (i = 0; i < 512; i = i + 1)
            rom8[i] = i[7:0] ^ {i[8], i[8:2]} ^ 8'hc3;
        for (i = 0; i < 1024; i = i + 1)
            rom4[i] = i[3:0] ^ i[7:4] ^ {i[9:8], i[9:8]};
        for (i = 0; i < 2048; i = i + 1)
            rom2[i] = i[1:0] ^ i[3:2] ^ i[10:9];
    end

    // ---- the serial transmitter -----------------------------------------
    localparam [6:0] DIV = 7'd104;
    reg  [9:0] out_n = 10'd0;          // complement of the shift register
    reg  [3:0] tbits = 4'd0;           // bits left to send
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

    // ---- a start bit on the receive line asks for another run ----------
    reg [1:0] rx_n = 2'b00;
    always @(posedge clk) rx_n <= {rx_n[0], ~rx};
    wire rx_start = rx_n[1];

    // ---- the sequencer --------------------------------------------------
    localparam [2:0] S_WAIT = 3'd0, S_SPW = 3'd1, S_SPM = 3'd2, S_SPR = 3'd3,
                     S_BRW = 3'd4, S_BRR = 3'd5, S_PRINT = 3'd6, S_DONE = 3'd7;
    reg [2:0]  state = S_WAIT;
    reg [19:0] boot = 20'd0;           // about 87 ms before the first run
    reg [14:0] a = 15'd0;              // address counter, one past the end too
    reg        chk = 1'b0;             // compare the word read last cycle
    reg [13:0] a_d = 14'd0;            // its address
    reg [3:0]  sp_bad = 4'd0;
    reg [3:0]  br_bad = 4'd0;
    reg        ran = 1'b0;

    // SPRAM pattern: the address, rotated, under a per-block key.
    function [15:0] sp_pat;
        input [1:0]  k;
        input [13:0] ad;
        begin
            case (k)
                2'd0: sp_pat = {ad[7:0], ad[13:6]} ^ 16'ha5c3;
                2'd1: sp_pat = {ad[7:0], ad[13:6]} ^ 16'h3c5a;
                2'd2: sp_pat = {ad[7:0], ad[13:6]} ^ 16'h0ff0;
                default: sp_pat = {ad[7:0], ad[13:6]} ^ 16'hf00f;
            endcase
        end
    endfunction

    wire        sp_we   = state == S_SPW || state == S_SPM;
    wire [3:0]  sp_mask = state == S_SPM ? 4'b0101 : 4'b1111;
    wire [13:0] sp_addr = a[13:0];
    wire [15:0] sp_q0, sp_q1, sp_q2, sp_q3;
    wire [15:0] sp_d0 = state == S_SPM ? ~sp_pat(2'd0, sp_addr) : sp_pat(2'd0, sp_addr);
    wire [15:0] sp_d1 = state == S_SPM ? ~sp_pat(2'd1, sp_addr) : sp_pat(2'd1, sp_addr);
    wire [15:0] sp_d2 = state == S_SPM ? ~sp_pat(2'd2, sp_addr) : sp_pat(2'd2, sp_addr);
    wire [15:0] sp_d3 = state == S_SPM ? ~sp_pat(2'd3, sp_addr) : sp_pat(2'd3, sp_addr);

    SB_SPRAM256KA spram0 (.ADDRESS(sp_addr), .DATAIN(sp_d0), .MASKWREN(sp_mask),
        .WREN(sp_we), .CHIPSELECT(1'b1), .CLOCK(clk), .STANDBY(1'b0),
        .SLEEP(1'b0), .POWEROFF(1'b1), .DATAOUT(sp_q0));
    SB_SPRAM256KA spram1 (.ADDRESS(sp_addr), .DATAIN(sp_d1), .MASKWREN(sp_mask),
        .WREN(sp_we), .CHIPSELECT(1'b1), .CLOCK(clk), .STANDBY(1'b0),
        .SLEEP(1'b0), .POWEROFF(1'b1), .DATAOUT(sp_q1));
    SB_SPRAM256KA spram2 (.ADDRESS(sp_addr), .DATAIN(sp_d2), .MASKWREN(sp_mask),
        .WREN(sp_we), .CHIPSELECT(1'b1), .CLOCK(clk), .STANDBY(1'b0),
        .SLEEP(1'b0), .POWEROFF(1'b1), .DATAOUT(sp_q2));
    SB_SPRAM256KA spram3 (.ADDRESS(sp_addr), .DATAIN(sp_d3), .MASKWREN(sp_mask),
        .WREN(sp_we), .CHIPSELECT(1'b1), .CLOCK(clk), .STANDBY(1'b0),
        .SLEEP(1'b0), .POWEROFF(1'b1), .DATAOUT(sp_q3));

    // What a word holds after both writes: the masked pass replaced nibbles
    // 0 and 2 with the complement.
    wire [15:0] sp_e0 = sp_pat(2'd0, a_d) ^ 16'h0f0f;
    wire [15:0] sp_e1 = sp_pat(2'd1, a_d) ^ 16'h0f0f;
    wire [15:0] sp_e2 = sp_pat(2'd2, a_d) ^ 16'h0f0f;
    wire [15:0] sp_e3 = sp_pat(2'd3, a_d) ^ 16'h0f0f;

    // The written block RAMs, one per width, and their patterns.
    reg [15:0] m16 [0:255];
    reg [7:0]  m8  [0:511];
    reg [3:0]  m4  [0:1023];
    reg [1:0]  m2  [0:2047];
    wire [10:0] ba = a[10:0];
    wire [15:0] p16 = {ba[7:0], ~ba[7:0]} ^ 16'h1357;
    wire [7:0]  p8  = ba[7:0] ^ {ba[8], ba[8:2]};
    wire [3:0]  p4  = ba[3:0] ^ ba[7:4] ^ {ba[9:8], ba[9:8]};
    wire [1:0]  p2  = ba[1:0] ^ ba[3:2] ^ ba[10:9];
    wire        bw  = state == S_BRW;
    always @(posedge clk) begin
        if (bw && a < 15'd256)  m16[ba[7:0]] <= p16;
        if (bw && a < 15'd512)  m8[ba[8:0]]  <= p8;
        if (bw && a < 15'd1024) m4[ba[9:0]]  <= p4;
        if (bw)                 m2[ba]       <= p2;
    end
    reg [15:0] q16 = 16'd0;
    reg [7:0]  q8  = 8'd0;
    reg [3:0]  q4  = 4'd0;
    reg [1:0]  q2  = 2'd0;
    always @(posedge clk) begin
        q16 <= m16[ba[7:0]];
        q8  <= m8[ba[8:0]];
        q4  <= m4[ba[9:0]];
        q2  <= m2[ba];
    end
    wire [10:0] bd = a_d[10:0];
    wire [15:0] e16 = {bd[7:0], ~bd[7:0]} ^ 16'h1357;
    wire [7:0]  e8  = bd[7:0] ^ {bd[8], bd[8:2]};
    wire [3:0]  e4  = bd[3:0] ^ bd[7:4] ^ {bd[9:8], bd[9:8]};
    wire [1:0]  e2  = bd[1:0] ^ bd[3:2] ^ bd[10:9];

    // The read-only memories, read on the printer's index.
    reg  [11:0] n = 12'd0;             // character within the section
    reg  [2:0]  sec = 3'd0;            // which section is printing
    wire [11:0] m = n - 12'd4;         // data index within it
    reg  [15:0] r16 = 16'd0;
    reg  [7:0]  r8  = 8'd0;
    reg  [3:0]  r4  = 4'd0;
    reg  [1:0]  r2  = 2'd0;
    always @(posedge clk) begin
        r16 <= rom16[m[9:2]];
        r8  <= rom8[m[9:1]];
        r4  <= rom4[m[9:0]];
        r2  <= rom2[m[10:0]];
    end

    function [7:0] hex;
        input [3:0] v;
        hex = v < 4'd10 ? 8'h30 + {4'd0, v} : 8'h57 + {4'd0, v};
    endfunction

    // The section lengths: header line, four dumps, END.
    reg [11:0] len;
    always @(*) begin
        case (sec)
            3'd0: len = 12'd17;
            3'd1: len = 12'd1030;
            3'd2: len = 12'd1030;
            3'd3: len = 12'd1030;
            3'd4: len = 12'd2054;
            default: len = 12'd5;
        endcase
    end

    // The character at (sec, n).
    reg [7:0] ch;
    reg [3:0] nib;
    always @(*) begin
        nib = 4'd0;
        case (sec)
            3'd1: nib = m[1:0] == 2'd0 ? r16[15:12] : m[1:0] == 2'd1 ? r16[11:8] :
                        m[1:0] == 2'd2 ? r16[7:4] : r16[3:0];
            3'd2: nib = m[0] ? r8[3:0] : r8[7:4];
            3'd3: nib = r4;
            default: nib = {2'd0, r2};
        endcase
        ch = 8'h3f;
        if (sec == 3'd0) begin
            case (n)
                12'd0: ch = "S";  12'd1: ch = "P";  12'd2: ch = ":";
                12'd3: ch = sp_bad[0] ? "F" : "P";  12'd4: ch = sp_bad[1] ? "F" : "P";
                12'd5: ch = sp_bad[2] ? "F" : "P";  12'd6: ch = sp_bad[3] ? "F" : "P";
                12'd7: ch = " ";  12'd8: ch = "B";  12'd9: ch = "R";  12'd10: ch = ":";
                12'd11: ch = br_bad[0] ? "F" : "P"; 12'd12: ch = br_bad[1] ? "F" : "P";
                12'd13: ch = br_bad[2] ? "F" : "P"; 12'd14: ch = br_bad[3] ? "F" : "P";
                12'd15: ch = 8'h0d;
                default: ch = 8'h0a;
            endcase
        end else if (sec == 3'd5) begin
            case (n)
                12'd0: ch = "E"; 12'd1: ch = "N"; 12'd2: ch = "D"; 12'd3: ch = 8'h0d;
                default: ch = 8'h0a;
            endcase
        end else if (n == 12'd0) begin
            ch = "R";
        end else if (n == 12'd1) begin
            ch = sec == 3'd1 ? "1" : "0";
        end else if (n == 12'd2) begin
            ch = sec == 3'd1 ? "6" : sec == 3'd2 ? "8" : sec == 3'd3 ? "4" : "2";
        end else if (n == 12'd3) begin
            ch = ":";
        end else if (n == len - 12'd2) begin
            ch = 8'h0d;
        end else if (n == len - 12'd1) begin
            ch = 8'h0a;
        end else begin
            ch = hex(nib);
        end
    end

    reg [1:0] settle = 2'd0;           // cycles since n last moved
    always @(posedge clk) begin
        tx_go <= 1'b0;
        chk <= 1'b0;
        case (state)
            S_WAIT: begin
                if (!ran) begin
                    boot <= boot + 20'd1;
                    if (boot == 20'hfffff) state <= S_SPW;
                end else if (rx_start) begin
                    state <= S_SPW;
                end
                a <= 15'd0;
                sp_bad <= 4'd0;
                br_bad <= 4'd0;
            end
            S_SPW, S_SPM: begin
                if (a == 15'd16383) begin
                    a <= 15'd0;
                    state <= state == S_SPW ? S_SPM : S_SPR;
                end else begin
                    a <= a + 15'd1;
                end
            end
            S_SPR: begin
                chk <= a < 15'd16384;
                a_d <= a[13:0];
                if (a == 15'd16384) begin
                    a <= 15'd0;
                    state <= S_BRW;
                end else begin
                    a <= a + 15'd1;
                end
            end
            S_BRW: begin
                if (a == 15'd2047) begin
                    a <= 15'd0;
                    state <= S_BRR;
                end else begin
                    a <= a + 15'd1;
                end
            end
            S_BRR: begin
                chk <= a < 15'd2048;
                a_d <= {3'd0, a[10:0]};
                if (a == 15'd2048) begin
                    a <= 15'd0;
                    n <= 12'd0;
                    sec <= 3'd0;
                    settle <= 2'd0;
                    state <= S_PRINT;
                end else begin
                    a <= a + 15'd1;
                end
            end
            S_PRINT: begin
                if (settle != 2'd3) begin
                    settle <= settle + 2'd1;
                end else if (!tx_busy && !tx_go) begin
                    tx_byte <= ch;
                    tx_go <= 1'b1;
                    settle <= 2'd0;
                    if (n == len - 12'd1) begin
                        n <= 12'd0;
                        if (sec == 3'd5) state <= S_DONE;
                        else sec <= sec + 3'd1;
                    end else begin
                        n <= n + 12'd1;
                    end
                end
            end
            default: begin
                ran <= 1'b1;
                if (!tx_busy && !tx_go) state <= S_WAIT;
            end
        endcase
        // The compares, one cycle behind the reads.
        if (chk && state == S_SPR) begin
            if (sp_q0 != sp_e0) sp_bad[0] <= 1'b1;
            if (sp_q1 != sp_e1) sp_bad[1] <= 1'b1;
            if (sp_q2 != sp_e2) sp_bad[2] <= 1'b1;
            if (sp_q3 != sp_e3) sp_bad[3] <= 1'b1;
        end
        if (chk && state == S_BRR) begin
            if (bd < 11'd256  && q16 != e16) br_bad[0] <= 1'b1;
            if (bd < 11'd512  && q8  != e8)  br_bad[1] <= 1'b1;
            if (bd < 11'd1024 && q4  != e4)  br_bad[2] <= 1'b1;
            if (q2 != e2) br_bad[3] <= 1'b1;
        end
    end

    // ---- the LED: blue while running, then green or red ------------------
    wire running = state != S_WAIT;
    wire bad = sp_bad != 4'd0 || br_bad != 4'd0;
    SB_RGBA_DRV #(
        .CURRENT_MODE("0b1"),
        .RGB0_CURRENT("0b000001"),
        .RGB1_CURRENT("0b000001"),
        .RGB2_CURRENT("0b000001")
    ) rgb (
        .CURREN(1'b1),
        .RGBLEDEN(1'b1),
        .RGB0PWM(!running && bad),
        .RGB1PWM(!running && !bad && ran),
        .RGB2PWM(running),
        .RGB0(led_r),
        .RGB1(led_g),
        .RGB2(led_b)
    );
endmodule
