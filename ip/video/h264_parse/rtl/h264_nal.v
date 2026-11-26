// h264_nal -- Annex B byte stream in, NAL unit bytes out, with the
// emulation prevention bytes removed.
//
// What it does
//   Finds the start codes of ITU-T H.264 Annex B (B.2: a `0x000001`,
//   with any number of zero bytes before it) and gives out each NAL
//   unit's bytes with `out_first` on its header byte and `out_last` on
//   its final byte. Inside a NAL unit, `0x000003` is the
//   emulation_prevention_three_byte of 7.3.1 / 7.4.1: the `0x03` is
//   dropped and the two zeros go out as payload.
//
// Why it holds bytes back
//   A NAL unit's end is only known when the next start code arrives, and
//   zero bytes in front of a start code (trailing_zero_8bits, or the
//   zero_byte of B.1) belong to no NAL unit. So the last non-zero byte is
//   held in `p_q`, and a run of zeros after it is held as a count in
//   `z_q`, until the byte after them says what they were:
//
//   - `0x01` after two or more zeros: a start code. The held byte is the
//     last of its NAL unit; the zeros were padding and are dropped.
//   - `0x03` after exactly two zeros: emulation prevention. The held byte
//     and both zeros are payload; the `0x03` is dropped.
//   - anything else: the held byte and the zeros are payload.
//
//   7.4.1 says a NAL unit's last byte is never `0x00`, which is what makes
//   holding the zeros sound: they can only be payload if something
//   non-zero follows them inside the same NAL unit.
//
// End of stream
//   `in_last` says the byte with it is the final one, and the held byte
//   then goes out with `out_last`. Without it the final NAL unit of a
//   stream stays held until something else arrives, which a decoder fed
//   continuously never notices and a test does.
//
// Readiness
//   `in_ready` is a function of this block's registers and of
//   `out_ready`; the parser's `out_ready` is a function of its registers,
//   so the package's `in_ready` is a function of registers.
module h264_nal (
    input  wire       clk,
    input  wire       rst_n,

    input  wire [7:0] in_byte,
    input  wire       in_valid,
    input  wire       in_last,
    output wire       in_ready,

    output reg  [7:0] out_byte,
    output reg        out_first,
    output reg        out_last,
    output reg        out_valid,
    input  wire       out_ready
);
    // The held byte and whether there is one; whether it is a NAL unit's
    // header byte.
    reg [7:0] p_q;
    reg       pv_q;
    reg       pf_q;
    // Zero bytes held after it, saturating: a count past three is a run
    // of padding or an invalid stream, and both end the same way.
    reg [1:0] z_q;
    // Inside a NAL unit: the first start code has been seen.
    reg       in_nal_q;
    // Zero bytes still to give out as payload, and the byte that arrived
    // behind them, which becomes the held byte once they are out.
    reg [1:0] fz_q;
    reg [7:0] nb_q;
    reg       nbv_q;
    // The stream's last byte has been taken; give the held byte out with
    // `out_last` once the zeros are flushed.
    reg       eos_q;

    wire slot = !out_valid || out_ready;
    wire busy = (fz_q != 2'd0) || eos_q;
    assign in_ready = slot && !busy;

    wire take = in_valid && in_ready;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            p_q       <= 8'd0;
            pv_q      <= 1'b0;
            pf_q      <= 1'b0;
            z_q       <= 2'd0;
            in_nal_q  <= 1'b0;
            fz_q      <= 2'd0;
            nb_q      <= 8'd0;
            nbv_q     <= 1'b0;
            eos_q     <= 1'b0;
            out_byte  <= 8'd0;
            out_first <= 1'b0;
            out_last  <= 1'b0;
            out_valid <= 1'b0;
        end else begin
            if (out_valid && out_ready)
                out_valid <= 1'b0;

            if (fz_q != 2'd0) begin
                // Flushing held zeros as payload, one a cycle.
                if (slot) begin
                    out_byte  <= 8'd0;
                    out_first <= 1'b0;
                    out_last  <= 1'b0;
                    out_valid <= 1'b1;
                    fz_q      <= fz_q - 2'd1;
                    if (fz_q == 2'd1) begin
                        p_q   <= nb_q;
                        pv_q  <= nbv_q;
                        pf_q  <= 1'b0;
                        nbv_q <= 1'b0;
                    end
                end
            end else if (eos_q) begin
                if (slot) begin
                    if (pv_q && in_nal_q) begin
                        out_byte  <= p_q;
                        out_first <= pf_q;
                        out_last  <= 1'b1;
                        out_valid <= 1'b1;
                    end
                    pv_q     <= 1'b0;
                    pf_q     <= 1'b0;
                    z_q      <= 2'd0;
                    in_nal_q <= 1'b0;
                    eos_q    <= 1'b0;
                end
            end else if (take) begin
                eos_q <= in_last;
                if (in_byte == 8'h00) begin
                    if (z_q != 2'd3)
                        z_q <= z_q + 2'd1;
                end else if (in_byte == 8'h01 && z_q >= 2'd2) begin
                    // A start code. The held byte ends its NAL unit.
                    if (pv_q && in_nal_q) begin
                        out_byte  <= p_q;
                        out_first <= pf_q;
                        out_last  <= 1'b1;
                        out_valid <= 1'b1;
                    end
                    pv_q     <= 1'b0;
                    pf_q     <= 1'b0;
                    z_q      <= 2'd0;
                    in_nal_q <= 1'b1;
                    // The next byte is a header byte: mark it through nb.
                    nbv_q    <= 1'b1;
                end else begin
                    // Payload: the held byte, then the zeros, then this
                    // byte becomes the held one -- except a 0x03 after
                    // two zeros, which is dropped.
                    if (pv_q && in_nal_q) begin
                        out_byte  <= p_q;
                        out_first <= pf_q;
                        out_last  <= 1'b0;
                        out_valid <= 1'b1;
                    end
                    if (in_byte == 8'h03 && z_q == 2'd2) begin
                        fz_q  <= in_nal_q ? 2'd2 : 2'd0;
                        nbv_q <= 1'b0;
                        pv_q  <= 1'b0;
                        pf_q  <= 1'b0;
                    end else if (z_q != 2'd0 && in_nal_q) begin
                        fz_q  <= (z_q == 2'd3) ? 2'd3 : z_q;
                        nb_q  <= in_byte;
                        nbv_q <= 1'b1;
                        pv_q  <= 1'b0;
                        pf_q  <= 1'b0;
                    end else begin
                        p_q   <= in_byte;
                        pv_q  <= in_nal_q;
                        // The first byte after a start code is the header.
                        pf_q  <= nbv_q;
                        nbv_q <= 1'b0;
                    end
                    z_q <= 2'd0;
                end
            end
        end
    end
endmodule
