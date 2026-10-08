// Does a variable part-select of a wide vector read the same on a part as it
// does in simulation?
//
// **Why this exists.** A receive queue for `iso7816_terminal.v` was written
// twice and reverted twice. The second version was one packed vector pushed
// by a fixed shift and read with `q[head*W +: W]` -- flip-flops and a
// multiplexer, nothing inferred, no memory. It passed `iso7816_terminal_tb.v`
// and `iso7816_burst_tb.v` (which drops 14 of 24 bytes without it and none
// with it), and then on a Basys 3 the card half reported no bytes at all,
// three runs out of three. Simulation and silicon disagree about a construct
// made of parts this flow is supposed to build well, and the one unusual
// thing in it is the variable part-select.
//
// So this isolates it. A 320-bit constant is read two ways at the same index:
// once with `pat[idx*W +: W]`, once through a `case` over every index. Both
// are pure combinational logic over the same bits, so they must agree; a
// mismatch is latched and counted, and the count is reported over the serial
// port. `0000` exonerates the construct and sends the queue hunt elsewhere;
// anything else is a backend defect with a one-page reproducer.
//
// Reported as eight hex characters, repeating: the mismatch count, then the
// index that last disagreed.
//
// **CHECKED on a Basys 3: 0000 mismatches, indexing a register vector.** So
// the construct is built
// correctly and the queue's failure is something else; this file stays as the
// thing that eliminated it, and as a pattern for the next such question.
//
// **It took three tries to get a trustworthy answer, and each wrong one was
// this test's fault, not the fabric's.** In order: a constant vector, which
// folds `pat[idx*W +: W]` into ten lookup tables of `idx` and never builds
// the multiplexer in question at all; then two combinational trees compared
// directly, which reported 10944 mismatches at about one per 7450 clocks;
// then a comparison enabled one cycle before the registers it compares were
// valid, worth exactly one mismatch at the last fill index.
//
// **The shape of each number said so before any further work.** A wrong
// 32:1 multiplexer fails every time its index comes round -- a million times
// per report here. A rate unrelated to the index is a race; a count of
// exactly one is a boundary. Thirty seconds of arithmetic on the figure beat
// a two-minute build each time.
//
// It also nearly reported a defect that does not exist. The first version
// compared the two combinational results directly and latched the
// difference, and that reported 10944 mismatches -- about one per 7450
// clocks. **The rate is what gave it away**: a wrong multiplexer fails every
// time its index comes round, which here would be a million times per
// report, not one in 7450. An intermittent rate unrelated to the thing being
// indexed can only be a race, and it was: whichever tree settled later made
// the comparison briefly true. Registering both sides first took it to zero.
// Before blaming a construct, check that the rate of failure matches the
// mechanism being blamed.

