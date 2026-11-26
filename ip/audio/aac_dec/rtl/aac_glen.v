// aac_glen -- the length of the window group starting at window `w0`
// (4.5.2.3.4): one, plus one for each following window whose
// scale_factor_grouping bit says it joins. A long window is one group of one.
//
// Window w (1..7) joins its predecessor's group when grp[7 - w] is set, so
// the length is one plus the run of ones in grp below bit 7 - w0.

module aac_glen (
    input  wire [6:0] grp,
    input  wire [3:0] w0,
    input  wire       short_win,
    output wire [3:0] glen
);
    // The run of set bits from the top of v down.
    function [3:0] run;
        input [6:0] v;
        begin
            casez (v)
                7'b1111111: run = 4'd7;
                7'b111111?: run = 4'd6;
                7'b11111??: run = 4'd5;
                7'b1111???: run = 4'd4;
                7'b111????: run = 4'd3;
                7'b11?????: run = 4'd2;
                7'b1??????: run = 4'd1;
                default:    run = 4'd0;
            endcase
        end
    endfunction

    // grp shifted so that the bit of window w0 + 1 is at the top.
    wire [6:0] from = (w0 >= 4'd7) ? 7'd0 : (grp << w0[2:0]);
    assign glen = short_win ? 4'd1 + run(from) : 4'd1;
endmodule
