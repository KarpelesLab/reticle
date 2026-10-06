// axil_gpio — general purpose IO behind an AXI4-Lite subordinate port.
//
// What it does
//   WIDTH pins, each independently an input or an output, driven and read
//   through four 32-bit registers at word offsets 0, 4, 8 and 12:
//
//     0x0  DATA_OUT  read/write   the value driven on pins set to output
//     0x4  DATA_IN   read only    the pins as sampled, synchronised
//     0x8  DIR       read/write   1 = output, 0 = input (reset: all in)
//     0xC  DATA_SET  write only   bits written 1 are set in DATA_OUT,
//                                 reads return DATA_OUT
//
//   Bits at and above WIDTH read as zero and ignore writes. The address
//   decode uses `addr[3:2]` only, so the block aliases every 16 bytes;
//   place it on a 16-byte-aligned window.
//
//   The pins come back as a tristate triple rather than a bidirectional
//   port: `gpio_o` is the value, `gpio_oe` is the output enable (1 =
//   drive), `gpio_i` is what the pad sees. A top level joins them to an
//   `inout` or to the device's IO primitive, which keeps this block free
//   of anything a simulator or a synthesiser has to special-case.
//
//   `gpio_i` is asynchronous to `s_axi_aclk` — it comes off a pin — so it
//   crosses into the bus domain through a two-flop synchroniser per bit
//   before DATA_IN is built from it.
//
//   The AXI4-Lite port is named for `bus::match_ports`: prefix
//   `s_axi_`, role subordinate, every required signal of the built-in
//   `axi4lite` definition present at the widths the parameters imply. The
//   manifest declares it as one `interface` line and the library checks
//   the two agree.
//
// What it does not do
//   No interrupt: there is no edge detector, no interrupt enable and no
//   status register, so an input change has to be polled. No per-bit
//   write strobes — `s_axi_wstrb` is accepted and ignored, which means a
//   byte write to DATA_OUT writes all four bytes. No read-modify-write
//   clear register to pair with DATA_SET. One outstanding transaction per
//   channel and no write/read reordering: this is the simple subordinate,
//   not a pipelined one. WIDTH is capped at 32 by the register width.
module axil_gpio #(
    // Pins, 1 to 32.
    parameter WIDTH      = 8,
    // AXI4-Lite address bits. Only bits 3:2 are decoded.
    parameter ADDR_WIDTH = 32,
    // AXI4-Lite data bits. Must be 32.
    parameter DATA_WIDTH = 32
) (
    input  wire                      s_axi_aclk,
    input  wire                      s_axi_aresetn,

    input  wire [ADDR_WIDTH-1:0]     s_axi_awaddr,
    input  wire [2:0]                s_axi_awprot,
    input  wire                      s_axi_awvalid,
    output wire                      s_axi_awready,

    input  wire [DATA_WIDTH-1:0]     s_axi_wdata,
    input  wire [(DATA_WIDTH/8)-1:0] s_axi_wstrb,
    input  wire                      s_axi_wvalid,
    output wire                      s_axi_wready,

    output wire [1:0]                s_axi_bresp,
    output wire                      s_axi_bvalid,
    input  wire                      s_axi_bready,

    input  wire [ADDR_WIDTH-1:0]     s_axi_araddr,
    input  wire [2:0]                s_axi_arprot,
    input  wire                      s_axi_arvalid,
    output wire                      s_axi_arready,

    output wire [DATA_WIDTH-1:0]     s_axi_rdata,
    output wire [1:0]                s_axi_rresp,
    output wire                      s_axi_rvalid,
    input  wire                      s_axi_rready,

    input  wire [WIDTH-1:0]          gpio_i,
    output wire [WIDTH-1:0]          gpio_o,
    output wire [WIDTH-1:0]          gpio_oe
);
    localparam [1:0] REG_DATA_OUT = 2'd0;
    localparam [1:0] REG_DATA_IN  = 2'd1;
    localparam [1:0] REG_DIR      = 2'd2;
    localparam [1:0] REG_DATA_SET = 2'd3;

    // ---------------------------------------------------------------
    // The pins.
    // ---------------------------------------------------------------
    reg [WIDTH-1:0] data_out_q;
    reg [WIDTH-1:0] dir_q;
    wire [WIDTH-1:0] data_in;

    cdc_sync #(
        .WIDTH  (WIDTH),
        .STAGES (2),
        .INIT   (0)
    ) u_in_sync (
        .clk   (s_axi_aclk),
        .rst_n (s_axi_aresetn),
        .d     (gpio_i),
        .q     (data_in)
    );

    assign gpio_o = data_out_q;
    assign gpio_oe = dir_q;

    // ---------------------------------------------------------------
    // The AXI4-Lite subordinate.
    // ---------------------------------------------------------------
    reg [ADDR_WIDTH-1:0] awaddr_q;
    reg [DATA_WIDTH-1:0] wdata_q;
    reg [DATA_WIDTH-1:0] rdata_q;
    reg                  aw_seen;
    reg                  w_seen;
    reg                  bvalid_q;
    reg                  rvalid_q;

    assign s_axi_awready = !aw_seen && !bvalid_q;
    assign s_axi_wready  = !w_seen  && !bvalid_q;
    assign s_axi_bvalid  = bvalid_q;
    assign s_axi_bresp   = 2'b00;
    assign s_axi_arready = !rvalid_q;
    assign s_axi_rvalid  = rvalid_q;
    assign s_axi_rdata   = rdata_q;
    assign s_axi_rresp   = 2'b00;

    // The registers widened to the bus, zero above WIDTH. Zero
    // extended by the assignment, which is what keeps WIDTH =
    // DATA_WIDTH legal: a replication of zero bits is not.
    wire [DATA_WIDTH-1:0] data_out_w = data_out_q;
    wire [DATA_WIDTH-1:0] data_in_w  = data_in;
    wire [DATA_WIDTH-1:0] dir_w      = dir_q;

    always @(posedge s_axi_aclk or negedge s_axi_aresetn) begin
        if (!s_axi_aresetn) begin
            data_out_q <= {WIDTH{1'b0}};
            dir_q      <= {WIDTH{1'b0}};
            awaddr_q   <= {ADDR_WIDTH{1'b0}};
            wdata_q    <= {DATA_WIDTH{1'b0}};
            rdata_q    <= {DATA_WIDTH{1'b0}};
            aw_seen    <= 1'b0;
            w_seen     <= 1'b0;
            bvalid_q   <= 1'b0;
            rvalid_q   <= 1'b0;
        end else begin
            if (s_axi_awvalid && s_axi_awready) begin
                aw_seen  <= 1'b1;
                awaddr_q <= s_axi_awaddr;
            end
            if (s_axi_wvalid && s_axi_wready) begin
                w_seen  <= 1'b1;
                wdata_q <= s_axi_wdata;
            end
            if (aw_seen && w_seen && !bvalid_q) begin
                case (awaddr_q[3:2])
                    REG_DATA_OUT: data_out_q <= wdata_q[WIDTH-1:0];
                    REG_DIR:      dir_q      <= wdata_q[WIDTH-1:0];
                    REG_DATA_SET: data_out_q <= data_out_q | wdata_q[WIDTH-1:0];
                    // DATA_IN is read only.
                    default:      data_out_q <= data_out_q;
                endcase
                aw_seen  <= 1'b0;
                w_seen   <= 1'b0;
                bvalid_q <= 1'b1;
            end
            if (bvalid_q && s_axi_bready) bvalid_q <= 1'b0;

            if (s_axi_arvalid && s_axi_arready) begin
                case (s_axi_araddr[3:2])
                    REG_DATA_IN: rdata_q <= data_in_w;
                    REG_DIR:     rdata_q <= dir_w;
                    default:     rdata_q <= data_out_w;
                endcase
                rvalid_q <= 1'b1;
            end else if (rvalid_q && s_axi_rready) begin
                rvalid_q <= 1'b0;
            end
        end
    end
endmodule
