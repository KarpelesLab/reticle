// Four 32-bit registers behind one AXI4-Lite subordinate port, selected
// by `addr[3:2]`. Deliberately the simplest subordinate that still uses
// every channel: address and data are accepted independently, the write
// happens once both have arrived, and reads take one cycle.
module axil_regs(
  input             clk,
  input             rst_n,
  input      [31:0] s_awaddr,
  input       [2:0] s_awprot,
  input             s_awvalid,
  output            s_awready,
  input      [31:0] s_wdata,
  input       [3:0] s_wstrb,
  input             s_wvalid,
  output            s_wready,
  output      [1:0] s_bresp,
  output            s_bvalid,
  input             s_bready,
  input      [31:0] s_araddr,
  input       [2:0] s_arprot,
  input             s_arvalid,
  output            s_arready,
  output     [31:0] s_rdata,
  output      [1:0] s_rresp,
  output            s_rvalid,
  input             s_rready
);
  reg [31:0] r0, r1, r2, r3;
  reg [31:0] awaddr_q, wdata_q, rdata_q;
  reg        aw_seen, w_seen, bvalid_q, rvalid_q;

  assign s_awready = !aw_seen && !bvalid_q;
  assign s_wready  = !w_seen  && !bvalid_q;
  assign s_bvalid  = bvalid_q;
  assign s_bresp   = 2'b00;
  assign s_arready = !rvalid_q;
  assign s_rvalid  = rvalid_q;
  assign s_rdata   = rdata_q;
  assign s_rresp   = 2'b00;

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      r0 <= 32'd0; r1 <= 32'd0; r2 <= 32'd0; r3 <= 32'd0;
      awaddr_q <= 32'd0; wdata_q <= 32'd0; rdata_q <= 32'd0;
      aw_seen <= 1'b0; w_seen <= 1'b0; bvalid_q <= 1'b0; rvalid_q <= 1'b0;
    end else begin
      if (s_awvalid && s_awready) begin
        aw_seen  <= 1'b1;
        awaddr_q <= s_awaddr;
      end
      if (s_wvalid && s_wready) begin
        w_seen  <= 1'b1;
        wdata_q <= s_wdata;
      end
      if (aw_seen && w_seen && !bvalid_q) begin
        case (awaddr_q[3:2])
          2'd0: r0 <= wdata_q;
          2'd1: r1 <= wdata_q;
          2'd2: r2 <= wdata_q;
          default: r3 <= wdata_q;
        endcase
        aw_seen  <= 1'b0;
        w_seen   <= 1'b0;
        bvalid_q <= 1'b1;
      end
      if (bvalid_q && s_bready) bvalid_q <= 1'b0;
      if (s_arvalid && s_arready) begin
        case (s_araddr[3:2])
          2'd0: rdata_q <= r0;
          2'd1: rdata_q <= r1;
          2'd2: rdata_q <= r2;
          default: rdata_q <= r3;
        endcase
        rvalid_q <= 1'b1;
      end else if (rvalid_q && s_rready) begin
        rvalid_q <= 1'b0;
      end
    end
  end
endmodule
