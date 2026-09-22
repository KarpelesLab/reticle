`timescale 1ns / 1ps
module tb (
  input wire ready,
  input wire [1:0] mode,
  output reg [7:0] q
);
  reg clk;
  reg rst_n;
  reg [7:0] d;
  reg [7:0] cnt;
  reg [7:0] i;
  reg [7:0] j;
  integer tick;
  reg [3:0] hi;
  reg [3:0] lo;
  reg [3:0] out;
  wire ok;
  reg [3:0] sense;
  reg [1:0] addr;
  reg [7:0] mem [0:3];
  assign ok = $onehot(mode);
  always @(posedge clk or negedge rst_n) begin : reset_ff
    if (!rst_n) begin
      q <= 8'h00;
    end else begin
      q <= d;
    end
  end
  always begin : clock
    clk = 1'b0;
    forever begin
      #(8'h05);
      clk = ~clk;
    end
  end
  initial begin : main
    rst_n = 1'b0;
    d = 8'h00;
    cnt = 8'h00;
    tick = tick - tick;
    $display("start %d", cnt);
    #(8'h0c);
    rst_n = 1'b1;
    @(posedge clk);
    for (i = 8'h00; i < 8'h04; i = i + 8'h01) begin : reticle_cnt_2
      if (i == 8'h02) begin
        disable reticle_cnt_2;
      end
      d = d + i;
      mem[i[1:0]] = d;
      @(posedge clk);
    end
    j = 8'h00;
    begin : reticle_brk_3
      while (j < 8'h0a) begin
        j = j + 8'h01;
        if (j >= 8'h03) begin
          disable reticle_brk_3;
        end
      end
    end
    cnt = tick[7:0];
    repeat (8'h02) begin
      cnt = cnt + 8'h01;
    end
    begin : named
      d <= #2 8'h07;
      {hi, lo} = d;
    end
    wait (ready);
    if (!(q == 8'h07)) $error("q is %h not 7", q);
    if (!(1'b1)) $info;
    $write("done\n");
    $finish;
    $stop;
    $finish;
  end
  always @* begin : decode
    (* parallel_case, full_case *)
    case (mode)
      2'h0: begin
        out = hi;
      end
      2'h1, 2'h2: begin
        out = lo;
      end
      default: begin
        out = 4'h0;
      end
    endcase
  end
  always @* begin : wild
    casez (d)
      8'b1zzzzzzz: begin
        addr = 2'h3;
      end
      8'b01xxxxxx: begin
        addr = 2'h2;
      end
      default: begin
        addr = 2'h0;
      end
    endcase
  end
  always @(d or hi) begin : mixed
    case (d)
      q: begin
        sense = hi;
      end
      8'h01: begin
        sense = lo;
      end
    endcase
    sense[0] = ready;
  end
endmodule
