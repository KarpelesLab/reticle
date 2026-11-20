// Drives mcu_ctrl (in smartcard_mcu_ctrl.v): the vendor control records, the
// image writes, the CRC walk, the status and notices, the LOAD-while-powered
// refusal, and the USB-reset clear. Prints one PASS line, or one FAIL line per
// fault.
//
// WHAT THIS WOULD CATCH and WHAT IT WOULD NOT
//   The CRC is asserted against values computed by Python's zlib.crc32 — an
//   independent implementation, not this design's — so a wrong polynomial,
//   init or final XOR is caught. It catches a LOAD that writes the wrong
//   address, a status with the fields in the wrong order, a notice that
//   mis-packs, a LOAD that is not refused while the card is powered, and a USB
//   reset that does not clear the state. It drives the vendor byte streams
//   directly, so it does NOT exercise `usb_bulk_ep`'s packetisation or the USB
//   enumeration — those are the composite device's, left to the board.
`timescale 1ns / 1ps
module smartcard_mcu_ctrl_tb;
    reg clk = 1'b0;
    always #8 clk = ~clk;
    reg rst_n = 1'b0;
    reg usb_reset = 1'b0;
    reg running = 1'b0;

    reg [7:0] vout_data = 8'd0;
    reg       vout_valid = 1'b0;
    wire      vout_ready;

    wire [7:0] vin_data;
    wire       vin_valid, vin_commit;

    wire        img_wr;
    wire [14:0] img_wr_addr;
    wire [7:0]  img_wr_data;
    wire [14:0] crc_rd_addr;

    reg        trig_stb = 1'b0, answered_stb = 1'b0;
    reg [79:0] notice = 80'd0;
    reg        notice_stb = 1'b0;
    wire       enabled, loaded;

    // The modelled 32 KB image: written by LOAD, read by the CRC walk.
    reg [7:0] mem [0:32767];
    integer   a;
    always @(posedge clk) if (img_wr) mem[img_wr_addr] <= img_wr_data;
    reg [7:0] crc_rd_q;
    always @(posedge clk) crc_rd_q <= mem[crc_rd_addr];

    mcu_ctrl dut (
        .clk(clk), .rst_n(rst_n), .usb_reset(usb_reset), .running(running),
        .vout_data(vout_data), .vout_valid(vout_valid), .vout_ready(vout_ready),
        .vin_data(vin_data), .vin_valid(vin_valid), .vin_ready(vin_ready), .vin_commit(vin_commit),
        .img_wr(img_wr), .img_wr_addr(img_wr_addr), .img_wr_data(img_wr_data),
        .crc_rd_addr(crc_rd_addr), .crc_rd_data(crc_rd_q), .resp_busy(1'b0),
        .enabled(enabled), .loaded(loaded),
        .trig_stb(trig_stb), .answered_stb(answered_stb), .notice(notice), .notice_stb(notice_stb)
    );

    // A bulk-IN sink: capture each record, pacing like a packet in flight.
    reg [7:0] rec [0:63];
    integer   recn = 0;
    reg [7:0] rec_type; reg [15:0] rec_plen; integer nrec = 0; integer rec_bytes = 0;
    reg [3:0] vcool = 4'd0;
    wire      vin_ready = (vcool == 4'd0);
    wire      vhs = vin_valid & vin_ready;
    always @(posedge clk) begin
        if (vhs) begin
            rec[recn] = vin_data;
            if (vin_commit) begin
                rec_type  = rec[0];
                rec_plen  = {rec[1], rec[2]};
                rec_bytes = recn + 1;
                nrec      = nrec + 1;
                recn      = 0;
                vcool     = 4'd8;   // the packet is in flight
            end else begin
                recn = recn + 1;
            end
        end else if (vcool != 4'd0) begin
            vcool = vcool - 4'd1;
        end
    end

    integer errors = 0;
    task chk(input bad, input [8*80-1:0] what);
        if (bad) begin $display("FAIL: %0s", what); errors = errors + 1; end
    endtask

    // Feed one vendor-OUT byte.
    task vb(input [7:0] b);
        begin
            @(negedge clk); vout_data = b; vout_valid = 1'b1;
            @(negedge clk); vout_valid = 1'b0;
        end
    endtask

    // Wait for the record count to reach `n`, or a bounded timeout.
    task wait_rec(input integer n, input integer maxc);
        integer c;
        begin
            c = 0;
            while (nrec < n && c < maxc) begin @(posedge clk); c = c + 1; end
            if (nrec < n) begin
                $display("FAIL: timed out waiting for record %0d (have %0d)", n, nrec);
                errors = errors + 1;
            end
        end
    endtask

    function [31:0] rec_u32; input integer base;
        rec_u32 = {rec[base], rec[base+1], rec[base+2], rec[base+3]};
    endfunction

    integer i;
    initial begin
        for (a = 0; a < 32768; a = a + 1) mem[a] = 8'hFF;   // the 0xff-padded empty image

        repeat (4) @(posedge clk);
        rst_n = 1'b1;
        repeat (4) @(posedge clk);

        // ---- LOAD 8 bytes 00..07 at offset 0 (card off) ----
        // record 01 | len=0x000A | off=0x0000 | 00 01 02 03 04 05 06 07
        vb(8'h01); vb(8'h00); vb(8'h0A); vb(8'h00); vb(8'h00);
        for (i = 0; i < 8; i = i + 1) vb(i[7:0]);
        repeat (4) @(posedge clk);
        chk(!loaded, "LOAD did not set loaded");
        chk(enabled, "LOAD did not clear enabled");
        for (i = 0; i < 8; i = i + 1)
            chk(mem[i] !== i[7:0], "LOAD wrote the wrong image byte");

        // ---- ENABLE 1 ----
        vb(8'h02); vb(8'h00); vb(8'h01); vb(8'h01);
        repeat (4) @(posedge clk);
        chk(!enabled, "ENABLE 1 did not enable");

        // ---- STATUS_REQ ----
        vb(8'h03); vb(8'h00); vb(8'h00);
        wait_rec(1, 2000);
        chk(rec_type !== 8'h82, "status record tag not 0x82");
        chk(rec_plen !== 16'd10, "status payload length not 10");
        chk(rec[3] !== 8'd1, "status loaded flag wrong");
        chk(rec[4] !== 8'd1, "status enabled flag wrong");
        chk(rec_u32(5) !== 32'd0, "status answered not zero");
        chk(rec_u32(9) !== 32'd0, "status triggers not zero");

        // ---- CRC_REQ: all 0xff but the first eight bytes 00..07 ----
        vb(8'h04); vb(8'h00); vb(8'h00);
        wait_rec(2, 200000);
        chk(rec_type !== 8'h83, "crc record tag not 0x83");
        chk(rec_u32(3) !== 32'h3811_4b10, "crc of the loaded image wrong");

        // ---- the responder's counters reach status ----
        @(negedge clk); trig_stb = 1'b1; @(negedge clk); trig_stb = 1'b0;
        @(negedge clk); trig_stb = 1'b1; answered_stb = 1'b1;
        @(negedge clk); trig_stb = 1'b0; answered_stb = 1'b0;
        repeat (4) @(posedge clk);
        vb(8'h03); vb(8'h00); vb(8'h00);
        wait_rec(3, 2000);
        chk(rec_u32(5) !== 32'd1, "status answered should be 1");
        chk(rec_u32(9) !== 32'd2, "status triggers should be 2");

        // ---- a notice from the responder ----
        @(negedge clk);
        notice = {32'h0000_1234, 16'h0010, 32'h0000_0064};
        notice_stb = 1'b1;
        @(negedge clk); notice_stb = 1'b0;
        wait_rec(4, 2000);
        chk(rec_type !== 8'h81, "notice record tag not 0x81");
        chk(rec_plen !== 16'd10, "notice payload length not 10");
        chk(rec_u32(3) !== 32'h0000_1234, "notice offset wrong");
        chk({rec[7], rec[8]} !== 16'h0010, "notice length wrong");
        chk(rec_u32(9) !== 32'h0000_0064, "notice latency wrong");

        // ---- a LOAD while the card is powered is refused ----
        running = 1'b1;
        repeat (2) @(posedge clk);
        vb(8'h01); vb(8'h00); vb(8'h03); vb(8'h00); vb(8'h00); vb(8'hAA);
        wait_rec(5, 2000);
        chk(rec_type !== 8'h84, "refused LOAD did not return an ERROR");
        chk(rec[3] !== 8'h01, "refused LOAD error code not 0x01");
        chk(mem[0] !== 8'd0, "a refused LOAD tore the image");
        chk(!loaded || !enabled, "a refused LOAD changed loaded/enabled");
        running = 1'b0;

        // ---- a USB reset clears everything ----
        @(negedge clk); usb_reset = 1'b1; @(negedge clk); usb_reset = 1'b0;
        repeat (4) @(posedge clk);
        chk(loaded,  "USB reset did not clear loaded");
        chk(enabled, "USB reset did not clear enabled");
        vb(8'h03); vb(8'h00); vb(8'h00);
        wait_rec(6, 2000);
        chk(rec[3] !== 8'd0 || rec[4] !== 8'd0, "status after USB reset not cleared");
        chk(rec_u32(5) !== 32'd0 || rec_u32(9) !== 32'd0, "counters after USB reset not cleared");

        // ---- the enable persists across a DTR cycle (no USB reset) ----
        vb(8'h02); vb(8'h00); vb(8'h01); vb(8'h01);
        repeat (2) @(posedge clk);
        running = 1'b1; repeat (4) @(posedge clk);   // card on
        running = 1'b0; repeat (4) @(posedge clk);   // DTR drop
        running = 1'b1; repeat (4) @(posedge clk);   // next ATR
        chk(!enabled, "the enable did not persist across a DTR power-cycle");

        // ---- a new LOAD resets the answer/trigger counts ----
        // Make the counts non-zero, then LOAD with the card off and read them
        // back: a new image is a new session, so both start from zero.
        @(negedge clk); trig_stb = 1'b1; answered_stb = 1'b1;
        @(negedge clk); trig_stb = 1'b0; answered_stb = 1'b0;
        running = 1'b0;
        repeat (2) @(posedge clk);
        vb(8'h01); vb(8'h00); vb(8'h03); vb(8'h00); vb(8'h00); vb(8'hBB);   // LOAD 1 byte
        repeat (4) @(posedge clk);
        vb(8'h03); vb(8'h00); vb(8'h00);
        wait_rec(7, 2000);
        chk(rec_u32(5) !== 32'd0 || rec_u32(9) !== 32'd0, "a new LOAD did not reset answered/triggers");

        if (errors == 0)
            $display("PASS: upload, CRC, status, notices, the powered-LOAD refusal, the per-LOAD counter reset and the USB-reset clear all hold");
        $finish;
    end
endmodule
