// A block RAM whose write port has a clock and a write enable the design
// ties to zero: the shape of `ip/bus/seph_mcu`'s injection buffer on
// `testdata/fpga/cynthion/nanos_seph_usb.v`, which never injects.
//
// The enable is a constant only once the hierarchy is flattened — the
// memory's own module sees a port — so the memory is inferred, and mapped
// to a `DP16KD`, with a write port that writes nothing. That port has a
// clock, which is what `TrellisFabric::configure_bram` used to take as
// "in use": it then held the undriven write enable low with
// `WEAMUX = INV`, and Project Trellis' `EBR<n>.MODE` record wants that
// bit clear for `DP16KD`. On `EBR0` or `EBR1` that leaves a bit no feature
// explains and `reticle fpga --bitstream` refuses to write the image; on
// `EBR2` or `EBR3`, where this design is placed, the tile holding the clash
// wants only clear bits for the mode, so nothing is left over and the
// image was written with a mode that no longer decodes in one of its two
// tiles. The test counts the tiles that read `DP16KD`, which catches both.
//
// The output is registered a second time for the same reason as in
// `block_ram_2048.v`: to give the clock enough pins to earn a global
// buffer. The balls are that design's.
module block_ram_never_written (
    input  wire        clk,
    input  wire [10:0] addr,
    input  wire [7:0]  wdata,
    output wire [7:0]  rdata
);
    wire [7:0] q;
    block_ram_never_written_mem mem (
        .clk(clk), .we(1'b0), .addr(addr), .wdata(wdata), .q(q)
    );
    reg [7:0] rdata_q;
    always @(posedge clk) rdata_q <= q;
    assign rdata = rdata_q;
endmodule

module block_ram_never_written_mem (
    input  wire       clk,
    input  wire       we,
    input  wire [10:0] addr,
    input  wire [7:0] wdata,
    output reg  [7:0] q
);
    reg [7:0] mem [0:2047];
    always @(posedge clk) begin
        if (we) mem[addr] <= wdata;
        q <= mem[addr];
    end
endmodule
