// A serial echo for an iCE40UP5K-SG48 board with an FT2232H: every byte
// received at 115200 baud, 8N1, on `rx` is sent back on `tx` plus one, so a
// host that writes "HAL" reads "IBM" -- an echo that a wire could not fake.
//
// 12 MHz / 115200 is 104.17 clocks a bit; 104 is 0.16 % fast, well inside
// what a UART tolerates. The receiver resynchronises `rx` through two
// flip-flops, samples each bit in its middle, and checks the stop bit; the
// transmitter is a ten-bit shift register. Every counter is exactly as
// wide as the values it holds.
//
// Every register starts at zero, because an iCE40 flip-flop does and this
// flow does not invert one to make it start at one. The two that idle high
// -- the line resynchroniser and the transmit shift register -- are
// therefore kept inverted: `sync_n` and `out_n` hold the complement of the
// line.
module uart_echo (
    input  wire clk,
    input  wire rx,
    output wire tx
);
    localparam [6:0] DIV = 7'd104;

    reg [1:0] sync_n = 2'b00;
    always @(posedge clk) sync_n <= {sync_n[0], ~rx};
    wire bit_in = ~sync_n[1];

    // Receiver.
    reg       busy = 1'b0;
    reg [6:0] rdiv = 7'd0;
    reg [3:0] rbit = 4'd0;
    reg [7:0] shift = 8'd0;
    reg       got = 1'b0;
    reg [7:0] byte_in = 8'd0;
    always @(posedge clk) begin
        got <= 1'b0;
        if (!busy) begin
            if (!bit_in) begin
                busy <= 1'b1;
                rdiv <= DIV >> 1;
                rbit <= 4'd0;
            end
        end else if (rdiv != 7'd0) begin
            rdiv <= rdiv - 7'd1;
        end else begin
            rdiv <= DIV - 7'd1;
            rbit <= rbit + 4'd1;
            if (rbit == 4'd0) begin
                if (bit_in) busy <= 1'b0;      // a glitch, not a start bit
            end else if (rbit <= 4'd8) begin
                shift <= {bit_in, shift[7:1]};
            end else begin
                busy <= 1'b0;
                if (bit_in) begin
                    byte_in <= shift;
                    got <= 1'b1;
                end
            end
        end
    end

    // Transmitter.
    reg [9:0] out_n = 10'h000;
    reg [6:0] tdiv = 7'd0;
    reg [3:0] left = 4'd0;
    assign tx = ~out_n[0];
    always @(posedge clk) begin
        if (got && left == 4'd0) begin
            out_n <= ~{1'b1, byte_in + 8'd1, 1'b0};
            left <= 4'd10;
            tdiv <= DIV - 7'd1;
        end else if (left != 4'd0) begin
            if (tdiv != 7'd0) begin
                tdiv <= tdiv - 7'd1;
            end else begin
                tdiv <= DIV - 7'd1;
                out_n <= {1'b0, out_n[9:1]};
                left <= left - 4'd1;
            end
        end
    end
endmodule
