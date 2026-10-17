// adpcm_step — the 89 step sizes of IMA/DVI ADPCM, as a lookup.
//
// One module rather than a function in each of the two blocks that need
// it, because this flow has no include mechanism and a table copied into
// two files is two tables: `ip/compress/inflate` keeps its window and
// its checksum in files of their own for the same reason.
//
// The table is a geometric progression in 1.1 with each entry rounded,
// from 7 to 32767. That is the only property of it that can be checked
// without a second copy to compare against, and
// `adpcm_step_is_a_rounded_geometric_progression` checks it.
//
// `index` is 0 to 88. The encoder and the decoder both clamp before
// they get here, so 89 to 127 cannot be reached; the `default` answers
// with the first entry rather than leaving the output undefined, which
// is what keeps this combinational block free of an inferred latch.
module adpcm_step (
    input  wire [6:0]  index,
    output reg  [14:0] step
);
    always @* begin
        case (index)
            7'd0: step = 15'd7;
            7'd1: step = 15'd8;
            7'd2: step = 15'd9;
            7'd3: step = 15'd10;
            7'd4: step = 15'd11;
            7'd5: step = 15'd12;
            7'd6: step = 15'd13;
            7'd7: step = 15'd14;
            7'd8: step = 15'd16;
            7'd9: step = 15'd17;
            7'd10: step = 15'd19;
            7'd11: step = 15'd21;
            7'd12: step = 15'd23;
            7'd13: step = 15'd25;
            7'd14: step = 15'd28;
            7'd15: step = 15'd31;
            7'd16: step = 15'd34;
            7'd17: step = 15'd37;
            7'd18: step = 15'd41;
            7'd19: step = 15'd45;
            7'd20: step = 15'd50;
            7'd21: step = 15'd55;
            7'd22: step = 15'd60;
            7'd23: step = 15'd66;
            7'd24: step = 15'd73;
            7'd25: step = 15'd80;
            7'd26: step = 15'd88;
            7'd27: step = 15'd97;
            7'd28: step = 15'd107;
            7'd29: step = 15'd118;
            7'd30: step = 15'd130;
            7'd31: step = 15'd143;
            7'd32: step = 15'd157;
            7'd33: step = 15'd173;
            7'd34: step = 15'd190;
            7'd35: step = 15'd209;
            7'd36: step = 15'd230;
            7'd37: step = 15'd253;
            7'd38: step = 15'd279;
            7'd39: step = 15'd307;
            7'd40: step = 15'd337;
            7'd41: step = 15'd371;
            7'd42: step = 15'd408;
            7'd43: step = 15'd449;
            7'd44: step = 15'd494;
            7'd45: step = 15'd544;
            7'd46: step = 15'd598;
            7'd47: step = 15'd658;
            7'd48: step = 15'd724;
            7'd49: step = 15'd796;
            7'd50: step = 15'd876;
            7'd51: step = 15'd963;
            7'd52: step = 15'd1060;
            7'd53: step = 15'd1166;
            7'd54: step = 15'd1282;
            7'd55: step = 15'd1411;
            7'd56: step = 15'd1552;
            7'd57: step = 15'd1707;
            7'd58: step = 15'd1878;
            7'd59: step = 15'd2066;
            7'd60: step = 15'd2272;
            7'd61: step = 15'd2499;
            7'd62: step = 15'd2749;
            7'd63: step = 15'd3024;
            7'd64: step = 15'd3327;
            7'd65: step = 15'd3660;
            7'd66: step = 15'd4026;
            7'd67: step = 15'd4428;
            7'd68: step = 15'd4871;
            7'd69: step = 15'd5358;
            7'd70: step = 15'd5894;
            7'd71: step = 15'd6484;
            7'd72: step = 15'd7132;
            7'd73: step = 15'd7845;
            7'd74: step = 15'd8630;
            7'd75: step = 15'd9493;
            7'd76: step = 15'd10442;
            7'd77: step = 15'd11487;
            7'd78: step = 15'd12635;
            7'd79: step = 15'd13899;
            7'd80: step = 15'd15289;
            7'd81: step = 15'd16818;
            7'd82: step = 15'd18500;
            7'd83: step = 15'd20350;
            7'd84: step = 15'd22385;
            7'd85: step = 15'd24623;
            7'd86: step = 15'd27086;
            7'd87: step = 15'd29794;
            7'd88: step = 15'd32767;            default: step = 15'd7;
        endcase
    end
endmodule
