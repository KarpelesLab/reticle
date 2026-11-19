// How fast an unregistered SB_MAC16 multiply really is, on an
// iCE40UP5K-SG48 board with an FT2232H (115200 baud, 8N1, pin 10 out, the
// 12 MHz oscillator on pin 20).
//
// The PLL makes a clock of 12 MHz x (DIVF + 1) / 2^DIVQ (DIVR 0). On it, a
// 32-bit linear-feedback register gives two 16-bit operands each cycle,
// registered in the fabric, and two DSP blocks multiply them:
//
// - `comb`, with no register at all: a fabric register, the multiplier,
//   and a fabric register again, all inside one clock period — the path
//   whose timing nextpnr and icetime disagree about;
// - `piped`, with every register it has on (inputs, partial products,
//   product), so that its own paths are short whatever the multiplier
//   does, three cycles later.
//
// Each cycle `comb`'s product, delayed three cycles, is compared with
// `piped`'s, and over 2^22 cycles the differing cycles are counted. Then
// the count is held, and the 12 MHz side prints it, forever:
//
//   M<6 hex digits: cycles differing> L<lock>
//
// All 0 says the unregistered multiply settles within one period at this
// clock; a count says it does not. The build is repeated with DIVF and
// DIVQ stepped (`reticle fpga --param`), and the board says where it
// starts to fail.
//
// `PIPE` 1 is the control: `comb` gets every register `piped` has, and is
// compared without the delay, so that a count at a clock where `PIPE` 0
// counts and `PIPE` 1 does not is the unregistered multiply's and nothing
// else's.
module dsp_speed #(
    parameter [6:0] DIVF = 7'd63,
    parameter [2:0] DIVQ = 3'd4,
    parameter [0:0] PIPE = 1'b0
) (
    input  wire clk,
    output wire tx
);
    wire fast, lock;
    SB_PLL40_CORE #(
        .FEEDBACK_PATH("SIMPLE"),
        .PLLOUT_SELECT("GENCLK"),
        .DIVR(4'd0),
        .DIVF(DIVF),
        .DIVQ(DIVQ),
        .FILTER_RANGE(3'd1)
    ) pll (
        .REFERENCECLK(clk),
        .PLLOUTGLOBAL(fast),
        .LOCK(lock),
        .RESETB(1'b1),
        .BYPASS(1'b0)
    );

    // The operands.
    reg [31:0] lfsr = 32'd0;   // an XNOR register, which may start at zero
    reg [15:0] a = 16'd0, b = 16'd0;
    always @(posedge fast) begin
        lfsr <= {lfsr[30:0], ~(lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0])};
        a <= lfsr[15:0];
        b <= lfsr[31:16];
    end

    wire [31:0] o_comb, o_piped;
    SB_MAC16 #(
        .A_REG(PIPE),
        .B_REG(PIPE),
        .TOP_8x8_MULT_REG(PIPE),
        .BOT_8x8_MULT_REG(PIPE),
        .PIPELINE_16x16_MULT_REG1(PIPE),
        .PIPELINE_16x16_MULT_REG2(PIPE),
        .TOPOUTPUT_SELECT(2'b11),
        .BOTOUTPUT_SELECT(2'b11)
    ) comb (
        .CLK(fast), .CE(1'b1), .A(a), .B(b), .C(16'd0), .D(16'd0),
        .AHOLD(1'b0), .BHOLD(1'b0), .CHOLD(1'b0), .DHOLD(1'b0),
        .IRSTTOP(1'b0), .IRSTBOT(1'b0), .ORSTTOP(1'b0), .ORSTBOT(1'b0),
        .OLOADTOP(1'b0), .OLOADBOT(1'b0), .ADDSUBTOP(1'b0), .ADDSUBBOT(1'b0),
        .OHOLDTOP(1'b0), .OHOLDBOT(1'b0), .CI(1'b0), .ACCUMCI(1'b0), .SIGNEXTIN(1'b0),
        .O(o_comb)
    );
    SB_MAC16 #(
        .A_REG(1'b1),
        .B_REG(1'b1),
        .TOP_8x8_MULT_REG(1'b1),
        .BOT_8x8_MULT_REG(1'b1),
        .PIPELINE_16x16_MULT_REG1(1'b1),
        .PIPELINE_16x16_MULT_REG2(1'b1),
        .TOPOUTPUT_SELECT(2'b11),
        .BOTOUTPUT_SELECT(2'b11)
    ) piped (
        .CLK(fast), .CE(1'b1), .A(a), .B(b), .C(16'd0), .D(16'd0),
        .AHOLD(1'b0), .BHOLD(1'b0), .CHOLD(1'b0), .DHOLD(1'b0),
        .IRSTTOP(1'b0), .IRSTBOT(1'b0), .ORSTTOP(1'b0), .ORSTBOT(1'b0),
        .OLOADTOP(1'b0), .OLOADBOT(1'b0), .ADDSUBTOP(1'b0), .ADDSUBBOT(1'b0),
        .OHOLDTOP(1'b0), .OHOLDBOT(1'b0), .CI(1'b0), .ACCUMCI(1'b0), .SIGNEXTIN(1'b0),
        .O(o_piped)
    );

    // comb's product captured, then delayed to line up with piped's, which
    // is captured too; the comparison pipelined a nibble at a time.
    reg [31:0] p0 = 32'd0, p1 = 32'd0, p2 = 32'd0, p3 = 32'd0, q = 32'd0;
    reg [7:0]  nib = 8'd0;
    reg        differ = 1'b0;
    wire [31:0] pc = PIPE ? p0 : p3;
    always @(posedge fast) begin
        p0 <= o_comb;
        p1 <= p0;
        p2 <= p1;
        p3 <= p2;
        q <= o_piped;
        nib <= {|(pc[31:28] ^ q[31:28]), |(pc[27:24] ^ q[27:24]),
                |(pc[23:20] ^ q[23:20]), |(pc[19:16] ^ q[19:16]),
                |(pc[15:12] ^ q[15:12]), |(pc[11:8] ^ q[11:8]),
                |(pc[7:4] ^ q[7:4]), |(pc[3:0] ^ q[3:0])};
        differ <= |nib;
    end

    // A window of 2^22 cycles, after 256 cycles for the pipelines to fill;
    // then the count is held.
    reg [22:0] cycles = 23'd0;
    reg        settled = 1'b0;
    reg [23:0] count = 24'd0;
    reg        done = 1'b0;
    always @(posedge fast) begin
        if (!settled) begin
            cycles <= cycles + 23'd1;
            if (cycles[8]) begin
                settled <= 1'b1;
                cycles <= 23'd0;
            end
        end else if (!done) begin
            cycles <= cycles + 23'd1;
            if (differ) count <= count + 24'd1;
            if (cycles[22]) done <= 1'b1;
        end
    end

    // The 12 MHz side: once `done` has crossed over, the count is stable.
    reg [2:0] done_s = 3'd0;
    reg [1:0] lock_s = 2'd0;
    reg [21:0] tick = 22'd0;
    reg        go = 1'b0;
    always @(posedge clk) begin
        done_s <= {done_s[1:0], done};
        lock_s <= {lock_s[0], lock};
        tick <= tick + 22'd1;
        go <= done_s[2] && &tick;
    end

    // "M<6 hex> L<lock>\r\n": twelve characters.
    function [7:0] hex;
        input [3:0] v;
        hex = v < 4'd10 ? 8'h30 + {4'd0, v} : 8'h41 + {4'd0, v} - 8'd10;
    endfunction
    reg [3:0] ch = 4'd0;
    reg       sending = 1'b0;
    reg [7:0] char;
    always @* begin
        case (ch)
            4'd0: char = "M";
            4'd1: char = hex(count[23:20]);
            4'd2: char = hex(count[19:16]);
            4'd3: char = hex(count[15:12]);
            4'd4: char = hex(count[11:8]);
            4'd5: char = hex(count[7:4]);
            4'd6: char = hex(count[3:0]);
            4'd7: char = " ";
            4'd8: char = "L";
            4'd9: char = 8'h30 + {7'd0, lock_s[1]};
            4'd10: char = 8'h0D;
            default: char = 8'h0A;
        endcase
    end
    reg [9:0] out_n = 10'd0;
    reg [3:0] bits = 4'd0;
    reg [6:0] div = 7'd0;
    always @(posedge clk) begin
        if (go) begin
            sending <= 1'b1;
            ch <= 4'd0;
        end
        if (bits != 4'd0) begin
            div <= div + 7'd1;
            if (div == 7'd103) begin
                div <= 7'd0;
                out_n <= {1'b0, out_n[9:1]};
                bits <= bits - 4'd1;
            end
        end else if (sending) begin
            out_n <= ~{1'b1, char, 1'b0};
            bits <= 4'd10;
            div <= 7'd0;
            if (ch == 4'd11) sending <= 1'b0;
            else ch <= ch + 4'd1;
        end
    end
    assign tx = ~out_n[0];
endmodule
