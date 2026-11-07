`timescale 1ns/1ps
package alu_pkg;
  typedef enum logic [2:0] {ADD, SUB, AND_, OR_, XOR_, SHL, SHR, SLT} op_t;
  typedef struct packed {
    op_t op;
    logic [7:0] a;
    logic [7:0] b;
  } req_t;
  typedef struct packed {
    logic [7:0] y;
    logic zero;
    logic carry;
  } rsp_t;
  function automatic rsp_t compute(req_t r);
    rsp_t o;
    logic [8:0] t;
    t = '0;
    case (r.op)
      ADD:  t = {1'b0, r.a} + {1'b0, r.b};
      SUB:  t = {1'b0, r.a} - {1'b0, r.b};
      AND_: t = {1'b0, r.a & r.b};
      OR_:  t = {1'b0, r.a | r.b};
      XOR_: t = {1'b0, r.a ^ r.b};
      SHL:  t = {r.a, 1'b0};
      SHR:  t = {1'b0, r.a >> 1};
      SLT:  t = {8'b0, $signed(r.a) < $signed(r.b)};
      default: t = '0;
    endcase
    o.y = t[7:0];
    o.carry = t[8];
    o.zero = (t[7:0] == 8'd0);
    return o;
  endfunction
endpackage

module alu import alu_pkg::*; (
  input logic clk,
  input req_t req,
  output rsp_t rsp
);
  always_ff @(posedge clk) rsp <= compute(req);
endmodule

module tb;
  import alu_pkg::*;
  logic clk = 0;
  req_t req;
  rsp_t rsp;
  alu dut (.clk, .req, .rsp);
  always #5 clk = ~clk;
  initial begin
    for (int k = 0; k < 8; k++) begin
      req.op = op_t'(k);
      req.a = 8'hC5;
      req.b = 8'h3B;
      @(posedge clk); #1;
      $display("ALU op=%0d y=%h zero=%b carry=%b rsp=%h", k, rsp.y, rsp.zero, rsp.carry, rsp);
    end
    req = {SUB, 8'd7, 8'd7};
    @(posedge clk); #1;
    $display("ALU sub-eq y=%h zero=%b carry=%b", rsp.y, rsp.zero, rsp.carry);
    $finish;
  end
endmodule
