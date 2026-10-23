// A stand-in for Vivado's `xpm_memory_sdpram`, the simple dual-port RAM
// macro of the Xilinx Parameterized Macros, in the one configuration
// NewAE's `fifo_sync` and `fifo_async` (fpga-common, with `pBRAM` set)
// instantiate it in. Built with `NOXILINXFIFO`, the Husky gateware uses
// those FIFOs, NewAE's own control logic, in place of the six FIFO
// Generator cores, and this memory is all that is left of Xilinx's IP.
//
// What it does, from UG974's description of the macro (a reading, not a
// measurement): port A writes `dina` at `addra` on `clka` when `ena` and
// `wea` are high; port B, `READ_LATENCY_B` = 1, loads `doutb` from
// `addrb` on its clock when `enb` is high, and `rstb` sets `doutb` to
// zero instead, synchronously. Its clock is `clka` in "common_clock" mode
// and `clkb` in "independent_clock". `WRITE_MODE_B` "read_first": a read
// of the address being written returns the old word.
//
// What it does not do, and refuses: any other read latency, byte-wide
// writes, a nonzero reset value, ECC, or initialisation; `regceb`,
// `sleep` and the error-injection inputs are ignored, which is right for
// NewAE's use (`regceb` is tied high, the rest low) and for nothing else.
// The memory is asked for in block RAM whatever `MEMORY_PRIMITIVE` says;
// NewAE's FIFOs ask for "block".
module xpm_memory_sdpram #(
    parameter ADDR_WIDTH_A = 6,
    parameter ADDR_WIDTH_B = 6,
    parameter AUTO_SLEEP_TIME = 0,
    parameter BYTE_WRITE_WIDTH_A = 32,
    parameter CASCADE_HEIGHT = 0,
    parameter CLOCKING_MODE = "common_clock",
    parameter ECC_MODE = "no_ecc",
    parameter MEMORY_INIT_FILE = "none",
    parameter MEMORY_INIT_PARAM = "0",
    parameter MEMORY_OPTIMIZATION = "true",
    parameter MEMORY_PRIMITIVE = "auto",
    parameter MEMORY_SIZE = 2048,
    parameter MESSAGE_CONTROL = 0,
    parameter READ_DATA_WIDTH_B = 32,
    parameter READ_LATENCY_B = 2,
    parameter READ_RESET_VALUE_B = "0",
    parameter RST_MODE_A = "SYNC",
    parameter RST_MODE_B = "SYNC",
    parameter SIM_ASSERT_CHK = 0,
    parameter USE_EMBEDDED_CONSTRAINT = 0,
    parameter USE_MEM_INIT = 1,
    parameter WAKEUP_TIME = "disable_sleep",
    parameter WRITE_DATA_WIDTH_A = 32,
    parameter WRITE_MODE_B = "no_change"
) (
    output wire                          dbiterrb,
    output reg  [READ_DATA_WIDTH_B-1:0]  doutb,
    output wire                          sbiterrb,
    input  wire [ADDR_WIDTH_A-1:0]       addra,
    input  wire [ADDR_WIDTH_B-1:0]       addrb,
    input  wire                          clka,
    input  wire                          clkb,
    input  wire [WRITE_DATA_WIDTH_A-1:0] dina,
    input  wire                          ena,
    input  wire                          enb,
    input  wire                          injectdbiterra,
    input  wire                          injectsbiterra,
    input  wire                          regceb,
    input  wire                          rstb,
    input  wire                          sleep,
    input  wire [WRITE_DATA_WIDTH_A/BYTE_WRITE_WIDTH_A-1:0] wea
);
  localparam DEPTH = MEMORY_SIZE / WRITE_DATA_WIDTH_A;
  localparam INDEPENDENT = CLOCKING_MODE == "independent_clock";

  // The configurations this does not model stop the build here, by
  // instantiating a module nothing declares, rather than becoming a
  // memory that behaves otherwise than the design expects.
  if (READ_LATENCY_B != 1 || BYTE_WRITE_WIDTH_A != WRITE_DATA_WIDTH_A ||
      READ_DATA_WIDTH_B != WRITE_DATA_WIDTH_A || READ_RESET_VALUE_B != "0" ||
      ECC_MODE != "no_ecc" || MEMORY_INIT_FILE != "none" ||
      WRITE_MODE_B != "read_first" ||
      (CLOCKING_MODE != "common_clock" && !INDEPENDENT)) begin : unsupported
    xpm_memory_sdpram_configuration_not_modelled refused ();
  end

  assign dbiterrb = 1'b0;
  assign sbiterrb = 1'b0;

  (* ram_style = "block" *) reg [WRITE_DATA_WIDTH_A-1:0] mem [0:DEPTH-1];
  always @(posedge clka) if (ena && wea[0]) mem[addra] <= dina;

  // The output register's synchronous reset, kept out of the memory's
  // read register, which is then a plain registered read and becomes the
  // block RAM's own; written `if (rstb) doutb <= 0; else if (enb) doutb <=
  // mem[addrb]` the reset made it a multiplexer, and Reticle put the memory
  // in LUT RAM. The read register loads on `enb || rstb`, and a flag that
  // loads with it says whether that load was a reset, in which case the
  // output is zero. On a reset the flag is set; on a read that is not one
  // the register takes the word and the flag clears; with neither, both
  // hold. It is the same register, at the cost of one multiplexer.
  // (The block RAM's own `RSTRAMB` would do it without; Reticle does not
  // put a reset there yet.)
  wire rclk = INDEPENDENT ? clkb : clka;
  reg [READ_DATA_WIDTH_B-1:0] word;
  reg reset_last = 1'b0;
  always @(posedge rclk)
    if (enb || rstb) begin
      word <= mem[addrb];
      reset_last <= rstb;
    end
  always @(*) doutb = reset_last ? {READ_DATA_WIDTH_B{1'b0}} : word;
endmodule
