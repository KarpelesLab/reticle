// sdram_burst — a burst SDRAM controller with bank-level parallelism and
// several client ports, for the Winbond W9825G6KH-6 (32 MB, x16, four
// banks of 8192 rows of 512 columns) and the parts like it.
//
// What it does
//   Serves NPORTS clients, each through the burst port of
//   `ip/memory/sdram_burst/README.md`: a command (read or write, a
//   sixteen-bit word address, one to sixteen words that never cross a
//   512-word row), the write data behind it with its own handshake, and
//   the read data back in order with a `last` flag. Every port's vectors
//   are packed: port `p` is bits `[p*W +: W]` of each.
//
//   The part is run in full-page burst mode. A READ or WRITE starts a
//   burst that would run on forever, and the cycle after its last word
//   always carries the command that ends it: the next READ or WRITE if
//   the next burst is ready, which makes the two seamless, and BURST
//   STOP otherwise (datasheet §7.7, §7.9, §7.11 and the charts of
//   §11.19). That gives any length from one to sixteen words with one
//   command, and leaves every cycle inside a burst free on the command
//   bus for an ACTIVE or PRECHARGE of another bank. Reads back to back
//   pay the CAS latency once per train.
//
//   Bank parallelism. Commands go through an in-order queue of eight;
//   the oldest four are a window, and the oldest entry of the window
//   that uses a bank is that bank's owner. An owner whose row is not
//   open gets its PRECHARGE and ACTIVE issued during the transfers in
//   front of it, so a train of bursts to rows in different banks runs
//   without a gap. Rows stay open after use (open page).
//
//   Ports. A command is taken into a one-entry buffer per port, so
//   `cmd_ready` is a register. An arbiter grants one buffered command at
//   most every other cycle: ports whose bit is set in PRIO first, then
//   the direction granted last (for up to BATCH grants), round robin
//   within that. A write's data is collected into a 64-word buffer
//   before the command joins the queue, so a burst never waits for a
//   client in the middle; a port's read waits until its writes before it
//   are collected, which keeps each port's commands in its own order.
//   Read data has no ready: a client takes a word every cycle `rd_valid`
//   is high.
//
//   Timings. Every constraint of the datasheet's §9.5 that a command
//   sequence can break is counted per bank or globally: tRCD, tRP, tRAS
//   (min; the max is kept by refresh, which closes every row every
//   tREFI), tRC, tRRD, tWR, tRFC (tRC again, as §9.5 states it for
//   refresh), tRSC (T_MRD) and the refresh interval. The ones the
//   datasheet gives in nanoseconds are parameters in nanoseconds next
//   to the clock (CLK_MHZ, or CLK_PS for a period that is not a whole
//   number of MHz) and rounded up to cycles (the refresh interval down);
//   tRRD, tWR and tRSC are in cycles, as the datasheet gives them.
//
//   Refresh. When it falls due the train in progress is ended at its
//   next burst boundary, every bank is closed with PRECHARGE ALL once
//   tRAS and tWR allow, and AUTO REFRESH follows. The interval between
//   two never exceeds T_REFI_NS: it is asked for REF_SLACK cycles early.
//
//   The bus. A read ends at least RD2WR_GAP empty data cycles before a
//   write begins, and `dq_oe` is raised one cycle before a write's first
//   word and dropped one cycle after its last, so the PHY's output
//   enable has a whole cycle to reach the pad. The README has the
//   arithmetic from tHZ.
//
//   Read capture and calibration. The PHY hands over each DQ bit
//   sampled on both clock edges (`dq_rise`, `dq_fall`, an IDDR's Q1 and
//   Q2). The core delays each by 0 to 3 cycles; one of those eight
//   candidates, chosen by (edge, latency), is the read data. The
//   calibration writes a sixteen-word pattern into the first sixteen
//   columns of CAL_ROW in CAL_BANK, then for each of the 32 input delay
//   taps reads it back and records which candidates returned it exactly;
//   it then picks the candidate with the longest run of good taps and
//   the tap in the middle of that run. It runs after initialisation when
//   CAL_EN is set, and again on `cal_req`.
//
// What it does not do
//   No reordering across the queue: transfers happen in the order the
//   arbiter granted them. No auto-precharge (full page forbids it), no
//   self-refresh, no power-down. Commands from different ports are not
//   ordered against each other.
module sdram_burst #(
    // Clients.
    parameter NPORTS         = 4,
    // Ports whose bit is set win over the others (the display's).
    parameter [31:0] PRIO    = 32'h8,
    // Grants in one direction before the other is let in, within a
    // priority class.
    parameter BATCH          = 8,
    // The clock, in MHz, or its period in picoseconds for a clock that is
    // not a whole number of MHz (166.67 MHz is CLK_PS 6000): the derived
    // timings below are worked out from CLK_PS, minimums rounded up and
    // the refresh interval down, so neither may say the clock is slower
    // than it is or faster than it is.
    parameter CLK_MHZ        = 166,
    parameter CLK_PS         = 1000000 / CLK_MHZ,
    parameter CAS_LATENCY    = 3,
    // Initialisation: the pause, and the AUTO REFRESH commands in it.
    parameter T_INIT_US      = 200,
    parameter INIT_REFRESHES = 8,
    // The datasheet's timings (W9825G6KH-6, §9.5).
    parameter T_RCD_NS       = 15,
    parameter T_RP_NS        = 15,
    parameter T_RAS_NS       = 42,
    parameter T_RC_NS        = 60,
    parameter T_RFC_NS       = 60,
    parameter T_REFI_NS      = 7812,
    parameter T_WR           = 2,
    parameter T_RRD          = 2,
    parameter T_MRD          = 2,
    // Empty data cycles between a read's last word and a write's first.
    parameter RD2WR_GAP      = 3,
    // Cycles of the PHY's read path: from the SDRAM's data cycle to the
    // core's latency-0 candidate, plus the nominal candidate latency.
    parameter PHY_RD_LAT     = 4,
    // Calibration.
    parameter CAL_EN         = 1,
    parameter CAL_BANK       = 3,
    parameter CAL_ROW        = 8191,
    // The capture point until (or unless) calibration chooses one.
    parameter DEF_TAP        = 0,
    parameter DEF_EDGE       = 0,
    parameter DEF_LAT        = 1,
    // Derived: never override these.
    parameter TRCD           = (T_RCD_NS * 1000 + CLK_PS - 1) / CLK_PS,
    parameter TRP            = (T_RP_NS * 1000 + CLK_PS - 1) / CLK_PS,
    parameter TRAS           = (T_RAS_NS * 1000 + CLK_PS - 1) / CLK_PS,
    parameter TRC            = (T_RC_NS * 1000 + CLK_PS - 1) / CLK_PS,
    parameter TRFC           = (T_RFC_NS * 1000 + CLK_PS - 1) / CLK_PS,
    parameter TREFI          = (T_REFI_NS * 1000) / CLK_PS,
    parameter TINIT          = (T_INIT_US * 1000000 + CLK_PS - 1) / CLK_PS,
    // The packed ports' widths, for the manifest.
    parameter ADDR_BITS      = NPORTS * 24,
    parameter LEN_BITS       = NPORTS * 4,
    parameter DATA_BITS      = NPORTS * 16,
    parameter BE_BITS        = NPORTS * 2
) (
    input  wire                    clk,
    input  wire                    rst_n,

    // Client ports, packed.
    input  wire [NPORTS-1:0]       cmd_valid,
    output wire [NPORTS-1:0]       cmd_ready,
    input  wire [NPORTS-1:0]       cmd_we,
    input  wire [ADDR_BITS-1:0]    cmd_addr,
    input  wire [LEN_BITS-1:0]     cmd_len,
    input  wire [NPORTS-1:0]       wr_valid,
    output wire [NPORTS-1:0]       wr_ready,
    input  wire [DATA_BITS-1:0]    wr_data,
    input  wire [BE_BITS-1:0]      wr_be,
    output wire [NPORTS-1:0]       rd_valid,
    output wire [DATA_BITS-1:0]    rd_data,
    output wire [NPORTS-1:0]       rd_last,

    // Status.
    output wire                    init_done,
    input  wire                    cal_req,
    output wire                    cal_busy,
    output wire                    cal_ok,
    output wire [4:0]              cal_tap,
    output wire                    cal_edge,
    output wire [1:0]              cal_lat,

    // To the PHY: one cycle's command, address, mask and data.
    output wire                    sdram_cke,
    output wire                    sdram_cs_n,
    output wire                    sdram_ras_n,
    output wire                    sdram_cas_n,
    output wire                    sdram_we_n,
    output wire [1:0]              sdram_ba,
    output wire [12:0]             sdram_a,
    output wire [1:0]              sdram_dqm,
    output wire [15:0]             sdram_dq_o,
    output wire                    sdram_dq_oe,
    // From the PHY: DQ sampled on the rising and the falling edge.
    input  wire [15:0]             dq_rise,
    input  wire [15:0]             dq_fall,
    // The input delay's tap, loaded when `dly_ld` is high.
    output wire [4:0]              dly_tap,
    output wire                    dly_ld
);
    // ------------------------------------------------------------------
    // Constants
    // ------------------------------------------------------------------
    localparam NP   = NPORTS + 1;          // the clients and calibration
    localparam CALP = NPORTS;
    localparam PW   = (NP <= 2) ? 1 : $clog2(NP);
    localparam Q    = 8;                   // queue entries
    localparam K    = 4;                   // the window that may prepare banks
    localparam TAGLAT = CAS_LATENCY + PHY_RD_LAT;

    // {cs_n, ras_n, cas_n, we_n}
    localparam [3:0] C_INH  = 4'b1111;
    localparam [3:0] C_NOP  = 4'b0111;
    localparam [3:0] C_ACT  = 4'b0011;
    localparam [3:0] C_RD   = 4'b0101;
    localparam [3:0] C_WR   = 4'b0100;
    localparam [3:0] C_PRE  = 4'b0010;
    localparam [3:0] C_REF  = 4'b0001;
    localparam [3:0] C_MRS  = 4'b0000;
    localparam [3:0] C_BST  = 4'b0110;

    // Full page, sequential, CAS_LATENCY, burst read and burst write.
    localparam [2:0]  CL3       = CAS_LATENCY;
    localparam [12:0] MODE_WORD = {6'b000000, CL3, 1'b0, 3'b111};

    // Counter width: the longest spacing any counter measures.
    localparam M0 = (TRC > TRFC) ? TRC : TRFC;
    localparam M1 = (TRAS > TRP) ? TRAS : TRP;
    localparam M2 = (M0 > M1) ? M0 : M1;
    localparam M3 = (CAS_LATENCY + RD2WR_GAP > T_MRD) ? CAS_LATENCY + RD2WR_GAP : T_MRD;
    localparam CMAX = (M2 > M3) ? M2 : M3;
    localparam CW = $clog2(CMAX + 1);
    localparam [CW-1:0] ZERO   = {CW{1'b0}};
    localparam [CW-1:0] W_RCD  = TRCD - 1;
    localparam [CW-1:0] W_RP   = TRP - 1;
    localparam [CW-1:0] W_RAS  = TRAS - 1;
    localparam [CW-1:0] W_RC   = TRC - 1;
    localparam [CW-1:0] W_RFC  = TRFC - 1;
    localparam [CW-1:0] W_WR   = T_WR - 1;
    localparam [CW-1:0] W_RRD  = T_RRD - 1;
    localparam [CW-1:0] W_MRD  = T_MRD - 1;
    localparam [CW-1:0] W_GAP  = CAS_LATENCY + RD2WR_GAP;

    // Refresh: asked for early enough to end a burst of sixteen, wait
    // out tWR and tRAS, close every bank and wait tRP.
    localparam REF_SLACK = 16 + T_WR + TRAS + TRP + 8;
    localparam REF_DUE   = TREFI - REF_SLACK;
    localparam RW        = $clog2(TREFI + 1);
    localparam IW        = $clog2(TINIT + 2);

    localparam [2:0] S_PAUSE = 3'd0;
    localparam [2:0] S_IPRE  = 3'd1;
    localparam [2:0] S_IREF  = 3'd2;
    localparam [2:0] S_IMRS  = 3'd3;
    localparam [2:0] S_RUN   = 3'd4;

    integer i, p, pa, pb, pc, ic, jc, id, ie, iu;

    // ------------------------------------------------------------------
    // Port buffers
    // ------------------------------------------------------------------
    reg [NPORTS-1:0]     pb_v;
    reg [NPORTS-1:0]     pb_we;
    reg [23:0]           pb_addr [0:NPORTS-1];
    reg [3:0]            pb_len  [0:NPORTS-1];
    reg                  run_q;           // init over: ports take commands

    // The calibration's own request.
    reg                  cq_v;
    reg                  cq_we;
    reg [23:0]           cq_addr;

    assign cmd_ready = ~pb_v & {NPORTS{run_q}};

    // Every request, the calibration's last.
    reg [NP-1:0]  rq_v;
    reg [NP-1:0]  rq_we;
    reg [NP*24-1:0] rq_addr_f;
    reg [NP*4-1:0]  rq_len_f;
    always @(*) begin
        for (pa = 0; pa < NPORTS; pa = pa + 1) begin
            rq_v[pa]    = pb_v[pa];
            rq_we[pa]   = pb_we[pa];
            rq_addr_f[pa*24 +: 24] = pb_addr[pa];
            rq_len_f[pa*4 +: 4]    = pb_len[pa];
        end
        rq_v[CALP]    = cq_v;
        rq_we[CALP]   = cq_we;
        rq_addr_f[CALP*24 +: 24] = cq_addr;
        rq_len_f[CALP*4 +: 4]    = 4'd15;
    end

    // ------------------------------------------------------------------
    // Write collector, read staging and the queue's state
    // ------------------------------------------------------------------
    reg              coll_busy;
    reg [PW-1:0]     coll_port;
    reg [3:0]        coll_cnt;
    reg [NP-1:0]     wrq;             // wr_ready, the calibration's last
    reg [23:0]       coll_addr;
    reg [3:0]        coll_len;

    // The write after the one being collected, granted already.
    reg              nx_v;
    reg [PW-1:0]     nx_port;
    reg [23:0]       nx_addr;
    reg [3:0]        nx_len;

    // A write whose last word was just taken, on its way to `ps`.
    reg              wc_v;
    reg [PW-1:0]     wc_port;
    reg [23:0]       wc_addr;
    reg [3:0]        wc_len;

    // A granted read or a collected write, joining the queue this cycle:
    // a queue entry is reserved for every command granted, so it always
    // can.
    reg              ps_v;
    reg              ps_we;
    reg [PW-1:0]     ps_port;
    reg [23:0]       ps_addr;
    reg [3:0]        ps_len;

    reg [Q-1:0]      q_v;
    reg [Q-1:0]      q_we;
    reg [Q-1:0]      q_hit;
    reg [1:0]        q_ba   [0:Q-1];
    reg [12:0]       q_row  [0:Q-1];
    reg [8:0]        q_col  [0:Q-1];
    reg [3:0]        q_len  [0:Q-1];
    reg [PW-1:0]     q_port [0:Q-1];
    reg [3:0]        qn;              // entries in use

    // Write data buffer: 64 words with their byte enables, so the
    // next write's data is collected while the last is written.
    reg [17:0]       wdf [0:63];
    reg [5:0]        wdf_wp, wdf_rp;
    reg [6:0]        wdf_res;         // words granted and not yet written

    assign wr_ready = wrq[NPORTS-1:0];

    // The write data the collector takes this cycle.
    wire [NP*16-1:0] wd_ext = {pat(coll_cnt), wr_data};
    wire [NP*2-1:0]  wb_ext = {2'b11, wr_be};
    wire [NP-1:0]    wv_ext = {1'b1, wr_valid};
    wire [15:0] cw_data = wd_ext[coll_port*16 +: 16];
    wire [1:0]  cw_be   = wb_ext[coll_port*2 +: 2];
    wire        cw_v    = wv_ext[coll_port];
    wire        c_acc   = coll_busy & wrq[coll_port] & cw_v;
    wire        c_last  = c_acc & (coll_cnt == coll_len);

    // ------------------------------------------------------------------
    // Arbiter
    // ------------------------------------------------------------------
    // The arbiter is three steps, each a cycle. The candidates are worked
    // out from registers and registered (`cls_q`): eligibility, the
    // priority class, and the direction last granted. A port is chosen
    // among them, round robin, when no choice is waiting (`gq`). The cycle
    // after, the choice is carried out. So a port is chosen at most every
    // other cycle, and `cls_q` is never older than the last choice: it was
    // worked out the cycle that choice was carried out, knowing it.
    reg [PW-1:0]  rr;                 // the last port granted
    reg           cal_mode;           // only the calibration is served
    reg [3:0]     resv;               // queue entries granted, not yet issued
    reg           wroom;              // a sixteen-word write fits the buffer
    reg           gq_v;
    reg           gq_we;
    reg [PW-1:0]  gq_p;
    // A read granted last cycle cannot reach `ps` while a collected write
    // does: it waits a cycle, and nothing new is chosen.
    wire          gq_stall = gq_v & ~gq_we & wc_v;
    wire          gq_go    = gq_v & ~gq_stall;
    wire          room_ok  = (resv < Q);

    reg [NP-1:0]  elig;
    reg [NP-1:0]  hi;
    reg [NP-1:0]  cls;
    reg [NP-1:0]  cls_q;
    reg [NP-1:0]  same;
    reg           g_dir;              // the direction granted last
    reg [3:0]     g_run;              // grants in it since it changed
    always @(*) begin
        for (pb = 0; pb < NP; pb = pb + 1) begin
            // A write needs a collector slot free once the grant before
            // it is carried out, and room in the buffer; a read waits for
            // its own port's writes to be collected, so it cannot overtake
            // them. The port chosen last is still marked busy.
            elig[pb] = rq_v[pb] & room_ok
                & ~(gq_v & (gq_p == pb))
                & (cal_mode ? (pb == CALP) : 1'b1)
                & (rq_we[pb] ? (~nx_v & ~(gq_v & gq_we) & wroom)
                             : (~(coll_busy & (coll_port == pb))
                              & ~(nx_v & (nx_port == pb))));
            hi[pb] = elig[pb] & ((pb == CALP) | PRIO[pb % 32]);
        end
        cls = (hi != {NP{1'b0}}) ? hi : elig;
        // Within a class, the direction granted last, for up to BATCH
        // grants: every change of direction costs the bus a turn-round.
        same = cls & (g_dir ? rq_we : ~rq_we);
        if (same != {NP{1'b0}} && g_run != BATCH[3:0]) cls = same;
    end

    // Round robin among the candidates: the first after the last port
    // granted, else the first.
    reg           g_v;
    reg [PW-1:0]  g_p;
    reg           g_found;
    always @(*) begin
        g_v = ~gq_v & (cls_q != {NP{1'b0}});
        g_p = {PW{1'b0}};
        g_found = 1'b0;
        for (pc = 0; pc < NP; pc = pc + 1)
            if (!g_found && cls_q[pc] && (pc > rr)) begin
                g_p = pc[PW-1:0];
                g_found = 1'b1;
            end
        for (pc = 0; pc < NP; pc = pc + 1)
            if (!g_found && cls_q[pc]) begin
                g_p = pc[PW-1:0];
                g_found = 1'b1;
            end
    end
    wire g_we = rq_we[g_p];
    // What the grant carried out this cycle asks for.
    wire [23:0] gq_addr = rq_addr_f[(gq_p)*24 +: 24];
    wire [3:0]  gq_len  = rq_len_f[(gq_p)*4 +: 4];
    wire        st2w    = gq_v & gq_we;

    // ------------------------------------------------------------------
    // Bank state
    // ------------------------------------------------------------------
    reg [3:0]        open;
    reg [12:0]       orow [0:3];
    reg [CW-1:0]     w_act [0:3];     // until ACTIVE may be issued
    reg [CW-1:0]     w_rcd [0:3];     // until READ or WRITE may be
    reg [CW-1:0]     w_pre [0:3];     // until PRECHARGE may be
    reg [CW-1:0]     w_rrd;           // until any ACTIVE may be
    reg [CW-1:0]     w_gap;           // until a WRITE may be
    reg [RW-1:0]     ref_cnt;
    reg              ref_pend;

    // The burst on the bus.
    reg              bact;
    reg              bwe;
    reg [1:0]        bbank;
    reg [3:0]        brem;
    reg [PW-1:0]     bport;

    reg [2:0]        state;
    reg [IW-1:0]     icnt;
    reg [3:0]        irefs;

    // The two output stages: decided -> s1 -> pins.
    reg [3:0]        s1_cmd;
    reg [1:0]        s1_ba;
    reg [12:0]       s1_a;
    reg [1:0]        s1_dqm;
    reg [15:0]       s1_dq;
    reg              s1_ws;          // a write data cycle
    reg [3:0]        o_cmd;
    reg [1:0]        o_ba;
    reg [12:0]       o_a;
    reg [1:0]        o_dqm;
    reg [15:0]       o_dq;
    reg              o_ws;
    reg              o_oe;
    reg              o_cke;

    assign sdram_cke   = o_cke;
    assign sdram_cs_n  = o_cmd[3];
    assign sdram_ras_n = o_cmd[2];
    assign sdram_cas_n = o_cmd[1];
    assign sdram_we_n  = o_cmd[0];
    assign sdram_ba    = o_ba;
    assign sdram_a     = o_a;
    assign sdram_dqm   = o_dqm;
    assign sdram_dq_o  = o_dq;
    assign sdram_dq_oe = o_oe;

    // ------------------------------------------------------------------
    // The scheduler's decision, from registers only
    // ------------------------------------------------------------------
    wire       run   = (state == S_RUN);
    wire       bend  = bact & (brem == 4'd0);
    wire       sfree = ~bact | bend;
    wire [1:0] b0    = q_ba[0];

    wire ready0 = q_v[0] & q_hit[0] & open[b0] & (w_rcd[b0] == ZERO) & ~ref_pend
                & (~q_we[0] | (w_gap == ZERO));
    wire issue  = run & sfree & ready0;
    wire bst    = run & bend & ~issue;

    // Owners and their wants.
    reg [K-1:0] own, cand, cand_act;
    reg         pv;
    reg [1:0]   cbank;
    reg [12:0]  crow;
    reg         cact;
    always @(*) begin
        for (jc = 0; jc < K; jc = jc + 1) begin
            own[jc] = q_v[jc] & ~(bact & (q_ba[jc] == bbank));
            for (ic = 0; ic < jc; ic = ic + 1)
                if (q_v[ic] && q_ba[ic] == q_ba[jc]) own[jc] = 1'b0;
            cand_act[jc] = ~open[q_ba[jc]];
            cand[jc] = own[jc] & ~ref_pend &
                (open[q_ba[jc]] ? (~q_hit[jc] & (w_pre[q_ba[jc]] == ZERO))
                               : ((w_act[q_ba[jc]] == ZERO) & (w_rrd == ZERO)));
        end
        pv = 1'b0;
        cbank = 2'd0;
        crow = 13'd0;
        cact = 1'b0;
        for (jc = K - 1; jc >= 0; jc = jc - 1)
            if (cand[jc]) begin
                pv = 1'b1;
                cbank = q_ba[jc];
                crow = q_row[jc];
                cact = cand_act[jc];
            end
    end
    // The candidate is registered and issued the cycle after, unless that
    // cycle's slot is taken or last cycle's command touched its bank (or
    // was an ACTIVE, for tRRD). Its timers were met when it was chosen and
    // only a command to its own bank could have set them again.
    reg         pq_v;
    reg [1:0]   pq_bank;
    reg [12:0]  pq_row;
    reg         pq_act;
    reg         issue_q;
    wire        l_bank  = (s1_cmd == C_ACT) | (s1_cmd == C_PRE) | (s1_cmd == C_RD)
                        | (s1_cmd == C_WR);
    wire        p_clash = (l_bank & ((s1_ba == pq_bank) | ((s1_cmd == C_PRE) & s1_a[10])))
                        | (pq_act & (s1_cmd == C_ACT) & (T_RRD > 1))
                        | (pq_act & ref_pend);
    wire prep = run & ~issue & ~bst & pq_v & ~p_clash;
    wire pact = pq_act;
    wire [1:0]  pbank = pq_bank;
    wire [12:0] prow  = pq_row;

    // Refresh in run mode.
    reg all_closable, all_actable;
    always @(*) begin
        all_closable = 1'b1;
        all_actable  = 1'b1;
        for (id = 0; id < 4; id = id + 1) begin
            if (open[id] && w_pre[id] != ZERO) all_closable = 1'b0;
            if (w_act[id] != ZERO) all_actable = 1'b0;
        end
    end
    wire ref_free = run & ref_pend & ~bact;
    wire do_pall  = ref_free & (open != 4'b0000) & all_closable;
    wire do_ref   = ref_free & (open == 4'b0000) & all_actable;

    // The data slot this cycle.
    wire       slot      = issue | (bact & (brem != 4'd0));
    wire       slot_we   = issue ? q_we[0] : bwe;
    wire [1:0] slot_bank = issue ? b0 : bbank;
    wire       slot_last = issue ? (q_len[0] == 4'd0) : (brem == 4'd1);
    wire [PW-1:0] slot_port = issue ? q_port[0] : bport;
    wire       wslot     = slot & slot_we;
    wire       rslot     = slot & ~slot_we;

    // Queue push.
    wire        push    = ps_v;
    wire [23:0] pu_addr = ps_addr;
    wire [3:0]  pu_len  = ps_len;
    wire [PW-1:0] pu_port = ps_port;
    wire        pu_we   = ps_we;
    wire [1:0]  pu_ba   = pu_addr[10:9];
    wire [12:0] pu_row  = pu_addr[23:11];
    wire        pu_hit  = open[pu_ba] & (orow[pu_ba] == pu_row);

    // The command issued last cycle, for the queue's hit flags.
    wire       l_act = (s1_cmd == C_ACT);
    wire       l_pre = (s1_cmd == C_PRE);

    // ------------------------------------------------------------------
    // Read data: the eight candidates and the tags that say whose
    // ------------------------------------------------------------------
    reg [15:0]   rs [0:3];
    reg [15:0]   fs [0:3];
    reg [TAGLAT-1:0] t_v;
    reg [TAGLAT-1:0] t_last;
    reg [PW-1:0] t_port [0:TAGLAT-1];
    reg          cfg_edge;
    reg [1:0]    cfg_lat;
    reg [4:0]    cfg_tap;
    wire [15:0]  sel = cfg_edge ? fs[cfg_lat] : rs[cfg_lat];
    wire         t_out  = t_v[TAGLAT-1];
    wire [PW-1:0] t_oport = t_port[TAGLAT-1];
    wire         t_olast = t_last[TAGLAT-1];

    reg [NPORTS-1:0] rv_q;
    reg [NPORTS-1:0] rl_q;
    reg [15:0]       rd_q;
    assign rd_valid = rv_q;
    assign rd_last  = rl_q;
    genvar gp;
    generate
        for (gp = 0; gp < NPORTS; gp = gp + 1) begin : rdbus
            assign rd_data[gp*16 +: 16] = rd_q;
        end
    endgenerate

    // ------------------------------------------------------------------
    // Calibration
    // ------------------------------------------------------------------
    localparam [3:0] K_IDLE  = 4'd0;
    localparam [3:0] K_DRAIN = 4'd1;
    localparam [3:0] K_WR    = 4'd2;
    localparam [3:0] K_WWAIT = 4'd3;
    localparam [3:0] K_TAP   = 4'd4;
    localparam [3:0] K_RD    = 4'd5;
    localparam [3:0] K_RWAIT = 4'd6;
    localparam [3:0] K_UPD   = 4'd7;
    localparam [3:0] K_PICK  = 4'd8;
    localparam [3:0] K_DONE  = 4'd9;

    reg [3:0]  kst;
    reg [4:0]  ktap;
    reg [3:0]  kcnt;          // a wait, a beat or a candidate
    reg [7:0]  kpass;         // candidates still right this read
    reg [15:0] kexp;          // the word expected next
    reg [7:0]  keq;           // which candidates had it, a cycle late
    reg        ks_v;
    reg        ks_last;
    reg [5:0]  kcur  [0:7];   // the run of good taps ending here
    reg [4:0]  kcst  [0:7];   // where it started
    reg [5:0]  kbest [0:7];   // the longest run
    reg [4:0]  kbst  [0:7];   // where it started
    reg [5:0]  pk_len;
    reg [4:0]  pk_start;
    reg [2:0]  pk;
    reg        kok;
    wire [5:0] pk_half = (pk_len - 6'd1) >> 1;
    reg  [4:0] pk_mid;
    reg        kdone;         // calibrated at least once (or never asked)
    reg        kreq;
    reg [4:0]  dtap;
    reg        dld;

    assign cal_busy = (kst != K_IDLE);
    assign cal_ok   = kok;
    assign cal_tap  = cfg_tap;
    assign cal_edge = cfg_edge;
    assign cal_lat  = cfg_lat;
    assign dly_tap  = dtap;
    assign dly_ld   = dld;

    reg rdy_q;
    assign init_done = rdy_q;

    // The pattern: every word different, every bit toggling.
    function [15:0] pat;
        input [3:0] n;
        begin
            pat = {~n, n, ~n ^ 4'h6, n ^ 4'h9};
        end
    endfunction

    wire idle = ~ps_v & ~wc_v & ~gq_v & ~coll_busy & ~nx_v & (qn == 4'd0) & ~bact
              & (t_v == {TAGLAT{1'b0}});

    wire [17:0] wdf_q = wdf[wdf_rp];

    function [CW-1:0] dec;
        input [CW-1:0] v;
        begin
            dec = (v == ZERO) ? ZERO : v - 1'b1;
        end
    endfunction
    function [CW-1:0] atleast;
        input [CW-1:0] v;
        input [CW-1:0] m;
        begin
            atleast = (dec(v) > m) ? dec(v) : m;
        end
    endfunction

    // The collector's next state.
    reg          c_nbusy;
    reg [PW-1:0] c_nport;
    reg [23:0]   c_naddr;
    reg [3:0]    c_nlen;
    reg [3:0]    c_ncnt;
    reg          n_nv;
    reg          g_nx;
    always @(*) begin
        g_nx    = 1'b0;
        c_nbusy = coll_busy;
        c_nport = coll_port;
        c_naddr = coll_addr;
        c_nlen  = coll_len;
        c_ncnt  = c_acc ? coll_cnt + 4'd1 : coll_cnt;
        n_nv    = nx_v;
        if (c_last) begin
            if (nx_v) begin
                c_nport = nx_port;
                c_naddr = nx_addr;
                c_nlen  = nx_len;
                c_ncnt  = 4'd0;
                n_nv    = 1'b0;
            end else begin
                c_nbusy = 1'b0;
            end
        end
        if (st2w) begin
            if (!c_nbusy) begin
                c_nbusy = 1'b1;
                c_nport = gq_p;
                c_naddr = gq_addr;
                c_nlen  = gq_len;
                c_ncnt  = 4'd0;
            end else begin
                n_nv = 1'b1;
                g_nx = 1'b1;
            end
        end
    end

    // The queue's next state: pop on issue, push behind, and the hit
    // flags brought up to date with last cycle's ACTIVE or PRECHARGE.
    reg [Q-1:0]  n_v, n_we, n_hit;
    reg [Q*2-1:0]  n_ba_f;
    reg [Q*13-1:0] n_row_f;
    reg [Q*9-1:0]  n_col_f;
    reg [Q*4-1:0]  n_len_f;
    reg [Q*PW-1:0] n_port_f;
    // Each entry's hit flag after last cycle's command, worked out before
    // the queue moves so the compare is not behind `issue`.
    reg [Q-1:0]  u_hit;
    always @(*) begin
        for (iu = 0; iu < Q; iu = iu + 1) begin
            u_hit[iu] = q_hit[iu];
            if (l_act && q_ba[iu] == s1_ba) u_hit[iu] = (q_row[iu] == s1_a);
            if (l_pre && (s1_a[10] || q_ba[iu] == s1_ba)) u_hit[iu] = 1'b0;
        end
    end
    always @(*) begin
        for (ie = 0; ie < Q; ie = ie + 1) begin
            if (issue && ie < Q - 1) begin
                n_v[ie] = q_v[ie+1]; n_we[ie] = q_we[ie+1]; n_hit[ie] = u_hit[ie+1];
                n_ba_f[(ie)*2 +: 2] = q_ba[ie+1]; n_row_f[(ie)*13 +: 13] = q_row[ie+1]; n_col_f[(ie)*9 +: 9] = q_col[ie+1];
                n_len_f[(ie)*4 +: 4] = q_len[ie+1]; n_port_f[(ie)*PW +: PW] = q_port[ie+1];
            end else if (issue) begin
                n_v[ie] = 1'b0; n_we[ie] = 1'b0; n_hit[ie] = 1'b0;
                n_ba_f[(ie)*2 +: 2] = 2'd0; n_row_f[(ie)*13 +: 13] = 13'd0; n_col_f[(ie)*9 +: 9] = 9'd0;
                n_len_f[(ie)*4 +: 4] = 4'd0; n_port_f[(ie)*PW +: PW] = {PW{1'b0}};
            end else begin
                n_v[ie] = q_v[ie]; n_we[ie] = q_we[ie]; n_hit[ie] = u_hit[ie];
                n_ba_f[(ie)*2 +: 2] = q_ba[ie]; n_row_f[(ie)*13 +: 13] = q_row[ie]; n_col_f[(ie)*9 +: 9] = q_col[ie];
                n_len_f[(ie)*4 +: 4] = q_len[ie]; n_port_f[(ie)*PW +: PW] = q_port[ie];
            end
            if (push && (issue ? (qn == ie + 1) : (qn == ie))) begin
                n_v[ie] = 1'b1; n_we[ie] = pu_we; n_hit[ie] = pu_hit;
                n_ba_f[(ie)*2 +: 2] = pu_ba; n_row_f[(ie)*13 +: 13] = pu_row; n_col_f[(ie)*9 +: 9] = pu_addr[8:0];
                n_len_f[(ie)*4 +: 4] = pu_len; n_port_f[(ie)*PW +: PW] = pu_port;
            end
        end
    end

    // The write data buffer has no reset: a word is written before it
    // is ever read.
    always @(posedge clk)
        if (c_acc) wdf[wdf_wp] <= {cw_be, cw_data};

    // ------------------------------------------------------------------
    // The one clocked block
    // ------------------------------------------------------------------
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            pb_v      <= {NPORTS{1'b0}};
            pb_we     <= {NPORTS{1'b0}};
            for (p = 0; p < NPORTS; p = p + 1) begin
                pb_addr[p] <= 24'd0;
                pb_len[p]  <= 4'd0;
            end
            run_q     <= 1'b0;
            cq_v      <= 1'b0;
            cq_we     <= 1'b0;
            cq_addr   <= 24'd0;
            coll_busy <= 1'b0;
            coll_port <= {PW{1'b0}};
            coll_cnt  <= 4'd0;
            coll_addr <= 24'd0;
            coll_len  <= 4'd0;
            wrq       <= {NP{1'b0}};
            nx_v      <= 1'b0;
            nx_port   <= {PW{1'b0}};
            nx_addr   <= 24'd0;
            nx_len    <= 4'd0;
            ps_v      <= 1'b0;
            ps_we     <= 1'b0;
            wc_v      <= 1'b0;
            wc_port   <= {PW{1'b0}};
            wc_addr   <= 24'd0;
            wc_len    <= 4'd0;
            gq_v      <= 1'b0;
            cls_q     <= {NP{1'b0}};
            gq_we     <= 1'b0;
            gq_p      <= {PW{1'b0}};
            resv      <= 4'd0;
            pq_v      <= 1'b0;
            pq_bank   <= 2'd0;
            pq_row    <= 13'd0;
            pq_act    <= 1'b0;
            issue_q   <= 1'b0;
            wroom     <= 1'b1;
            ps_port   <= {PW{1'b0}};
            ps_addr   <= 24'd0;
            ps_len    <= 4'd0;
            q_v       <= {Q{1'b0}};
            q_we      <= {Q{1'b0}};
            q_hit     <= {Q{1'b0}};
            for (i = 0; i < Q; i = i + 1) begin
                q_ba[i]   <= 2'd0;
                q_row[i]  <= 13'd0;
                q_col[i]  <= 9'd0;
                q_len[i]  <= 4'd0;
                q_port[i] <= {PW{1'b0}};
            end
            qn        <= 4'd0;
            wdf_wp    <= 6'd0;
            wdf_rp    <= 6'd0;
            wdf_res   <= 7'd0;
            rr        <= {PW{1'b0}};
            g_dir     <= 1'b0;
            g_run     <= 4'd0;
            cal_mode  <= 1'b0;
            open      <= 4'b0000;
            for (i = 0; i < 4; i = i + 1) begin
                orow[i]  <= 13'd0;
                w_act[i] <= ZERO;
                w_rcd[i] <= ZERO;
                w_pre[i] <= ZERO;
            end
            w_rrd     <= ZERO;
            w_gap     <= ZERO;
            ref_cnt   <= {RW{1'b0}};
            ref_pend  <= 1'b0;
            bact      <= 1'b0;
            bwe       <= 1'b0;
            bbank     <= 2'd0;
            brem      <= 4'd0;
            bport     <= {PW{1'b0}};
            state     <= S_PAUSE;
            icnt      <= {IW{1'b0}};
            irefs     <= 4'd0;
            s1_cmd    <= C_INH;
            s1_ba     <= 2'd0;
            s1_a      <= 13'd0;
            s1_dqm    <= 2'b11;
            s1_dq     <= 16'd0;
            s1_ws     <= 1'b0;
            o_cmd     <= C_INH;
            o_ba      <= 2'd0;
            o_a       <= 13'd0;
            o_dqm     <= 2'b11;
            o_dq      <= 16'd0;
            o_ws      <= 1'b0;
            o_oe      <= 1'b0;
            o_cke     <= 1'b0;
            for (i = 0; i < 4; i = i + 1) begin
                rs[i] <= 16'd0;
                fs[i] <= 16'd0;
            end
            t_v       <= {TAGLAT{1'b0}};
            t_last    <= {TAGLAT{1'b0}};
            for (i = 0; i < TAGLAT; i = i + 1)
                t_port[i] <= {PW{1'b0}};
            cfg_edge  <= DEF_EDGE;
            cfg_lat   <= DEF_LAT;
            cfg_tap   <= DEF_TAP;
            rv_q      <= {NPORTS{1'b0}};
            rl_q      <= {NPORTS{1'b0}};
            rd_q      <= 16'd0;
            kst       <= K_IDLE;
            ktap      <= 5'd0;
            kcnt      <= 4'd0;
            kpass     <= 8'd0;
            kexp      <= 16'd0;
            keq       <= 8'd0;
            ks_v      <= 1'b0;
            ks_last   <= 1'b0;
            for (i = 0; i < 8; i = i + 1) begin
                kcur[i]  <= 6'd0;
                kcst[i]  <= 5'd0;
                kbest[i] <= 6'd0;
                kbst[i]  <= 5'd0;
            end
            pk_len    <= 6'd0;
            pk_start  <= 5'd0;
            pk        <= 3'd0;
            pk_mid    <= 5'd0;
            kok       <= 1'b0;
            kdone     <= 1'b0;
            kreq      <= 1'b0;
            dtap      <= DEF_TAP;
            dld       <= 1'b0;
            rdy_q     <= 1'b0;
        end else begin
            // ----------------------------------------------------------
            // Ports and arbiter
            // ----------------------------------------------------------
            for (p = 0; p < NPORTS; p = p + 1)
                if (cmd_valid[p] && !pb_v[p] && run_q) begin
                    pb_v[p]    <= 1'b1;
                    pb_we[p]   <= cmd_we[p];
                    pb_addr[p] <= cmd_addr[p*24 +: 24];
                    pb_len[p]  <= cmd_len[p*4 +: 4];
                end
            // The bank to prepare next, and whether a burst began.
            pq_v    <= pv;
            pq_bank <= cbank;
            pq_row  <= crow;
            pq_act  <= cact;
            issue_q <= issue;

            // Choose: the candidates, then one of them.
            cls_q <= (g_v || gq_stall) ? {NP{1'b0}} : cls;
            if (!gq_stall) begin
                gq_v  <= g_v;
                gq_we <= g_we;
                gq_p  <= g_p;
            end
            if (g_v) begin
                rr <= g_p;
                g_dir <= g_we;
                g_run <= (g_we != g_dir) ? 4'd1
                       : (g_run == BATCH[3:0]) ? g_run : g_run + 4'd1;
            end
            resv <= resv + {3'd0, g_v} - {3'd0, issue_q};

            // Stage two: take the command out of its port's buffer; a
            // read goes to `ps` unless a collected write does.
            if (gq_go) begin
                if (gq_p == CALP)
                    cq_v <= 1'b0;
                else
                    pb_v[gq_p] <= 1'b0;
            end
            ps_v <= 1'b0;
            if (wc_v) begin
                ps_v    <= 1'b1;
                ps_we   <= 1'b1;
                ps_port <= wc_port;
                ps_addr <= wc_addr;
                ps_len  <= wc_len;
            end else if (gq_go && !gq_we) begin
                ps_v    <= 1'b1;
                ps_we   <= 1'b0;
                ps_port <= gq_p;
                ps_addr <= gq_addr;
                ps_len  <= gq_len;
            end

            // Collector: the write being collected, and the one granted
            // behind it, which takes over without a gap.
            coll_busy <= c_nbusy;
            coll_port <= c_nport;
            coll_addr <= c_naddr;
            coll_len  <= c_nlen;
            coll_cnt  <= c_ncnt;
            nx_v      <= n_nv;
            if (g_nx) begin
                nx_port <= gq_p;
                nx_addr <= gq_addr;
                nx_len  <= gq_len;
            end
            for (p = 0; p < NP; p = p + 1)
                wrq[p] <= c_nbusy & (c_nport == p);
            if (c_acc) wdf_wp <= wdf_wp + 6'd1;
            wc_v <= c_last;
            if (c_last) begin
                wc_port <= coll_port;
                wc_addr <= coll_addr;
                wc_len  <= coll_len;
            end
            wdf_res <= wdf_res + (st2w ? {3'b000, gq_len} + 7'd1 : 7'd0) - {6'd0, wslot};
            // Room for a sixteen-word write after the one being carried
            // out, not counting the words leaving this cycle.
            wroom   <= (wdf_res + (st2w ? {3'b000, gq_len} + 7'd1 : 7'd0)) <= 7'd48;
            if (wslot) wdf_rp <= wdf_rp + 6'd1;

            // ----------------------------------------------------------
            // Queue: pop on issue, push behind, hit flags kept current
            // ----------------------------------------------------------
            for (i = 0; i < Q; i = i + 1) begin
                q_v[i]    <= n_v[i];
                q_we[i]   <= n_we[i];
                q_hit[i]  <= n_hit[i];
                q_ba[i]   <= n_ba_f[(i)*2 +: 2];
                q_row[i]  <= n_row_f[(i)*13 +: 13];
                q_col[i]  <= n_col_f[(i)*9 +: 9];
                q_len[i]  <= n_len_f[(i)*4 +: 4];
                q_port[i] <= n_port_f[(i)*PW +: PW];
            end
            qn <= qn + {3'd0, push} - {3'd0, issue};

            // ----------------------------------------------------------
            // Timers
            // ----------------------------------------------------------
            for (i = 0; i < 4; i = i + 1) begin
                w_act[i] <= dec(w_act[i]);
                w_rcd[i] <= dec(w_rcd[i]);
                w_pre[i] <= dec(w_pre[i]);
            end
            w_rrd <= dec(w_rrd);
            w_gap <= rslot ? W_GAP : dec(w_gap);
            if (wslot) w_pre[slot_bank] <= atleast(w_pre[slot_bank], W_WR);
            if (ref_cnt != {RW{1'b1}}) ref_cnt <= ref_cnt + 1'b1;
            if (run && ref_cnt >= REF_DUE) ref_pend <= 1'b1;

            // ----------------------------------------------------------
            // The command
            // ----------------------------------------------------------
            s1_cmd <= (state == S_PAUSE) ? C_INH : C_NOP;
            s1_dqm <= (state == S_RUN) ? 2'b00 : 2'b11;
            s1_ws  <= wslot;
            o_cke  <= 1'b1;
            if (wslot) begin
                s1_dq  <= wdf_q[15:0];
                s1_dqm <= ~wdf_q[17:16];
            end

            case (state)
                S_PAUSE: begin
                    if (icnt == TINIT) begin
                        s1_cmd <= C_PRE;
                        s1_a   <= 13'h0400;
                        icnt   <= {IW{1'b0}};
                        for (i = 0; i < 4; i = i + 1)
                            w_act[i] <= W_RP;
                        state  <= S_IPRE;
                    end else begin
                        icnt <= icnt + 1'b1;
                    end
                end
                S_IPRE, S_IREF: begin
                    if (all_actable) begin
                        if (irefs == INIT_REFRESHES) begin
                            s1_cmd <= C_MRS;
                            s1_ba  <= 2'b00;
                            s1_a   <= MODE_WORD;
                            icnt   <= W_MRD;
                            state  <= S_IMRS;
                        end else begin
                            s1_cmd <= C_REF;
                            irefs  <= irefs + 4'd1;
                            ref_cnt <= {RW{1'b0}};
                            for (i = 0; i < 4; i = i + 1)
                                w_act[i] <= W_RFC;
                            state  <= S_IREF;
                        end
                    end
                end
                S_IMRS: begin
                    if (icnt == {IW{1'b0}}) begin
                        state <= S_RUN;
                        run_q <= 1'b1;
                    end else begin
                        icnt <= icnt - 1'b1;
                    end
                end
                default: begin
                    if (issue) begin
                        s1_cmd <= q_we[0] ? C_WR : C_RD;
                        s1_ba  <= b0;
                        s1_a   <= {4'd0, q_col[0]};
                        bact   <= 1'b1;
                        bwe    <= q_we[0];
                        bbank  <= b0;
                        brem   <= q_len[0];
                        bport  <= q_port[0];
                    end else if (bst) begin
                        s1_cmd <= C_BST;
                        if (bwe) s1_dqm <= 2'b11;
                        bact   <= 1'b0;
                    end else if (prep) begin
                        s1_ba <= pbank;
                        if (pact) begin
                            s1_cmd <= C_ACT;
                            s1_a   <= prow;
                            open[pbank]  <= 1'b1;
                            orow[pbank]  <= prow;
                            w_rcd[pbank] <= W_RCD;
                            w_pre[pbank] <= W_RAS;
                            w_act[pbank] <= W_RC;
                            w_rrd        <= W_RRD;
                        end else begin
                            s1_cmd <= C_PRE;
                            s1_a   <= 13'd0;
                            open[pbank]  <= 1'b0;
                            w_act[pbank] <= atleast(w_act[pbank], W_RP);
                        end
                    end else if (do_pall) begin
                        s1_cmd <= C_PRE;
                        s1_a   <= 13'h0400;
                        open   <= 4'b0000;
                        for (i = 0; i < 4; i = i + 1)
                            w_act[i] <= atleast(w_act[i], W_RP);
                    end else if (do_ref) begin
                        s1_cmd   <= C_REF;
                        ref_cnt  <= {RW{1'b0}};
                        ref_pend <= 1'b0;
                        for (i = 0; i < 4; i = i + 1)
                            w_act[i] <= W_RFC;
                    end
                    if (bact && !bend) brem <= brem - 4'd1;
                end
            endcase

            // Second stage, and the output enable a cycle either side
            // of every write data cycle.
            o_cmd <= s1_cmd;
            o_ba  <= s1_ba;
            o_a   <= s1_a;
            o_dqm <= s1_dqm;
            o_dq  <= s1_dq;
            o_ws  <= s1_ws;
            o_oe  <= wslot | s1_ws | o_ws;

            // ----------------------------------------------------------
            // Read data
            // ----------------------------------------------------------
            rs[0] <= dq_rise;
            fs[0] <= dq_fall;
            for (i = 1; i < 4; i = i + 1) begin
                rs[i] <= rs[i-1];
                fs[i] <= fs[i-1];
            end
            t_v[0]    <= rslot;
            t_last[0] <= slot_last;
            t_port[0] <= slot_port;
            for (i = 1; i < TAGLAT; i = i + 1) begin
                t_v[i]    <= t_v[i-1];
                t_last[i] <= t_last[i-1];
                t_port[i] <= t_port[i-1];
            end
            for (p = 0; p < NPORTS; p = p + 1) begin
                rv_q[p] <= t_out & (t_oport == p);
                rl_q[p] <= t_out & t_olast & (t_oport == p);
            end
            rd_q <= sel;

            // ----------------------------------------------------------
            // Calibration
            // ----------------------------------------------------------
            dld <= 1'b0;
            if (cal_req) kreq <= 1'b1;
            for (i = 0; i < 4; i = i + 1) begin
                keq[i]   <= (rs[i] == kexp);
                keq[i+4] <= (fs[i] == kexp);
            end
            ks_v    <= (kst == K_RWAIT) & t_out & (t_oport == CALP);
            pk_mid  <= pk_start + pk_half[4:0];
            ks_last <= t_olast;
            rdy_q <= run_q & kdone & (kst == K_IDLE);
            case (kst)
                K_IDLE: begin
                    if (run_q && ((CAL_EN != 0 && !kdone) || kreq)) begin
                        kreq     <= 1'b0;
                        cal_mode <= 1'b1;
                        kst      <= K_DRAIN;
                    end else if (run_q && !kdone) begin
                        // No calibration: the default tap, once.
                        kdone <= 1'b1;
                        dld   <= 1'b1;
                    end
                end
                K_DRAIN: begin
                    if (idle) begin
                        cq_v    <= 1'b1;
                        cq_we   <= 1'b1;
                        cq_addr <= {CAL_ROW[12:0], CAL_BANK[1:0], 9'd0};
                        ktap    <= 5'd0;
                        for (i = 0; i < 8; i = i + 1) begin
                            kcur[i]  <= 6'd0;
                            kbest[i] <= 6'd0;
                        end
                        kst <= K_WR;
                    end
                end
                K_WR: begin
                    if (!cq_v) kst <= K_WWAIT;
                end
                K_WWAIT: begin
                    if (idle) kst <= K_TAP;
                    kcnt <= 4'd0;
                end
                K_TAP: begin
                    if (kcnt == 4'd0) begin
                        dtap <= ktap;
                        dld  <= 1'b1;
                    end
                    kcnt <= kcnt + 4'd1;
                    if (kcnt == 4'd15) begin
                        cq_v  <= 1'b1;
                        cq_we <= 1'b0;
                        kst   <= K_RD;
                    end
                end
                K_RD: begin
                    kpass <= 8'hFF;
                    kcnt  <= 4'd0;
                    kexp  <= pat(4'd0);
                    if (!cq_v) kst <= K_RWAIT;
                end
                K_RWAIT: begin
                    // A word arrives: compare every candidate with the
                    // word expected, and fold that in the cycle after.
                    if (t_out && t_oport == CALP) begin
                        kcnt <= kcnt + 4'd1;
                        kexp <= pat(kcnt + 4'd1);
                    end
                    if (ks_v) begin
                        kpass <= kpass & keq;
                        if (ks_last) kst <= K_UPD;
                    end
                end
                K_UPD: begin
                    for (i = 0; i < 8; i = i + 1) begin
                        if (kpass[i]) begin
                            if (kcur[i] == 6'd0) kcst[i] <= ktap;
                            kcur[i] <= kcur[i] + 6'd1;
                            if (kcur[i] + 6'd1 > kbest[i]) begin
                                kbest[i] <= kcur[i] + 6'd1;
                                kbst[i]  <= (kcur[i] == 6'd0) ? ktap : kcst[i];
                            end
                        end else begin
                            kcur[i] <= 6'd0;
                        end
                    end
                    kcnt <= 4'd0;
                    pk_len <= 6'd0;
                    if (ktap == 5'd31) begin
                        kst <= K_PICK;
                    end else begin
                        ktap <= ktap + 5'd1;
                        kst  <= K_TAP;
                    end
                end
                K_PICK: begin
                    if (!kcnt[3] && kbest[kcnt[2:0]] > pk_len) begin
                        pk_len   <= kbest[kcnt[2:0]];
                        pk_start <= kbst[kcnt[2:0]];
                        pk       <= kcnt[2:0];
                    end
                    kcnt <= kcnt + 4'd1;
                    // One cycle more, for `pk_mid` to be registered.
                    if (kcnt == 4'd8) kst <= K_DONE;
                end
                default: begin
                    // K_DONE: the middle of the longest run, or the
                    // defaults if nothing ever read back right.
                    if (pk_len != 6'd0) begin
                        cfg_edge <= pk[2];
                        cfg_lat  <= pk[1:0];
                        cfg_tap  <= pk_mid;
                        dtap     <= pk_mid;
                        kok      <= 1'b1;
                    end else begin
                        cfg_edge <= DEF_EDGE;
                        cfg_lat  <= DEF_LAT;
                        cfg_tap  <= DEF_TAP;
                        dtap     <= DEF_TAP;
                        kok      <= 1'b0;
                    end
                    dld      <= 1'b1;
                    kdone    <= 1'b1;
                    cal_mode <= 1'b0;
                    kst      <= K_IDLE;
                end
            endcase
        end
    end
endmodule
