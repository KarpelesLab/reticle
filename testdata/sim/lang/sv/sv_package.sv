`timescale 1ns/1ps
package cfg_pkg;
  parameter int WIDTH = 12;
  localparam int DEPTH = 1 << 3;
  typedef logic [WIDTH-1:0] data_t;
  typedef enum logic [1:0] {OP_ADD, OP_SUB, OP_AND, OP_OR} op_t;
  typedef struct packed { op_t op; logic [3:0] a; logic [3:0] b; } instr_t;
  function automatic data_t alu(op_t op, data_t a, data_t b);
    case (op)
      OP_ADD: return a + b;
      OP_SUB: return a - b;
      OP_AND: return a & b;
      default: return a | b;
    endcase
  endfunction
  function automatic int sq(int x);
    return x * x;
  endfunction
endpackage

package other_pkg;
  parameter int WIDTH = 4;
  localparam string NAME = "other";
endpackage

import cfg_pkg::*;

module tb;
  data_t a, b;
  op_t op;
  instr_t ins;
  initial begin
    $display("PK1 WIDTH=%0d DEPTH=%0d other::WIDTH=%0d", WIDTH, DEPTH, other_pkg::WIDTH);
    $display("PK2 bits data_t=%0d bits instr_t=%0d", $bits(data_t), $bits(cfg_pkg::instr_t));
    a = 12'h0F0; b = 12'h00F;
    for (int k = 0; k < 4; k++) begin
      op = op_t'(k);
      $display("PK3 op=%0d alu=%h", op, alu(op, a, b));
    end
    $display("PK4 explicit alu=%h", cfg_pkg::alu(cfg_pkg::OP_SUB, a, b));
    $display("PK5 sq=%0d", sq(7));
    ins = '{op: OP_AND, a: 4'hC, b: 4'hA};
    $display("PK6 ins=%h op=%0d", ins, ins.op);
    $display("PK7 name=%s", other_pkg::NAME);
    $display("PK8 OP_OR=%0d", cfg_pkg::OP_OR);
    $finish;
  end
endmodule
