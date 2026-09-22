// An interface with modports, a task and a parameter, used by modules
// through typed and generic interface ports.
interface bus_if #(parameter int AW = 8, DW = 32) (input logic clk);
  logic [AW - 1:0] addr;
  logic [DW - 1:0] wdata, rdata;
  logic we, req, ack;

  modport master (
    output addr,
    output wdata,
    output we,
    output req,
    input rdata,
    input ack,
    import do_write
  );
  modport slave (input addr, input wdata, input we, input req, output rdata, output ack);
  modport monitor (input addr, input .data(wdata), input clk);

  task automatic do_write(input logic [AW - 1:0] a, input logic [DW - 1:0] d);
    addr  <= a;
    wdata <= d;
    we    <= 1'b1;
    req   <= 1'b1;
    @(posedge clk iff ack);
    req <= 1'b0;
  endtask

  clocking cb @(posedge clk);
    default input # 1step output # 2 ; input rdata , ack ; output addr , wdata ;
  endclocking
endinterface

module producer (bus_if.master bus, input logic start);
  always_ff @(posedge bus.clk) if (start) bus.do_write(8'h10, 32'hdead_beef);
endmodule

module consumer (bus_if bus);
  always_comb bus.ack = bus.req;
endmodule

module generic_port (interface any, interface.slave s);
  assign s.rdata = any.wdata;
endmodule

module top;
  logic clk = 0;
  bus_if #(.AW(8), .DW(32)) bus (.clk(clk));
  producer p (.bus(bus), .start(1'b1));
  consumer c (.bus);
  generic_port g (.any(bus), .s(bus));
endmodule
