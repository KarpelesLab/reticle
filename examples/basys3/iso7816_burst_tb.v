// How much of a burst at the fast rate survives the terminal's receive path.
//
// **This measures; it does not assert a number.** What it prints depends on
// the console's rate against the card's, which is a property of a board and
// not a defect, and this project's rule is that a throughput figure is
// printed and never asserted.
//
// What it is for is the thing that is missing: a queue. Two were written and
// both reverted, because both passed `iso7816_terminal_tb.v` and then failed
// differently on the part -- 256 entries as an array read asynchronously
// (distributed RAM; it silenced the card half the moment the card was
// activated) and 32 entries as a packed vector with a multiplexed read (it
// flooded the console with the banner table's default character). Neither
// failure reproduced off the board, so neither could be debugged.
//
// A third attempt should make this print `lost 0` first. With the one-deep
// buffer it prints a loss, which is how it is known to be measuring anything
// at all.
//
// **Measured again on 9 October 2026, at the board's own ratios.** This
// used to end a line after every card character (`GAP_ETU` 8, shorter than
// one 12-etu character) and to model a console character as 7.5 fast etu;
// the board ends a line after 256 etu of silence, and a 2 Mbaud console
// character is 10 etu of `F/D = 4`. At those ratios, with the one-deep
// buffer:
//
//   F/D = 4 (PPS1 87, 2 Mbaud on the contact)   24 bytes: 10 lost   200: 85
//   F/D = 8 (PPS1 97, 1 Mbaud on the contact)   24 bytes:  0 lost   200:  0
//
// A card byte is 12 etu and printing it costs two characters, 20 bits: at
// F/D = 4 that is 10 us against 6, and at F/D = 8 10 us against 12. So the
// terminal asks for `97` now and this runs at F/D = 8. What a queue is
// still for is a reply that starts while the display half holds the shared
// port mid-line, which this testbench does not model.
//
// Measured on hardware for comparison: a full sequence against a real card
// leaves `lost_q` at 4, and the card's reply to its initialisation frame came
// back as `4E 00 00 4E 00 00 31 00 01 60 00 00` with 7 dropped. That reply
// carries a `60` -- T=0's NULL procedure byte, which asks for more time -- so
// answering it correctly needs all of it.

