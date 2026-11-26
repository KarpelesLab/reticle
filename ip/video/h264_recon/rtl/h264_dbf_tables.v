// h264_dbf_tables — the deblocking thresholds of ITU-T H.264 §8.7.2.2
// and §8.7.2.3: Table 8-16 (alpha-prime from indexA, beta-prime from
// indexB) and Table 8-17 (tC0-prime from indexA and bS).
//
// What it does
//   Takes the two quantiser averages' indices, already clipped to 0..51
//   by 8-454 and 8-455, and a bS of 1, 2 or 3, and gives the three
//   values `h264_dbf_filter` wants. Combinational; one case statement a
//   table, transcribed from the Recommendation's (V16, 06/2026) tables
//   and checked against the same tables typed independently into
//   `tests/ip_h264_recon.rs`.
//
//   A bS of 0 or 4 gives a tC0 of 0: bS 0 never filters and bS 4 does
//   not read tC0.
module h264_dbf_tables (
    input  wire [5:0] index_a,
    input  wire [5:0] index_b,
    input  wire [2:0] bs,
    output reg  [7:0] alpha,
    output reg  [4:0] beta,
    output reg  [4:0] tc0
);
    reg [4:0] tc1, tc2, tc3;
    always @* begin
        case (index_a)
            6'd16: alpha = 8'd4;
            6'd17: alpha = 8'd4;
            6'd18: alpha = 8'd5;
            6'd19: alpha = 8'd6;
            6'd20: alpha = 8'd7;
            6'd21: alpha = 8'd8;
            6'd22: alpha = 8'd9;
            6'd23: alpha = 8'd10;
            6'd24: alpha = 8'd12;
            6'd25: alpha = 8'd13;
            6'd26: alpha = 8'd15;
            6'd27: alpha = 8'd17;
            6'd28: alpha = 8'd20;
            6'd29: alpha = 8'd22;
            6'd30: alpha = 8'd25;
            6'd31: alpha = 8'd28;
            6'd32: alpha = 8'd32;
            6'd33: alpha = 8'd36;
            6'd34: alpha = 8'd40;
            6'd35: alpha = 8'd45;
            6'd36: alpha = 8'd50;
            6'd37: alpha = 8'd56;
            6'd38: alpha = 8'd63;
            6'd39: alpha = 8'd71;
            6'd40: alpha = 8'd80;
            6'd41: alpha = 8'd90;
            6'd42: alpha = 8'd101;
            6'd43: alpha = 8'd113;
            6'd44: alpha = 8'd127;
            6'd45: alpha = 8'd144;
            6'd46: alpha = 8'd162;
            6'd47: alpha = 8'd182;
            6'd48: alpha = 8'd203;
            6'd49: alpha = 8'd226;
            6'd50: alpha = 8'd255;
            6'd51: alpha = 8'd255;
            default: alpha = 8'd0;
        endcase
    end

    always @* begin
        case (index_b)
            6'd16: beta = 5'd2;
            6'd17: beta = 5'd2;
            6'd18: beta = 5'd2;
            6'd19: beta = 5'd3;
            6'd20: beta = 5'd3;
            6'd21: beta = 5'd3;
            6'd22: beta = 5'd3;
            6'd23: beta = 5'd4;
            6'd24: beta = 5'd4;
            6'd25: beta = 5'd4;
            6'd26: beta = 5'd6;
            6'd27: beta = 5'd6;
            6'd28: beta = 5'd7;
            6'd29: beta = 5'd7;
            6'd30: beta = 5'd8;
            6'd31: beta = 5'd8;
            6'd32: beta = 5'd9;
            6'd33: beta = 5'd9;
            6'd34: beta = 5'd10;
            6'd35: beta = 5'd10;
            6'd36: beta = 5'd11;
            6'd37: beta = 5'd11;
            6'd38: beta = 5'd12;
            6'd39: beta = 5'd12;
            6'd40: beta = 5'd13;
            6'd41: beta = 5'd13;
            6'd42: beta = 5'd14;
            6'd43: beta = 5'd14;
            6'd44: beta = 5'd15;
            6'd45: beta = 5'd15;
            6'd46: beta = 5'd16;
            6'd47: beta = 5'd16;
            6'd48: beta = 5'd17;
            6'd49: beta = 5'd17;
            6'd50: beta = 5'd18;
            6'd51: beta = 5'd18;
            default: beta = 5'd0;
        endcase
    end

    always @* begin
        case (index_a)
            6'd23: tc1 = 5'd1;
            6'd24: tc1 = 5'd1;
            6'd25: tc1 = 5'd1;
            6'd26: tc1 = 5'd1;
            6'd27: tc1 = 5'd1;
            6'd28: tc1 = 5'd1;
            6'd29: tc1 = 5'd1;
            6'd30: tc1 = 5'd1;
            6'd31: tc1 = 5'd1;
            6'd32: tc1 = 5'd1;
            6'd33: tc1 = 5'd2;
            6'd34: tc1 = 5'd2;
            6'd35: tc1 = 5'd2;
            6'd36: tc1 = 5'd2;
            6'd37: tc1 = 5'd3;
            6'd38: tc1 = 5'd3;
            6'd39: tc1 = 5'd3;
            6'd40: tc1 = 5'd4;
            6'd41: tc1 = 5'd4;
            6'd42: tc1 = 5'd4;
            6'd43: tc1 = 5'd5;
            6'd44: tc1 = 5'd6;
            6'd45: tc1 = 5'd6;
            6'd46: tc1 = 5'd7;
            6'd47: tc1 = 5'd8;
            6'd48: tc1 = 5'd9;
            6'd49: tc1 = 5'd10;
            6'd50: tc1 = 5'd11;
            6'd51: tc1 = 5'd13;
            default: tc1 = 5'd0;
        endcase
    end

    always @* begin
        case (index_a)
            6'd21: tc2 = 5'd1;
            6'd22: tc2 = 5'd1;
            6'd23: tc2 = 5'd1;
            6'd24: tc2 = 5'd1;
            6'd25: tc2 = 5'd1;
            6'd26: tc2 = 5'd1;
            6'd27: tc2 = 5'd1;
            6'd28: tc2 = 5'd1;
            6'd29: tc2 = 5'd1;
            6'd30: tc2 = 5'd1;
            6'd31: tc2 = 5'd2;
            6'd32: tc2 = 5'd2;
            6'd33: tc2 = 5'd2;
            6'd34: tc2 = 5'd2;
            6'd35: tc2 = 5'd3;
            6'd36: tc2 = 5'd3;
            6'd37: tc2 = 5'd3;
            6'd38: tc2 = 5'd4;
            6'd39: tc2 = 5'd4;
            6'd40: tc2 = 5'd5;
            6'd41: tc2 = 5'd5;
            6'd42: tc2 = 5'd6;
            6'd43: tc2 = 5'd7;
            6'd44: tc2 = 5'd8;
            6'd45: tc2 = 5'd8;
            6'd46: tc2 = 5'd10;
            6'd47: tc2 = 5'd11;
            6'd48: tc2 = 5'd12;
            6'd49: tc2 = 5'd13;
            6'd50: tc2 = 5'd15;
            6'd51: tc2 = 5'd17;
            default: tc2 = 5'd0;
        endcase
    end

    always @* begin
        case (index_a)
            6'd17: tc3 = 5'd1;
            6'd18: tc3 = 5'd1;
            6'd19: tc3 = 5'd1;
            6'd20: tc3 = 5'd1;
            6'd21: tc3 = 5'd1;
            6'd22: tc3 = 5'd1;
            6'd23: tc3 = 5'd1;
            6'd24: tc3 = 5'd1;
            6'd25: tc3 = 5'd1;
            6'd26: tc3 = 5'd1;
            6'd27: tc3 = 5'd2;
            6'd28: tc3 = 5'd2;
            6'd29: tc3 = 5'd2;
            6'd30: tc3 = 5'd2;
            6'd31: tc3 = 5'd3;
            6'd32: tc3 = 5'd3;
            6'd33: tc3 = 5'd3;
            6'd34: tc3 = 5'd4;
            6'd35: tc3 = 5'd4;
            6'd36: tc3 = 5'd4;
            6'd37: tc3 = 5'd5;
            6'd38: tc3 = 5'd6;
            6'd39: tc3 = 5'd6;
            6'd40: tc3 = 5'd7;
            6'd41: tc3 = 5'd8;
            6'd42: tc3 = 5'd9;
            6'd43: tc3 = 5'd10;
            6'd44: tc3 = 5'd11;
            6'd45: tc3 = 5'd13;
            6'd46: tc3 = 5'd14;
            6'd47: tc3 = 5'd16;
            6'd48: tc3 = 5'd18;
            6'd49: tc3 = 5'd20;
            6'd50: tc3 = 5'd23;
            6'd51: tc3 = 5'd25;
            default: tc3 = 5'd0;
        endcase
    end

    always @* begin
        case (bs)
            3'd1:    tc0 = tc1;
            3'd2:    tc0 = tc2;
            3'd3:    tc0 = tc3;
            default: tc0 = 5'd0;
        endcase
    end
endmodule
