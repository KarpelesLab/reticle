// hyperram_ctrl — a HyperBus controller for a x8 HyperRAM.
//
// What it does
//   Turns one sixteen-bit access on a request port into one HyperBus
//   transaction, for memory or for the part's registers. HyperBus is
//   double data rate: eight data lines carry a byte on each edge of CK,
//   and the read-write data strobe RWDS travels beside them. Every data
//   pin goes through a double-data-rate IO register, asked for with a
//   `ddr` attribute on the port, so a sixteen-bit port here is eight
//   pins: the low byte of the port is the one on the pin for the first
//   half of the cycle, the high byte the one for the second half.
//
//   A transaction is three clocks of command-address (CA), the initial
//   latency, and the data. The CA word carries read or write in CA[47],
//   the address space — memory or registers — in CA[46], a linear burst
//   in CA[45], the word address's upper bits in CA[44:16] and its low
//   three in CA[2:0]; it goes out most significant byte first. The
//   latency is counted from the rising CK edge that begins the third CA
//   clock, which is where the Infineon S27KL/S70KL datasheets draw it:
//   with a latency count of N, the first data byte is on the rising CK
//   edge that begins clock 3 + N of the transaction.
//
//   Fixed and variable latency. With fixed latency (the part's default)
//   every transaction waits twice the count. With variable latency the
//   part says, by driving RWDS high during CA, that this one collides
//   with its own refresh and needs the doubled count; RWDS low means the
//   single count is enough. The block samples RWDS during CA and times a
//   write's data by it. A read does not need it: the part marks its data
//   by toggling RWDS, high with the first byte of each word and low with
//   the second, and the block takes the word where RWDS says it is.
//
//   Writes use RWDS as the byte mask: high masks a byte. Register writes
//   have no latency at all — the sixteen bits follow the CA directly —
//   and no mask, as the protocol says. Register reads have the ordinary
//   latency.
//
//   Configuration. Register accesses are requests with `req_reg` high;
//   the word address of configuration register 0 is 12'h800, of
//   configuration register 1 12'h801, of the two identification registers
//   0 and 1. A write to configuration register 0 is also taken by the
//   block itself, which reads the latency code in bits 7:4 (0 is five
//   clocks, 1 six, 2 seven, 14 three, 15 four) and the fixed-latency bit
//   3, so the block always times its transactions the way the part it
//   has just configured will. After reset it assumes LATENCY and
//   FIXED_LATENCY, which default to the part's own reset state: six
//   clocks, fixed.
//
//   The clock. `hram_ck` is launched by a double-data-rate output
//   register with the data, so it would change on the same edge as the
//   data does; its `io_delay` attribute puts CK_DELAY steps of the
//   device's IO delay after it, which is what moves each CK edge into
//   the middle of the byte it clocks. On the ECP5, whose DELAYG steps
//   are about 25 ps, the default of 100 steps is a quarter of a 100 MHz
//   cycle. The read data comes back through the double-data-rate input
//   registers, sampled on both edges of `clk`, which fall in the middle
//   of the bytes the part launched on the delayed CK.
//
//   CS# stays high at least T_RWR cycles between two transactions (the
//   part's read-write recovery time), and CK only toggles while it is
//   low.
//
// What it does not do
//   One word per transaction. No bursts: the part could stream a whole
//   row, and a cache line fill is the obvious user of that, but each
//   request here is one sixteen-bit word and pays the whole CA and
//   latency. A read ends the transaction as soon as its word is in.
//
//   RWDS is not used as a capture clock. The read data is sampled by
//   `clk` and RWDS is sampled beside it as data, which is the
//   arrangement that fits one clock domain and the IO registers an FPGA
//   has; it relies on the delay above to put `clk`'s edges inside the
//   read data's eye, and above roughly 100 MHz a board needs that delay
//   calibrated, which this block does not do. A read that never sees
//   RWDS toggle ends after the longest latency and eight more clocks,
//   with `rd_error` high.
//
//   On the iCE40 the IO buffer registers both edges itself and nothing
//   can sit between it and the pad, so the delay is not applied (Reticle
//   says so with a warning) and CK is edge-aligned with the data. That
//   works only at a clock slow enough for the part's hold time; the
//   block is written for a family with an IO delay.
//
//   The data and RWDS lines are bidirectional, and like every block in
//   this library it splits them — `_o`, `_oe`, `_i` — because a DDR
//   register on an `inout` port is something the FPGA backend does not
//   build. The three-state buffers belong at the top of the design. The
//   output enables are aligned with the pins, one cycle behind the DDR
//   data they enable, because the DDR output register adds that cycle.
//
//   No power-up wait (the part needs 150 us after its supply is stable
//   before the first access; that is the system's reset to provide), no
//   deep power-down, no hybrid sleep, no wrapped bursts and no x16
//   HyperRAM. `hram_rst_n` simply follows `rst_n`, registered.
module hyperram_ctrl #(
    // Word address bits: 22 for a 64 Mbit part.
    parameter ADDR_WIDTH    = 22,
    // What the part is assumed to be set to after reset.
    parameter LATENCY       = 6,
    parameter FIXED_LATENCY = 1,
    // Clock cycles CS# stays high between transactions.
    parameter T_RWR         = 4,
    // IO delay steps on CK: a quarter of a cycle.
    parameter CK_DELAY      = 100
) (
    input  wire                  clk,
    input  wire                  rst_n,

    // The request port. `req_reg` selects the register space.
    input  wire                  req_valid,
    output wire                  req_ready,
    input  wire                  req_we,
    input  wire                  req_reg,
    input  wire [ADDR_WIDTH-1:0] req_addr,
    input  wire [15:0]           req_wdata,
    input  wire [1:0]            req_be,
    output wire [15:0]           rd_data,
    output wire                  rd_valid,
    output wire                  rd_error,

    // The latency the block is timing transactions with.
    output wire [2:0]            cur_latency,
    output wire                  cur_fixed,

    // The HyperBus pins. Each `ddr` port is two bits per pin.
    output wire                  hram_rst_n,
    output wire                  hram_cs_n,
    (* ddr = "clk", io_delay = CK_DELAY *)
    output wire [1:0]            hram_ck,
    (* ddr = "clk" *)
    output wire [15:0]           hram_dq_o,
    output wire                  hram_dq_oe,
    (* ddr = "clk" *)
    input  wire [15:0]           hram_dq_i,
    (* ddr = "clk" *)
    output wire [1:0]            hram_rwds_o,
    output wire                  hram_rwds_oe,
    (* ddr = "clk" *)
    input  wire [1:0]            hram_rwds_i
);
    localparam [2:0] S_IDLE = 3'd0;
    localparam [2:0] S_CA   = 3'd1;
    localparam [2:0] S_WAIT = 3'd2;
    localparam [2:0] S_DATA = 3'd3;
    localparam [2:0] S_READ = 3'd4;
    localparam [2:0] S_END  = 3'd5;

    // Long enough for the doubled seven-clock latency, the IO pipeline
    // and a word, with margin; a read still waiting then has failed.
    localparam [5:0] TIMEOUT = 6'd32;
    localparam [ADDR_WIDTH-1:0] CR0_ADDR = 12'h800;
    localparam [2:0] LAT_RESET = LATENCY;
    localparam [3:0] RWR_LAST = T_RWR;

    reg [2:0]  state;
    reg [5:0]  t;
    reg [3:0]  gap;
    reg        rst_q;

    // The current latency setting.
    reg [2:0]  lat;
    reg        fixed;
    // This transaction's latency, in clocks, once known.
    reg [4:0]  lat_eff;

    // The request being served.
    reg                  cur_we;
    reg                  cur_reg;
    reg [ADDR_WIDTH-1:0] cur_addr;
    reg [15:0]           cur_wdata;
    reg [1:0]            cur_be;

    // What goes to the DDR output registers, and the enables and chip
    // select that go out a cycle later to line up with them.
    reg [1:0]  ck_q;
    reg [15:0] dq_o_q;
    reg [1:0]  rwds_o_q;
    reg        cs_n_e, dq_oe_e, rwds_oe_e;
    reg        cs_n_q, dq_oe_q, rwds_oe_q;

    // Read capture.
    reg        have_hi;
    reg [7:0]  hi_byte;
    reg [15:0] rd_data_q;
    reg        rd_valid_q;
    reg        rd_error_q;

    // The command-address word.
    wire [31:0] wide_addr = {{(32-ADDR_WIDTH){1'b0}}, cur_addr};
    wire [47:0] ca = {~cur_we, cur_reg, 1'b1, wide_addr[31:3], 13'd0, wide_addr[2:0]};

    // The two samples of this cycle, oldest first.
    wire       r0 = hram_rwds_i[0];
    wire       r1 = hram_rwds_i[1];
    wire [7:0] d0 = hram_dq_i[7:0];
    wire [7:0] d1 = hram_dq_i[15:8];

    // A word is complete this cycle when a high byte is waiting from the
    // last one, or when this cycle holds both halves.
    wire       word_done  = have_hi | (r0 & ~r1);
    wire [15:0] word_in   = have_hi ? {hi_byte, d0} : {d0, d1};

    assign req_ready    = (state == S_IDLE) & (gap >= RWR_LAST);
    assign rd_data      = rd_data_q;
    assign rd_valid     = rd_valid_q;
    assign rd_error     = rd_error_q;
    assign cur_latency  = lat;
    assign cur_fixed    = fixed;

    assign hram_rst_n   = rst_q;
    assign hram_cs_n    = cs_n_q;
    assign hram_ck      = ck_q;
    assign hram_dq_o    = dq_o_q;
    assign hram_dq_oe   = dq_oe_q;
    assign hram_rwds_o  = rwds_o_q;
    assign hram_rwds_oe = rwds_oe_q;

    // The latency code of configuration register 0, in clocks.
    function [2:0] lat_of;
        input [3:0] code;
        begin
            case (code)
                4'd0:    lat_of = 3'd5;
                4'd1:    lat_of = 3'd6;
                4'd2:    lat_of = 3'd7;
                4'd14:   lat_of = 3'd3;
                4'd15:   lat_of = 3'd4;
                default: lat_of = 3'd6;
            endcase
        end
    endfunction

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state      <= S_IDLE;
            t          <= 6'd0;
            gap        <= 4'd0;
            rst_q      <= 1'b0;
            lat        <= LAT_RESET;
            fixed      <= (FIXED_LATENCY != 0);
            lat_eff    <= 5'd0;
            cur_we     <= 1'b0;
            cur_reg    <= 1'b0;
            cur_addr   <= {ADDR_WIDTH{1'b0}};
            cur_wdata  <= 16'd0;
            cur_be     <= 2'b00;
            ck_q       <= 2'b00;
            dq_o_q     <= 16'd0;
            rwds_o_q   <= 2'b00;
            cs_n_e     <= 1'b1;
            dq_oe_e    <= 1'b0;
            rwds_oe_e  <= 1'b0;
            cs_n_q     <= 1'b1;
            dq_oe_q    <= 1'b0;
            rwds_oe_q  <= 1'b0;
            have_hi    <= 1'b0;
            hi_byte    <= 8'd0;
            rd_data_q  <= 16'd0;
            rd_valid_q <= 1'b0;
            rd_error_q <= 1'b0;
        end else begin
            rst_q      <= 1'b1;
            cs_n_q     <= cs_n_e;
            dq_oe_q    <= dq_oe_e;
            rwds_oe_q  <= rwds_oe_e;
            rd_valid_q <= 1'b0;
            rd_error_q <= 1'b0;
            t          <= t + 6'd1;
            if (cs_n_e && gap != 4'hF) gap <= gap + 4'd1;

            case (state)
                S_IDLE: begin
                    t <= 6'd0;
                    if (req_valid && gap >= RWR_LAST) begin
                        cur_we    <= req_we;
                        cur_reg   <= req_reg;
                        cur_addr  <= req_addr;
                        cur_wdata <= req_wdata;
                        cur_be    <= req_be;
                        state     <= S_CA;
                    end
                end
                S_CA: begin
                    // Three clocks of CA, most significant byte first:
                    // the low byte of each pair goes out on the rising
                    // CK edge, the high byte on the falling one.
                    cs_n_e  <= 1'b0;
                    ck_q    <= 2'b01;
                    dq_oe_e <= 1'b1;
                    gap     <= 4'd0;
                    case (t)
                        6'd0:    dq_o_q <= {ca[39:32], ca[47:40]};
                        6'd1:    dq_o_q <= {ca[23:16], ca[31:24]};
                        default: dq_o_q <= {ca[7:0], ca[15:8]};
                    endcase
                    if (t == 6'd2) begin
                        // A register write's data follows at once; every
                        // other transaction waits for its latency.
                        state   <= (cur_we & cur_reg) ? S_DATA : S_WAIT;
                        lat_eff <= fixed ? {1'b0, lat, 1'b0} : {2'b00, lat};
                    end
                end
                S_WAIT: begin
                    dq_oe_e <= 1'b0;
                    have_hi <= 1'b0;
                    // RWDS as the part drove it during CA reaches the
                    // fabric now: high asks for the doubled latency.
                    if (t == 6'd3 && !fixed && r0) lat_eff <= {1'b0, lat, 1'b0};
                    if (!cur_we) begin
                        state <= S_READ;
                    end else if (t + 6'd1 >= {1'b0, lat_eff} + 6'd2) begin
                        state <= S_DATA;
                    end
                end
                S_DATA: begin
                    // One word: the high byte on the rising CK edge, the
                    // low byte on the falling one, and for a memory
                    // write RWDS high over each byte that is masked.
                    dq_o_q   <= {cur_wdata[7:0], cur_wdata[15:8]};
                    dq_oe_e  <= 1'b1;
                    rwds_o_q <= {~cur_be[0], ~cur_be[1]};
                    rwds_oe_e <= ~cur_reg;
                    if (cur_reg && cur_addr == CR0_ADDR) begin
                        lat   <= lat_of(cur_wdata[7:4]);
                        fixed <= cur_wdata[3];
                    end
                    state <= S_END;
                end
                S_READ: begin
                    // The CA-time RWDS has passed through the input
                    // registers by t = 5; from then on RWDS high marks
                    // the first byte of a word. The word ends the
                    // transaction at once, CK stopping with it.
                    if (t >= 6'd5 && word_done) begin
                        rd_data_q  <= word_in;
                        rd_valid_q <= 1'b1;
                        have_hi    <= 1'b0;
                        ck_q       <= 2'b00;
                        cs_n_e     <= 1'b1;
                        state      <= S_END;
                    end else if (t == TIMEOUT) begin
                        rd_data_q  <= 16'd0;
                        rd_valid_q <= 1'b1;
                        rd_error_q <= 1'b1;
                        ck_q       <= 2'b00;
                        cs_n_e     <= 1'b1;
                        state      <= S_END;
                    end else if (t >= 6'd5 && r1) begin
                        have_hi <= 1'b1;
                        hi_byte <= d1;
                    end
                end
                default: begin
                    // S_END: CS# up, CK stopped, the bus released.
                    ck_q      <= 2'b00;
                    cs_n_e    <= 1'b1;
                    dq_oe_e   <= 1'b0;
                    rwds_oe_e <= 1'b0;
                    rwds_o_q  <= 2'b00;
                    have_hi   <= 1'b0;
                    state     <= S_IDLE;
                end
            endcase
        end
    end
endmodule
