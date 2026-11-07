`timescale 1ns/1ps
package sp_pkg;
  typedef struct packed {
    logic [7:0] addr;
    logic [15:0] data;
    logic we;
  } cmd_t;
  typedef enum logic [1:0] {OK, ERR, RETRY} resp_t;
endpackage

module slave_m import sp_pkg::*; (input cmd_t c, output resp_t r, output logic [15:0] rd);
  assign r = c.we ? (c.addr[7] ? ERR : OK) : RETRY;
  assign rd = c.data ^ {c.addr, c.addr};
endmodule

module tb;
  import sp_pkg::*;
  cmd_t c;
  resp_t r;
  logic [15:0] rd;
  slave_m u (.c(c), .r(r), .rd(rd));
  initial begin
    c.addr = 8'h12; c.data = 16'hFFFF; c.we = 1;
    #1 $display("SPT1 r=%0d rd=%h c=%h", r, rd, c);
    c.addr = 8'h80;
    #1 $display("SPT2 r=%0d rd=%h", r, rd);
    c = 25'h0;
    #1 $display("SPT3 r=%0d rd=%h", r, rd);
    $finish;
  end
endmodule
