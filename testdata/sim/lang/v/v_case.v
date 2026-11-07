`timescale 1ns/1ps
module tb;
  reg [3:0] s;
  integer k;
  task tc(input [3:0] v);
    begin
      case (v)
        4'd0, 4'd1: $display("case %b -> 0or1", v);
        4'd2: $display("case %b -> 2", v);
        4'd2: $display("case %b -> 2 dup", v);
        4'b1x00: $display("case %b -> 1x00", v);
        default: $display("case %b -> default", v);
      endcase
      casez (v)
        4'b1???: $display("casez %b -> 1???", v);
        4'b01?1: $display("casez %b -> 01?1", v);
        4'b0z0z: $display("casez %b -> 0z0z", v);
        default: $display("casez %b -> default", v);
      endcase
      casex (v)
        4'b1x1x: $display("casex %b -> 1x1x", v);
        4'b00xx: $display("casex %b -> 00xx", v);
        4'b0?11: $display("casex %b -> 0?11", v);
        default: $display("casex %b -> default", v);
      endcase
    end
  endtask
  initial begin
    tc(4'd0); tc(4'd2); tc(4'd3); tc(4'd5); tc(4'd7); tc(4'd10);
    tc(4'd15); tc(4'b1x00); tc(4'bz010); tc(4'b0zz1); tc(4'bxxxx);
    tc(4'b01x1); tc(4'b0100);
    for (k = 0; k < 4; k = k + 1)
      case (1'b1)
        k[0]: $display("onehot k=%0d bit0", k);
        k[1]: $display("onehot k=%0d bit1", k);
        default: $display("onehot k=%0d none", k);
      endcase
    case (3)
      1+2: $display("expr item 1+2");
      3: $display("expr item 3");
    endcase
    case (4'd5)
      5: $display("width int item matched");
      default: $display("width int item default");
    endcase
    case (-1)
      4'b1111: $display("sign case matched");
      default: $display("sign case default");
    endcase
    case (4'sb1111)
      -1: $display("signed item matched");
      default: $display("signed item default");
    endcase
    case (8'd255)
      -1: $display("mixed item matched");
      default: $display("mixed item default");
    endcase
    s = 4'd9;
    case (s) endcase
    case (s)
      4'd9: begin $display("block a"); $display("block b"); end
    endcase
    $finish;
  end
endmodule
