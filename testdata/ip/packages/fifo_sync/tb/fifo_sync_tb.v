// The testbench the manifest's `testbench` line names. `reticle.proj`
// builds do not elaborate testbenches; `reticle test` will.
module fifo_sync_tb;
  reg        clk = 1'b0;
  reg        rst_n = 1'b0;
  reg  [7:0] wdata = 8'd0;
  reg        push = 1'b0;
  reg        pop = 1'b0;
  wire [7:0] rdata;
  wire       full;
  wire       empty;

  fifo_sync u_dut (
    .clk   (clk),
    .rst_n (rst_n),
    .wdata (wdata),
    .push  (push),
    .pop   (pop),
    .rdata (rdata),
    .full  (full),
    .empty (empty)
  );

  always #5 clk = ~clk;

  initial begin
    #20 rst_n = 1'b1;
    @(posedge clk);
    wdata = 8'h5a;
    push  = 1'b1;
    @(posedge clk);
    push = 1'b0;
    @(posedge clk);
    if (rdata !== 8'h5a) $display("FAIL: rdata=%h", rdata);
    $finish;
  end
endmodule