module partsel #(
    parameter CLK_DIV = 50          // 100 MHz / 50 = 2.000 Mbaud
) (
    input  wire        clk,
    output wire        uart_tx_pin,
    input  wire        uart_rx_pin,
    output wire [14:0] led
);
    localparam integer W = 10;
    localparam integer N = 32;

    // **No PLL.** The board's 100 MHz pin divides exactly for the console
    // (100 / 50 = 2.000 Mbaud), and a PLL here would only add a way for this
    // test to fail for a reason that is not the one under test -- which it
    // did: with one, nothing came out at all.
    wire sys = clk;

    reg [15:0] por = 16'd0;
    always @(posedge sys) por <= {por[14:0], 1'b1};
    wire rst_n = por[15];

    // A pattern with no repeats, so a wrong index cannot look right -- and
    // built without arithmetic, because `i * 41 + 21` and `31 - k` are adders
    // and the placer cannot always fit their carry groups:
    //
    //     entry(i) = {i, ~i}
    //
    // Ten bits, distinct for every `i`, and no two entries share a half.
    function [W-1:0] entry;
        input [4:0] i;
        entry = {i, ~i};
    endfunction

    // **A register vector, not a constant.** The first version of this test
    // used a constant here, and `pat[idx*W +: W]` of a constant folds into
    // about ten lookup tables of `idx` -- it never builds the multiplexer the
    // thing under test actually uses. A queue indexes 320 live flip-flops,
    // which is ten 32:1 multiplexers over registers, so that is what this
    // holds now: the same shift that a queue's push performs, loading the
    // same pattern.
    reg [N*W-1:0] pat = {(N*W){1'b0}};
    reg [5:0]     fill = 6'd0;
    always @(posedge sys) begin
        if (!rst_n) begin
            pat  <= {(N*W){1'b0}};
            fill <= 6'd0;
        end else if (fill != N) begin
            // Entries enter at offset 0 and shift up, exactly as a queue's
            // push does, so entry `i` ends at offset `N-1-i`.
            pat  <= {pat[(N-1)*W-1:0], entry(fill[4:0])};
            fill <= fill + 6'd1;
        end
    end

    // What the shift above leaves at each offset, once `fill` reaches N:
    // offset k holds `entry(N-1-k)`.
    // Offset k holds `entry(N-1-k)`, and for five bits `31 - k` is `~k`, so
    // this needs no subtractor either.
    function [W-1:0] expect_at;
        input [4:0] k;
        expect_at = entry(~k);
    endfunction

    reg [4:0] idx = 5'd0;
    always @(posedge sys) if (rst_n) idx <= idx + 5'd1;

    // The construct under test.
    wire [W-1:0] got = pat[idx*W +: W];
    // And the same bits, selected without arithmetic on the offset.
    reg  [W-1:0] want;
    integer k;
    always @(*) begin
        want = {W{1'b0}};
        for (k = 0; k < N; k = k + 1)
            if (idx == k[4:0]) want = pat[k*W +: W];
    end

    // And a third opinion that does not read `pat` at all, so a multiplexer
    // that is wrong in both readings cannot pass: what the shift must have
    // left there.
    wire [W-1:0] pure = expect_at(idx);

    // **Delayed by as many cycles as the comparison is.** `fill` reaching N
    // means the vector is loaded *now*, but `got_q` and `want_q` hold the
    // previous cycle's values, so comparing on the first `loaded` cycle
    // compares a reading taken while the vector was still filling. That is
    // worth exactly one mismatch, at the last fill index -- which is what it
    // reported, and the count being precisely 1 is what gave it away.
    wire       loaded_now = (fill == N);
    reg  [2:0] loaded_sr  = 3'd0;
    always @(posedge sys) begin
        if (!rst_n) loaded_sr <= 3'd0;
        else        loaded_sr <= {loaded_sr[1:0], loaded_now};
    end
    wire loaded = loaded_sr[2];

    // **Both sides are registered before they are compared.**
    //
    // Comparing two combinational mux trees and latching the difference is
    // itself setup-sensitive: whichever settles later makes the comparison
    // momentarily true, and a flip-flop that samples it counts a mismatch
    // that is not one. An earlier version of this did exactly that and
    // reported about one mismatch per 7450 clocks -- not one per occurrence
    // of an index, which is what a wrong multiplexer would give, so the rate
    // itself said the test was at fault. Registering both and comparing them
    // a cycle later removes the race and leaves only a real disagreement.
    reg [W-1:0] got_q  = {W{1'b0}};
    reg [W-1:0] want_q = {W{1'b0}};
    reg [W-1:0] pure_q = {W{1'b0}};
    reg [15:0]  bad_q  = 16'd0;
    reg [15:0]  last_q = 16'd0;
    reg [4:0]   idx_q  = 5'd0;

    always @(posedge sys) begin
        if (!rst_n) begin
            got_q  <= {W{1'b0}};
            want_q <= {W{1'b0}};
            pure_q <= {W{1'b0}};
            bad_q  <= 16'd0;
            last_q <= 16'd0;
            idx_q  <= 5'd0;
        end else begin
            got_q  <= got;
            want_q <= want;
            pure_q <= pure;
            idx_q  <= idx;
            if (loaded && (got_q != want_q || got_q != pure_q)) begin
                if (bad_q != 16'hFFFF) bad_q <= bad_q + 16'd1;
                last_q <= {11'd0, idx_q};
            end
        end
    end

    assign led = {bad_q != 16'd0, last_q[4:0], bad_q[8:0]};

    // ---- The report ----
    reg [31:0] word     = 32'd0;
    reg [3:0]  left     = 4'd0;
    reg [2:0]  pos      = 3'd0;
    reg [7:0]  tx_data  = 8'd0;
    reg        tx_valid = 1'b0;
    wire       tx_ready;
    reg [25:0] wait_q   = 26'd0;

    function [7:0] hex;
        input [3:0] n;
        hex = (n < 4'd10) ? (8'd48 + {4'd0, n}) : (8'd55 + {4'd0, n});
    endfunction

    always @(posedge sys) begin
        if (!rst_n) begin
            left <= 4'd0; pos <= 3'd0; tx_valid <= 1'b0; wait_q <= 26'd0;
        end else if (!tx_valid) begin
            if (left != 0) begin
                tx_data  <= hex(word[31:28]);
                tx_valid <= 1'b1;
            end else if (pos != 0) begin
                tx_data  <= (pos == 3'd1) ? 8'd13 : 8'd10;
                tx_valid <= 1'b1;
            end else if (wait_q == 0) begin
                word   <= {bad_q, last_q};
                left   <= 4'd8;
                wait_q <= {1'b1, {25{1'b0}}};
            end else begin
                wait_q <= wait_q - 26'd1;
            end
        end else if (tx_ready) begin
            tx_valid <= 1'b0;
            if (left != 0) begin
                word <= {word[27:0], 4'd0};
                left <= left - 4'd1;
                if (left == 4'd1) pos <= 3'd1;
            end else if (pos == 3'd1) begin
                pos <= 3'd2;
            end else begin
                pos <= 3'd0;
            end
        end
    end

    uart #(.CLK_DIV(CLK_DIV)) port (
        .clk(sys), .rst_n(rst_n), .div(16'd0),
        .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
        .tx(uart_tx_pin),
        .rx(uart_rx_pin),
        .rx_data(), .rx_valid(), .rx_error(), .rx_frame_error(),
        .rx_parity_error(), .rx_break());
endmodule
