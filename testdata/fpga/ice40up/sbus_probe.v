// The UltraPlus's hard SPI and I2C blocks on their system bus, for an
// iCE40UP5K-SG48 board with an FT2232H (115200 baud, 8N1, pin 10 out, the
// 12 MHz oscillator on pin 20). Nothing reaches a pin but the serial line:
// the blocks' SPI and I2C sides are left unconnected.
//
// The left SPI block answers on bus addresses 0x0_ (its BUS_ADDR74 is
// 0b0000) and the left I2C block on 0x1_ (0b0001). Once after
// configuration every one of their sixteen registers is written a value of
// its own, `8'h5A ^ {addr, addr}` for the SPI block and `8'hC3 ^ {addr,
// addr}` for the I2C one; then, over and over, every register is read and
// the line
//
//   S<16 bytes as hex> I<16 bytes as hex> A<acks as 8 hex digits>
//
// is printed, a read that is not acknowledged within 255 cycles giving
// `00` and a 0 bit in A. What a register holds after being written is the
// block's, not this design's, so the host reads the line rather than
// expecting one: a block that acknowledges and gives back what was written
// to its read/write registers is a block on the bus.
module sbus_probe (
    input  wire clk,
    output wire tx
);
    // The bus, driven to both blocks; each answers its own addresses.
    reg       stb = 1'b0;
    reg       rw = 1'b0;
    reg [7:0] adr = 8'd0;
    reg [7:0] dat = 8'd0;
    wire [7:0] spi_do, i2c_do;
    wire spi_ack, i2c_ack;

    SB_SPI #(.BUS_ADDR74("0b0000")) spi (
        .SBCLKI(clk), .SBRWI(rw), .SBSTBI(stb),
        .SBADRI7(adr[7]), .SBADRI6(adr[6]), .SBADRI5(adr[5]), .SBADRI4(adr[4]),
        .SBADRI3(adr[3]), .SBADRI2(adr[2]), .SBADRI1(adr[1]), .SBADRI0(adr[0]),
        .SBDATI7(dat[7]), .SBDATI6(dat[6]), .SBDATI5(dat[5]), .SBDATI4(dat[4]),
        .SBDATI3(dat[3]), .SBDATI2(dat[2]), .SBDATI1(dat[1]), .SBDATI0(dat[0]),
        .MI(1'b0), .SI(1'b0), .SCKI(1'b0), .SCSNI(1'b1),
        .SBDATO7(spi_do[7]), .SBDATO6(spi_do[6]), .SBDATO5(spi_do[5]), .SBDATO4(spi_do[4]),
        .SBDATO3(spi_do[3]), .SBDATO2(spi_do[2]), .SBDATO1(spi_do[1]), .SBDATO0(spi_do[0]),
        .SBACKO(spi_ack)
    );
    SB_I2C #(.BUS_ADDR74("0b0001")) i2c (
        .SBCLKI(clk), .SBRWI(rw), .SBSTBI(stb),
        .SBADRI7(adr[7]), .SBADRI6(adr[6]), .SBADRI5(adr[5]), .SBADRI4(adr[4]),
        .SBADRI3(adr[3]), .SBADRI2(adr[2]), .SBADRI1(adr[1]), .SBADRI0(adr[0]),
        .SBDATI7(dat[7]), .SBDATI6(dat[6]), .SBDATI5(dat[5]), .SBDATI4(dat[4]),
        .SBDATI3(dat[3]), .SBDATI2(dat[2]), .SBDATI1(dat[1]), .SBDATI0(dat[0]),
        .SCLI(1'b1), .SDAI(1'b1),
        .SBDATO7(i2c_do[7]), .SBDATO6(i2c_do[6]), .SBDATO5(i2c_do[5]), .SBDATO4(i2c_do[4]),
        .SBDATO3(i2c_do[3]), .SBDATO2(i2c_do[2]), .SBDATO1(i2c_do[1]), .SBDATO0(i2c_do[0]),
        .SBACKO(i2c_ack)
    );
    wire       ack = adr[4] ? i2c_ack : spi_ack;
    wire [7:0] rdata = adr[4] ? i2c_do : spi_do;

    // The registers read, and which acknowledged.
    reg [7:0]  got [0:31];
    reg [31:0] acks = 32'd0;

    // Bus sequencer: 32 writes once, then 32 reads, then a line, forever.
    localparam [2:0] IDLE = 3'd0, ACCESS = 3'd1, RELEASE = 3'd2, PRINT = 3'd3, WAIT = 3'd4;
    reg [2:0]  state = IDLE;
    reg        writing_n = 1'b0;   // 0: the writes, 1: the reads
    reg [4:0]  index = 5'd0;
    reg [7:0]  timeout = 8'd0;
    reg [21:0] pause = 22'd0;
    reg        start_line = 1'b0;
    wire       line_busy;

    always @(posedge clk) begin
        start_line <= 1'b0;
        case (state)
            IDLE: begin
                adr <= {3'b000, index};
                dat <= (index[4] ? 8'hC3 : 8'h5A) ^ {index[3:0], index[3:0]};
                rw <= ~writing_n;
                stb <= 1'b1;
                timeout <= 8'd0;
                state <= ACCESS;
            end
            ACCESS: begin
                timeout <= timeout + 8'd1;
                if (ack || timeout == 8'hFF) begin
                    if (writing_n) begin
                        got[index] <= ack ? rdata : 8'h00;
                        acks[index] <= ack;
                    end
                    stb <= 1'b0;
                    state <= RELEASE;
                end
            end
            RELEASE: begin
                index <= index + 5'd1;
                if (index == 5'd31) begin
                    if (writing_n) begin
                        start_line <= 1'b1;
                        state <= PRINT;
                    end else begin
                        writing_n <= 1'b1;
                        state <= IDLE;
                    end
                end else begin
                    state <= IDLE;
                end
            end
            PRINT: if (!line_busy && !start_line) begin
                pause <= 22'd0;
                state <= WAIT;
            end
            WAIT: begin
                pause <= pause + 22'd1;
                if (&pause) state <= IDLE;
            end
            default: state <= IDLE;
        endcase
    end

    // The line: 'S', 32 hex digits, ' ', 'I', 32 hex digits, ' ', 'A', 8
    // hex digits, CR, LF -- 80 characters.
    reg [6:0] ch = 7'd0;
    reg       sending = 1'b0;
    assign line_busy = sending;
    function [7:0] hex;
        input [3:0] v;
        hex = v < 4'd10 ? 8'h30 + {4'd0, v} : 8'h41 + {4'd0, v} - 8'd10;
    endfunction
    reg [7:0] char;
    always @* begin
        if (ch == 7'd0) char = "S";
        else if (ch <= 7'd32) char = hex(ch[0] ? got[(ch - 7'd1) >> 1][7:4] : got[(ch - 7'd1) >> 1][3:0]);
        else if (ch == 7'd33) char = " ";
        else if (ch == 7'd34) char = "I";
        else if (ch <= 7'd66) char = hex(ch[0] ? got[16 + ((ch - 7'd35) >> 1)][7:4] : got[16 + ((ch - 7'd35) >> 1)][3:0]);
        else if (ch == 7'd67) char = " ";
        else if (ch == 7'd68) char = "A";
        else if (ch <= 7'd76) char = hex(acks[31 - 4 * (ch - 7'd69) -: 4]);
        else if (ch == 7'd77) char = 8'h0D;
        else char = 8'h0A;
    end

    // The transmitter: 104 clocks a bit, idle high, kept inverted so that
    // it starts at zero.
    reg [9:0] out_n = 10'd0;
    reg [3:0] bits = 4'd0;
    reg [6:0] div = 7'd0;
    always @(posedge clk) begin
        if (start_line) begin
            sending <= 1'b1;
            ch <= 7'd0;
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
            if (ch == 7'd78) sending <= 1'b0;
            else ch <= ch + 7'd1;
        end
    end
    assign tx = ~out_n[0];
endmodule
