// computer_tb — runs the 6502 computer and reads what it says on its
// serial line.
//
// The testbench is a UART receiver of its own, written from the 8N1
// frame rather than taken from the library, so it checks the library's
// transmitter instead of agreeing with it. It waits for a start bit on
// `uart_tx`, samples the middle of each of the eight data bits and
// checks the stop bit, all at CLK_DIV clocks per bit — the rate the
// computer is built with. Every received line is printed with $display;
// the run ends after the first one, or with a message if nothing
// arrives.
`timescale 1ns / 1ns

module computer_tb;
    // The same divisor computer_top uses by default: 115200 baud from
    // 12 MHz.
    localparam CLK_DIV  = 104;
    // A 12 MHz clock is 83.3 ns; 84 keeps the arithmetic whole.
    localparam HALF     = 42;
    localparam BIT_TIME = CLK_DIV * 2 * HALF;
    // Long enough for a few dozen characters. The 6502 spends two clocks
    // per bus cycle here and a few cycles per character, which is still
    // far faster than one bit time.
    localparam TIMEOUT  = 64 * 10 * BIT_TIME;

    reg  clk = 1'b0;
    wire uart_tx;

    computer_top #(
        .CLK_DIV (CLK_DIV)
    ) dut (
        .clk     (clk),
        .uart_tx (uart_tx),
        .uart_rx (1'b1)
    );

    // The clock is all the computer needs: its power-on reset holds the
    // core for the first eight edges, exactly as it does on the board.
    always #HALF clk = ~clk;

    // The receiver: one byte per frame, gathered into a line.
    reg [7:0]      rx_byte;
    reg [8*64-1:0] line;
    integer        i;

    initial begin
        line = 0;
        forever begin
            @(negedge uart_tx);
            // Half a bit into the start bit, which must still be low.
            #(BIT_TIME / 2);
            if (uart_tx !== 1'b0) begin
                $display("computer_tb: glitch on uart_tx at %0t", $time);
            end else begin
                for (i = 0; i < 8; i = i + 1) begin
                    #(BIT_TIME);
                    rx_byte[i] = uart_tx;
                end
                #(BIT_TIME);
                if (uart_tx !== 1'b1) begin
                    $display("computer_tb: framing error at %0t", $time);
                    $finish;
                end
                if (rx_byte == 8'h0A) begin
                    $display("%s", line);
                    $finish;
                end
                line = {line[8*63-1:0], rx_byte};
            end
        end
    end

    initial begin
        #(TIMEOUT);
        $display("computer_tb: timed out with nothing but \"%s\" received", line);
        $finish;
    end
endmodule
