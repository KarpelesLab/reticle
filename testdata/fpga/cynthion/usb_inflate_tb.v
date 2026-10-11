// `inflate_bridge`: does the length prefix frame a stream correctly?
//
// The decompressor itself is checked against RFC 1951, RFC 1950 and
// `compcol` by `tests/ip_library.rs`, over 132 streams and four stalling
// consumers. What is new in `usb_inflate.v` is the framing around it -- two
// length bytes in, the plaintext and one status byte out -- and that is what
// this drives, with no USB in the way.
//
// The streams are zlib's own output, taken from `python3 -c "import zlib"`
// and checked back through it, so they are not this project marking its own
// homework:
//
//     7801010500faff68656c6c6f062c0215   "hello", a stored block
//     7801010500faff68656c6c6f062c02ea   the same with the check byte wrong
//
// What this would catch: a length prefix read in the wrong order, a stream
// fed with `in_last` in the wrong place, a status byte that does not follow
// the plaintext, a decoder left unable to start a second stream, and an
// error reported as success. What it would not: anything the corpus covers
// and this does not -- dynamic Huffman tables, long matches, a window that
// wraps -- and anything about USB.

`timescale 1ns / 1ps
module usb_inflate_tb;
    reg clk = 1'b0;
    always #5 clk = ~clk;
    reg rst_n = 1'b0;

    integer    gap = 0;        // clocks of `host_valid` low before a byte
    reg  [7:0] host_data  = 8'd0;
    reg        host_valid = 1'b0;
    wire       host_ready;
    wire [7:0] dev_data;
    wire       dev_valid;
    reg        dev_ready  = 1'b1;
    wire       dev_commit, feeding_o, error_o, busy_o;

    // 9 is enough for "hello" and keeps the simulation small; the board
    // builds 15, which is the only setting that decodes every legal stream.
    inflate_bridge #(.WINDOW_BITS(9)) dut (
        .clk(clk), .rst_n(rst_n),
        .host_data(host_data), .host_valid(host_valid), .host_ready(host_ready),
        .dev_data(dev_data), .dev_valid(dev_valid), .dev_ready(dev_ready),
        .dev_commit(dev_commit),
        .feeding_o(feeding_o), .error_o(error_o), .busy_o(busy_o));

    // Everything the bridge gives back, in order.
    reg [7:0] got [0:255];
    integer   got_n = 0;
    always @(posedge clk) begin
        if (dev_valid && dev_ready) begin
            got[got_n] = dev_data;
            got_n      = got_n + 1;
        end
    end

    // What actually moved, since there are no hierarchical references here.
    integer took = 0;
    always @(posedge clk) if (host_valid && host_ready) took = took + 1;
    reg feed_seen = 1'b0, busy_seen = 1'b0;
    always @(posedge clk) begin
        if (feeding_o) feed_seen <= 1'b1;
        if (busy_o)    busy_seen <= 1'b1;
    end

    // **A gap before every byte**, with `host_valid` low.
    //
    // Without one this never presented `host_valid` low while a stream still
    // had bytes to come, and so never reached the state a USB burst reaches
    // constantly. `in_last` was raised on the count alone, which tells the
    // decoder a stream has ended while its last byte is still on the way;
    // on the part that delivered four bytes of a five-byte answer and then
    // stopped, and this testbench passed.
    task put;
        input [7:0] b;
        begin
            host_valid = 1'b0;
            repeat (gap) @(posedge clk);
            host_data  = b;
            host_valid = 1'b1;
            @(posedge clk);
            while (!host_ready) @(posedge clk);
            host_valid = 1'b0;
            @(posedge clk);
        end
    endtask

    // A stream, with its two length bytes in front.
    reg [7:0] stream [0:31];
    integer   i, n;
    task send;
        begin
            put(8'd0);
            put(n[7:0]);
            for (i = 0; i < n; i = i + 1) put(stream[i]);
        end
    endtask

    task settle;
        input integer clocks;
        begin
            repeat (clocks) @(posedge clk);
        end
    endtask

    integer k;
    initial begin
        #4_000_000;
        $display("FAIL: did not finish; %0d byte(s) out", got_n);
        $finish;
    end

    initial begin
        repeat (8) @(posedge clk);
        rst_n = 1'b1;
        repeat (8) @(posedge clk);

        // ---- A stream that decodes ----
        n = 16;
        stream[0]=8'h78; stream[1]=8'h01; stream[2]=8'h01; stream[3]=8'h05;
        stream[4]=8'h00; stream[5]=8'hfa; stream[6]=8'hff; stream[7]=8'h68;
        stream[8]=8'h65; stream[9]=8'h6c; stream[10]=8'h6c; stream[11]=8'h6f;
        stream[12]=8'h06; stream[13]=8'h2c; stream[14]=8'h02; stream[15]=8'h15;
        got_n = 0;
        send;
        settle(4000);
        if (got_n != 6) begin
            $display("FAIL: wanted 5 plaintext byte(s) and a status, got %0d", got_n);
            $display("      host bytes taken %0d of 18, feeding seen %b, busy seen %b",
                     took, feed_seen, busy_seen);
            for (k = 0; k < got_n; k = k + 1) $display("   [%0d] %02x", k, got[k]);
            $finish;
        end
        if (got[0] !== 8'h68 || got[1] !== 8'h65 || got[2] !== 8'h6c
            || got[3] !== 8'h6c || got[4] !== 8'h6f) begin
            $display("FAIL: plaintext %02x%02x%02x%02x%02x, wanted 68656c6c6f",
                     got[0], got[1], got[2], got[3], got[4]);
            $finish;
        end
        if (got[5] !== 8'h00) begin
            $display("FAIL: status %02x on a good stream, wanted 00", got[5]);
            $finish;
        end
        $display("note: \"hello\" decoded back to back, status 00");

        // The same stream again, with the host dawdling between bytes.
        gap = 3;
        got_n = 0;
        send;
        settle(4000);
        if (got_n != 6 || got[5] !== 8'h00 || got[0] !== 8'h68) begin
            $display("FAIL: with gaps between bytes, %0d byte(s) and status %02x",
                     got_n, got_n > 0 ? got[got_n-1] : 8'hxx);
            $finish;
        end
        $display("note: and again with three idle clocks before every byte");
        gap = 0;

        // ---- The same stream with its check byte wrong ----
        stream[15] = 8'hea;
        got_n = 0;
        send;
        settle(4000);
        if (got_n < 1) begin
            $display("FAIL: a corrupt stream produced nothing at all");
            $finish;
        end
        // The plaintext is still delivered -- the Adler-32 is only known to
        // be wrong once every byte has gone past -- so the status is last.
        if (got[got_n-1] !== 8'h87) begin
            $display("FAIL: status %02x on a bad check, wanted 87 (80 | E_ADLER)",
                     got[got_n-1]);
            for (k = 0; k < got_n; k = k + 1) $display("   [%0d] %02x", k, got[k]);
            $finish;
        end
        $display("note: a wrong Adler-32 reported 87, and %0d byte(s) came first",
                 got_n - 1);

        // ---- And a third stream, to prove the bridge rearms ----
        stream[15] = 8'h15;
        got_n = 0;
        send;
        settle(4000);
        if (got_n != 6 || got[5] !== 8'h00 || got[0] !== 8'h68) begin
            $display("FAIL: the bridge did not take a third stream: %0d byte(s)", got_n);
            for (k = 0; k < got_n; k = k + 1) $display("   [%0d] %02x", k, got[k]);
            $finish;
        end

        // ---- A stream whose zlib header is wrong ----
        //
        // **The case the wrong check byte above does not reach.** An
        // Adler-32 is only known to be wrong once every byte has gone
        // past, so that stream is taken in full and the bridge ends in
        // `S_DRAIN`, which is where the error was read. A bad header is
        // refused on the first byte: `in_ready` drops with fourteen bytes
        // still to come, `S_FEED` waits for a byte the decompressor will
        // not accept, and there was nothing there that read `error` and no
        // timer either.
        //
        // `put` waits for `host_ready`, so this hangs on the second byte
        // and the watchdog at the top reports it. Against a part it was
        // worse than a hang: the board had to be reconfigured to answer
        // again, and because a *one-byte* bad stream reports correctly --
        // its single byte is taken before the refusal -- a corpus of 123
        // cases looked like seventy-five hard ones instead of one bug.
        stream[0] = 8'hff; stream[1] = 8'hff;
        got_n = 0;
        send;
        settle(4000);
        if (got_n != 1 || got[0] !== 8'h81) begin
            $display("FAIL: a bad zlib header gave %0d byte(s) and status %02x, wanted one 81 (80 | E_HEADER)",
                     got_n, got[0]);
            for (k = 0; k < got_n; k = k + 1) $display("   [%0d] %02x", k, got[k]);
            $finish;
        end
        $display("note: a bad zlib header reported 81 with no plaintext, and the rest of the frame was swallowed");

        // ---- And a fourth stream, which is the half that matters ----
        //
        // Reporting the error is not the fix on its own: the fourteen
        // bytes the decompressor never took are still in flight, and
        // reading them as the next frame's length is the other way to
        // lose a host. This is what says they were dropped.
        stream[0] = 8'h78; stream[1] = 8'h01;
        got_n = 0;
        send;
        settle(4000);
        if (got_n != 6 || got[5] !== 8'h00 || got[0] !== 8'h68) begin
            $display("FAIL: the bridge did not take a stream after a refused one: %0d byte(s)",
                     got_n);
            for (k = 0; k < got_n; k = k + 1) $display("   [%0d] %02x", k, got[k]);
            $finish;
        end

        // ---- A header refused *after* a byte has been taken ----
        //
        // `ff ff` above is refused on the first byte, so no byte is taken
        // in the cycle the error appears. `78 00` is a legal `CM` and
        // `CINFO` with a wrong `FCHECK`, which cannot be known until the
        // second byte -- so the refusal and a byte being taken land in
        // the same cycle, and whether `left` counts that byte decides
        // whether the swallow stops in the right place.
        //
        // It did not, and the cost was not this frame: this one reported
        // `81` and looked right. The swallow then waited for one byte
        // more than the host had left to send, ate the next frame's
        // length instead, and that frame came back as nine bytes of a
        // hundred and one with `85`. On the part it was four cases of a
        // hundred and twenty-three, all of them the ones that happened to
        // follow a refusal, which reads like four hard streams rather
        // than one off-by-one.
        stream[0] = 8'h78; stream[1] = 8'h00;
        got_n = 0;
        send;
        settle(4000);
        if (got_n != 1 || got[0] !== 8'h81) begin
            $display("FAIL: a wrong FCHECK gave %0d byte(s) and status %02x, wanted one 81",
                     got_n, got[0]);
            for (k = 0; k < got_n; k = k + 1) $display("   [%0d] %02x", k, got[k]);
            $finish;
        end

        // ---- And the frame after it, which is the assertion ----
        stream[0] = 8'h78; stream[1] = 8'h01;
        got_n = 0;
        send;
        settle(4000);
        if (got_n != 6 || got[5] !== 8'h00 || got[0] !== 8'h68) begin
            $display("FAIL: the frame after a late refusal came back as %0d byte(s), so the swallow took the wrong number",
                     got_n);
            for (k = 0; k < got_n; k = k + 1) $display("   [%0d] %02x", k, got[k]);
            $finish;
        end
        $display("note: a refusal on the second byte swallowed exactly the rest, and the next frame decoded");

        $display("PASS: a zlib stored block decoded through the length-prefix framing, a wrong check byte reported 80|E_ADLER after its plaintext, a bad header reported 80|E_HEADER with nothing before it, and a stream decoded after each of them");
        $finish;
    end
endmodule
