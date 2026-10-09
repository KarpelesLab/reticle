// seph_mcu against a model secure element that plays a scripted session.
//
// Every byte the block sends is collected and parsed into events, and each
// event is checked against what the script expects at that turn. The
// model SE answers each event with the next step of its script and always
// ends with GENERAL_STATUS, as a real SE does; the script is shaped after
// a real Nano X boot (`seph_mcu.v`'s header says where that comes from).
//
// What it checks:
//
//   - SESSION_START goes out on `start`, byte for byte the parameter;
//   - nothing is sent until the SE ends its turn with a status, and then
//     exactly one event per turn;
//   - SET_TICKER_INTERVAL is obeyed: consecutive tickers are at least
//     that many milliseconds apart by their own timestamps;
//   - REQUEST_STATUS is answered with the STATUS parameter, verbatim;
//   - two BLE commands are answered with the exact bytes a real MCU sent
//     for them, including the GAP init's return parameters;
//   - a button press and its release are each reported, shifted;
//   - commands that need no answer change nothing.
//
// What it would not catch: an SE that wants something this script does
// not ask for. The script is one boot, not the protocol.
`timescale 1ns / 1ps
module seph_mcu_tb;
    localparam integer MS = 4;          // clocks per millisecond, for speed

    reg clk = 1'b0;
    always #5 clk = ~clk;
    reg rst_n = 1'b0;

    reg        start = 1'b0;
    reg  [7:0] rx_data = 8'd0;
    reg        rx_valid = 1'b0;
    wire [7:0] tx_data;
    wire       tx_valid;
    reg        tx_ready = 1'b0;
    reg  [1:0] buttons = 2'b00;
    wire       active, started;
    wire [15:0] rx_packets, tx_events;
    wire [7:0] last_rx_tag;
    wire [31:0] ms;

    seph_mcu #(.MS_CYCLES(MS), .TURN_CYCLES(6)) dut (
        .clk(clk), .rst_n(rst_n), .start(start), .stop(1'b0),
        .rx_data(rx_data), .rx_valid(rx_valid),
        .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
        .buttons(buttons), .active(active), .started(started),
        .rx_packets(rx_packets), .tx_events(tx_events),
        .last_rx_tag(last_rx_tag), .ms(ms));

    // ---- What the MCU sends, collected as it goes ----
    //
    // `tx_ready` follows a slow pattern so that the handshake is exercised
    // with gaps, as a UART gives it.
    reg [7:0] got [0:4095];
    integer   got_n = 0;
    reg [3:0] pace = 4'd0;
    always @(posedge clk) begin
        pace <= pace + 4'd1;
        tx_ready <= (pace[1:0] == 2'd0);
        if (tx_valid && tx_ready) begin
            got[got_n] = tx_data;
            got_n = got_n + 1;
        end
    end

    // ---- The SE's side ----
    task se_byte;
        input [7:0] b;
        begin
            @(posedge clk);
            rx_data  = b;
            rx_valid = 1'b1;
            @(posedge clk);
            rx_valid = 1'b0;
            repeat (3) @(posedge clk);
        end
    endtask
    task se_status;
        begin
            se_byte(8'h60); se_byte(8'h00); se_byte(8'h02); se_byte(8'h00); se_byte(8'h00);
        end
    endtask

    // ---- Reading one event back ----
    integer rd = 0;
    integer limit;
    reg [7:0] etag;
    integer elen;
    reg [7:0] e [0:63];
    task next_event;
        integer k;
        begin
            limit = 0;
            while (got_n < rd + 3 && limit < MS * 1000) begin
                @(posedge clk); limit = limit + 1;
            end
            if (got_n < rd + 3) begin
                $display("FAIL: no event from the MCU (%0d bytes so far)", got_n);
                $finish;
            end
            etag = got[rd];
            elen = {got[rd + 1], got[rd + 2]};
            while (got_n < rd + 3 + elen && limit < MS * 1000) begin
                @(posedge clk); limit = limit + 1;
            end
            if (got_n < rd + 3 + elen) begin
                $display("FAIL: event %02x cut short at %0d of %0d bytes",
                         etag, got_n - rd - 3, elen);
                $finish;
            end
            for (k = 0; k < elen; k = k + 1) e[k] = got[rd + 3 + k];
            rd = rd + 3 + elen;
        end
    endtask

    // Nothing more may arrive while the SE holds the turn.
    task expect_quiet;
        input integer cycles;
        begin
            repeat (cycles) @(posedge clk);
            if (got_n != rd) begin
                $display("FAIL: the MCU sent %0d byte(s) out of turn, first %02x",
                         got_n - rd, got[rd]);
                $finish;
            end
        end
    endtask

    localparam [511:0] SESSION = {104'd0,
        408'h010030000800030304322e323804f4d8aa4304312e313604f1308974100702020014150c090106312e31312e3005312e322e30};
    localparam [511:0] STATUS = {328'd0, 184'h15001400000009000000000000000fa050db1bffffffdb};

    integer i;
    reg [31:0] t_prev, t_now;

    task expect_ticker;
        begin
            next_event;
            if (etag !== 8'h0e || elen != 4) begin
                $display("FAIL: wanted a TICKER_EVENT, got tag %02x length %0d", etag, elen);
                $finish;
            end
            t_now = {e[0], e[1], e[2], e[3]};
        end
    endtask

    task expect_bytes;
        input [8*32-1:0] want;
        input integer n;
        input [8*16-1:0] what;
        integer k;
        begin
            next_event;
            if (3 + elen != n) begin
                $display("FAIL: %0s is %0d bytes, wanted %0d", what, 3 + elen, n);
                $finish;
            end
            if ({etag, elen[15:8], elen[7:0]} !== want[8 * n - 1 -: 24]) begin
                $display("FAIL: %0s header %02x %04x", what, etag, elen);
                $finish;
            end
            for (k = 0; k < elen; k = k + 1)
                if (e[k] !== want[8 * (n - 4 - k) +: 8]) begin
                    $display("FAIL: %0s payload byte %0d is %02x, wanted %02x",
                             what, k, e[k], want[8 * (n - 4 - k) +: 8]);
                    $finish;
                end
        end
    endtask

    initial begin
        repeat (5) @(posedge clk);
        rst_n = 1'b1;
        repeat (5) @(posedge clk);

        // ---- Session start ----
        @(posedge clk); start = 1'b1; @(posedge clk); start = 1'b0;
        next_event;
        if (etag !== 8'h01 || elen != 48) begin
            $display("FAIL: the first event is %02x, %0d bytes, not SESSION_START", etag, elen);
            $finish;
        end
        for (i = 0; i < 48; i = i + 1)
            if (e[i] !== SESSION[8 * (47 - i) +: 8]) begin
                $display("FAIL: SESSION_START byte %0d is %02x", i + 3, e[i]);
                $finish;
            end
        expect_quiet(MS * 300);    // no ticker before the SE's first status

        // The SE sets a 50 ms ticker, locks the MCU and ends its turn.
        se_byte(8'h4e); se_byte(8'h00); se_byte(8'h02); se_byte(8'h00); se_byte(8'h32);
        se_byte(8'h31); se_byte(8'h00); se_byte(8'h01); se_byte(8'h01);
        if (started) begin
            $display("FAIL: started before the SE ended its turn");
            $finish;
        end
        se_status;

        // ---- Tickers at the SE's interval ----
        expect_ticker;
        t_prev = t_now;
        if (!started) begin
            $display("FAIL: not started after the SE's first status");
            $finish;
        end
        expect_quiet(MS * 20);     // one event per turn
        se_status;
        expect_ticker;
        if (t_now - t_prev < 32'd50) begin
            $display("FAIL: tickers %0d ms apart, the SE asked for 50", t_now - t_prev);
            $finish;
        end

        // ---- A command that needs no answer, then a status request ----
        se_byte(8'h4f); se_byte(8'h00); se_byte(8'h05);
        se_byte(8'h04); se_byte(8'h01); se_byte(8'h00); se_byte(8'h01); se_byte(8'h40);
        se_byte(8'h52); se_byte(8'h00); se_byte(8'h00);
        se_status;
        expect_bytes({72'd0, STATUS[183:0]}, 23, "STATUS_EVENT");

        // ---- BLE: hci_reset, then aci_gap_init with return parameters ----
        se_byte(8'h38); se_byte(8'h00); se_byte(8'h02); se_byte(8'h0c); se_byte(8'h03);
        se_status;
        expect_bytes({176'd0, 80'h18000704_0e040103_0c00}, 10, "hci_reset reply");
        se_byte(8'h38); se_byte(8'h00); se_byte(8'h05);
        se_byte(8'hfc); se_byte(8'h8a); se_byte(8'h01); se_byte(8'h00); se_byte(8'h14);
        se_status;
        expect_bytes({128'd0, 128'h18000d04_0e0a018a_fc000500_06000800}, 16, "aci_gap_init reply");

        // ---- Buttons: press left, release ----
        buttons = 2'b01;
        se_status;
        expect_bytes({224'd0, 32'h05000102}, 4, "left pressed");
        buttons = 2'b00;
        se_status;
        expect_bytes({224'd0, 32'h05000100}, 4, "left released");

        // ---- And back to tickers ----
        se_status;
        expect_ticker;
        repeat (2) @(posedge clk);   // the counters update after the last byte

        if (rx_packets != 16'd14) begin
            $display("FAIL: %0d SE packets counted, sent 14", rx_packets);
            $finish;
        end
        if (tx_events != 16'd9) begin
            $display("FAIL: %0d events counted, sent 9", tx_events);
            $finish;
        end
        $display("PASS: SESSION_START byte for byte; nothing before the SE's status and one event per turn after; tickers at the SE's 50 ms; REQUEST_STATUS answered with STATUS; hci_reset and aci_gap_init answered as a real MCU answers them; a button press and release reported; 14 SE packets and 9 events counted");
        $finish;
    end
endmodule
