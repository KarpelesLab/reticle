// crypto_console_tb — a session at the crypto console, written in Verilog.
//
// What it does
//   Types the lines a person would type and prints the whole transcript,
//   one answer a line, so that `reticle sim` on this file is a readable
//   session and `tests/ip_library.rs` can check what came out against
//   digests it computed for itself.
//
//       reticle sim testdata/fpga/cynthion/crypto_console_tb.v \
//           testdata/fpga/cynthion/crypto_console.v \
//           ip/crypto/sha256/rtl/sha256.v ip/crypto/sha256/rtl/sha256_core.v \
//           ip/crypto/chacha20/rtl/chacha20.v \
//           ip/crypto/chacha20/rtl/chacha20_core.v \
//           ip/crypto/chacha20/rtl/chacha20_qr.v
//
//   The typing is **handshaked and not timed**: `sendbyte` waits for
//   `rx_ready`, which is low for the sixty-five cycles a SHA-256 block
//   compresses in, so the run is the same whatever the cores do and a
//   testbench cannot type over a busy core. That is the arrangement
//   `examples/mos6502_monitor/tb/monitor_tb.v` uses, for the same reason.
//
//   `tx_ready` is held high throughout, which models a host that is always
//   reading. A host that is not reading is a different test and belongs
//   where the endpoint is: `tests/ip_library.rs` drives this design through
//   `usb_bulk_ep` and a transceiver model, and that is where back-pressure
//   from a real IN endpoint is exercised.
//
// What it would and would not catch
//   It catches every wrong digest, every wrong keystream byte and every
//   parsing decision this console makes, because each line's answer is
//   printed in full. It catches the empty message, the padding edge, and a
//   `K` line that disturbs the key.
//
//   It would **not** catch anything about the USB stack above it, anything
//   about the board, or anything about timing on the part — `reticle sim`
//   has no delays and no unrouted wires. `CLAUDE.md` says why that last one
//   is not optional, and `usb_crypto_console.v`'s header says what was done
//   on the part instead.
module crypto_console_tb;
    localparam integer HALF = 8;   // 16 ns, near enough 60 MHz

    reg clk = 1'b0;
    always #HALF clk = ~clk;

    reg rst_n = 1'b0;

    reg  [7:0] rx_data  = 8'd0;
    reg        rx_valid = 1'b0;
    wire       rx_ready;
    wire [7:0] tx_data;
    wire       tx_valid;
    wire       tx_commit;

    crypto_console dut (
        .clk           (clk),
        .rst_n         (rst_n),
        .rx_data       (rx_data),
        .rx_valid      (rx_valid),
        .rx_ready      (rx_ready),
        .tx_data       (tx_data),
        .tx_valid      (tx_valid),
        .tx_ready      (1'b1),
        .tx_commit     (tx_commit),
        .hash_busy     (),
        .cipher_active (),
        .saw_line      (),
        .saw_answer    ()
    );

    // Everything the console has said, printed a character at a time so a
    // line of any length comes out whole. A carriage return is dropped and
    // the line feed after it ends the line, which is what makes the
    // transcript below one answer per line.
    integer lines = 0;
    always @(posedge clk) begin
        if (rst_n && tx_valid) begin
            if (tx_data == 8'h0A) begin
                $display("");
                lines = lines + 1;
            end else if (tx_data != 8'h0D) begin
                $write("%c", tx_data);
            end
        end
    end

    // One byte in, once the console has room for it.
    task sendbyte(input [7:0] value);
        begin
            @(negedge clk);
            while (!rx_ready) @(negedge clk);
            rx_data  = value;
            rx_valid = 1'b1;
            @(negedge clk);
            rx_valid = 1'b0;
        end
    endtask

    // `text` is `n` bytes, most significant first, and a carriage return
    // after them.
    task command(input [8*72-1:0] text, input integer n);
        integer         k;
        reg [8*72-1:0]  rest;
        begin
            for (k = n; k > 0; k = k - 1) begin
                rest = text >> (8 * (k - 1));
                sendbyte(rest[7:0]);
            end
            sendbyte(8'h0D);
        end
    endtask

    // Wait until `n` answer lines in all have been printed, or give up
    // loudly rather than hanging. The budget is in clock edges and never in
    // seconds.
    task settle(input integer n);
        integer guard;
        begin
            guard = 0;
            while (lines < n && guard < 4000000) begin
                @(posedge clk);
                guard = guard + 1;
            end
            if (lines < n) begin
                $display("");
                $display("crypto_console_tb: stuck after %0d line(s), wanted %0d",
                         lines, n);
                $finish;
            end
        end
    endtask

    initial begin
        repeat (8) @(posedge clk);
        rst_n = 1'b1;

        // 1. The banner, which is the console's reset state: a person who
        //    opens the port with `cat` is told what to type.
        settle(1);

        // 2. FIPS 180-4 Appendix B.1, typed as text.
        command("H abc", 5);
        settle(2);

        // 3. The same three bytes as hex, which must give the same digest.
        command("h 616263", 8);
        settle(3);

        // 4. The empty message: `h` and nothing else. No other command here
        //    can express it.
        command("h", 1);
        settle(4);

        // 5. FIPS 180-4 Appendix B.2 is 56 bytes long, which is the padding
        //    edge: no room for the length field, so the padding spills into
        //    a second block.
        command("H abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq", 58);
        settle(5);

        // 6. Sixty-four zero bytes made on the device, which is a whole
        //    block of message and then a whole block of nothing but
        //    padding.
        command("Z 40", 4);
        settle(6);

        // 7. The counter, the nonce and the key as they come out of reset,
        //    which are RFC 8439 §2.4.2's.
        command("c", 1);
        settle(7);
        command("n", 1);
        settle(8);
        command("k", 1);
        settle(9);

        // 8. Sixty-four bytes of keystream, which RFC 8439 §2.4.2 prints.
        command("e 40", 4);
        settle(10);

        // 9. The same keystream reached the other way: `E` of sixty-four
        //    zero bytes is the keystream, because exclusive-or with zero is
        //    the identity. Eight bytes of it, so the line stays readable.
        command("E 0000000000000000", 18);
        settle(11);

        // 10. The digest of that keystream, which is the command that makes
        //     a megabyte of cipher output checkable in sixty-four digits.
        command("X 40", 4);
        settle(12);

        // 11. A key of all zeros, printed back, and then its keystream at
        //     counter zero with an all-zero nonce — RFC 8439 Appendix A.1
        //     vector #1.
        command("K 0000000000000000000000000000000000000000000000000000000000000000", 66);
        settle(13);
        command("N 000000000000000000000000", 26);
        settle(14);
        command("C 00000000", 10);
        settle(15);
        command("k", 1);
        settle(16);
        command("e 40", 4);
        settle(17);

        // 12. The things that are errors, each a different one.
        command("Q", 1);          // no such command
        settle(18);
        command("h 6", 3);        // an odd number of hex digits
        settle(19);
        command("h 6g", 4);       // not a hex digit
        settle(20);
        command("C 0000", 6);     // the wrong number of digits
        settle(21);
        command("Z", 1);          // a count with no digits
        settle(22);

        // 13. And the console still works after all of those, which is the
        //     half of error handling that is easy to get wrong. The counter
        //     was left at zero by step 11 and `C 0000` above was refused, so
        //     this is a digest and not a cipher.
        command("H abc", 5);
        settle(23);

        $finish;
    end
endmodule
