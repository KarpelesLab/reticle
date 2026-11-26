// h264_itrans4 — the 4x4 inverse transform of ITU-T H.264 §8.5.12.2,
// and the 4x4 Hadamard of §8.5.10 (8-320) on the same butterflies.
//
// What it does
//   Takes a block's sixteen scaled coefficients d[i][j] one a cycle in
//   raster order — row i = 0 first, j = 0..3 along it — and gives back
//   the sixteen results a row of four at a time, rows in order.
//
//   `had` low: the transform of 8-338 to 8-353 followed by 8-354,
//   r = (h + 32) >> 6, ten bits signed each. The bitstream constraint on
//   h (8-353's range note, at most 2^15 - 33) keeps r inside -512..511,
//   so ten bits are the whole answer for a conforming stream.
//
//   `had` high: the Hadamard f = A c A of 8-320, A = [1 1 1 1; 1 1 -1 -1;
//   1 -1 -1 1; 1 -1 1 -1], unscaled, twenty bits signed each. It is the
//   same pair of passes with the two `>> 1`s of 8-340 / 8-341 removed:
//   e0 + e3 = d0 + d1 + d2 + d3, e1 + e2 = d0 + d1 - d2 - d3,
//   e1 - e2 = d0 - d1 - d2 + d3 and e0 - e3 = d0 - d1 + d2 - d3, which
//   are A's four rows.
//
// The order of the passes is the specification's: rows first
// ("each (horizontal) row ... is transformed"), then columns. The
// `>> 1`s truncate, so the other order is not the same function.
//
// Timing
//   `in_valid` may be high every cycle: a block's last coefficient
//   arrives, the column pass runs on the next edge, and its four rows
//   leave over the four cycles after that while the next block's rows
//   are already coming in. `tag` travels with the block. `had` is
//   sampled with the block's first coefficient and must not change
//   until its last.
module h264_itrans4 (
    input  wire               clk,
    input  wire               rst_n,
    input  wire               in_valid,
    input  wire signed [15:0] in_d,
    input  wire               had,
    input  wire        [5:0]  in_tag,
    output reg                out_valid,
    output reg         [1:0]  out_row,
    output reg         [5:0]  out_tag,
    output reg                out_had,
    // Row `out_row`, column j in bits [20*j +: 20]. With `had` low only
    // the low ten bits of each are meaningful (and sign-extended).
    output wire        [79:0] out_data
);
    // One one-dimensional transform: four in, four out, twenty bits.
    function [79:0] butterfly;
        input signed [19:0] x0, x1, x2, x3;
        input               h;
        reg signed [19:0] e0, e1, e2, e3;
        begin
            e0 = x0 + x2;                                    // 8-338
            e1 = x0 - x2;                                    // 8-339
            e2 = h ? (x1 - x3) : ((x1 >>> 1) - x3);          // 8-340
            e3 = h ? (x1 + x3) : (x1 + (x3 >>> 1));          // 8-341
            butterfly = {e0 - e3, e1 - e2, e1 + e2, e0 + e3}; // 8-342 .. 8-345
        end
    endfunction

    // The row being gathered.
    reg signed [15:0] g0, g1, g2;
    reg        [1:0]  in_col;
    reg        [1:0]  in_rowi;
    reg               blk_had;
    reg        [5:0]  blk_tag;

    // The row pass results, row by row: f[i][j] at rows[i][20*j +: 20].
    reg        [79:0] rows0, rows1, rows2, rows3;

    wire signed [19:0] din = {{4{in_d[15]}}, in_d};
    wire        [79:0] row_out = butterfly({{4{g0[15]}}, g0}, {{4{g1[15]}}, g1},
                                           {{4{g2[15]}}, g2}, din, blk_had);

    // The column pass, all four columns at once, once a block.
    reg               col_go;
    reg        [5:0]  col_tag;
    reg               col_had;
    wire       [79:0] c0 = butterfly(rows0[19:0],  rows1[19:0],  rows2[19:0],  rows3[19:0],  col_had);
    wire       [79:0] c1 = butterfly(rows0[39:20], rows1[39:20], rows2[39:20], rows3[39:20], col_had);
    wire       [79:0] c2 = butterfly(rows0[59:40], rows1[59:40], rows2[59:40], rows3[59:40], col_had);
    wire       [79:0] c3 = butterfly(rows0[79:60], rows1[79:60], rows2[79:60], rows3[79:60], col_had);

    // h[i][j] for the block being sent: column pass output i of column j.
    reg        [79:0] hrow0, hrow1, hrow2, hrow3;

    // The four results of the row being sent.
    reg        [79:0] sel;
    always @* begin
        case (out_row)
            2'd0:    sel = hrow0;
            2'd1:    sel = hrow1;
            2'd2:    sel = hrow2;
            default: sel = hrow3;
        endcase
    end

    // 8-354 on each, or the Hadamard result as it stands.
    function [19:0] finish;
        input signed [19:0] h;
        input               hd;
        reg signed [19:0] r;
        begin
            r = (h + 20'sd32) >>> 6;
            finish = hd ? h : {{10{r[9]}}, r[9:0]};
        end
    endfunction

    assign out_data = {finish(sel[79:60], out_had), finish(sel[59:40], out_had),
                       finish(sel[39:20], out_had), finish(sel[19:0],  out_had)};

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            g0 <= 16'sd0; g1 <= 16'sd0; g2 <= 16'sd0;
            in_col <= 2'd0; in_rowi <= 2'd0;
            blk_had <= 1'b0; blk_tag <= 6'd0;
            rows0 <= 80'd0; rows1 <= 80'd0; rows2 <= 80'd0; rows3 <= 80'd0;
            col_go <= 1'b0; col_tag <= 6'd0; col_had <= 1'b0;
            hrow0 <= 80'd0; hrow1 <= 80'd0; hrow2 <= 80'd0; hrow3 <= 80'd0;
            out_valid <= 1'b0; out_row <= 2'd0; out_tag <= 6'd0; out_had <= 1'b0;
        end else begin
            col_go <= 1'b0;
            if (in_valid) begin
                if (in_col == 2'd0 && in_rowi == 2'd0) begin
                    blk_had <= had;
                    blk_tag <= in_tag;
                end
                case (in_col)
                    2'd0: g0 <= in_d;
                    2'd1: g1 <= in_d;
                    2'd2: g2 <= in_d;
                    default: begin
                        case (in_rowi)
                            2'd0: rows0 <= row_out;
                            2'd1: rows1 <= row_out;
                            2'd2: rows2 <= row_out;
                            default: begin
                                rows3   <= row_out;
                                col_go  <= 1'b1;
                                col_tag <= blk_tag;
                                col_had <= blk_had;
                            end
                        endcase
                        in_rowi <= in_rowi + 2'd1;
                    end
                endcase
                in_col <= in_col + 2'd1;
            end

            // Column pass: column j's outputs are h[0..3][j].
            if (col_go) begin
                hrow0 <= {c3[19:0],  c2[19:0],  c1[19:0],  c0[19:0]};
                hrow1 <= {c3[39:20], c2[39:20], c1[39:20], c0[39:20]};
                hrow2 <= {c3[59:40], c2[59:40], c1[59:40], c0[59:40]};
                hrow3 <= {c3[79:60], c2[79:60], c1[79:60], c0[79:60]};
                out_valid <= 1'b1;
                out_row   <= 2'd0;
                out_tag   <= col_tag;
                out_had   <= col_had;
            end else if (out_valid) begin
                if (out_row == 2'd3) out_valid <= 1'b0;
                out_row <= out_row + 2'd1;
            end
        end
    end
endmodule
