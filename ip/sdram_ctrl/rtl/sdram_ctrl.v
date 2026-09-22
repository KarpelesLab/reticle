// sdram_ctrl — a controller for single-data-rate SDRAM, 16 bits wide.
//
// What it does
//   Drives one of the common x16 SDR SDRAM parts — Micron MT48LC16M16A2,
//   ISSI IS42S16160 and their smaller siblings — from a simple request
//   port: an address, sixteen bits of write data, a byte enable per
//   octet, read or write, and a valid / ready handshake. A read comes
//   back later on `rd_data` with a one-cycle `rd_valid` strobe.
//
//   Power-up. After reset the block holds COMMAND INHIBIT on the pins
//   for T_INIT_US microseconds with the clock running and CKE high,
//   then issues PRECHARGE ALL, INIT_REFRESHES AUTO REFRESH commands and
//   LOAD MODE REGISTER, each separated by the time the part asks for.
//   `init_done` rises with the mode register set, and no request is
//   taken before it has settled. The mode register is programmed for a
//   burst length of one, sequential bursts and CAS_LATENCY.
//
//   Refresh. A counter restarted at every AUTO REFRESH asks for the next
//   one early enough that a request already in flight can finish first,
//   so the gap between two refreshes never exceeds T_REFI_NS — 64 ms
//   over 8192 rows on every part this block is for. Refresh wins over a
//   waiting request. Before it, every open row is closed with PRECHARGE
//   ALL, respecting tRAS and tWR on each bank.
//
//   Address mapping. `req_addr` is a sixteen-bit word address split as
//   {row, bank, column}: the column in the low COL_BITS, then the two
//   bank bits, then the row. Consecutive addresses therefore walk along
//   a row and then on to the same row of the next bank, so a linear
//   stream moves into a bank that can stay open instead of closing the
//   one it was using.
//
//   Open rows. Each of the four banks remembers the row it has open. An
//   access to that row is a READ or WRITE straight away, with no ACTIVE;
//   an access to another row of the bank precharges it, waits tRP and
//   activates the new one; an access to a closed bank activates it. Rows
//   are left open after an access (an open-page policy), which is what
//   makes the next access to the same row cheap.
//
//   Timings. tRCD, tRP, tRAS, tRC, tRFC, tWR and tRRD are parameters in
//   clock cycles, and they are *derived*: the defaults compute each one
//   as the nanosecond value rounded up to whole cycles of a CLK_MHZ
//   clock, and the refresh interval rounded down, so a design states
//   the datasheet's numbers and its clock and never does the division.
//   tMRD is in cycles already in every datasheet. Each is enforced
//   between the commands it governs, per bank where the datasheet says
//   per bank: a row is not closed before tRAS, nor before tWR after its
//   last write; a bank is not reopened before tRC; two activations are
//   tRRD apart; nothing follows a precharge by less than tRP, a refresh
//   by less than tRFC or a mode register set by less than tMRD; and a
//   READ or WRITE comes tRCD after its row's ACTIVE.
//
//   Clock. `sdram_clk` is a double-data-rate output, as its `ddr`
//   attribute says: two bits for one pin, the low one launched on the
//   rising edge of `clk` and the high one on the falling edge. It is
//   driven 2'b10, so the pin carries `clk` inverted, launched from the
//   same kind of IO register as every other pin. The SDRAM therefore
//   samples the command, the address and the write data in the middle of
//   the cycle they are held for, and the read data it launches on its
//   own edge is captured by `clk` CAS_LATENCY + 1 rising edges after the
//   edge that put the READ on the pins.
//
//   The data bus is split, as every block in this library splits a
//   bidirectional pin: `sdram_dq_o` and `sdram_dq_oe` out, `sdram_dq_i`
//   in. The three-state buffer belongs at the top of the design, which
//   is where the pad is.
//
// What it does not do
//   One access at a time. A request is taken, finished and answered
//   before the next is looked at, so a read costs its CAS latency every
//   time and reads are never pipelined behind one another: a row hit is
//   the READ and CAS_LATENCY + 1 cycles, a row miss adds a precharge and
//   an activate. No bursts: every access is one sixteen-bit word, even
//   though the part could stream a whole row. No write buffer and no
//   reordering: requests happen in the order they arrive.
//
//   Always 16 bits wide and always four banks. No x4 or x8 parts, no
//   32-bit parts, and no two chips side by side. The column must fit in
//   A[9:0] (COL_BITS at most 10), since A10 is the auto-precharge bit.
//
//   No auto-precharge, no self-refresh, no power-down and no clock
//   suspend: CKE is high from the end of reset and stays there, so the
//   part is refreshed by this block for as long as it runs. The extended
//   mode register of the mobile parts is not written.
//
//   The inverted forwarded clock gives the SDRAM half a cycle of setup
//   and half of hold, and gives the read data back half a cycle to
//   arrive in. That closes comfortably at 50 MHz and stops closing when
//   half a cycle is shorter than the part's access time plus the board
//   (tAC is 5.4 ns for a -7 part at CAS latency 3), somewhere near
//   80 MHz. Above that the clock needs a phase-shifted PLL output and a
//   calibrated capture point, which this block does not provide.
//
//   DQM is the write byte mask only. It is held low during reads, so a
//   read always returns both octets.
module sdram_ctrl #(
    // The clock, in MHz. Every derived timing below is rounded to it.
    parameter CLK_MHZ        = 50,
    // CAS latency, 2 or 3 cycles, as programmed into the mode register.
    parameter CAS_LATENCY    = 2,
    // The part's geometry: row and column address bits. Four banks.
    parameter ROW_BITS       = 13,
    parameter COL_BITS       = 9,
    // AUTO REFRESH commands in the power-up sequence: 2 for Micron, 8
    // for ISSI; 8 satisfies both.
    parameter INIT_REFRESHES = 8,
    // The datasheet's timings, in nanoseconds except where marked.
    parameter T_INIT_US      = 200,
    parameter T_RCD_NS       = 20,
    parameter T_RP_NS        = 20,
    parameter T_RAS_NS       = 44,
    parameter T_RC_NS        = 66,
    parameter T_RFC_NS       = 66,
    parameter T_WR_NS        = 15,
    parameter T_RRD_NS       = 15,
    parameter T_REFI_NS      = 7812,
    parameter T_MRD          = 2,
    // Derived: never override these; override the values above.
    parameter TRCD           = (T_RCD_NS * CLK_MHZ + 999) / 1000,
    parameter TRP            = (T_RP_NS * CLK_MHZ + 999) / 1000,
    parameter TRAS           = (T_RAS_NS * CLK_MHZ + 999) / 1000,
    parameter TRC            = (T_RC_NS * CLK_MHZ + 999) / 1000,
    parameter TRFC           = (T_RFC_NS * CLK_MHZ + 999) / 1000,
    parameter TWR            = (T_WR_NS * CLK_MHZ + 999) / 1000,
    parameter TRRD           = (T_RRD_NS * CLK_MHZ + 999) / 1000,
    parameter TREFI          = (T_REFI_NS * CLK_MHZ) / 1000,
    parameter TINIT          = T_INIT_US * CLK_MHZ,
    parameter ADDR_WIDTH     = ROW_BITS + 2 + COL_BITS
) (
    input  wire                  clk,
    input  wire                  rst_n,

    // The request port.
    input  wire                  req_valid,
    output wire                  req_ready,
    input  wire                  req_we,
    input  wire [ADDR_WIDTH-1:0] req_addr,
    input  wire [15:0]           req_wdata,
    input  wire [1:0]            req_be,
    output wire [15:0]           rd_data,
    output wire                  rd_valid,
    output wire                  init_done,

    // The SDRAM pins. `sdram_clk` is two bits for one pin: see above.
    (* ddr = "clk" *)
    output wire [1:0]            sdram_clk,
    output wire                  sdram_cke,
    output wire                  sdram_cs_n,
    output wire                  sdram_ras_n,
    output wire                  sdram_cas_n,
    output wire                  sdram_we_n,
    output wire [1:0]            sdram_ba,
    output wire [ROW_BITS-1:0]   sdram_a,
    output wire [1:0]            sdram_dqm,
    output wire [15:0]           sdram_dq_o,
    output wire                  sdram_dq_oe,
    input  wire [15:0]           sdram_dq_i
);
    // {cs_n, ras_n, cas_n, we_n}
    localparam [3:0] CMD_INHIBIT = 4'b1111;
    localparam [3:0] CMD_NOP     = 4'b0111;
    localparam [3:0] CMD_ACTIVE  = 4'b0011;
    localparam [3:0] CMD_READ    = 4'b0101;
    localparam [3:0] CMD_WRITE   = 4'b0100;
    localparam [3:0] CMD_PRE     = 4'b0010;
    localparam [3:0] CMD_REFRESH = 4'b0001;
    localparam [3:0] CMD_MODE    = 4'b0000;

    localparam [2:0] S_INIT     = 3'd0;
    localparam [2:0] S_INIT_REF = 3'd1;
    localparam [2:0] S_INIT_MRS = 3'd2;
    localparam [2:0] S_IDLE     = 3'd3;
    localparam [2:0] S_REF_PRE  = 3'd4;
    localparam [2:0] S_REF      = 3'd5;
    localparam [2:0] S_ACCESS   = 3'd6;
    localparam [2:0] S_READ     = 3'd7;

    // The longest spacing any counter has to measure, and a width that
    // holds it with room to saturate one above.
    localparam M0   = (TRC > TRAS) ? TRC : TRAS;
    localparam M1   = (TRFC > TWR) ? TRFC : TWR;
    localparam M2   = (TRCD > TRP) ? TRCD : TRP;
    localparam M3   = (TRRD > T_MRD) ? TRRD : T_MRD;
    localparam M4   = (M0 > M1) ? M0 : M1;
    localparam M5   = (M2 > M3) ? M2 : M3;
    localparam M6   = (M4 > M5) ? M4 : M5;
    localparam CMAX = (M6 > CAS_LATENCY + 1) ? M6 : CAS_LATENCY + 1;
    localparam CW   = $clog2(CMAX + 2);
    localparam [CW-1:0] SAT = {CW{1'b1}};
    localparam [CW-1:0] ONE = 1;

    // How early the refresh is asked for, in cycles: enough to finish
    // the worst request in flight (close a row, open another, read it)
    // and then close every bank, with cycles of state changes to spare.
    localparam REF_SLACK = 2 * TRAS + 2 * TRP + TRC + TRCD + TWR + CAS_LATENCY + 8;
    localparam REF_DUE   = TREFI - REF_SLACK;
    localparam RW        = $clog2(TREFI + 2);
    localparam IW        = $clog2(TINIT + 2);

    localparam [2:0] CL3 = CAS_LATENCY;
    // Mode register: burst length 1, sequential, the CAS latency,
    // standard operation, writes at the programmed burst length.
    localparam [ROW_BITS-1:0] MODE_WORD = {{(ROW_BITS-7){1'b0}}, CL3, 4'b0000};
    // A10 high: PRECHARGE means every bank.
    localparam [ROW_BITS-1:0] ALL_BANKS = {{(ROW_BITS-11){1'b0}}, 1'b1, 10'd0};

    reg [2:0]            state;
    reg [IW-1:0]         init_cnt;
    reg [3:0]            init_refs;
    reg                  init_done_q;
    reg                  cke_q;

    reg [3:0]            cmd_q;
    reg [1:0]            ba_q;
    reg [ROW_BITS-1:0]   a_q;
    reg [1:0]            dqm_q;
    reg [15:0]           dq_o_q;
    reg                  dq_oe_q;
    reg [15:0]           rd_data_q;
    reg                  rd_valid_q;

    // Cycles since the last command, and how many the next one needs.
    reg [CW-1:0]         cmd_cnt;
    reg [CW-1:0]         need;
    // Cycles since the last ACTIVE to any bank.
    reg [CW-1:0]         rrd_cnt;
    // Cycles since the last AUTO REFRESH.
    reg [RW-1:0]         ref_cnt;

    // Per bank: open or not, which row, cycles since its ACTIVE, cycles
    // since its last WRITE.
    reg [3:0]            open;
    reg [ROW_BITS-1:0]   row0, row1, row2, row3;
    reg [CW-1:0]         act0, act1, act2, act3;
    reg [CW-1:0]         wr0, wr1, wr2, wr3;

    // The request being served.
    reg                  cur_we;
    reg [ROW_BITS-1:0]   cur_row;
    reg [1:0]            cur_bank;
    reg [COL_BITS-1:0]   cur_col;
    reg [15:0]           cur_wdata;
    reg [1:0]            cur_be;

    wire ok      = (cmd_cnt >= need);
    wire ref_due = (ref_cnt >= REF_DUE);

    // The served bank's state.
    reg [ROW_BITS-1:0]   b_row;
    reg [CW-1:0]         b_act;
    reg [CW-1:0]         b_wr;
    reg                  b_open;
    always @(*) begin
        case (cur_bank)
            2'd0:    begin b_row = row0; b_act = act0; b_wr = wr0; b_open = open[0]; end
            2'd1:    begin b_row = row1; b_act = act1; b_wr = wr1; b_open = open[1]; end
            2'd2:    begin b_row = row2; b_act = act2; b_wr = wr2; b_open = open[2]; end
            default: begin b_row = row3; b_act = act3; b_wr = wr3; b_open = open[3]; end
        endcase
    end
    wire b_hit = b_open & (b_row == cur_row);

    // A bank may be closed once tRAS has passed since it was opened and
    // tWR since it was last written.
    wire can_close0 = ~open[0] | ((act0 >= TRAS) & (wr0 >= TWR));
    wire can_close1 = ~open[1] | ((act1 >= TRAS) & (wr1 >= TWR));
    wire can_close2 = ~open[2] | ((act2 >= TRAS) & (wr2 >= TWR));
    wire can_close3 = ~open[3] | ((act3 >= TRAS) & (wr3 >= TWR));
    wire can_close_all = can_close0 & can_close1 & can_close2 & can_close3;

    assign req_ready   = (state == S_IDLE) & ~ref_due;
    assign rd_data     = rd_data_q;
    assign rd_valid    = rd_valid_q;
    assign init_done   = init_done_q;

    assign sdram_clk   = 2'b10;
    assign sdram_cke   = cke_q;
    assign sdram_cs_n  = cmd_q[3];
    assign sdram_ras_n = cmd_q[2];
    assign sdram_cas_n = cmd_q[1];
    assign sdram_we_n  = cmd_q[0];
    assign sdram_ba    = ba_q;
    assign sdram_a     = a_q;
    assign sdram_dqm   = dqm_q;
    assign sdram_dq_o  = dq_o_q;
    assign sdram_dq_oe = dq_oe_q;

    // A cycle counter that stops at its top rather than wrapping.
    function [CW-1:0] bump;
        input [CW-1:0] v;
        begin
            bump = (v == SAT) ? SAT : v + ONE;
        end
    endfunction

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state       <= S_INIT;
            init_cnt    <= {IW{1'b0}};
            init_refs   <= 4'd0;
            init_done_q <= 1'b0;
            cke_q       <= 1'b0;
            cmd_q       <= CMD_INHIBIT;
            ba_q        <= 2'b00;
            a_q         <= {ROW_BITS{1'b0}};
            dqm_q       <= 2'b11;
            dq_o_q      <= 16'd0;
            dq_oe_q     <= 1'b0;
            rd_data_q   <= 16'd0;
            rd_valid_q  <= 1'b0;
            cmd_cnt     <= SAT;
            need        <= {CW{1'b0}};
            rrd_cnt     <= SAT;
            ref_cnt     <= {RW{1'b0}};
            open        <= 4'b0000;
            row0        <= {ROW_BITS{1'b0}};
            row1        <= {ROW_BITS{1'b0}};
            row2        <= {ROW_BITS{1'b0}};
            row3        <= {ROW_BITS{1'b0}};
            act0        <= SAT;
            act1        <= SAT;
            act2        <= SAT;
            act3        <= SAT;
            wr0         <= SAT;
            wr1         <= SAT;
            wr2         <= SAT;
            wr3         <= SAT;
            cur_we      <= 1'b0;
            cur_row     <= {ROW_BITS{1'b0}};
            cur_bank    <= 2'b00;
            cur_col     <= {COL_BITS{1'b0}};
            cur_wdata   <= 16'd0;
            cur_be      <= 2'b00;
        end else begin
            // Every command is one cycle long: NOP unless a state below
            // issues something, and the bus is released after a write.
            cmd_q      <= (state == S_INIT) ? CMD_INHIBIT : CMD_NOP;
            dq_oe_q    <= 1'b0;
            dqm_q      <= 2'b00;
            rd_valid_q <= 1'b0;
            cke_q      <= 1'b1;

            cmd_cnt <= bump(cmd_cnt);
            rrd_cnt <= bump(rrd_cnt);
            act0    <= bump(act0);
            act1    <= bump(act1);
            act2    <= bump(act2);
            act3    <= bump(act3);
            wr0     <= bump(wr0);
            wr1     <= bump(wr1);
            wr2     <= bump(wr2);
            wr3     <= bump(wr3);
            if (ref_cnt != {RW{1'b1}}) ref_cnt <= ref_cnt + 1'b1;

            case (state)
                S_INIT: begin
                    // COMMAND INHIBIT with the clock running, then
                    // PRECHARGE ALL.
                    if (init_cnt == TINIT[IW-1:0]) begin
                        cmd_q   <= CMD_PRE;
                        a_q     <= ALL_BANKS;
                        cmd_cnt <= ONE;
                        need    <= TRP[CW-1:0];
                        state   <= S_INIT_REF;
                    end else begin
                        init_cnt <= init_cnt + 1'b1;
                    end
                end
                S_INIT_REF: begin
                    if (ok) begin
                        cmd_q     <= CMD_REFRESH;
                        cmd_cnt   <= ONE;
                        need      <= TRFC[CW-1:0];
                        ref_cnt   <= 1;
                        init_refs <= init_refs + 4'd1;
                        if (init_refs == INIT_REFRESHES - 1) state <= S_INIT_MRS;
                    end
                end
                S_INIT_MRS: begin
                    if (ok) begin
                        cmd_q       <= CMD_MODE;
                        ba_q        <= 2'b00;
                        a_q         <= MODE_WORD;
                        cmd_cnt     <= ONE;
                        need        <= T_MRD[CW-1:0];
                        init_done_q <= 1'b1;
                        state       <= S_IDLE;
                    end
                end
                S_IDLE: begin
                    if (ref_due) begin
                        state <= S_REF_PRE;
                    end else if (req_valid) begin
                        cur_we    <= req_we;
                        cur_col   <= req_addr[COL_BITS-1:0];
                        cur_bank  <= req_addr[COL_BITS+1:COL_BITS];
                        cur_row   <= req_addr[ADDR_WIDTH-1:COL_BITS+2];
                        cur_wdata <= req_wdata;
                        cur_be    <= req_be;
                        state     <= S_ACCESS;
                    end
                end
                S_REF_PRE: begin
                    if (open == 4'b0000) begin
                        state <= S_REF;
                    end else if (ok & can_close_all) begin
                        cmd_q   <= CMD_PRE;
                        a_q     <= ALL_BANKS;
                        cmd_cnt <= ONE;
                        need    <= TRP[CW-1:0];
                        open    <= 4'b0000;
                        state   <= S_REF;
                    end
                end
                S_REF: begin
                    if (ok) begin
                        cmd_q   <= CMD_REFRESH;
                        cmd_cnt <= ONE;
                        need    <= TRFC[CW-1:0];
                        ref_cnt <= 1;
                        state   <= S_IDLE;
                    end
                end
                S_ACCESS: begin
                    if (b_hit) begin
                        if (ok & (b_act >= TRCD)) begin
                            cmd_q   <= cur_we ? CMD_WRITE : CMD_READ;
                            ba_q    <= cur_bank;
                            a_q     <= {{(ROW_BITS-COL_BITS){1'b0}}, cur_col};
                            cmd_cnt <= ONE;
                            need    <= ONE;
                            if (cur_we) begin
                                dq_o_q  <= cur_wdata;
                                dq_oe_q <= 1'b1;
                                dqm_q   <= ~cur_be;
                                case (cur_bank)
                                    2'd0:    wr0 <= ONE;
                                    2'd1:    wr1 <= ONE;
                                    2'd2:    wr2 <= ONE;
                                    default: wr3 <= ONE;
                                endcase
                                state <= S_IDLE;
                            end else begin
                                state <= S_READ;
                            end
                        end
                    end else if (b_open) begin
                        // Another row is open in this bank: close it.
                        if (ok & (b_act >= TRAS) & (b_wr >= TWR)) begin
                            cmd_q   <= CMD_PRE;
                            ba_q    <= cur_bank;
                            a_q     <= {ROW_BITS{1'b0}};
                            cmd_cnt <= ONE;
                            need    <= TRP[CW-1:0];
                            case (cur_bank)
                                2'd0:    open[0] <= 1'b0;
                                2'd1:    open[1] <= 1'b0;
                                2'd2:    open[2] <= 1'b0;
                                default: open[3] <= 1'b0;
                            endcase
                        end
                    end else begin
                        if (ok & (b_act >= TRC) & (rrd_cnt >= TRRD)) begin
                            cmd_q   <= CMD_ACTIVE;
                            ba_q    <= cur_bank;
                            a_q     <= cur_row;
                            cmd_cnt <= ONE;
                            need    <= ONE;
                            rrd_cnt <= ONE;
                            case (cur_bank)
                                2'd0: begin
                                    open[0] <= 1'b1;
                                    row0    <= cur_row;
                                    act0    <= ONE;
                                end
                                2'd1: begin
                                    open[1] <= 1'b1;
                                    row1    <= cur_row;
                                    act1    <= ONE;
                                end
                                2'd2: begin
                                    open[2] <= 1'b1;
                                    row2    <= cur_row;
                                    act2    <= ONE;
                                end
                                default: begin
                                    open[3] <= 1'b1;
                                    row3    <= cur_row;
                                    act3    <= ONE;
                                end
                            endcase
                        end
                    end
                end
                default: begin
                    // S_READ: the data is on the pins at the rising edge
                    // CAS_LATENCY + 1 after the one that issued the READ.
                    if (cmd_cnt == CAS_LATENCY + 1) begin
                        rd_data_q  <= sdram_dq_i;
                        rd_valid_q <= 1'b1;
                        state      <= S_IDLE;
                    end
                end
            endcase
        end
    end
endmodule
