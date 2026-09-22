// cdc_pulse — a single-cycle pulse carried across two clock domains by a
// toggle and a full handshake.
//
// What it does
//   A one-cycle pulse on `src_pulse` (in the `src_clk` domain) produces
//   exactly one one-cycle pulse on `dst_pulse` (in the `dst_clk` domain),
//   whatever the ratio of the two frequencies, including the case where
//   `dst_clk` is much slower than `src_clk`.
//
//   The pulse is turned into a level: an accepted request flips
//   `toggle_q`. The level crosses through a two-flop synchroniser, and an
//   edge detector in the destination domain turns it back into a pulse.
//   The synchronised level is then sent *back* to the source domain
//   through a second synchroniser, so the source knows the destination
//   has seen the request; `src_busy` is high from the moment a request is
//   accepted until that acknowledgement has come home.
//
// What it does not do
//   It does not queue. A pulse presented while `src_busy` is high is
//   dropped, silently: there is no room in one toggle bit for two
//   outstanding requests. Check `src_busy` before pulsing, or gate the
//   pulse with it. The round trip is about two source clocks plus three
//   destination clocks, so the maximum pulse rate is set by the slower of
//   the two domains.
//
//   It carries no data. Pair it with a value held stable in the source
//   domain for as long as `src_busy` is high and sample that value in the
//   destination domain on `dst_pulse`, which is the usual data handshake.
//
//   Both resets are asynchronous and active low, and each belongs to its
//   own domain. Releasing one while the other is held leaves the toggle
//   and its acknowledgement disagreeing for one round trip, which shows
//   up as one spurious `dst_pulse`; reset both together, or reset the
//   destination first.
module cdc_pulse (
    input  wire src_clk,
    input  wire src_rst_n,
    // One cycle high requests a pulse; ignored while `src_busy` is high.
    input  wire src_pulse,
    // High while a request is in flight.
    output wire src_busy,

    input  wire dst_clk,
    input  wire dst_rst_n,
    // One cycle high per accepted request.
    output wire dst_pulse
);
    // Source domain: the request toggle, and the acknowledgement coming
    // back from the destination domain.
    reg  toggle_q;
    wire ack_level;

    assign src_busy = toggle_q ^ ack_level;

    always @(posedge src_clk or negedge src_rst_n) begin
        if (!src_rst_n) toggle_q <= 1'b0;
        else if (src_pulse && !src_busy) toggle_q <= ~toggle_q;
    end

    // The request level in the destination domain, and its edge detector.
    wire dst_level;
    reg  dst_level_q;

    cdc_sync #(
        .WIDTH  (1),
        .STAGES (2),
        .INIT   (0)
    ) u_req_sync (
        .clk   (dst_clk),
        .rst_n (dst_rst_n),
        .d     (toggle_q),
        .q     (dst_level)
    );

    always @(posedge dst_clk or negedge dst_rst_n) begin
        if (!dst_rst_n) dst_level_q <= 1'b0;
        else            dst_level_q <= dst_level;
    end

    assign dst_pulse = dst_level ^ dst_level_q;

    // The acknowledgement: the destination's view of the toggle, brought
    // back to the source domain.
    cdc_sync #(
        .WIDTH  (1),
        .STAGES (2),
        .INIT   (0)
    ) u_ack_sync (
        .clk   (src_clk),
        .rst_n (src_rst_n),
        .d     (dst_level),
        .q     (ack_level)
    );
endmodule
