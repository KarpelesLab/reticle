// Drives mcu_flash_resp (in smartcard_mcu_resp.v): the SEPROXYHAL framing, the
// kind-2 trigger, the exact answer bytes read from the image, the
// end-of-flash clamp, the disabled pass-through, and the framing reset on a
// new ATR. Prints one PASS line, or one FAIL line per fault.
//
// WHAT THIS WOULD CATCH and WHAT IT WOULD NOT
//   It catches a wrong answer tag or length, a byte read from the wrong image
//   address, a missed or spurious trigger, a clamp that runs past the image, a
//   responder that drives the line while disabled, and a framer that does not
//   lock after an ATR/PPS preamble. It drives one open-drain wire's bytes as
//   clean strobes with an idealised transmit acceptor, so it does NOT exercise
//   the real iso7816_uart turnaround, the parity-error path, or the TX arbiter
//   against the host FIFO — those are the top's, and are left to the board.
`timescale 1ns / 1ps
module smartcard_mcu_resp_tb;
    localparam integer GAP = 6;   // the idealised UART takes a byte every GAP cycles

    reg clk = 1'b0;
    always #8 clk = ~clk;
    reg rst_n = 1'b0;
    reg running = 1'b0;
    reg [7:0] rx_data = 8'd0;
    reg       rx_valid = 1'b0;
    reg       enabled = 1'b1, loaded = 1'b1;

    // The image, a pattern that depends on the address so a wrong offset shows.
    reg [7:0] mem [0:32767];
    integer a;
    reg [7:0] patt;

    wire [14:0] rd_addr; wire rd_en;
    reg  [7:0]  rd_q;
    always @(posedge clk) rd_q <= mem[rd_addr];

    wire [7:0] tx_data; wire tx_valid, tx_active;
    reg  [7:0] cool = 8'd0;
    wire uart_ready = (cool == 8'd0);
    wire tx_ready = tx_active & uart_ready;
    wire hs = tx_valid & tx_ready;

    wire trig_stb, answered_stb, busy;
    wire [79:0] notice; wire notice_stb;

    mcu_flash_resp dut (
        .clk(clk), .rst_n(rst_n), .running(running),
        .rx_data(rx_data), .rx_valid(rx_valid),
        .enabled(enabled), .loaded(loaded),
        .rd_addr(rd_addr), .rd_en(rd_en), .rd_data(rd_q),
        .tx_data(tx_data), .tx_valid(tx_valid), .tx_active(tx_active), .tx_ready(tx_ready),
        .trig_stb(trig_stb), .answered_stb(answered_stb), .busy(busy),
        .notice(notice), .notice_stb(notice_stb)
    );

    // Capture the transmitted answer.
    reg [7:0] txcap [0:65540];
    integer   txn = 0;
    always @(posedge clk) begin
        if (hs) begin txcap[txn] = tx_data; txn = txn + 1; cool = GAP; end
        else if (cool != 8'd0) cool = cool - 8'd1;
    end

    // Count triggers and answers, and latch the last notice.
    integer ntrig = 0, nans = 0;
    reg [31:0] n_off; reg [15:0] n_len; reg [31:0] n_lat;
    always @(posedge clk) begin
        if (trig_stb)     ntrig = ntrig + 1;
        if (answered_stb) nans  = nans + 1;
        if (notice_stb) begin n_off = notice[79:48]; n_len = notice[47:32]; n_lat = notice[31:0]; end
    end

    integer errors = 0;
    task chk(input bad, input [8*80-1:0] what);
        if (bad) begin $display("FAIL: %0s", what); errors = errors + 1; end
    endtask

    // Feed one byte on the receive stream.
    task rxb(input [7:0] b);
        begin
            @(negedge clk); rx_data = b; rx_valid = 1'b1;
            @(negedge clk); rx_valid = 1'b0;
            @(negedge clk);   // a gap, so framing is exercised between bytes
        end
    endtask

    // A kind-2 read request: 33 00 07 LL(2 BE) 02 OFF(4 BE).
    task probe(input [15:0] ll, input [31:0] off);
        begin
            rxb(8'h33); rxb(8'h00); rxb(8'h07);
            rxb(ll[15:8]); rxb(ll[7:0]); rxb(8'h02);
            rxb(off[31:24]); rxb(off[23:16]); rxb(off[15:8]); rxb(off[7:0]);
        end
    endtask

    // The end-of-turn status 60 00 02 00 00.
    task status; begin rxb(8'h60); rxb(8'h00); rxb(8'h02); rxb(8'h00); rxb(8'h00); end endtask

    // Wait for an answer to finish (or a bounded timeout).
    task settle;
        integer g;
        begin
            for (g = 0; g < 4000; g = g + 1) @(posedge clk);
        end
    endtask

    // Check the captured answer against 12 | LL' | image[off .. off+llp].
    task expect_answer(input [31:0] off, input [15:0] llp, input [8*40-1:0] what);
        integer i; reg bad;
        begin
            bad = 1'b0;
            if (txn !== 3 + llp) begin
                $display("FAIL: %0s: sent %0d bytes, want %0d", what, txn, 3 + llp);
                errors = errors + 1; bad = 1'b1;
            end
            if (!bad) begin
                if (txcap[0] !== 8'h12) begin $display("FAIL: %0s: tag %02x not 12", what, txcap[0]); errors = errors + 1; end
                if (txcap[1] !== llp[15:8] || txcap[2] !== llp[7:0]) begin
                    $display("FAIL: %0s: length %02x%02x not %04x", what, txcap[1], txcap[2], llp);
                    errors = errors + 1;
                end
                for (i = 0; i < llp; i = i + 1)
                    if (txcap[3 + i] !== mem[off + i]) begin
                        $display("FAIL: %0s: byte %0d = %02x, want %02x (addr %05x)",
                                 what, i, txcap[3 + i], mem[off + i], off + i);
                        errors = errors + 1;
                    end
            end
        end
    endtask

    initial begin
        patt = 8'd0;
        for (a = 0; a < 32768; a = a + 1) begin mem[a] = patt; patt = patt + 8'd7; end

        repeat (4) @(posedge clk);
        rst_n = 1'b1;
        repeat (4) @(posedge clk);
        running = 1'b1;   // card out of reset
        repeat (4) @(posedge clk);

        // ---- a plain 32-byte probe ----
        txn = 0;
        probe(16'h0020, 32'h0000_0100);
        status;
        settle;
        expect_answer(32'h0000_0100, 16'h0020, "32-byte read at 0x100");
        chk(ntrig !== 1, "one trigger after the first probe");
        chk(nans  !== 1, "one answer after the first probe");
        chk(n_off !== 32'h0000_0100 || n_len !== 16'h0020, "notice off/len wrong");
        chk(n_lat === 32'd0, "notice latency not measured");

        // ---- the full read a real SE does (odd length) ----
        txn = 0;
        probe(16'h00F0, 32'h0000_0000);
        status;
        settle;
        expect_answer(32'h0000_0000, 16'h00F0, "240-byte read at 0");
        chk(nans !== 2, "two answers after the full read");

        // ---- the clamp at the end of flash ----
        txn = 0;
        probe(16'h0040, 32'h0000_7FF0);   // only 0x10 bytes remain
        status;
        settle;
        expect_answer(32'h0000_7FF0, 16'h0010, "clamped read at end of flash");
        chk(n_len !== 16'h0010, "clamped notice length wrong");

        // ---- an offset past the image: an empty answer ----
        txn = 0;
        probe(16'h0020, 32'h0000_8000);
        status;
        settle;
        expect_answer(32'h0000_8000, 16'h0000, "read entirely past the image");

        // ---- disabled: the host answers, the responder is silent ----
        enabled = 1'b0;
        txn = 0;
        probe(16'h0020, 32'h0000_0200);
        status;
        settle;
        chk(txn !== 0, "the responder drove the line while disabled");
        chk(nans !== 4, "a disabled responder counted an answer");
        chk(ntrig !== 5, "a disabled probe was not counted as a trigger");
        enabled = 1'b1;

        // ---- a 0x33 not followed by the status does not trigger ----
        txn = 0;
        probe(16'h0020, 32'h0000_0100);
        rxb(8'h65); rxb(8'h00); rxb(8'h01); rxb(8'h00);   // a display packet instead
        status;                                            // and then a status
        settle;
        chk(txn !== 0, "a kind-2 read with a packet before its status still answered");
        chk(nans !== 4, "a broken kind-2 / status pair was answered");

        // ---- framing reset on a new ATR, then lock through the preamble ----
        running = 1'b0;
        repeat (20) @(posedge clk);
        running = 1'b1;
        repeat (4) @(posedge clk);
        txn = 0;
        // ATR 3B 90 98 10 00 then PPS echo FF 10 98 77 — the preamble the host
        // strips. None of it must answer or desynchronise the framer.
        rxb(8'h3B); rxb(8'h90); rxb(8'h98); rxb(8'h10); rxb(8'h00);
        rxb(8'hFF); rxb(8'h10); rxb(8'h98); rxb(8'h77);
        chk(txn !== 0, "the ATR/PPS preamble produced an answer");
        probe(16'h0010, 32'h0000_1234);
        status;
        settle;
        expect_answer(32'h0000_1234, 16'h0010, "a probe after the ATR/PPS preamble");

        if (errors == 0)
            $display("PASS: kind-2 reads are answered from the image, clamped, silent when disabled, and reframed on each ATR");
        $finish;
    end
endmodule
