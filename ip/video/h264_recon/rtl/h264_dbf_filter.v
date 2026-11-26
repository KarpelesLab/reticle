// h264_dbf_filter — one line of eight samples across one edge, the
// filters of ITU-T H.264 §8.7.2.3 (bS < 4) and §8.7.2.4 (bS = 4).
//
// What it does
//   Given p3..p0 | q0..q3 — the eight samples across a 4x4 block edge,
//   p0 and q0 touching it — the edge's boundary strength and the three
//   thresholds §8.7.2.2 derives from the quantiser, gives back the six
//   samples the filter may change. It is combinational and holds no
//   state: `h264_deblock` puts a register on each side of it and feeds it
//   a line a cycle.
//
//   `chroma` selects chromaStyleFilteringFlag (8-450), which for the
//   4:2:0 pictures this package decodes is chromaEdgeFlag itself: a
//   chroma line reads only p1, p0, q0 and q1, never changes p1 or q1,
//   and for bS = 4 takes the three-tap filter on both sides.
//
// The thresholds
//   `alpha`, `beta` and `tc0` are Table 8-16's α′, β′ and Table 8-17's
//   t′C0, looked up by `h264_dbf_tables`. At eight bits of depth the
//   scaling of 8-456 to 8-462 is a multiplication by one, so they arrive
//   as the tables give them. `tc0` for a bS of 4 is not used.
//
// Arithmetic
//   Every quantity is carried in an eleven-bit signed word, which holds
//   the widest intermediate exactly: 4 * (q0 - p0) + (p1 - q1) + 4 is
//   at most 4 * 255 + 255 + 4 = 1279. The shifts are arithmetic, which
//   is what the specification's `>>` means for a negative operand
//   (§5.7: "arithmetic right shift of a two's complement integer").
module h264_dbf_filter (
    input  wire [7:0] p3,
    input  wire [7:0] p2,
    input  wire [7:0] p1,
    input  wire [7:0] p0,
    input  wire [7:0] q0,
    input  wire [7:0] q1,
    input  wire [7:0] q2,
    input  wire [7:0] q3,
    input  wire [2:0] bs,
    input  wire       chroma,
    input  wire [7:0] alpha,
    input  wire [4:0] beta,
    input  wire [4:0] tc0,
    output wire [7:0] p2o,
    output wire [7:0] p1o,
    output wire [7:0] p0o,
    output wire [7:0] q0o,
    output wire [7:0] q1o,
    output wire [7:0] q2o
);
    // Eleven-bit signed copies of the samples.
    wire signed [10:0] sp2 = {3'b000, p2};
    wire signed [10:0] sp1 = {3'b000, p1};
    wire signed [10:0] sp0 = {3'b000, p0};
    wire signed [10:0] sq0 = {3'b000, q0};
    wire signed [10:0] sq1 = {3'b000, q1};
    wire signed [10:0] sq2 = {3'b000, q2};

    // Absolute differences, eight bits each.
    wire [7:0] d_p0q0 = (p0 > q0) ? (p0 - q0) : (q0 - p0);
    wire [7:0] d_p1p0 = (p1 > p0) ? (p1 - p0) : (p0 - p1);
    wire [7:0] d_q1q0 = (q1 > q0) ? (q1 - q0) : (q0 - q1);
    wire [7:0] ap     = (p2 > p0) ? (p2 - p0) : (p0 - p2);     // 8-463
    wire [7:0] aq     = (q2 > q0) ? (q2 - q0) : (q0 - q2);     // 8-464

    wire [7:0] beta8 = {3'b000, beta};

    // 8-460. A bS of zero never filters.
    wire filter = (bs != 3'd0) && (d_p0q0 < alpha) &&
                  (d_p1p0 < beta8) && (d_q1q0 < beta8);

    wire ap_small = (ap < beta8);
    wire aq_small = (aq < beta8);

    // -----------------------------------------------------------------
    // bS < 4, §8.7.2.3
    // -----------------------------------------------------------------
    // 8-465 / 8-466. At most 25 + 2 = 27.
    wire [5:0] tc_l = {1'b0, tc0} + {5'd0, ap_small} + {5'd0, aq_small};
    wire [5:0] tc_c = {1'b0, tc0} + 6'd1;
    wire [5:0] tc   = chroma ? tc_c : tc_l;
    wire signed [10:0] tcs  = {5'b00000, tc};
    wire signed [10:0] tc0s = {6'b000000, tc0};

    // 8-467.
    wire signed [10:0] dsum  = ((sq0 - sp0) <<< 2) + (sp1 - sq1) + 11'sd4;
    wire signed [10:0] draw  = dsum >>> 3;
    wire signed [10:0] delta = (draw < -tcs) ? -tcs : ((draw > tcs) ? tcs : draw);

    // 8-468 / 8-469, with Clip1.
    wire signed [10:0] p0d = sp0 + delta;
    wire signed [10:0] q0d = sq0 - delta;
    wire [7:0] p0_weak = p0d[10] ? 8'd0 : ((p0d > 11'sd255) ? 8'd255 : p0d[7:0]);
    wire [7:0] q0_weak = q0d[10] ? 8'd0 : ((q0d > 11'sd255) ? 8'd255 : q0d[7:0]);

    // 8-470 / 8-472: luma only, and only on a side whose a is small.
    wire signed [10:0] avg_pq = (sp0 + sq0 + 11'sd1) >>> 1;
    wire signed [10:0] p1raw  = (sp2 + avg_pq - (sp1 <<< 1)) >>> 1;
    wire signed [10:0] q1raw  = (sq2 + avg_pq - (sq1 <<< 1)) >>> 1;
    wire signed [10:0] p1clip = (p1raw < -tc0s) ? -tc0s : ((p1raw > tc0s) ? tc0s : p1raw);
    wire signed [10:0] q1clip = (q1raw < -tc0s) ? -tc0s : ((q1raw > tc0s) ? tc0s : q1raw);
    // 8-470 adds without a Clip1, and needs none: p1 + p1clip lies
    // between p1 and (p2 + avg_pq) / 2, both samples, so the low eight
    // bits are the whole answer. Likewise q1.
    wire signed [10:0] p1d = sp1 + p1clip;
    wire signed [10:0] q1d = sq1 + q1clip;
    wire [7:0] p1_weak = (!chroma && ap_small) ? p1d[7:0] : p1;
    wire [7:0] q1_weak = (!chroma && aq_small) ? q1d[7:0] : q1;

    // -----------------------------------------------------------------
    // bS = 4, §8.7.2.4
    // -----------------------------------------------------------------
    // 8-476 / 8-483: ((alpha >> 2) + 2), at most 65.
    wire [7:0] strong_lim = {2'b00, alpha[7:2]} + 8'd2;
    wire       near       = (d_p0q0 < strong_lim);
    wire       p_strong   = !chroma && ap_small && near;
    wire       q_strong   = !chroma && aq_small && near;

    // 8-477 .. 8-479.
    wire [10:0] p0_s = ({3'd0, p2} + {2'd0, p1, 1'b0} + {2'd0, p0, 1'b0} +
                        {2'd0, q0, 1'b0} + {3'd0, q1} + 11'd4) >> 3;
    wire [10:0] p1_s = ({3'd0, p2} + {3'd0, p1} + {3'd0, p0} + {3'd0, q0} + 11'd2) >> 2;
    wire [10:0] p2_s = ({2'd0, p3, 1'b0} + {3'd0, p2} + {2'd0, p2, 1'b0} + {3'd0, p1} +
                        {3'd0, p0} + {3'd0, q0} + 11'd4) >> 3;
    // 8-480.
    wire [10:0] p0_3 = ({2'd0, p1, 1'b0} + {3'd0, p0} + {3'd0, q1} + 11'd2) >> 2;

    // 8-484 .. 8-486.
    wire [10:0] q0_s = ({3'd0, p1} + {2'd0, p0, 1'b0} + {2'd0, q0, 1'b0} +
                        {2'd0, q1, 1'b0} + {3'd0, q2} + 11'd4) >> 3;
    wire [10:0] q1_s = ({3'd0, p0} + {3'd0, q0} + {3'd0, q1} + {3'd0, q2} + 11'd2) >> 2;
    wire [10:0] q2_s = ({2'd0, q3, 1'b0} + {3'd0, q2} + {2'd0, q2, 1'b0} + {3'd0, q1} +
                        {3'd0, q0} + {3'd0, p0} + 11'd4) >> 3;
    // 8-487.
    wire [10:0] q0_3 = ({2'd0, q1, 1'b0} + {3'd0, q0} + {3'd0, p1} + 11'd2) >> 2;

    wire strong = (bs == 3'd4);

    assign p0o = !filter ? p0 : (strong ? (p_strong ? p0_s[7:0] : p0_3[7:0]) : p0_weak);
    assign p1o = !filter ? p1 : (strong ? (p_strong ? p1_s[7:0] : p1) : p1_weak);
    assign p2o = !filter ? p2 : ((strong && p_strong) ? p2_s[7:0] : p2);
    assign q0o = !filter ? q0 : (strong ? (q_strong ? q0_s[7:0] : q0_3[7:0]) : q0_weak);
    assign q1o = !filter ? q1 : (strong ? (q_strong ? q1_s[7:0] : q1) : q1_weak);
    assign q2o = !filter ? q2 : ((strong && q_strong) ? q2_s[7:0] : q2);

endmodule
