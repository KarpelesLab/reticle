// The framing and the decompressor, with no USB in sight.
//
// Split out of `usb_inflate` so that `usb_inflate_tb.v` can drive it with
// plain byte streams: simulating a ULPI transceiver to check a length prefix
// would be a great deal of machinery around a small sequencer, and the USB
// path is CHECKED separately by `usb_cdc_uart.v` on this same board.
module inflate_bridge #(
    parameter integer WINDOW_BITS = 15,
    parameter integer STALL_BITS = 22
) (
    input  wire       clk,
    input  wire       rst_n,

    // From the host: `len_hi len_lo`, then that many bytes.
    input  wire [7:0] host_data,
    input  wire       host_valid,
    output wire       host_ready,

    // To the host: the plaintext, then one status byte.
    output wire [7:0] dev_data,
    output wire       dev_valid,
    input  wire       dev_ready,
    output wire       dev_commit,

    // For the lamps, and for a testbench that wants to know why.
    output wire       feeding_o,
    output wire       error_o,
    output wire       busy_o
);
    // -----------------------------------------------------------------
    // THE SEQUENCER
    // -----------------------------------------------------------------
    localparam [2:0] S_LEN_HI = 3'd0;
    localparam [2:0] S_LEN_LO = 3'd1;
    localparam [2:0] S_FEED   = 3'd2;
    localparam [2:0] S_DRAIN  = 3'd3;
    localparam [2:0] S_STATUS = 3'd4;
    // Swallowing the rest of a frame the decompressor has already refused.
    // Without it the only way out of `S_FEED` on an error would be to
    // report at once, and the bytes still in flight would then be read as
    // the next frame's length. See `S_SINK` below.
    localparam [2:0] S_SINK   = 3'd5;

    reg [2:0]  state = S_LEN_HI;
    reg [15:0] left  = 16'd0;
    reg        start_q = 1'b0;
    reg        err_q   = 1'b0;
    // Cycles of no output before a drain is called stalled. 2**22 is 70 ms
    // at 60 MHz, far longer than any corpus stream takes and far shorter
    // than a person waiting.
    reg [STALL_BITS-1:0] stall_q = {STALL_BITS{1'b0}};
    reg        hung_q  = 1'b0;

    wire [7:0] inf_out;
    wire       inf_out_valid, inf_in_ready, inf_done, inf_error, inf_busy;
    wire [2:0] inf_code;

    // Taking a byte from the host: only in the two length states and while
    // feeding, and while feeding only when the decompressor can take it.
    wire feeding   = (state == S_FEED);
    wire sinking   = (state == S_SINK);
    wire want_len  = (state == S_LEN_HI) || (state == S_LEN_LO);
    // While sinking the bytes are taken and thrown away: `in_valid` below
    // is `feeding && host_valid`, so nothing reaches the decompressor.
    assign host_ready = want_len || (feeding && inf_in_ready) || sinking;
    wire took_host = host_valid & host_ready;

    // The last byte of the declared length carries `in_last`; a zero-length
    // stream raises it with no byte at all, which is the only way to say
    // that a stream is empty.
    // **Qualified by the byte actually being there.** `inflate.v`'s header
    // says `in_last` with `in_valid` low ends a stream with no byte at this
    // point, which is how an empty stream is expressed -- so raising it
    // whenever the count reaches one tells the decoder the stream ended
    // while the last byte is still on its way. Over USB the bytes arrive in
    // bursts, so that gap is the common case, not a rare one: on the part
    // it delivered four bytes of a five-byte answer and then never finished.
    wire last_byte = feeding && host_valid && (left == 16'd1);
    wire empty_now = (state == S_LEN_LO) && took_host && ({left[15:8], host_data} == 16'd0);

    inflate #(
        .WINDOW_BITS (WINDOW_BITS),
        .WRAPPER     (1)
    ) u_inflate (
        .clk        (clk),
        .rst_n      (rst_n),
        .start      (start_q),
        .in_byte    (host_data),
        .in_valid   (feeding && host_valid),
        .in_last    (last_byte || empty_now),
        .in_ready   (inf_in_ready),
        .out_byte   (inf_out),
        .out_valid  (inf_out_valid),
        .out_ready  (give_out),
        .done       (inf_done),
        .error      (inf_error),
        .error_code (inf_code),
        .busy       (inf_busy)
    );

    // Giving a byte to the host: the plaintext while draining, then one
    // status byte. `in_commit` with every byte rather than at the end,
    // because a host waiting for the last few bytes of a stream must not
    // wait for a packet to fill.
    // **Output flows while input is still going in.** A decompressor
    // produces as it consumes: a stored block emits its bytes as they
    // arrive, and a long match emits far more than it takes. Draining only
    // after the last input byte deadlocks -- `inflate`'s output backs up,
    // its `in_ready` drops, and the feed never finishes. That is what this
    // did, and `usb_inflate_tb.v` caught it before any part saw it.
    wire give_out = ((state == S_FEED) || (state == S_DRAIN))
                    && inf_out_valid && dev_ready;
    wire give_sts = (state == S_STATUS) && dev_ready;
    assign dev_data  = (state == S_STATUS)
                       ? (hung_q ? 8'hC0
                                 : err_q ? {1'b1, 4'd0, inf_code} : 8'h00)
                       : inf_out;
    assign dev_valid = give_out || give_sts;
    assign dev_commit = dev_valid;

    always @(posedge clk) begin
        if (!rst_n) begin
            state   <= S_LEN_HI;
            left    <= 16'd0;
            start_q <= 1'b0;
            err_q   <= 1'b0;
            hung_q  <= 1'b0;
            stall_q <= {STALL_BITS{1'b0}};
        end else begin
            start_q <= 1'b0;
            // **The stall counter runs in every state that waits on the
            // decompressor**, not only in `S_DRAIN`, which is where it
            // used to be and why a wedge got past it. A stream whose zlib
            // header is wrong is refused before the first byte is taken:
            // `inf_in_ready` goes low, `took_host` never asserts, `left`
            // never reaches one, and `S_FEED` waits for a byte the
            // decompressor will not accept. There was no timer there, so
            // the part sat in it until it was reconfigured -- and a
            // one-byte bad stream reported `81` correctly, because its
            // single byte was taken before the refusal, which is why this
            // looked like a corpus of hard cases rather than one bug.
            //
            // The two length states are excluded on purpose: waiting
            // there is the device being idle between frames, and must
            // never time out.
            if (want_len || took_host || give_out || give_sts)
                stall_q <= {STALL_BITS{1'b0}};
            else
                stall_q <= stall_q + 1'b1;
            case (state)
                S_LEN_HI:
                    if (took_host) begin
                        left[15:8] <= host_data;
                        state      <= S_LEN_LO;
                    end
                S_LEN_LO:
                    if (took_host) begin
                        left[7:0] <= host_data;
                        err_q     <= 1'b0;
                        hung_q    <= 1'b0;
                        stall_q   <= {STALL_BITS{1'b0}};
                        // A zero length is an empty stream: `in_last` has
                        // already gone out with this cycle, so the next stop
                        // is the output.
                        state     <= ({left[15:8], host_data} == 16'd0)
                                     ? S_DRAIN : S_FEED;
                    end
                S_FEED: begin
                    // The refusal has to be read here as well as in
                    // `S_DRAIN`: the decompressor can reject a stream
                    // before it has taken all of it, and the bytes it has
                    // not taken are still coming.
                    if (inf_error) begin
                        err_q <= 1'b1;
                        state <= S_SINK;
                    end else if (took_host) begin
                        left  <= left - 16'd1;
                        if (left == 16'd1) state <= S_DRAIN;
                    end
                end
                // The frame is over for the decompressor and not for the
                // host. `left` is the number of bytes it declared and this
                // half never handed over, so exactly that many are taken
                // and dropped, and only then is the status byte sent --
                // which keeps the next frame's length where the host put
                // it.
                S_SINK:
                    if (took_host) begin
                        left <= left - 16'd1;
                        if (left == 16'd1) state <= S_STATUS;
                    end
                S_DRAIN: begin
                    if (inf_error) err_q <= 1'b1;
                    // `done` is high only after the last output byte has
                    // been taken, so there is nothing left to drain.
                    if (inf_done || inf_error) state <= S_STATUS;
                end
                S_STATUS:
                    if (give_sts) begin
                        start_q <= 1'b1;      // arm the next stream
                        state   <= S_LEN_HI;
                    end
                default: state <= S_LEN_HI;
            endcase
            // **After the case, so that a timeout wins.** Whatever an arm
            // above decided, a stall is the one answer that must get out:
            // a wedged device teaches a host nothing and a status byte
            // teaches it where to look. From `S_FEED` the rest of the
            // frame still has to be swallowed first; from anywhere else
            // there is nothing in flight.
            //
            // A timeout in `S_SINK` is the one case that cannot keep the
            // framing, because it means the host stopped mid-frame. It
            // reports rather than waiting, and the host that broke the
            // framing is the one that has to resynchronise.
            if (!want_len && &stall_q) begin
                err_q  <= 1'b1;
                hung_q <= 1'b1;
                state  <= (state == S_FEED) ? S_SINK : S_STATUS;
            end
        end
    end


    assign feeding_o = (state == S_FEED);
    assign error_o   = err_q;
    assign busy_o    = inf_busy;
endmodule
