// rv32i — a multi-cycle RV32I machine-mode processor core.
//
// What it does
//   The whole RV32I base integer instruction set, machine mode only, in a
//   multi-cycle datapath with no pipeline and therefore no hazards to get
//   wrong: LUI, AUIPC, JAL, JALR, the six branches, the five loads and
//   three stores with sub-word placement and sign extension, the nine
//   register-immediate and ten register-register operations, FENCE and
//   FENCE.I, ECALL, EBREAK, MRET, WFI and the six Zicsr instructions.
//
//   Against memories that answer in the cycle they are asked, one
//   instruction takes two clocks and one that touches data memory takes
//   three, plus a clock per wait state either memory adds. There are
//   exactly three states:
//
//     FETCH   `imem_req` is high with `imem_addr` = pc. The word is taken
//             on the rising edge where `imem_ready` is also high.
//     EXEC    the instruction is decoded, the register file read, the ALU
//             evaluated and — for everything but a load or a store — the
//             result written and pc advanced, all in one cycle.
//     MEM     `dmem_req` is high; the access completes on the rising edge
//             where `dmem_ready` is also high, and a load's result is
//             written to `rd` on that edge.
//
//   Both memory ports use the same handshake: the core holds the request
//   and its address stable until it sees `*_ready` high at a rising clock
//   edge, and that edge is the transfer. A memory that answers in the
//   cycle it is asked ties `*_ready` high and needs nothing else.
//   `dmem_addr` is always word aligned — the low two bits are zero — and
//   `dmem_be` says which byte lanes a store writes. A read is always a
//   whole word; the core extracts the byte or half-word itself, so
//   `dmem_be` is only meaningful while `dmem_we` is high.
//
//   Machine-mode CSRs: `mstatus` (MIE and MPIE; MPP reads 2'b11),
//   `mtvec`, `mepc`, `mcause`, `mie`, `mip`, `mcycle` / `mcycleh` and
//   `minstret` / `minstreth`, plus `misa`, `mvendorid`, `marchid`,
//   `mimpid` and `mhartid` as read-only constants. An unknown CSR, or a
//   write to one whose address says it is read only, is an illegal
//   instruction.
//
//   Traps. A synchronous exception writes `mepc` with the address of the
//   instruction that caused it, `mcause` with the code below, pushes MIE
//   into MPIE, clears MIE and jumps to `mtvec`:
//
//     0   instruction address misaligned (a taken branch, JAL or JALR
//         whose target is not a multiple of four)
//     2   illegal instruction
//     3   breakpoint (EBREAK)
//     4   load address misaligned
//     6   store address misaligned
//     11  environment call from M-mode (ECALL)
//
//   An interrupt is taken between instructions, before the fetch, when
//   MIE is set and `mie & mip` is non-zero; `mepc` is then the
//   instruction that has not run yet. `mip` is not a register: bits 11,
//   7 and 3 are the `irq_external`, `irq_timer` and `irq_software`
//   inputs, and the causes are 0x8000000B, 0x80000007 and 0x80000003,
//   taken in that order of priority. MRET restores MIE from MPIE, sets
//   MPIE and jumps to `mepc`.
//
//   FENCE and FENCE.I retire as no-ops: there is one hart, no cache and
//   no store buffer, so there is nothing for them to order. WFI is a
//   no-op too — the core does not stop the clock — which is a legal
//   implementation of it.
//
//   REGFILE_BRAM chooses how x1..x31 are stored. With 0 the array is read
//   asynchronously, which is distributed (LUT) RAM or flip-flops and
//   costs nothing extra in time. With 1 the two read ports are clocked:
//   the addresses are presented during FETCH and the data is there in
//   EXEC, which is the shape a block RAM has, at no extra cycle because
//   FETCH was going to happen anyway.
//
// What it does not do
//   RV32I only: no M (multiply and divide), no A, no C, no floating
//   point, no B. Machine mode only: no user or supervisor mode, no PMP,
//   no virtual memory, no `mideleg` / `medeleg`, no `mtval`, no
//   `mcountinhibit` and no counters beyond `mcycle` and `minstret` —
//   which are read only here, so a write to them is accepted and
//   ignored. `mtvec` is direct mode only: the low two bits read as zero
//   and vectored mode is not implemented.
//
//   Misaligned loads and stores trap rather than being emulated. There is
//   no bus error input, so a memory that cannot answer must simply not
//   raise `*_ready`; a core hang is the only failure mode. There is no
//   instruction cache, no branch prediction, no pipelining and no
//   forwarding — a multi-cycle machine needs none of it — so the clocks
//   per instruction are 2, or 3 with a data access, plus wait states.
//
//   `x1`..`x31` are undefined until written, exactly as the ISA says
//   they are after reset: the register file is not cleared, because
//   clearing it would cost a reset sequence and a mux, and correct code
//   writes a register before reading it.
//
//   The three interrupt inputs are sampled on `clk` and must already be
//   synchronous to it; put `cdc_sync` in front of an asynchronous one.
//   There is no debug module, no trigger unit, no hardware breakpoint
//   beyond EBREAK and no trace port beyond the three `dbg_*` signals,
//   which exist so a testbench can follow retirement and are free to be
//   left unconnected.
module rv32i #(
    // The address the core fetches from after reset.
    parameter [31:0] RESET_VECTOR = 32'h0000_0000,
    // 1 stores x1..x31 in a clocked (block RAM) array, 0 in an
    // asynchronously read one.
    parameter        REGFILE_BRAM = 0
) (
    input  wire        clk,
    input  wire        rst_n,

    // Instruction port. `imem_req` is held until a rising edge sees
    // `imem_ready`; that edge takes `imem_rdata`.
    output wire [31:0] imem_addr,
    output wire        imem_req,
    input  wire        imem_ready,
    input  wire [31:0] imem_rdata,

    // Data port. `dmem_addr` is word aligned; `dmem_be` selects the byte
    // lanes of a store. Reads are always a whole word.
    output wire [31:0] dmem_addr,
    output wire        dmem_req,
    output wire        dmem_we,
    output wire [3:0]  dmem_be,
    output wire [31:0] dmem_wdata,
    input  wire        dmem_ready,
    input  wire [31:0] dmem_rdata,

    // Machine interrupts, synchronous to `clk`.
    input  wire        irq_timer,
    input  wire        irq_software,
    input  wire        irq_external,

    // Retirement trace: `dbg_pc` is the address of the instruction that
    // retired or trapped on the previous edge.
    output wire [31:0] dbg_pc,
    output wire        dbg_retire,
    output wire        dbg_trap
);
    // -----------------------------------------------------------------
    // Opcodes and the state machine.
    // -----------------------------------------------------------------
    localparam [6:0] OP_LUI    = 7'b0110111;
    localparam [6:0] OP_AUIPC  = 7'b0010111;
    localparam [6:0] OP_JAL    = 7'b1101111;
    localparam [6:0] OP_JALR   = 7'b1100111;
    localparam [6:0] OP_BRANCH = 7'b1100011;
    localparam [6:0] OP_LOAD   = 7'b0000011;
    localparam [6:0] OP_STORE  = 7'b0100011;
    localparam [6:0] OP_IMM    = 7'b0010011;
    localparam [6:0] OP_REG    = 7'b0110011;
    localparam [6:0] OP_FENCE  = 7'b0001111;
    localparam [6:0] OP_SYSTEM = 7'b1110011;

    localparam [1:0] S_FETCH = 2'd0;
    localparam [1:0] S_EXEC  = 2'd1;
    localparam [1:0] S_MEM   = 2'd2;

    // RV32I, no extensions: bits 30:0 name the letters, 31:30 the width.
    localparam [31:0] MISA = 32'h4000_0100;

    reg [1:0]  state;
    reg [31:0] pc;
    reg [31:0] ir;

    // -----------------------------------------------------------------
    // Decode.
    // -----------------------------------------------------------------
    wire [6:0]  opcode = ir[6:0];
    wire [4:0]  rd_a   = ir[11:7];
    wire [2:0]  funct3 = ir[14:12];
    wire [4:0]  rs1_a  = ir[19:15];
    wire [4:0]  rs2_a  = ir[24:20];
    wire [6:0]  funct7 = ir[31:25];
    wire [11:0] csr_a  = ir[31:20];

    wire is_lui    = (opcode == OP_LUI);
    wire is_auipc  = (opcode == OP_AUIPC);
    wire is_jal    = (opcode == OP_JAL);
    wire is_jalr   = (opcode == OP_JALR);
    wire is_branch = (opcode == OP_BRANCH);
    wire is_load   = (opcode == OP_LOAD);
    wire is_store  = (opcode == OP_STORE);
    wire is_opimm  = (opcode == OP_IMM);
    wire is_op     = (opcode == OP_REG);
    wire is_fence  = (opcode == OP_FENCE);
    wire is_system = (opcode == OP_SYSTEM);

    // The privileged instructions all have funct3 = 000 and no register
    // operands; everything else in SYSTEM is a CSR access.
    wire priv_shape = is_system & (funct3 == 3'b000) & (rd_a == 5'd0) & (rs1_a == 5'd0);
    wire is_ecall   = priv_shape & (csr_a == 12'h000);
    wire is_ebreak  = priv_shape & (csr_a == 12'h001);
    wire is_mret    = priv_shape & (csr_a == 12'h302);
    wire is_wfi     = priv_shape & (csr_a == 12'h105);
    wire is_csr     = is_system & (funct3 != 3'b000) & (funct3 != 3'b100);

    wire [31:0] imm_i = {{20{ir[31]}}, ir[31:20]};
    wire [31:0] imm_s = {{20{ir[31]}}, ir[31:25], ir[11:7]};
    wire [31:0] imm_b = {{19{ir[31]}}, ir[31], ir[7], ir[30:25], ir[11:8], 1'b0};
    wire [31:0] imm_u = {ir[31:12], 12'b0};
    wire [31:0] imm_j = {{11{ir[31]}}, ir[31], ir[19:12], ir[20], ir[30:21], 1'b0};

    reg [31:0] imm;
    always @(*) begin
        case (opcode)
            OP_STORE:            imm = imm_s;
            OP_BRANCH:           imm = imm_b;
            OP_LUI, OP_AUIPC:    imm = imm_u;
            OP_JAL:              imm = imm_j;
            default:             imm = imm_i;
        endcase
    end

    // Everything but the two register-operand forms takes the immediate.
    wire use_imm = ~is_op & ~is_branch;

    // -----------------------------------------------------------------
    // Legality. An instruction that is not in this list traps with
    // cause 2 rather than doing something unspecified.
    // -----------------------------------------------------------------
    wire imm_legal = (funct3 == 3'b001) ? (funct7 == 7'b0000000)
                   : (funct3 == 3'b101) ? ((funct7 == 7'b0000000) | (funct7 == 7'b0100000))
                   : 1'b1;
    wire op_legal  = (funct7 == 7'b0000000)
                   | ((funct7 == 7'b0100000) & ((funct3 == 3'b000) | (funct3 == 3'b101)));

    wire decode_legal =
          is_lui | is_auipc | is_jal
        | (is_jalr   & (funct3 == 3'b000))
        | (is_branch & (funct3 != 3'b010) & (funct3 != 3'b011))
        | (is_load   & (funct3 != 3'b011) & (funct3 != 3'b110) & (funct3 != 3'b111))
        | (is_store  & (funct3[2] == 1'b0) & (funct3 != 3'b011))
        | (is_opimm  & imm_legal)
        | (is_op     & op_legal)
        | (is_fence  & ((funct3 == 3'b000) | (funct3 == 3'b001)))
        | is_csr | is_ecall | is_ebreak | is_mret | is_wfi;

    // -----------------------------------------------------------------
    // The register file.
    // -----------------------------------------------------------------
    reg  [31:0] regs [0:31];
    wire [31:0] rs1_val;
    wire [31:0] rs2_val;
    wire        rf_we;
    wire [4:0]  rf_wa;
    wire [31:0] rf_wd;

    // With a clocked register file the addresses come off the word the
    // fetch is about to latch, so the read lands in the same edge that
    // loads `ir` and EXEC sees the operands with no extra cycle.
    wire        rf_re  = (state == S_FETCH) & imem_ready;
    wire [4:0]  rf_ra1 = (state == S_FETCH) ? imem_rdata[19:15] : rs1_a;
    wire [4:0]  rf_ra2 = (state == S_FETCH) ? imem_rdata[24:20] : rs2_a;

    generate
        if (REGFILE_BRAM != 0) begin : g_rf_clocked
            reg [31:0] rs1_q;
            reg [31:0] rs2_q;
            reg        rs1_zero_q;
            reg        rs2_zero_q;
            always @(posedge clk) begin
                if (rf_re) begin
                    rs1_q      <= regs[rf_ra1];
                    rs2_q      <= regs[rf_ra2];
                    rs1_zero_q <= (rf_ra1 == 5'd0);
                    rs2_zero_q <= (rf_ra2 == 5'd0);
                end
                if (rf_we) regs[rf_wa] <= rf_wd;
            end
            assign rs1_val = rs1_zero_q ? 32'd0 : rs1_q;
            assign rs2_val = rs2_zero_q ? 32'd0 : rs2_q;
        end else begin : g_rf_async
            always @(posedge clk) begin
                if (rf_we) regs[rf_wa] <= rf_wd;
            end
            assign rs1_val = (rs1_a == 5'd0) ? 32'd0 : regs[rs1_a];
            assign rs2_val = (rs2_a == 5'd0) ? 32'd0 : regs[rs2_a];
        end
    endgenerate

    // -----------------------------------------------------------------
    // The ALU: one 33-bit adder serves ADD, SUB, both comparisons and
    // every address calculation, and one shifter serves all three shifts.
    // -----------------------------------------------------------------
    wire [31:0] op_b = use_imm ? imm : rs2_val;

    wire alu_slt    = (is_op | is_opimm) & ((funct3 == 3'b010) | (funct3 == 3'b011));
    wire do_sub     = (is_op & funct7[5] & (funct3 == 3'b000)) | alu_slt | is_branch;
    wire cmp_signed = is_branch ? ~funct3[1] : (funct3 == 3'b010);

    wire [32:0] a_ext  = {rs1_val[31] & cmp_signed, rs1_val};
    wire [32:0] b_ext  = {op_b[31]    & cmp_signed, op_b};
    wire [32:0] addsub = do_sub ? (a_ext - b_ext) : (a_ext + b_ext);
    wire [31:0] sum    = addsub[31:0];
    wire        less   = addsub[32];
    wire        equal  = (rs1_val == op_b);

    wire [4:0]  shamt    = use_imm ? rs2_a : rs2_val[4:0];
    wire        sra_op   = funct7[5] & (funct3 == 3'b101);
    wire [32:0] shr_in   = {sra_op & rs1_val[31], rs1_val};
    wire [32:0] shr_full = $signed(shr_in) >>> shamt;
    wire [31:0] shr_res  = shr_full[31:0];
    wire [31:0] shl_res  = rs1_val << shamt;

    reg [31:0] alu_result;
    always @(*) begin
        case (funct3)
            3'b000:  alu_result = sum;
            3'b001:  alu_result = shl_res;
            3'b010:  alu_result = {31'd0, less};
            3'b011:  alu_result = {31'd0, less};
            3'b100:  alu_result = rs1_val ^ op_b;
            3'b101:  alu_result = shr_res;
            3'b110:  alu_result = rs1_val | op_b;
            default: alu_result = rs1_val & op_b;
        endcase
    end

    reg cond;
    always @(*) begin
        case (funct3)
            3'b000:  cond = equal;
            3'b001:  cond = ~equal;
            3'b100:  cond = less;
            3'b101:  cond = ~less;
            3'b110:  cond = less;
            default: cond = ~less;
        endcase
    end

    // -----------------------------------------------------------------
    // The program counter.
    // -----------------------------------------------------------------
    wire [31:0] pc_plus4  = pc + 32'd4;
    wire [31:0] pc_target = pc + imm;             // AUIPC, JAL, branches
    wire [31:0] jalr_tgt  = {sum[31:1], 1'b0};
    wire        taken     = is_branch & cond;

    wire [31:0] mepc_r_out;
    reg  [31:0] next_pc;
    always @(*) begin
        if (is_jal)        next_pc = pc_target;
        else if (is_jalr)  next_pc = jalr_tgt;
        else if (taken)    next_pc = pc_target;
        else if (is_mret)  next_pc = mepc_r_out;
        else               next_pc = pc_plus4;
    end

    // -----------------------------------------------------------------
    // Memory access shaping.
    // -----------------------------------------------------------------
    wire [1:0] byte_off = sum[1:0];
    wire       half_bad = byte_off[0];
    wire       word_bad = byte_off[0] | byte_off[1];
    wire       size_h   = (funct3[1:0] == 2'b01);
    wire       size_w   = (funct3[1:0] == 2'b10);
    wire       ld_bad   = is_load  & ((size_h & half_bad) | (size_w & word_bad));
    wire       st_bad   = is_store & ((size_h & half_bad) | (size_w & word_bad));

    reg [3:0]  st_be;
    reg [31:0] st_data;
    always @(*) begin
        case (funct3[1:0])
            2'b00: begin
                st_be   = 4'b0001 << byte_off;
                st_data = {4{rs2_val[7:0]}};
            end
            2'b01: begin
                st_be   = byte_off[1] ? 4'b1100 : 4'b0011;
                st_data = {2{rs2_val[15:0]}};
            end
            default: begin
                st_be   = 4'b1111;
                st_data = rs2_val;
            end
        endcase
    end

    wire [7:0]  ld_byte = byte_off[1] ? (byte_off[0] ? dmem_rdata[31:24] : dmem_rdata[23:16])
                                      : (byte_off[0] ? dmem_rdata[15:8]  : dmem_rdata[7:0]);
    wire [15:0] ld_half = byte_off[1] ? dmem_rdata[31:16] : dmem_rdata[15:0];

    reg [31:0] ld_data;
    always @(*) begin
        case (funct3)
            3'b000:  ld_data = {{24{ld_byte[7]}}, ld_byte};
            3'b001:  ld_data = {{16{ld_half[15]}}, ld_half};
            3'b100:  ld_data = {24'd0, ld_byte};
            3'b101:  ld_data = {16'd0, ld_half};
            default: ld_data = dmem_rdata;
        endcase
    end

    // -----------------------------------------------------------------
    // Machine-mode CSRs.
    // -----------------------------------------------------------------
    reg        mstatus_mie;
    reg        mstatus_mpie;
    reg [31:0] mtvec_r;
    reg [31:0] mepc_r;
    reg [31:0] mcause_r;
    reg [31:0] mie_r;
    reg [63:0] mcycle_r;
    reg [63:0] minstret_r;

    assign mepc_r_out = mepc_r;

    wire [31:0] mstatus_w = {19'd0, 2'b11, 3'd0, mstatus_mpie, 3'd0, mstatus_mie, 3'd0};
    wire [31:0] mip_w     = {20'd0, irq_external, 3'd0, irq_timer, 3'd0, irq_software, 3'd0};

    reg [31:0] csr_rdata;
    reg        csr_known;
    always @(*) begin
        csr_rdata = 32'd0;
        csr_known = 1'b1;
        case (csr_a)
            12'h300: csr_rdata = mstatus_w;
            12'h301: csr_rdata = MISA;
            12'h304: csr_rdata = mie_r;
            12'h305: csr_rdata = mtvec_r;
            12'h341: csr_rdata = mepc_r;
            12'h342: csr_rdata = mcause_r;
            12'h344: csr_rdata = mip_w;
            12'hB00: csr_rdata = mcycle_r[31:0];
            12'hB02: csr_rdata = minstret_r[31:0];
            12'hB80: csr_rdata = mcycle_r[63:32];
            12'hB82: csr_rdata = minstret_r[63:32];
            12'hF11, 12'hF12, 12'hF13, 12'hF14: csr_rdata = 32'd0;
            default: csr_known = 1'b0;
        endcase
    end

    // The top two bits of a CSR address say whether it is read only.
    wire csr_ro    = (csr_a[11:10] == 2'b11);
    wire csr_wsel  = (funct3[1:0] == 2'b01);          // CSRRW / CSRRWI
    wire csr_we    = is_csr & (csr_wsel | (rs1_a != 5'd0));
    wire [31:0] csr_src = funct3[2] ? {27'd0, rs1_a} : rs1_val;

    reg [31:0] csr_wdata;
    always @(*) begin
        case (funct3[1:0])
            2'b01:   csr_wdata = csr_src;
            2'b10:   csr_wdata = csr_rdata | csr_src;
            default: csr_wdata = csr_rdata & ~csr_src;
        endcase
    end

    wire csr_bad = is_csr & (~csr_known | (csr_we & csr_ro));

    // -----------------------------------------------------------------
    // Traps.
    // -----------------------------------------------------------------
    wire [31:0] irq_active = mip_w & mie_r;
    wire        irq_now    = mstatus_mie & (irq_active != 32'd0);
    wire [31:0] irq_cause  = irq_active[11] ? 32'h8000_000B
                           : irq_active[3]  ? 32'h8000_0003
                           :                  32'h8000_0007;

    wire illegal  = ~decode_legal | csr_bad;
    wire pc_bad   = ~is_load & ~is_store & (next_pc[1:0] != 2'b00);
    wire mem_bad  = ld_bad | st_bad;
    wire trap_now = illegal | is_ecall | is_ebreak | mem_bad | pc_bad;

    reg [31:0] exc_cause;
    always @(*) begin
        if      (illegal)  exc_cause = 32'd2;
        else if (is_ecall) exc_cause = 32'd11;
        else if (is_ebreak)exc_cause = 32'd3;
        else if (ld_bad)   exc_cause = 32'd4;
        else if (st_bad)   exc_cause = 32'd6;
        else               exc_cause = 32'd0;
    end

    // -----------------------------------------------------------------
    // What each state does.
    // -----------------------------------------------------------------
    wire take_irq    = (state == S_FETCH) & irq_now;
    wire exec_now    = (state == S_EXEC);
    wire take_exc    = exec_now & trap_now;
    wire to_mem      = exec_now & ~trap_now & (is_load | is_store);
    wire commit_exec = exec_now & ~trap_now & ~is_load & ~is_store;
    wire commit_mem  = (state == S_MEM) & dmem_ready;
    wire retire      = commit_exec | commit_mem;

    wire writes_rd = is_lui | is_auipc | is_jal | is_jalr | is_opimm | is_op | is_csr;

    reg [31:0] exec_rd;
    always @(*) begin
        if      (is_lui)            exec_rd = imm_u;
        else if (is_auipc)          exec_rd = pc_target;
        else if (is_jal | is_jalr)  exec_rd = pc_plus4;
        else if (is_csr)            exec_rd = csr_rdata;
        else                        exec_rd = alu_result;
    end

    assign rf_we = (commit_exec & writes_rd & (rd_a != 5'd0))
                 | (commit_mem  & is_load   & (rd_a != 5'd0));
    assign rf_wa = rd_a;
    assign rf_wd = commit_mem ? ld_data : exec_rd;

    assign imem_addr  = pc;
    assign imem_req   = (state == S_FETCH) & ~irq_now;
    assign dmem_addr  = {sum[31:2], 2'b00};
    assign dmem_req   = (state == S_MEM);
    assign dmem_we    = (state == S_MEM) & is_store;
    assign dmem_be    = is_store ? st_be : 4'b1111;
    assign dmem_wdata = st_data;

    // -----------------------------------------------------------------
    // The sequential core.
    // -----------------------------------------------------------------
    reg [31:0] dbg_pc_q;
    reg        dbg_retire_q;
    reg        dbg_trap_q;

    assign dbg_pc     = dbg_pc_q;
    assign dbg_retire = dbg_retire_q;
    assign dbg_trap   = dbg_trap_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state        <= S_FETCH;
            pc           <= RESET_VECTOR;
            ir           <= 32'd0;
            mstatus_mie  <= 1'b0;
            mstatus_mpie <= 1'b0;
            mtvec_r      <= 32'd0;
            mepc_r       <= 32'd0;
            mcause_r     <= 32'd0;
            mie_r        <= 32'd0;
            mcycle_r     <= 64'd0;
            minstret_r   <= 64'd0;
            dbg_pc_q     <= 32'd0;
            dbg_retire_q <= 1'b0;
            dbg_trap_q   <= 1'b0;
        end else begin
            mcycle_r     <= mcycle_r + 64'd1;
            dbg_retire_q <= retire;
            dbg_trap_q   <= take_irq | take_exc;
            if (retire | take_irq | take_exc) dbg_pc_q <= pc;
            if (retire) minstret_r <= minstret_r + 64'd1;

            if (take_irq) begin
                // Between instructions: nothing has run, so `mepc` is
                // the instruction that has not run yet.
                mepc_r       <= pc;
                mcause_r     <= irq_cause;
                mstatus_mpie <= mstatus_mie;
                mstatus_mie  <= 1'b0;
                pc           <= {mtvec_r[31:2], 2'b00};
            end else begin
                case (state)
                    S_FETCH: begin
                        if (imem_ready) begin
                            ir    <= imem_rdata;
                            state <= S_EXEC;
                        end
                    end
                    S_EXEC: begin
                        if (trap_now) begin
                            mepc_r       <= pc;
                            mcause_r     <= exc_cause;
                            mstatus_mpie <= mstatus_mie;
                            mstatus_mie  <= 1'b0;
                            pc           <= {mtvec_r[31:2], 2'b00};
                            state        <= S_FETCH;
                        end else if (is_load | is_store) begin
                            state <= S_MEM;
                        end else begin
                            if (is_mret) begin
                                mstatus_mie  <= mstatus_mpie;
                                mstatus_mpie <= 1'b1;
                            end
                            if (is_csr & csr_we) begin
                                case (csr_a)
                                    12'h300: begin
                                        mstatus_mie  <= csr_wdata[3];
                                        mstatus_mpie <= csr_wdata[7];
                                    end
                                    12'h304: mie_r   <= csr_wdata & 32'h0000_0888;
                                    12'h305: mtvec_r <= {csr_wdata[31:2], 2'b00};
                                    12'h341: mepc_r  <= {csr_wdata[31:2], 2'b00};
                                    12'h342: mcause_r<= csr_wdata;
                                    // misa, mip and the counters are
                                    // fixed here: the write is legal and
                                    // has no effect.
                                    default: mie_r   <= mie_r;
                                endcase
                            end
                            pc    <= next_pc;
                            state <= S_FETCH;
                        end
                    end
                    default: begin
                        if (dmem_ready) begin
                            pc    <= pc_plus4;
                            state <= S_FETCH;
                        end
                    end
                endcase
            end
        end
    end
endmodule