`timescale 1ns / 1ps
module iso7816_burst_tb;
    // Scaled so the ratio that decides the answer is the real one: a card
    // byte every 12 etu, against a console spending two characters on each
    // of them. (This said three, which is what an 8-etu `GAP_ETU` made it:
    // a line ending after every byte.)
    localparam integer CARD_DIV    = 2;
    localparam integer ETU_CYCLES  = 16;      // slow etu = 32 clocks
    localparam integer FAST_ETU    = 8;       // fast etu = 16 clocks, F/D = 8
    // A 2 Mbaud console character is 10 bits of 56 system clocks, which is 10
    // etu of `F/D = 4` (56 clocks too): 10 * 4 * CARD_DIV here.
    localparam integer CHAR_CLOCKS = 10 * 4 * CARD_DIV;
    localparam integer BURST       = 24;

    reg clk = 1'b0;
    always #5 clk = ~clk;

    reg       cmd_valid = 1'b0;
    reg [7:0] cmd_data  = 8'd0;
    reg       out_ready = 1'b0;
    reg       card_low  = 1'b0;

    wire clk_card, rst_card, vcc_en, term_oe, term_o, hex_run;
    wire out_valid;
    wire [7:0] out_data;
    wire [14:0] led; wire [6:0] seg; wire dp; wire [3:0] an;

    wire io = ~((term_oe & ~term_o) | card_low);

    iso7816_terminal #(
        .CARD_DIV(CARD_DIV), .ETU_CYCLES(ETU_CYCLES),
        .FAST_ETU_CYCLES(FAST_ETU), .RST_HOLD(8), .VCC_BITS(4),
        .GAP_ETU(256), .WDOG_BITS(28)
    ) dut (
        .clk(clk), .locked(1'b1), .clk_card(clk_card), .rst_card(rst_card),
        .vcc_en(vcc_en), .io_i(io), .io_oe(term_oe), .io_o(term_o),
        .cmd_valid(cmd_valid), .cmd_data(cmd_data), .hex_run(hex_run),
        .out_valid(out_valid), .out_data(out_data), .out_ready(out_ready),
        .led(led), .seg(seg), .dp(dp), .an(an));

    task command;
        input [7:0] c;
        begin
            cmd_data  = c;
            cmd_valid = 1'b1;
            @(posedge clk);
            cmd_valid = 1'b0;
            repeat (8) @(posedge clk);
        end
    endtask

    // **The console, modelled as one character every `CHAR_CLOCKS`.** The
    // terminal's output is only as fast as whatever drains it, and that ratio
    // against a card byte is the whole question.
    reg [7:0]  got [0:4095];
    integer    got_n = 0;
    reg [15:0] drain = 16'd0;

    always @(posedge clk) begin
        if (out_valid && out_ready) begin
            got[got_n] = out_data;
            got_n      = got_n + 1;
            out_ready  <= 1'b0;
            drain      <= CHAR_CLOCKS[15:0];
        end else if (drain != 0) begin
            drain <= drain - 16'd1;
        end else begin
            out_ready <= 1'b1;
        end
    end

    // One 8E2 character onto the contact, at a given etu in clocks.
    integer b, ones;
    task card_char;
        input [7:0] v;
        input integer etu;
        begin
            ones = 0;
            card_low = 1'b1; repeat (etu) @(posedge clk);
            for (b = 0; b < 8; b = b + 1) begin
                card_low = ~v[b];
                if (v[b]) ones = ones + 1;
                repeat (etu) @(posedge clk);
            end
            // `card_low` pulls the contact down, so the parity bit is
            // inverted like the data bits are: an odd count of ones needs a
            // parity bit of one, which means releasing the line. Getting this
            // backwards made all 24 characters arrive with bad parity, which
            // reads as a broken receiver.
            card_low = (ones % 2) ? 1'b0 : 1'b1;
            repeat (etu) @(posedge clk);
            card_low = 1'b0;
            repeat (etu * 2) @(posedge clk);     // the mandatory guard
        end
    endtask

    integer i;
    initial begin
        #20_000_000;
        $display("note: did not finish; %0d character(s) out", got_n);
        $finish;
    end

    initial begin
        repeat (40) @(posedge clk);
        command(8'h41);                          // A: activate
        repeat (ETU_CYCLES * CARD_DIV * 60) @(posedge clk);

        // **The terminal has to be at the fast rate too.** Sending bytes at
        // the fast etu while it still samples at the slow one mis-reads
        // almost all of them -- 6 of 24, with parity errors -- which looks
        // like a receive fault and is only a rate mismatch. A real sequence
        // reaches this through a PPS the card echoes; here `F` is enough.
        command(8'h46);                          // F: the fast rate
        repeat (CHAR_CLOCKS * 8) @(posedge clk);

        // A burst at the fast rate, back to back, as a card answers.
        for (i = 0; i < BURST; i = i + 1)
            card_char(i[7:0] ^ 8'h5A, FAST_ETU * CARD_DIV);

        repeat (CHAR_CLOCKS * BURST * 4) @(posedge clk);

        got_n = 0;
        command(8'h73);                          // s
        repeat (CHAR_CLOCKS * 40) @(posedge clk);

        begin : status
            reg [127:0] w;
            reg [7:0]   sub;
            reg [3:0]   nib;
            integer k;
            w = 128'd0;
            if (got_n < 32) begin
                $display("note: the status line was %0d characters, not 32", got_n);
            end else begin
                for (k = 0; k < 32; k = k + 1) begin
                    sub = (got[k] <= 8'd57) ? (got[k] - 8'd48) : (got[k] - 8'd55);
                    nib = sub[3:0];
                    w   = (w << 4) | {124'd0, nib};
                end
                $display("note: %0d bytes at the fast rate -> rx %0d, parity errors %0d, lost %0d",
                         BURST, w[127:112], w[111:96], w[7:0]);
                if (w[7:0] == 0)
                    $display("PASS: nothing was lost; the receive path keeps up");
                else
                    $display("note: %0d lost -- the gap a queue closes. See the header.",
                             w[7:0]);
            end
        end
        $finish;
    end
endmodule
