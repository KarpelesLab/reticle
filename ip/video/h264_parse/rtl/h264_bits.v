// h264_bits -- the bit reader: one NAL unit's RBSP bytes in, a 32-bit
// window onto the next bits out, any number of them consumed a cycle.
//
// The buffer
//   Eight bytes in a ring, `ring_q`, written a byte at a time at `wr_q`
//   and read from the bit position `rd_q`. `win` is the 32 bits starting
//   at `rd_q`, most significant first, which is one rotator: no shifting
//   register, so a cycle that consumes bits and one that loads a byte
//   touch different things. `avail_q` counts the buffered bits.
//
//   A byte is loaded whenever there is room for it (`avail_q <= 56`), and
//   the parser may consume up to 32 bits a cycle. So the window is full
//   whenever `win_ok` is high, which is when 32 bits are buffered **or**
//   the NAL unit's last byte is in: then the window holds everything left
//   and zeros after it, never stale bytes, so a decoder reading near the
//   end sees what 7.2's next_bits() would.
//
// One NAL unit at a time
//   Loading stops after the byte marked `in_last`, and nothing more is
//   taken until the parser says `drop`: then the buffer empties and bytes
//   are discarded up to the next one marked `in_first`. A NAL unit's bits
//   therefore never run into the next one's, and a parser that gives up
//   on a NAL unit half way through drops the rest of it with one signal.
//
// more_rbsp_data()
//   7.2: true unless the only bits left before the end of the RBSP are its
//   trailing bits, the stop bit and the zeros after it. The last byte of a
//   NAL unit is never zero (7.4.1), so the stop bit is its lowest set bit,
//   and `trail_q` -- one plus the number of zeros below it -- is how many
//   bits at the end are trailing. Before the last byte has arrived every
//   buffered bit comes before it, so the answer is yes whenever anything
//   is buffered and unknown when nothing is: `more_ok` says when `more`
//   can be believed.
module h264_bits (
    input  wire        clk,
    input  wire        rst_n,

    input  wire [7:0]  in_byte,
    input  wire        in_first,
    input  wire        in_last,
    input  wire        in_valid,
    output wire        in_ready,

    output wire [31:0] win,
    output wire        win_ok,
    output wire [6:0]  avail,
    // The NAL unit's last byte has been loaded.
    output wire        at_end,
    output wire        more,
    output wire        more_ok,
    // The next bit is the first of a byte.
    output wire        aligned,
    // The bit position within its byte, for pcm_alignment_zero_bit.
    output wire [2:0]  bitpos,

    // Bits to take this cycle, 0..32. Never more than `avail`; the
    // parser checks.
    input  wire [5:0]  consume,
    // Abandon the rest of this NAL unit and wait for the next one.
    input  wire        drop,
    // Sticky until `drop`: a consumption went past the NAL unit's last bit.
    // The parser reports it on its next state; checking before consuming
    // would put a comparison of every length against `avail` in front of
    // the window's next value.
    output wire        short
);
    reg [63:0] ring_q;
    reg [5:0]  rd_q;
    reg [2:0]  wr_q;
    reg [6:0]  avail_q;
    reg        end_q;
    // Discarding up to the next `in_first`.
    reg        seek_q;
    reg [3:0]  trail_q;
    reg        short_q;
    assign short = short_q;

    assign in_ready = drop || seek_q || (!end_q && avail_q <= 7'd56);
    wire take = in_valid && in_ready;
    // While seeking, and in the cycle a drop starts one, only a header
    // byte is kept.
    wire keep = take && ((!seek_q && !drop) || in_first);

    // The next state, computed whole so that the window can be registered
    // from it: `win` is a flip-flop output, and what reaches the parser's
    // decoders starts at a register rather than behind a 64-bit rotator.
    wire [3:0] trail_in = trail_of(in_byte);
    // The byte slots a consumption of `consume` bits passes completely:
    // from the slot of `rd_q` up to, not including, the slot of the new
    // read pointer -- at most four of them.
    wire [5:0] rd_sum = rd_q + consume;
    wire [2:0] s_from = rd_q[5:3];
    wire [2:0] s_span = rd_sum[5:3] - rd_q[5:3];
    reg  [7:0] gone;
    integer zi;
    always @* begin
        for (zi = 0; zi < 8; zi = zi + 1)
            gone[zi] = ((zi[2:0] - s_from) < s_span);
    end
    reg [63:0] ring_n;
    reg [5:0]  rd_n;
    reg [2:0]  wr_n;
    reg [6:0]  avail_n;
    reg        end_n;
    reg        seek_n;
    reg [3:0]  trail_n;
    always @* begin
        ring_n  = ring_q;
        rd_n    = rd_q;
        wr_n    = wr_q;
        avail_n = avail_q;
        end_n   = end_q;
        seek_n  = seek_q;
        trail_n = trail_q;
        if (drop) begin
            ring_n  = 64'd0;
            rd_n    = 6'd0;
            wr_n    = keep ? 3'd1 : 3'd0;
            avail_n = keep ? 7'd8 : 7'd0;
            end_n   = keep && in_last;
            seek_n  = !keep;
            if (keep) begin
                ring_n[63:56] = in_byte;
                if (in_last)
                    trail_n = trail_in;
            end
        end else begin
            rd_n    = rd_q + consume;
            // Every byte the read pointer leaves behind is zeroed, so that
            // the ring past the last byte of a NAL unit reads as zeros.
            if (gone[0]) ring_n[63:56] = 8'd0;
            if (gone[1]) ring_n[55:48] = 8'd0;
            if (gone[2]) ring_n[47:40] = 8'd0;
            if (gone[3]) ring_n[39:32] = 8'd0;
            if (gone[4]) ring_n[31:24] = 8'd0;
            if (gone[5]) ring_n[23:16] = 8'd0;
            if (gone[6]) ring_n[15:8] = 8'd0;
            if (gone[7]) ring_n[7:0] = 8'd0;
            avail_n = (end_q && {1'b0, consume} > avail_q) ? 7'd0 :
                      avail_q - {1'b0, consume} + (keep ? 7'd8 : 7'd0);
            if (keep) begin
                case (wr_q)
                    3'd0: ring_n[63:56] = in_byte;
                    3'd1: ring_n[55:48] = in_byte;
                    3'd2: ring_n[47:40] = in_byte;
                    3'd3: ring_n[39:32] = in_byte;
                    3'd4: ring_n[31:24] = in_byte;
                    3'd5: ring_n[23:16] = in_byte;
                    3'd6: ring_n[15:8]  = in_byte;
                    default: ring_n[7:0] = in_byte;
                endcase
                wr_n   = wr_q + 3'd1;
                seek_n = 1'b0;
                if (in_last) begin
                    end_n   = 1'b1;
                    trail_n = trail_in;
                end
            end
        end
    end

    // The rotator: the next ring as seen from the next `rd`. Past the bits
    // it holds are the slots consumed before, which are zeros.
    wire [63:0] rot  = (ring_n << rd_n) | ((rd_n == 6'd0) ? 64'd0 : (ring_n >> (7'd64 - {1'b0, rd_n})));
    reg  [31:0] win_q;
    assign win = win_q;
    assign win_ok  = (avail_q >= 7'd32) || end_q;
    assign avail   = avail_q;
    assign at_end  = end_q;
    assign more    = end_q ? (avail_q > {3'd0, trail_q}) : 1'b1;
    assign more_ok = end_q || (avail_q != 7'd0);
    assign aligned = (rd_q[2:0] == 3'd0);
    assign bitpos  = rd_q[2:0];

    // One plus the trailing zeros of the last byte: the stop bit and the
    // alignment bits after it.
    function [3:0] trail_of;
        input [7:0] b;
        begin
            casez (b)
                8'b???????1: trail_of = 4'd1;
                8'b??????10: trail_of = 4'd2;
                8'b?????100: trail_of = 4'd3;
                8'b????1000: trail_of = 4'd4;
                8'b???10000: trail_of = 4'd5;
                8'b??100000: trail_of = 4'd6;
                8'b?1000000: trail_of = 4'd7;
                8'b10000000: trail_of = 4'd8;
                default:     trail_of = 4'd8;
            endcase
        end
    endfunction

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            ring_q  <= 64'd0;
            rd_q    <= 6'd0;
            wr_q    <= 3'd0;
            avail_q <= 7'd0;
            end_q   <= 1'b0;
            seek_q  <= 1'b1;
            trail_q <= 4'd8;
            win_q   <= 32'd0;
            short_q <= 1'b0;
        end else begin
            ring_q  <= ring_n;
            rd_q    <= rd_n;
            wr_q    <= wr_n;
            avail_q <= avail_n;
            end_q   <= end_n;
            seek_q  <= seek_n;
            trail_q <= trail_n;
            win_q   <= rot[63:32];
            short_q <= drop ? 1'b0 : (short_q || (end_q && {1'b0, consume} > avail_q));
        end
    end
endmodule
