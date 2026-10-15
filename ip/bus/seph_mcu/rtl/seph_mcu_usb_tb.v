// seph_mcu's USB: the cable, the bus reset, IN completions and injected
// transfers, against a model SE whose USB packets are the capture's.
//
// Every SE packet below and every event checked against is from a real
// Nano X enumerating on a real host (`visgrok`,
// `capture-20261009-125946.seph.log`, around 21.22 s and 558.07 s): its
// USB_CONFIG and USB_EP_PREPARE packets, and the MCU's USB_EVENT reset and
// USB_EP_XFER_EVENTs. The host's SETUP and OUT transfers go in through
// `inj_*`, as the console's `!` sends them.
//
// What it checks:
//
//   - no bus reset while the cable is out, nor while it is in and the SE
//     has not connected; one reset when both are true, and none after;
//   - STATUS's low flag byte: 0x09 with the cable in, 0x0b after the
//     reset, 0x00 with it out again -- and a second reset when it goes
//     back in;
//   - an injected SETUP and a 70-byte OUT transfer go out byte for byte;
//   - each IN the SE prepares is completed as the real MCU completed it:
//     endpoint 0 with the length, endpoint 2 with length 0;
//   - an injection that is malformed, or that starts while another is
//     waiting, is refused with `inj_bad` and sends nothing, and the one
//     waiting still goes out whole.
//
// What it would not catch: anything a real host does that the capture's
// does not, and timing -- a real host reads an IN when it likes, and this
// block completes it on the next turn.
`timescale 1ns / 1ps
module seph_mcu_usb_tb;
    localparam integer MS = 4;

    reg clk = 1'b0;
    always #5 clk = ~clk;
    reg rst_n = 1'b0;

    reg        start = 1'b0;
    reg  [7:0] rx_data = 8'd0;
    reg        rx_valid = 1'b0;
    wire [7:0] tx_data;
    wire       tx_valid;
    reg        usb_plugged = 1'b0;
    wire       usb_on;
    reg        inj_start = 1'b0, inj_valid = 1'b0, inj_commit = 1'b0;
    reg  [7:0] inj_data = 8'd0;
    wire       inj_busy, inj_bad;
    wire       active, started;
    wire [15:0] rx_packets, tx_events;
    wire [7:0] last_rx_tag;
    wire [31:0] ms;

    seph_mcu #(.MS_CYCLES(MS), .TURN_CYCLES(6)) dut (
        .clk(clk), .rst_n(rst_n), .start(start), .stop(1'b0),
        .rx_data(rx_data), .rx_valid(rx_valid),
        .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(1'b1),
        .buttons(2'b00),
        .usb_plugged(usb_plugged), .usb_on(usb_on),
        .inj_start(inj_start), .inj_data(inj_data), .inj_valid(inj_valid),
        .inj_commit(inj_commit), .inj_busy(inj_busy), .inj_bad(inj_bad),
        .active(active), .started(started),
        .rx_packets(rx_packets), .tx_events(tx_events),
        .last_rx_tag(last_rx_tag), .ms(ms));

    // What the MCU sends, queued as it goes.
    reg [7:0] q [0:4095];
    integer   q_w = 0, q_r = 0;
    always @(posedge clk)
        if (tx_valid) begin
            q[q_w % 4096] = tx_data;
            q_w = q_w + 1;
        end

    // `inj_bad` is a pulse; remember that one happened.
    integer bads = 0;
    always @(posedge clk) if (inj_bad) bads = bads + 1;

    reg [7:0] b;
    integer waited;
    task mcu_byte;
        begin
            waited = 0;
            while (q_r >= q_w) begin
                @(posedge clk);
                waited = waited + 1;
                if (waited > 200000) begin
                    $display("FAIL: the MCU stopped sending (%0s)", step);
                    $finish;
                end
            end
            b   = q[q_r % 4096];
            q_r = q_r + 1;
        end
    endtask

    task se_byte;
        input [7:0] v;
        begin
            rx_data  = v;
            rx_valid = 1'b1;
            @(posedge clk);
            rx_valid = 1'b0;
            repeat (3) @(posedge clk);
        end
    endtask

    // A packet from a hex string, most significant byte first.
    integer k;
    task se_packet;
        input [8*80-1:0] p;
        input integer n;
        begin
            for (k = n - 1; k >= 0; k = k - 1)
                se_byte(p[8 * k +: 8]);
        end
    endtask

    task se_status;
        se_packet({8'h60, 8'h00, 8'h02, 8'h00, 8'h00}, 5);
    endtask

    // One event into `ev`, length in `ev_n`.
    reg [7:0] ev [0:255];
    integer   ev_n, i, len;
    task mcu_event;
        begin
            mcu_byte; ev[0] = b;
            mcu_byte; ev[1] = b; mcu_byte; ev[2] = b;
            len = ev[1] * 256 + ev[2];
            for (i = 0; i < len; i = i + 1) begin
                mcu_byte; ev[3 + i] = b;
            end
            ev_n = len + 3;
        end
    endtask

    // The next event that is not a ticker; tickers get a bare status.
    // Bounded: an event that never comes is a FAIL naming the step, not a
    // simulation that never ends.
    integer tickers;
    task next_non_ticker;
        begin
            mcu_event;
            tickers = 0;
            while (ev[0] == 8'h0e) begin
                tickers = tickers + 1;
                if (tickers > 50) begin
                    $display("FAIL: %0s: only tickers, 50 of them", step);
                    $finish;
                end
                se_status;
                mcu_event;
            end
        end
    endtask

    reg [8*48-1:0] step;
    task expect_event;
        input [8*80-1:0] want;
        input integer n;
        begin
            next_non_ticker;
            if (ev_n != n) begin
                $display("FAIL: %0s: an event of %0d bytes, tag %02x; wanted %0d bytes",
                         step, ev_n, ev[0], n);
                $finish;
            end
            for (i = 0; i < n; i = i + 1)
                if (ev[i] !== want[8 * (n - 1 - i) +: 8]) begin
                    $display("FAIL: %0s: byte %0d is %02x, wanted %02x",
                             step, i, ev[i], want[8 * (n - 1 - i) +: 8]);
                    $finish;
                end
        end
    endtask

    // Ask for the status and check its low flag byte.
    task expect_flags;
        input [7:0] want;
        begin
            se_packet({8'h52, 8'h00, 8'h00}, 3);
            se_status;
            next_non_ticker;
            if (ev[0] !== 8'h15 || ev[6] !== want) begin
                $display("FAIL: %0s: status tag %02x flags %02x, wanted 15 and %02x",
                         step, ev[0], ev[6], want);
                $finish;
            end
        end
    endtask

    // Nothing but tickers for a while: no reset, no completion.
    task expect_only_tickers;
        input integer turns;
        integer t;
        begin
            for (t = 0; t < turns; t = t + 1) begin
                se_status;
                mcu_event;
                if (ev[0] !== 8'h0e) begin
                    $display("FAIL: %0s: the MCU sent tag %02x, wanted only tickers",
                             step, ev[0]);
                    $finish;
                end
            end
        end
    endtask

    // An injected event, as the console's `!` run delivers it.
    task inject;
        input [8*80-1:0] p;
        input integer n;
        begin
            @(posedge clk); inj_start = 1'b1;
            @(posedge clk); inj_start = 1'b0;
            for (k = n - 1; k >= 0; k = k - 1) begin
                inj_data = p[8 * k +: 8]; inj_valid = 1'b1;
                @(posedge clk); inj_valid = 1'b0;
                @(posedge clk);
            end
            inj_commit = 1'b1;
            @(posedge clk); inj_commit = 1'b0;
            repeat (2) @(posedge clk);
        end
    endtask

    // A 70-byte OUT on endpoint 2: GET_APP_NAME_AND_VERSION on channel
    // 0x09dd, padded to 64 bytes, as the capture's host sent it.
    localparam [8*70-1:0] OUT_EP2 = {
        48'h1000_4382_0440, 64'h09dd_0500_0000_05b0, 32'h0100_0000, 416'd0};
    localparam [8*70-1:0] IN_EP2 = {
        48'h5000_4382_2040, 64'h09dd_0500_0000_0255, 8'h15, 440'd0};

    integer bads_before;
    initial begin
        repeat (4) @(posedge clk);
        rst_n = 1'b1;
        @(posedge clk);
        start = 1'b1;
        @(posedge clk);
        start = 1'b0;
        step = "session start";
        mcu_event;
        if (ev[0] !== 8'h01) begin
            $display("FAIL: the first event was %02x, not SESSION_START", ev[0]);
            $finish;
        end

        step = "cable out";
        expect_only_tickers(2);
        expect_flags(8'h00);

        step = "cable in, SE not connected";
        usb_plugged = 1'b1;
        expect_only_tickers(2);
        expect_flags(8'h09);

        step = "SE connects";
        se_packet({8'h4f, 8'h00, 8'h01, 8'h01}, 4);
        se_status;
        expect_event({8'h0f, 8'h00, 8'h01, 8'h01}, 4);
        se_packet(64'h4f00_0504_0100_0140, 8);
        se_packet(64'h4f00_0504_0180_0140, 8);
        step = "after the reset";
        expect_flags(8'h0b);
        expect_only_tickers(2);

        step = "SET_ADDRESS";
        inject(112'h1000_0b00_0108_0005_0100_0000_0000, 14);
        se_status;
        expect_event(112'h1000_0b00_0108_0005_0100_0000_0000, 14);
        se_packet(40'h4f00_0203_01, 5);
        se_packet(48'h5000_0300_2000, 6);
        se_status;
        expect_event(48'h1000_0380_0200, 6);

        step = "GET_DESCRIPTOR";
        inject(112'h1000_0b00_0108_8006_0001_0000_0800, 14);
        se_status;
        expect_event(112'h1000_0b00_0108_8006_0001_0000_0800, 14);
        se_packet(112'h5000_0b00_2008_1201_1002_0000_0040, 14);
        se_packet(48'h5000_0300_3000, 6);
        se_status;
        expect_event(48'h1000_0380_0208, 6);

        step = "an APDU over endpoint 2";
        inject(OUT_EP2, 70);
        se_status;
        expect_event(OUT_EP2, 70);
        se_packet(48'h5000_0302_3040, 6);
        se_packet(IN_EP2, 70);
        se_status;
        expect_event(48'h1000_0382_0200, 6);

        step = "a malformed injection";
        bads_before = bads;
        inject(48'h1000_0480_0200, 6);          // says 4 bytes, carries 3
        if (bads != bads_before + 1) begin
            $display("FAIL: a malformed injection was not refused");
            $finish;
        end
        expect_only_tickers(2);

        step = "an injection while one waits";
        inject(112'h1000_0b00_0108_8006_0001_0000_1200, 14);
        bads_before = bads;
        inject(48'h1000_0380_0200, 6);
        if (bads != bads_before + 1) begin
            $display("FAIL: an injection while another waited was not refused");
            $finish;
        end
        se_status;
        expect_event(112'h1000_0b00_0108_8006_0001_0000_1200, 14);

        step = "cable out again";
        usb_plugged = 1'b0;
        expect_flags(8'h00);
        expect_only_tickers(2);
        step = "cable back in";
        usb_plugged = 1'b1;
        se_status;
        expect_event({8'h0f, 8'h00, 8'h01, 8'h01}, 4);

        $display("PASS: no reset without the cable and the SE connected, one when both; status flags 00, 09, 0b, 00 as the cable went in, the bus reset and the cable went out, and a reset again when it came back; SETUP, GET_DESCRIPTOR and a 70-byte OUT injected byte for byte; IN completions as the real MCU sent them, endpoint 0 with its length and endpoint 2 with 0; a malformed injection and one made while another waited refused, and the waiting one sent whole");
        $finish;
    end
endmodule
