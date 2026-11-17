// All eight SB_MAC16 DSP blocks of an iCE40UP5K, each configured another
// way, fed one stream of vectors and reported over the serial line of an
// iCE40UP5K-SG48 board with an FT2232H (115200 baud, 8N1, pin 10 out, pin
// 9 in, the 12 MHz oscillator on pin 20), so that a host can check every
// result against a model of the cell.
//
// At power-up, and again whenever a byte arrives, it runs 64 steps. Each
// step presents a vector -- A, B, C, D and a control byte K -- waits for
// the blocks' outputs to settle, prints one line of 84 hex digits,
//
//   K(2) A(4) B(4) C(4) D(4) O0(8) O1(8) .. O7(8) CO(2)
//
// with block i's 32-bit O and its carry out in bit i of CO, and then
// pulses every block's CE for one clock, which is the only time any
// register in them loads. So line k is the blocks' outputs for vector k
// and the registers left by vectors 0..k-1, which a model can replay
// exactly. After the 64th line it prints END.
//
// K is {3'b0, ci, hold, addsub, oload, rst}:
//   rst    step 0 only: every block's IRSTTOP/IRSTBOT/ORSTTOP/ORSTBOT
//   oload  steps 1, 9, 17, ..: OLOADTOP/OLOADBOT of the accumulators
//   addsub ADDSUBTOP/ADDSUBBOT of block 6, ADDSUBTOP of block 7
//   hold   AHOLD and OHOLDBOT of block 6
//   ci     CI of block 7
//
// The blocks:
//   0  16x16 unsigned multiply, no register (O = A * B)
//   1  16x16 signed multiply, no register
//   2  16x16 multiply, A signed and B not, every pipeline register on
//   3  8x8 split, A signed and B not, the top product registered
//   4  8x8 split, unsigned, each product plus C (top) or registered D
//   5  32-bit multiply-accumulate, A and B and C registered, loaded from
//      C by oload
//   6  the same with add/subtract, holds and the carry select's XOR
//   7  a 32-bit adder, {A,B} + {C,D} + CI, with no multiplier at all
//
// Every register starts at zero (an iCE40 flip-flop does) and is as wide as
// the values it holds; the transmit line idles high, so it is kept inverted.
module dsp_selftest (
    input  wire clk,
    input  wire rx,
    output wire tx
);
    // ---- the serial transmitter (as in ram_selftest.v) --------------------
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

    reg [1:0] rx_n = 2'b00;
    always @(posedge clk) rx_n <= {rx_n[0], ~rx};
    wire rx_start = rx_n[1];

    // ---- the vectors -------------------------------------------------------
    reg  [5:0]  step = 6'd0;
    reg  [31:0] lab = 32'd0;           // A and B after the first eight steps
    reg  [31:0] lcd = 32'd0;           // C and D, and the control bits
    function [31:0] lfsr;
        input [31:0] s;
        lfsr = s[0] ? (s >> 1) ^ 32'h80200003 : s >> 1;
    endfunction
    // The first eight vectors are the corners a multiplier gets wrong.
    reg [31:0] edge_ab;
    always @(*) begin
        case (step[2:0])
            3'd0: edge_ab = 32'h0000_0000;
            3'd1: edge_ab = 32'hffff_ffff;
            3'd2: edge_ab = 32'h8000_8000;
            3'd3: edge_ab = 32'hffff_8000;
            3'd4: edge_ab = 32'h7fff_8000;
            3'd5: edge_ab = 32'h0001_ffff;
            3'd6: edge_ab = 32'h00ff_ff00;
            default: edge_ab = 32'h8001_7fff;
        endcase
    end
    wire [31:0] ab = step < 6'd8 ? edge_ab : lab;
    wire [15:0] a = ab[31:16];
    wire [15:0] b = ab[15:0];
    wire [15:0] c = lcd[31:16];
    wire [15:0] d = lcd[15:0];
    wire rst    = step == 6'd0;
    wire oload  = step[2:0] == 3'd1;
    wire addsub = lcd[0];
    wire hold   = lcd[5] & lcd[9];
    wire ci     = lab[7];
    wire [7:0] k = {3'b000, ci, hold, addsub, oload, rst};

    // ---- the blocks --------------------------------------------------------
    localparam S_IDLE = 3'd0, S_SETTLE = 3'd1, S_CAPTURE = 3'd2, S_PRINT = 3'd3,
               S_STEP = 3'd4, S_END = 3'd5;
    reg  [2:0] state = S_IDLE;
    wire ce = state == S_STEP;
    wire [31:0] o0, o1, o2, o3, o4, o5, o6, o7;
    wire [7:0]  co;

    SB_MAC16 #(.TOPOUTPUT_SELECT(2'd3), .BOTOUTPUT_SELECT(2'd3)) mul_u (
        .CLK(clk), .CE(ce), .A(a), .B(b),
        .IRSTTOP(rst), .IRSTBOT(rst), .ORSTTOP(rst), .ORSTBOT(rst),
        .O(o0), .CO(co[0]));
    SB_MAC16 #(.TOPOUTPUT_SELECT(2'd3), .BOTOUTPUT_SELECT(2'd3),
               .A_SIGNED(1'b1), .B_SIGNED(1'b1)) mul_s (
        .CLK(clk), .CE(ce), .A(a), .B(b),
        .IRSTTOP(rst), .IRSTBOT(rst), .ORSTTOP(rst), .ORSTBOT(rst),
        .O(o1), .CO(co[1]));
    SB_MAC16 #(.TOPOUTPUT_SELECT(2'd3), .BOTOUTPUT_SELECT(2'd3),
               .A_SIGNED(1'b1), .A_REG(1'b1), .B_REG(1'b1),
               .TOP_8x8_MULT_REG(1'b1), .BOT_8x8_MULT_REG(1'b1),
               .PIPELINE_16x16_MULT_REG1(1'b1),
               .PIPELINE_16x16_MULT_REG2(1'b1)) mul_p (
        .CLK(clk), .CE(ce), .A(a), .B(b),
        .IRSTTOP(rst), .IRSTBOT(rst), .ORSTTOP(rst), .ORSTBOT(rst),
        .O(o2), .CO(co[2]));
    SB_MAC16 #(.MODE_8x8(1'b1), .TOPOUTPUT_SELECT(2'd2),
               .BOTOUTPUT_SELECT(2'd2), .A_SIGNED(1'b1),
               .TOP_8x8_MULT_REG(1'b1)) mul_8 (
        .CLK(clk), .CE(ce), .A(a), .B(b),
        .IRSTTOP(rst), .IRSTBOT(rst), .ORSTTOP(rst), .ORSTBOT(rst),
        .O(o3), .CO(co[3]));
    SB_MAC16 #(.MODE_8x8(1'b1), .D_REG(1'b1),
               .TOPADDSUB_LOWERINPUT(2'd1), .TOPADDSUB_UPPERINPUT(1'b1),
               .BOTADDSUB_LOWERINPUT(2'd1), .BOTADDSUB_UPPERINPUT(1'b1)) mad_8 (
        .CLK(clk), .CE(ce), .A(a), .B(b), .C(c), .D(d),
        .IRSTTOP(rst), .IRSTBOT(rst), .ORSTTOP(rst), .ORSTBOT(rst),
        .O(o4), .CO(co[4]));
    SB_MAC16 #(.A_REG(1'b1), .B_REG(1'b1), .C_REG(1'b1),
               .TOPOUTPUT_SELECT(2'd1), .BOTOUTPUT_SELECT(2'd1),
               .TOPADDSUB_LOWERINPUT(2'd2), .BOTADDSUB_LOWERINPUT(2'd2),
               .TOPADDSUB_CARRYSELECT(2'd2)) mac (
        .CLK(clk), .CE(ce), .A(a), .B(b), .C(c), .D(d),
        .IRSTTOP(rst), .IRSTBOT(rst), .ORSTTOP(rst), .ORSTBOT(rst),
        .OLOADTOP(oload), .OLOADBOT(oload),
        .O(o5), .CO(co[5]));
    SB_MAC16 #(.A_SIGNED(1'b1), .B_SIGNED(1'b1),
               .A_REG(1'b1), .B_REG(1'b1), .C_REG(1'b1), .D_REG(1'b1),
               .TOPOUTPUT_SELECT(2'd1), .BOTOUTPUT_SELECT(2'd1),
               .TOPADDSUB_LOWERINPUT(2'd2), .BOTADDSUB_LOWERINPUT(2'd2),
               .TOPADDSUB_CARRYSELECT(2'd3), .BOTADDSUB_CARRYSELECT(2'd1)) mac_s (
        .CLK(clk), .CE(ce), .A(a), .B(b), .C(c), .D(d),
        .IRSTTOP(rst), .IRSTBOT(rst), .ORSTTOP(rst), .ORSTBOT(rst),
        .OLOADTOP(oload), .OLOADBOT(oload),
        .ADDSUBTOP(addsub), .ADDSUBBOT(addsub),
        .AHOLD(hold), .OHOLDBOT(hold),
        .O(o6), .CO(co[6]));
    SB_MAC16 #(.TOPADDSUB_UPPERINPUT(1'b1), .BOTADDSUB_UPPERINPUT(1'b1),
               .TOPADDSUB_CARRYSELECT(2'd2), .BOTADDSUB_CARRYSELECT(2'd3)) add32 (
        .CLK(clk), .CE(ce), .A(a), .B(b), .C(c), .D(d), .CI(ci),
        .IRSTTOP(rst), .IRSTBOT(rst), .ORSTTOP(rst), .ORSTBOT(rst),
        .ADDSUBTOP(addsub),
        .O(o7), .CO(co[7]));

    // ---- the sequencer and the printer -------------------------------------
    reg  [19:0] boot = 20'd0;          // about 87 ms before the first run
    reg         ran = 1'b0;
    reg  [6:0]  n = 7'd0;              // character within the line
    reg  [335:0] line = 336'd0;
    reg         last = 1'b0;           // the line printed is the 64th

    function [7:0] hex;
        input [3:0] v;
        hex = v < 4'd10 ? 8'h30 + {4'd0, v} : 8'h57 + {4'd0, v};
    endfunction

    always @(posedge clk) begin
        tx_go <= 1'b0;
        if (!boot[19]) boot <= boot + 20'd1;
        case (state)
            S_IDLE: begin
                if ((boot[19] && !ran) || rx_start) begin
                    ran   <= 1'b1;
                    step  <= 6'd0;
                    lab   <= 32'h1234_5678;
                    lcd   <= 32'h9abc_def1;
                    state <= S_SETTLE;
                end
            end
            S_SETTLE: state <= S_CAPTURE;
            S_CAPTURE: begin
                line  <= {k, a, b, c, d, o0, o1, o2, o3, o4, o5, o6, o7, co};
                last  <= step == 6'd63;
                n     <= 7'd0;
                state <= S_PRINT;
            end
            S_PRINT: begin
                if (!tx_busy && !tx_go) begin
                    tx_go <= 1'b1;
                    if (n < 7'd84) begin
                        tx_byte <= hex(line[335:332]);
                        line    <= {line[331:0], 4'd0};
                        n       <= n + 7'd1;
                    end else if (n == 7'd84) begin
                        tx_byte <= 8'h0d;
                        n       <= n + 7'd1;
                    end else begin
                        tx_byte <= 8'h0a;
                        n       <= 7'd0;
                        state   <= last ? S_END : S_STEP;
                    end
                end
            end
            S_STEP: begin
                // CE is high for exactly this clock: every block loads from
                // vector `step`, and the next vector follows.
                step  <= step + 6'd1;
                lab   <= lfsr(lab);
                lcd   <= lfsr(lcd);
                state <= S_SETTLE;
            end
            default: begin
                // "END\r\n", reusing the line register.
                if (!tx_busy && !tx_go) begin
                    tx_go <= 1'b1;
                    case (n)
                        7'd0: tx_byte <= 8'h45;
                        7'd1: tx_byte <= 8'h4e;
                        7'd2: tx_byte <= 8'h44;
                        7'd3: tx_byte <= 8'h0d;
                        default: tx_byte <= 8'h0a;
                    endcase
                    n <= n + 7'd1;
                    if (n == 7'd4) state <= S_IDLE;
                end
            end
        endcase
    end
endmodule
