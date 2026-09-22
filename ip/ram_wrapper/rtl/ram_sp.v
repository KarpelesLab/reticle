// ram_sp — a portable single-port synchronous RAM.
//
// What it does
//   DEPTH words of WIDTH bits with one address, one clock and one port:
//   on a rising `clk` edge with `en` high, `we` high writes `din` and
//   `we` low reads. The read result appears on `dout` in the next cycle
//   (OUT_REG = 0) or the one after (OUT_REG = 1, an extra output
//   register, which is what a block RAM's optional pipeline stage is and
//   what buys it its clock rate on a real device).
//
//   Write-first or read-first is *not* configurable and the behaviour is
//   read-first: a cycle that writes leaves the previous contents of the
//   address on `dout`. That is the shape every FPGA family can build
//   without extra logic, which is the point of a portable wrapper.
//
//   Nothing here is vendor specific: it is an ordinary array with a
//   clocked access, written so that Reticle's memory inference produces a
//   single memory with one read port and one write port, which
//   `fpga::synthesize_for` then maps onto the device's block RAM where
//   the shape fits and onto flip-flops or LUT RAM where it does not.
//
// What it does not do
//   No initial contents — there is no `$readmemh` and no INIT parameter,
//   because a memory image belongs in a file the build system places, not
//   in a library block. No byte enables: `we` is one bit for the whole
//   word. No asynchronous read, no reset on `dout` and no collision
//   handling (there is one port, so there is nothing to collide). DEPTH
//   need not be a power of two, but a device's block RAM has a fixed
//   shape, so anything that is not will be padded or spread by the
//   mapper.
module ram_sp #(
    // Bits per word.
    parameter WIDTH      = 8,
    // Words.
    parameter DEPTH      = 256,
    // 1 adds an output register, so reads take two cycles.
    parameter OUT_REG    = 0,
    // Derived from DEPTH; do not override.
    parameter ADDR_WIDTH = $clog2(DEPTH)
) (
    input  wire                  clk,
    input  wire                  en,
    input  wire                  we,
    input  wire [ADDR_WIDTH-1:0] addr,
    input  wire [WIDTH-1:0]      din,
    output wire [WIDTH-1:0]      dout
);
    reg [WIDTH-1:0] mem [0:DEPTH-1];
    reg [WIDTH-1:0] dout_q;

    always @(posedge clk) begin
        if (en) begin
            if (we) mem[addr] <= din;
            dout_q <= mem[addr];
        end
    end

    generate
        if (OUT_REG != 0) begin : g_pipelined
            reg [WIDTH-1:0] dout_q2;
            always @(posedge clk) begin
                if (en) dout_q2 <= dout_q;
            end
            assign dout = dout_q2;
        end else begin : g_direct
            assign dout = dout_q;
        end
    endgenerate
endmodule
