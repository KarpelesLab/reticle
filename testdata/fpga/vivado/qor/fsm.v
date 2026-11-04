// A one-hot-able state machine with enough states to tell encodings apart.
module fsm (input wire clk, input wire si, output reg so);
  reg [7:0] in;
  reg [4:0] s;
  reg [15:0] out;
  always @(posedge clk) begin
    in <= {in[6:0], si};
    case (s)
      5'd0: s <= in[0] ? 5'd1 : 5'd7;
      5'd1: s <= in[1] ? 5'd2 : 5'd13;
      5'd2: s <= in[2] ? 5'd3 : 5'd19;
      5'd3: s <= in[3] ? 5'd4 : 5'd0;
      5'd4: s <= in[0] & in[1] ? 5'd5 : 5'd9;
      5'd5: s <= in[2] | in[3] ? 5'd6 : 5'd1;
      5'd6: s <= in[4] ? 5'd7 : 5'd21;
      5'd7: s <= in[5] ? 5'd8 : 5'd2;
      5'd8: s <= in[6] ? 5'd9 : 5'd17;
      5'd9: s <= in[7] ? 5'd10 : 5'd3;
      5'd10: s <= in[0] ^ in[7] ? 5'd11 : 5'd5;
      5'd11: s <= in[1] ^ in[6] ? 5'd12 : 5'd23;
      5'd12: s <= in[2] ^ in[5] ? 5'd13 : 5'd4;
      5'd13: s <= in[3] ^ in[4] ? 5'd14 : 5'd6;
      5'd14: s <= in[0] ? 5'd15 : 5'd11;
      5'd15: s <= in[1] ? 5'd16 : 5'd8;
      5'd16: s <= in[2] ? 5'd17 : 5'd10;
      5'd17: s <= in[3] ? 5'd18 : 5'd12;
      5'd18: s <= in[4] ? 5'd19 : 5'd14;
      5'd19: s <= in[5] ? 5'd20 : 5'd15;
      5'd20: s <= in[6] ? 5'd21 : 5'd16;
      5'd21: s <= in[7] ? 5'd22 : 5'd18;
      5'd22: s <= in[0] ? 5'd23 : 5'd20;
      5'd23: s <= in[1] ? 5'd0 : 5'd22;
      default: s <= 5'd0;
    endcase
    out <= {out[10:0], s};
    so <= ^out;
  end
endmodule
