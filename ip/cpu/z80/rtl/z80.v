// z80 — a T-state accurate Zilog Z80 core.
//
// What it does
//   The unprefixed opcode space in full — all 256 encodings — and the
//   whole of the `CB` space: the eight rotates and shifts, `BIT`, `RES`
//   and `SET` on every register and on `(HL)`. That is the 8-bit load
//   and arithmetic set, the 16-bit loads and `ADD HL,rp`, the stack,
//   the jumps and calls including every condition, `DJNZ`, `RST`,
//   `IN A,(n)` / `OUT (n),A`, `EX` in all four forms, `EXX`, `DAA`,
//   `HALT`, `DI` / `EI`, and `NMI` and `INT` in interrupt mode 1.
//
//   This is a reimplementation from the published description of the
//   part — a machine's observable behaviour, not anyone's creative
//   work. No ROM, no system software and no vendor netlist is in this
//   package, and none is needed to run it.
//
//   **What is not here, and traps rather than guesses:** the `ED`,
//   `DD` and `FD` prefixes, and therefore `IX`, `IY`, the block
//   instructions, the 16-bit `ADC` / `SBC`, `LD I,A`, `IM 0` / `IM 2`
//   and `RETN` / `RETI`. Fetching one of those three bytes raises
//   `dbg_trap` for a cycle and the core treats it as a `NOP`, so a
//   program that reaches one says so instead of quietly computing
//   something else. `BUSRQ` / `BUSAK` are not implemented either: this
//   core never gives up the bus.
//
// The bus is the part's own, and that is the point
//   `M1`, `MREQ`, `IORQ`, `RD`, `WR`, `RFSH`, `HALT` and `WAIT`, all
//   active low, because the value of a Z80 is dropping it into a system
//   built from published timings. A Z80's documentation counts
//   **T-states**, not bus accesses, and the counts depend on which
//   M-cycle of an instruction you are in: an opcode fetch is four
//   T-states and sometimes five or six, a memory access three and
//   sometimes four or five, an I/O access four. So this core is
//   sequenced in T-states and an instruction takes exactly the number
//   the tables print. `mos6502` in this library made the opposite
//   choice — one bus access per clock — because a 6502's documentation
//   counts accesses. Each core follows its own reference.
//
//   The data bus is split into `din`, `dout` and `dout_oe` rather than
//   being one bidirectional port, which is this library's idiom for a
//   bus a pad drives: see `usb_device_ulpi`. `dout_oe` is high for
//   exactly the T-states in which the real part drives D.
//
//   `din` is sampled at the **end of the T-state in which the real
//   part latches it** — T3 of an opcode fetch or a memory read, the
//   last T-state of an I/O read. A synchronous core has no falling
//   edge to use, and this is the edge that gives a memory the same
//   setup time the part's own timing diagram asks for.
//
//   `WAIT` is sampled in T2 of a memory cycle and in the wait state of
//   an I/O cycle, as the part does, and extends that T-state for as
//   long as it is low. An M1 cycle's refresh half does not sample it.
//
//   `R` counts. The low seven bits increment after every opcode fetch
//   and the refresh address `{I, R}` goes out in T3 and T4 of M1 with
//   `RFSH` low. `I` is a register this core cannot load — `LD I,A` is
//   an `ED` opcode — so the refresh address's high byte is zero, which
//   is what it is after a reset in any case. A dynamic memory being
//   refreshed sees the right sequence of row addresses.
//
// The T-state shapes
//   Every instruction is one of the shapes below. The number in each
//   M-cycle is its T-state count, and the total is the figure Zilog
//   prints. Where a condition decides, both are given.
//
//     NOP, LD r,r', ALU A,r, rotates of A, EX DE,HL, EX AF,AF',
//     EXX, JP (HL), DI, EI, HALT, DAA, CPL, SCF, CCF
//                                     M1(4)                        = 4
//     LD r,n / LD r,(HL) / LD (HL),r / ALU A,n / ALU A,(HL)
//                                     M1(4) + 3                    = 7
//     LD (HL),n                       M1(4) + 3 + 3                = 10
//     LD rp,nn / POP rp / RET / JP nn / JP cc,nn
//                                     M1(4) + 3 + 3                = 10
//     LD A,(BC) / LD A,(DE) / LD (BC),A / LD (DE),A
//                                     M1(4) + 3                    = 7
//     LD A,(nn) / LD (nn),A           M1(4) + 3 + 3 + 3            = 13
//     LD HL,(nn) / LD (nn),HL         M1(4) + 3 + 3 + 3 + 3        = 16
//     INC (HL) / DEC (HL)             M1(4) + 4 + 3                = 11
//     INC rp / DEC rp / LD SP,HL      M1(6)                        = 6
//     ADD HL,rp                       M1(4) + 4 + 3                = 11
//     PUSH rp / RST n                 M1(5) + 3 + 3                = 11
//     JR e                            M1(4) + 3 + 5                = 12
//     JR cc,e                         M1(4) + 3 [+ 5]           = 12 / 7
//     DJNZ e                          M1(5) + 3 [+ 5]           = 13 / 8
//     CALL nn                         M1(4) + 3 + 4 + 3 + 3        = 17
//     CALL cc,nn   taken              M1(4) + 3 + 4 + 3 + 3        = 17
//                  not taken          M1(4) + 3 + 3                = 10
//     RET cc                          M1(5) [+ 3 + 3]           = 11 / 5
//     IN A,(n) / OUT (n),A            M1(4) + 3 + 4                = 11
//     EX (SP),HL                      M1(4) + 3 + 4 + 3 + 5        = 19
//     CB rot/res/set r                M1(4) + M1(4)                = 8
//     CB bit r                        M1(4) + M1(4)                = 8
//     CB rot/res/set (HL)             M1(4) + M1(4) + 4 + 3        = 15
//     CB bit (HL)                     M1(4) + M1(4) + 4            = 12
//     NMI                             M1(5) + 3 + 3                = 11
//     INT, mode 1                     M1(6) + 3 + 3                = 13
//
//   An `M1(4)` whose instruction is longer than one byte spends its
//   fetch on the opcode; the operand bytes are the 3-T-state reads
//   after it. `JP cc,nn` reads both operand bytes whether it jumps or
//   not, which is why it is 10 either way and `CALL cc,nn` is not.
//
//   Note where `CALL cc,nn` loses its seven T-states: not only the two
//   writes but **one T-state of the second operand read**, which is
//   where the push begins. A core that kept that read at four either
//   way comes out at 11 where the reference says 10, which is the kind
//   of thing only a count catches.
//
// The flags, including the two that are not documented
//   F is `S Z Y H X P/V N C` from bit 7 down. `Y` (bit 5) and `X`
//   (bit 3) are the two no documented instruction reads, and which
//   real programs test anyway; `UNDOCUMENTED_FLAGS` builds them. They
//   are bits 5 and 3 of the result for almost everything, and the
//   exceptions are the interesting part:
//
//     CP A,s        X and Y come from the **operand**, not from the
//                   difference, which is the one place the result is
//                   discarded and the flags are not.
//     SCF, CCF      from A, since there is no result.
//     ADD HL,rp     from the high byte of the sum.
//     BIT n,r       from the operand.
//
//   `BIT n,(HL)` is a stated deviation: on the real part X and Y come
//   from an internal address latch, and here they come from the byte
//   fetched. Nothing documented reads them, and reproducing the latch
//   means modelling a register the part does not expose.
//
//   `P/V` is parity for the logical operations and overflow for the
//   arithmetic ones, which is one bit doing two jobs and the reason
//   `AND`, `OR` and `XOR` cannot be folded into the adder's flag path.
//
//   After a reset: `PC` and `I` and `R` are zero, both interrupt
//   enables are clear, and `AF` and `SP` are `FFFF` — the state a Z80
//   comes up in, not zero.
//
// What a reader should check before trusting a number here
//   The T-state counts above are quoted from the Zilog tables. What
//   measures them is `tests/z80_asm/mod.rs`, whose opcode table and
//   cycle counts are typed from the same reference and **not** from
//   this decoder: a testbench that assembled its programs with the
//   core it is testing would agree with it however wrong both were.
module z80 #(
    // 1 builds bits 3 and 5 of F, which no documented instruction
    // reads; 0 leaves them reading zero and does not synthesise the
    // logic that derives them.
    parameter UNDOCUMENTED_FLAGS = 1
) (
    input  wire        clk,
    input  wire        rst_n,

    // The bus, with the part's own names and polarities.
    output wire [15:0] addr,
    output wire [7:0]  dout,
    input  wire [7:0]  din,
    output wire        dout_oe,
    output wire        m1_n,
    output wire        mreq_n,
    output wire        iorq_n,
    output wire        rd_n,
    output wire        wr_n,
    output wire        rfsh_n,
    output wire        halt_n,
    // Low extends the current T-state, sampled where the part samples
    // it: T2 of a memory cycle, the wait state of an I/O cycle.
    input  wire        wait_n,

    // Interrupt inputs, active low. `nmi_n` is edge triggered and
    // `int_n` is level sensitive, as on the part.
    input  wire        int_n,
    input  wire        nmi_n,

    // Retirement trace, the same three signals `mos6502` offers.
    // `dbg_pc` is the address of the opcode whose instruction finished
    // on the previous edge.
    output wire [15:0] dbg_pc,
    output wire        dbg_retire,
    output wire        dbg_trap
);

    // ------------------------------------------------------------------
    // THE REGISTERS
    // ------------------------------------------------------------------
    // **Plain registers, not an indexed file.** Sixteen bytes behind a
    // variable index cost eleven 16:1 byte multiplexers to read and
    // eighty-one write ports to maintain, which is where more than half
    // of this core's area went; `docs/ip-library.md` had the count. A
    // named register costs one multiplexer on its own D input and
    // nothing at all to read.
    //
    // The alternate set is **swapped** rather than selected, so `EXX`
    // and `EX AF,AF'` move bytes instead of flipping a bit that then
    // appears in every index. Both are single-M-cycle instructions and
    // have the cycle to spare, and a byte swap between two registers is
    // a multiplexer each and no decode.
    reg [7:0] r_b = 8'h00, r_c = 8'h00, r_d = 8'h00;
    reg [7:0] r_e = 8'h00, r_h = 8'h00, r_l = 8'h00;
    reg [7:0] r_a = 8'hff, r_f = 8'hff;
    reg [7:0] x_b = 8'h00, x_c = 8'h00, x_d = 8'h00;
    reg [7:0] x_e = 8'h00, x_h = 8'h00, x_l = 8'h00;
    reg [7:0] x_a = 8'hff, x_f = 8'hff;

    reg [15:0] pc = 16'h0000;
    reg [15:0] sp = 16'hffff;
    reg [15:0] wz = 16'h0000;    // the operand latch, W high and Z low
    reg [7:0]  ir = 8'h00;       // the opcode being executed
    reg [7:0]  tmp = 8'h00;      // a fetched byte on its way through
    reg [6:0]  rreg = 7'h00;     // the refresh counter's low seven bits
    reg        rbit7 = 1'b0;     // which only software can change

    reg        iff1 = 1'b0;
    reg        iff2 = 1'b0;
    reg        halted = 1'b0;
    reg        ei_pending = 1'b0;   // EI enables after the next opcode
    reg        nmi_seen = 1'b0;     // the edge, latched until taken
    reg        nmi_last = 1'b1;
    reg [7:0]  cbop = 8'h00;     // the opcode after a CB prefix
    reg        in_nmi = 1'b0;    // an interrupt is being taken, and the
    reg        in_int = 1'b0;    // shape has to stay its shape
    reg        trap = 1'b0;
    // The retirement trace. Registered, so it describes the instruction
    // that ended on the previous edge rather than the T-state it is in:
    // that is what this module's header promises, what `mos6502` does,
    // and what lets a bench read it after taking the edge.
    reg        retire_q = 1'b0;
    reg [15:0] retire_pc = 16'h0000;
    reg [15:0] op_pc = 16'h0000;    // the opcode being executed

    wire [7:0] a_reg = r_a;
    wire [7:0] f_reg = r_f;
    wire [7:0] reg_b = r_b;
    wire [7:0] reg_c = r_c;
    wire [7:0] reg_d = r_d;
    wire [7:0] reg_e = r_e;
    wire [7:0] reg_h = r_h;
    wire [7:0] reg_l = r_l;
    wire [15:0] bc = {r_b, r_c};
    wire [15:0] de = {r_d, r_e};
    wire [15:0] hl = {r_h, r_l};

    // One byte of the `r[]` encoding: 0..5 are B C D E H L, 6 is `(HL)`
    // and never reaches here, and 7 is A.
    function [7:0] rd8;
        input [2:0] idx;
        begin
            case (idx)
                3'd0: rd8 = r_b;
                3'd1: rd8 = r_c;
                3'd2: rd8 = r_d;
                3'd3: rd8 = r_e;
                3'd4: rd8 = r_h;
                3'd5: rd8 = r_l;
                3'd7: rd8 = r_a;
                default: rd8 = 8'h00;
            endcase
        end
    endfunction

    // Flag bit positions, named because `f_reg[2]` meaning overflow in
    // one instruction and parity in the next is the subject of a
    // paragraph in the header.
    localparam F_C = 0, F_N = 1, F_PV = 2, F_X = 3,
               F_H = 4, F_Y = 5, F_Z = 6, F_S = 7;

    // ------------------------------------------------------------------
    // THE OPCODE, TAKEN APART
    // ------------------------------------------------------------------
    // Every Z80 reference lays the unprefixed space out as an octal
    // matrix, and the decode below is that matrix: x selects the
    // quadrant, z the column, and y the row, with y split into p and q
    // where a row pairs up.
    wire [1:0] x = ir[7:6];
    wire [2:0] y = ir[5:3];
    wire [2:0] z = ir[2:0];
    wire [1:0] p = ir[5:4];
    wire       q = ir[3];

    // The three prefixes this core does not decode. They have to be
    // named here because the matrix does not leave holes for them:
    // `ED`, `DD` and `FD` all land on `x = 3, z = 5, q = 1`, which is
    // `CALL nn`, so a core that only *documented* them as unimplemented
    // would quietly push a return address and jump.
    wire ir_is_prefix = (ir == 8'hed) || (ir == 8'hdd) || (ir == 8'hfd);

    // The condition codes, in the order the matrix puts them. The
    // fourth quadrant names all eight in `y`; `JR cc,e` has room for
    // only the first four and puts them in `y - 4`, so the index is not
    // the same field in both places.
    wire [2:0] cc_idx = (x == 2'd0) ? {1'b0, y[1:0]} : y;
    reg cond_met;
    always @(*) begin
        case (cc_idx[2:1])
            2'd0: cond_met = (f_reg[F_Z]  == cc_idx[0]);   // NZ, Z
            2'd1: cond_met = (f_reg[F_C]  == cc_idx[0]);   // NC, C
            2'd2: cond_met = (f_reg[F_PV] == cc_idx[0]);   // PO, PE
            2'd3: cond_met = (f_reg[F_S]  == cc_idx[0]);   // P,  M
        endcase
    end

    // The register a `LD (HL),r` or an ALU operation reads, and the one
    // a `LD r,n` or `LD r,(HL)` writes.
    wire [7:0] gp_rd = rd8(z);
    wire [7:0] gp_rd_y = rd8(y);

    // `POP rp` and `RET` take their two bytes off the stack; `LD rp,nn`
    // and the jumps take theirs from the program. Of the `q == 1` row
    // only `RET` reaches this shape at all -- `EXX`, `JP (HL)` and
    // `LD SP,HL` have no bus cycle after the fetch.
    wire is_pop   = (x == 2'd3) && (z == 3'd1);

    // ------------------------------------------------------------------
    // THE ALU
    // ------------------------------------------------------------------
    // One 8-bit adder serving ADD, ADC, SUB, SBC and CP, with the
    // logical operations beside it because P/V is parity for those and
    // overflow for these.
    reg  [2:0] alu_op;
    reg  [7:0] alu_rhs;

    wire       alu_sub = (alu_op == 3'd2) || (alu_op == 3'd3) || (alu_op == 3'd7);
    wire       alu_cin = ((alu_op == 3'd1) || (alu_op == 3'd3)) ? f_reg[F_C] : 1'b0;
    wire [7:0] alu_b   = alu_sub ? ~alu_rhs : alu_rhs;
    wire [8:0] alu_sum = {1'b0, a_reg} + {1'b0, alu_b} + {8'd0, alu_sub ^ alu_cin};
    wire [7:0] alu_add = alu_sum[7:0];
    // A subtraction is an addition of the complement, so the adder's
    // carry out is the borrow's complement.
    wire       alu_cout = alu_sub ? ~alu_sum[8] : alu_sum[8];
    wire       alu_half = a_reg[4] ^ alu_rhs[4] ^ alu_add[4];
    // Signed overflow: set when a subtraction's operands differ in sign
    // and the difference takes the wrong one, or when an addition's
    // agree and the sum does not.
    wire       alu_ovf  = alu_sub
                          ? ( (a_reg[7] ^ alu_rhs[7]) & (a_reg[7] ^ alu_add[7]))
                          : (~(a_reg[7] ^ alu_rhs[7]) & (a_reg[7] ^ alu_add[7]));
    // Which byte the two undocumented flags come from: the difference,
    // except for CP, which keeps the flags and throws the difference
    // away and takes them from what it compared against.
    wire [7:0] alu_xy   = (alu_op == 3'd7) ? alu_rhs : alu_add;

    reg [7:0] logic_res;
    always @(*) begin
        case (alu_op)
            3'd4: logic_res = a_reg & alu_rhs;
            3'd5: logic_res = a_reg ^ alu_rhs;
            default: logic_res = a_reg | alu_rhs;
        endcase
    end
    wire logic_parity = ~(^logic_res);

    // The two flag words are built separately and then chosen between,
    // because bit 2 is parity for AND, OR and XOR and signed overflow
    // for the four that use the adder -- one bit of F doing two jobs,
    // which is the reason the logical operations cannot share the
    // adder's flag path.
    wire alu_is_logic = alu_op[2] && (alu_op != 3'd7);
    wire [7:0] logic_f = {logic_res[7], logic_res == 8'h00,
                          UNDOCUMENTED_FLAGS ? logic_res[5] : 1'b0,
                          alu_op == 3'd4,
                          UNDOCUMENTED_FLAGS ? logic_res[3] : 1'b0,
                          logic_parity, 1'b0, 1'b0};
    wire [7:0] arith_f = {alu_add[7], alu_add == 8'h00,
                          UNDOCUMENTED_FLAGS ? alu_xy[5] : 1'b0,
                          alu_half,
                          UNDOCUMENTED_FLAGS ? alu_xy[3] : 1'b0,
                          alu_ovf, alu_sub, alu_cout};
    wire [7:0] alu_res = alu_is_logic ? logic_res : alu_add;
    wire [7:0] alu_f   = alu_is_logic ? logic_f : arith_f;

    // INC and DEC of a byte, which leave C alone — the one arithmetic
    // pair that does.
    reg  [7:0] step_in;
    reg        step_dec;
    wire [7:0] step_res = step_dec ? (step_in - 8'd1) : (step_in + 8'd1);
    reg  [7:0] step_f;
    always @(*) begin
        step_f[F_S]  = step_res[7];
        step_f[F_Z]  = (step_res == 8'h00);
        step_f[F_H]  = step_dec ? (step_in[3:0] == 4'h0) : (step_in[3:0] == 4'hf);
        step_f[F_PV] = step_dec ? (step_in == 8'h80) : (step_in == 8'h7f);
        step_f[F_N]  = step_dec;
        step_f[F_C]  = f_reg[F_C];
        step_f[F_Y]  = UNDOCUMENTED_FLAGS ? step_res[5] : 1'b0;
        step_f[F_X]  = UNDOCUMENTED_FLAGS ? step_res[3] : 1'b0;
    end

    // The CB rotates and shifts, and the rotates of A, which are the
    // same eight shifters with different flag rules.
    reg  [7:0] rot_in;
    reg  [2:0] rot_op;
    reg  [7:0] rot_res;
    reg        rot_cout;
    always @(*) begin
        case (rot_op)
            3'd0: begin rot_res = {rot_in[6:0], rot_in[7]};   rot_cout = rot_in[7]; end // RLC
            3'd1: begin rot_res = {rot_in[0], rot_in[7:1]};   rot_cout = rot_in[0]; end // RRC
            3'd2: begin rot_res = {rot_in[6:0], f_reg[F_C]};  rot_cout = rot_in[7]; end // RL
            3'd3: begin rot_res = {f_reg[F_C], rot_in[7:1]};  rot_cout = rot_in[0]; end // RR
            3'd4: begin rot_res = {rot_in[6:0], 1'b0};        rot_cout = rot_in[7]; end // SLA
            3'd5: begin rot_res = {rot_in[7], rot_in[7:1]};   rot_cout = rot_in[0]; end // SRA
            3'd6: begin rot_res = {rot_in[6:0], 1'b1};        rot_cout = rot_in[7]; end // SLL
            default: begin rot_res = {1'b0, rot_in[7:1]};     rot_cout = rot_in[0]; end // SRL
        endcase
    end
    reg [7:0] rot_f;
    always @(*) begin
        rot_f[F_S]  = rot_res[7];
        rot_f[F_Z]  = (rot_res == 8'h00);
        rot_f[F_H]  = 1'b0;
        rot_f[F_PV] = ~(^rot_res);
        rot_f[F_N]  = 1'b0;
        rot_f[F_C]  = rot_cout;
        rot_f[F_Y]  = UNDOCUMENTED_FLAGS ? rot_res[5] : 1'b0;
        rot_f[F_X]  = UNDOCUMENTED_FLAGS ? rot_res[3] : 1'b0;
    end

    // DAA, whose table is the one piece of the part that is easier to
    // write down than to describe.
    reg [7:0] daa_res;
    reg       daa_c;
    reg       daa_h;
    always @(*) begin
        daa_res = a_reg;
        daa_c   = f_reg[F_C];
        daa_h   = 1'b0;
        if (f_reg[F_N]) begin
            if (f_reg[F_H]) daa_res = daa_res - 8'h06;
            if (f_reg[F_C]) daa_res = daa_res - 8'h60;
            daa_h = f_reg[F_H] && (a_reg[3:0] < 4'h6);
        end else begin
            if (f_reg[F_H] || (a_reg[3:0] > 4'h9)) begin
                daa_res = daa_res + 8'h06;
                daa_h   = 1'b1;
            end
            if (f_reg[F_C] || (a_reg > 8'h99)) begin
                daa_res = daa_res + 8'h60;
                daa_c   = 1'b1;
            end
        end
    end

    // The 16-bit add of ADD HL,rp: H crosses bit 11 and C bit 15, and
    // S, Z and P/V are left alone.
    reg  [15:0] add16_rhs;
    wire [16:0] add16_sum = {1'b0, hl} + {1'b0, add16_rhs};
    wire        add16_h   = hl[12] ^ add16_rhs[12] ^ add16_sum[12];

    // ------------------------------------------------------------------
    // THE SHAPES
    // ------------------------------------------------------------------
    // One name per T-state shape in the header's table. The sequencer
    // below turns a shape and an M-cycle index into a bus cycle.
    localparam [4:0]
        SH_ONE      = 5'd0,   // M1(4) and nothing else
        SH_RD8      = 5'd1,   // M1(4) + read 3           immediate or (HL)
        SH_WR8      = 5'd2,   // M1(4) + write 3
        SH_RD_WR    = 5'd3,   // M1(4) + read 3 + write 3  LD (HL),n
        SH_RD16     = 5'd4,   // M1(4) + read 3 + read 3   LD rp,nn / POP / RET / JP
        SH_ADDR_RD  = 5'd5,   // M1(4) + read 3 + read 3 + read 3        LD A,(nn)
        SH_ADDR_WR  = 5'd6,   // M1(4) + read 3 + read 3 + write 3       LD (nn),A
        SH_ADDR_RD16 = 5'd7,  // + read + read                           LD HL,(nn)
        SH_ADDR_WR16 = 5'd8,  // + write + write                         LD (nn),HL
        SH_RMW      = 5'd9,   // M1(4) + read 4 + write 3  INC/DEC (HL)
        SH_M1_6     = 5'd10,  // M1(6)                     INC rp, LD SP,HL
        SH_ADD16    = 5'd11,  // M1(4) + 4 + 3             ADD HL,rp
        SH_PUSH     = 5'd12,  // M1(5) + write + write
        SH_JR       = 5'd13,  // M1(4) + read 3 [+ 5]
        SH_DJNZ     = 5'd14,  // M1(5) + read 3 [+ 5]
        SH_CALL     = 5'd15,  // M1(4) + read 3 + read 4 [+ write + write]
        SH_RET_CC   = 5'd16,  // M1(5) [+ read + read]
        SH_IO       = 5'd17,  // M1(4) + read 3 + io 4
        SH_EXSP     = 5'd18,  // M1(4) + read 3 + read 4 + write 3 + write 5
        SH_CB       = 5'd19,  // M1(4) + M1(4) [+ read 4 [+ write 3]]
        SH_NMI      = 5'd20,  // M1(5) + write + write
        SH_INT      = 5'd21;  // M1(6) + write + write

    reg [2:0] m = 3'd0;        // which M-cycle of the instruction
    reg [2:0] t = 3'd1;        // which T-state of that M-cycle
    reg [2:0] kind;            // what the current M-cycle is
    reg [2:0] tlen;            // how many T-states it takes
    reg       last_cycle;      // the instruction ends with this one

    reg [4:0] shape;
    // True when the opcode's operand is `(HL)` rather than a register,
    // which decides whether a load or an ALU operation needs a bus
    // cycle at all.
    wire at_hl = (z == 3'd6);

    always @(*) begin
        shape = SH_ONE;
        case (x)
            2'd0: case (z)
                3'd0: shape = (y == 3'd0) ? SH_ONE          // NOP        00
                            : (y == 3'd1) ? SH_ONE          // EX AF,AF'  08
                            : (y == 3'd2) ? SH_DJNZ         // DJNZ e     10
                            : (y == 3'd3) ? SH_JR           // JR e       18
                            :               SH_JR;          // JR cc,e 20..38
                3'd1: shape = q ? SH_ADD16 : SH_RD16;       // ADD HL,rp / LD rp,nn
                3'd2: shape = (p == 2'd2) ? (q ? SH_ADDR_RD16 : SH_ADDR_WR16)
                            : (p == 2'd3) ? (q ? SH_ADDR_RD : SH_ADDR_WR)
                            :               (q ? SH_RD8 : SH_WR8);
                3'd3: shape = SH_M1_6;                      // INC rp / DEC rp
                3'd4: shape = (y == 3'd6) ? SH_RMW : SH_ONE; // INC r
                3'd5: shape = (y == 3'd6) ? SH_RMW : SH_ONE; // DEC r
                3'd6: shape = (y == 3'd6) ? SH_RD_WR : SH_RD8; // LD r,n
                default: shape = SH_ONE;                     // the eight on A
            endcase
            2'd1: begin
                // LD r,r', and 76h is HALT rather than LD (HL),(HL).
                if ((y == 3'd6) && (z == 3'd6)) shape = SH_ONE;
                else if (z == 3'd6)             shape = SH_RD8;
                else if (y == 3'd6)             shape = SH_WR8;
                else                            shape = SH_ONE;
            end
            2'd2: shape = at_hl ? SH_RD8 : SH_ONE;          // ALU A,r
            default: case (z)
                3'd0: shape = SH_RET_CC;
                // POP rp when q is clear. When it is set the column
                // holds RET, EXX, JP (HL) and LD SP,HL, and only RET
                // has anything to read: it pops.
                3'd1: shape = q ? ((p == 2'd0) ? SH_RD16            // RET
                                 : (p == 2'd3) ? SH_M1_6            // LD SP,HL
                                 :               SH_ONE)            // EXX, JP (HL)
                                : SH_RD16;                          // POP rp
                3'd2: shape = SH_RD16;                      // JP cc,nn
                3'd3: shape = (y == 3'd0) ? SH_RD16         // JP nn
                            : (y == 3'd1) ? SH_CB
                            : (y == 3'd2) ? SH_IO           // OUT (n),A
                            : (y == 3'd3) ? SH_IO           // IN A,(n)
                            : (y == 3'd4) ? SH_EXSP
                            :               SH_ONE;         // EX DE,HL, DI, EI
                3'd4: shape = SH_CALL;
                3'd5: shape = q ? SH_CALL : SH_PUSH;
                3'd6: shape = SH_RD8;                       // ALU A,n
                default: shape = SH_PUSH;                   // RST n
            endcase
        endcase
        // An undecoded prefix is one M-cycle and raises `dbg_trap`,
        // which is the whole of its behaviour: see `ir_is_prefix`.
        if (ir_is_prefix) shape = SH_ONE;
        // Held for the whole of the interrupt's own three M-cycles:
        // reading it off the live inputs made it vanish after one
        // T-state and the sequencer finish the discarded opcode.
        if (in_nmi) shape = SH_NMI;
        else if (in_int) shape = SH_INT;
    end

    // The CB opcode, taken apart the same way. `cbop` has held it since
    // the rising edge of its own T3, which is before anything below
    // reads it. Taking it off `din` instead would read the refresh
    // address, since that is what the bus carries by the end of an M1.
    wire [7:0] cbo = cbop;
    wire [1:0] cbx = cbo[7:6];
    wire [2:0] cby = cbo[5:3];
    wire [2:0] cbz = cbo[2:0];
    // The operand: a register, or the byte read from `(HL)` — which is
    // on `din` at the end of that read and in `tmp` by the write.
    wire [7:0] cb_val = (cbz == 3'd6) ? ((m == 3'd2) ? din : tmp) : rd8(cbz);
    reg  [7:0] cb_mask;
    always @(*) begin
        case (cby)
            3'd0: cb_mask = 8'h01;
            3'd1: cb_mask = 8'h02;
            3'd2: cb_mask = 8'h04;
            3'd3: cb_mask = 8'h08;
            3'd4: cb_mask = 8'h10;
            3'd5: cb_mask = 8'h20;
            3'd6: cb_mask = 8'h40;
            default: cb_mask = 8'h80;
        endcase
    end

    // What each of those is looking at, in one place rather than
    // scattered through the execution: an operand that comes off the bus
    // is taken from `din` on the edge it arrives, because that is the
    // edge the result is wanted on.
    always @(*) begin
        // ALU A,r / ALU A,n / ALU A,(HL): the operation is `y` in both
        // the second and the fourth quadrant.
        alu_op  = y;
        alu_rhs = (z == 3'd6) ? din : gp_rd;
        // INC r / DEC r, and INC (HL) / DEC (HL).
        step_dec = (z == 3'd5);
        step_in  = (shape == SH_RMW) ? ((m == 3'd1) ? din : tmp) : gp_rd_y;
        // The shifter serves the four rotates of A and the whole of the
        // CB quadrant below it.
        rot_in = (shape == SH_CB) ? cb_val : a_reg;
        rot_op = (shape == SH_CB) ? cby : y;
        case (p)
            2'd0: add16_rhs = bc;
            2'd1: add16_rhs = de;
            2'd2: add16_rhs = hl;
            default: add16_rhs = sp;
        endcase
    end

    // The CB result and its flags. RES and SET leave every flag alone,
    // which is why they cannot share the shifter's flag path.
    // BIT puts the same answer in P/V as in Z, and S only when it
    // tested bit 7. X and Y come from the operand; see the header on
    // what the real part does for `BIT n,(HL)` and this does not.
    wire       bit_z = ~|(cb_val & cb_mask);
    wire [7:0] bit_f = {(cby == 3'd7) && cb_val[7], bit_z,
                        UNDOCUMENTED_FLAGS ? cb_val[5] : 1'b0, 1'b1,
                        UNDOCUMENTED_FLAGS ? cb_val[3] : 1'b0, bit_z,
                        1'b0, f_reg[F_C]};
    reg [7:0] cb_res;
    reg [7:0] cb_f;
    always @(*) begin
        case (cbx)
            2'd0: begin cb_res = rot_res;             cb_f = rot_f; end
            2'd1: begin cb_res = cb_val;              cb_f = bit_f; end
            2'd2: begin cb_res = cb_val & ~cb_mask;   cb_f = f_reg; end
            default: begin cb_res = cb_val | cb_mask; cb_f = f_reg; end
        endcase
    end

    // ------------------------------------------------------------------
    // THE SEQUENCER
    // ------------------------------------------------------------------
    localparam [2:0] CY_M1 = 3'd0, CY_RD = 3'd1, CY_WR = 3'd2,
                     CY_IOR = 3'd3, CY_IOW = 3'd4, CY_NONE = 3'd5;

    // `JP cc,nn` reads both operand bytes either way; `CALL cc` and
    // `RET cc` and the relative jumps are the ones a condition
    // shortens, and `in_cb` decides whether the CB shape has a memory
    // half at all.
    wire cb_at_hl = (cbz == 3'd6);
    wire cb_is_bit = (cbx == 2'd1);

    always @(*) begin
        kind       = CY_M1;
        tlen       = 3'd4;
        last_cycle = 1'b1;
        case (shape)
            SH_ONE:  begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b1; end
            SH_M1_6: begin kind = CY_M1; tlen = 3'd6; last_cycle = 1'b1; end
            SH_RD8: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                default: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_WR8: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                default: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_RD_WR: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b0; end
                default: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_RD16: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b0; end
                default: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_ADDR_RD, SH_ADDR_WR: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b0; end
                3'd2: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b0; end
                default: begin
                    kind = (shape == SH_ADDR_RD) ? CY_RD : CY_WR;
                    tlen = 3'd3;
                    last_cycle = 1'b1;
                end
            endcase
            SH_ADDR_RD16, SH_ADDR_WR16: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1, 3'd2: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b0; end
                3'd3: begin
                    kind = (shape == SH_ADDR_RD16) ? CY_RD : CY_WR;
                    tlen = 3'd3;
                    last_cycle = 1'b0;
                end
                default: begin
                    kind = (shape == SH_ADDR_RD16) ? CY_RD : CY_WR;
                    tlen = 3'd3;
                    last_cycle = 1'b1;
                end
            endcase
            SH_RMW: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin kind = CY_RD; tlen = 3'd4; last_cycle = 1'b0; end
                default: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_ADD16: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin kind = CY_NONE; tlen = 3'd4; last_cycle = 1'b0; end
                default: begin kind = CY_NONE; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_PUSH, SH_NMI: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd5; last_cycle = 1'b0; end
                3'd1: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b0; end
                default: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_INT: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd6; last_cycle = 1'b0; end
                3'd1: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b0; end
                default: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_JR: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin
                    kind = CY_RD; tlen = 3'd3;
                    // An unconditional JR always takes the extra five.
                    last_cycle = (y == 3'd3) ? 1'b0 : ~cond_met;
                end
                default: begin kind = CY_NONE; tlen = 3'd5; last_cycle = 1'b1; end
            endcase
            SH_DJNZ: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd5; last_cycle = 1'b0; end
                3'd1: begin kind = CY_RD; tlen = 3'd3; last_cycle = (reg_b == 8'h00); end
                default: begin kind = CY_NONE; tlen = 3'd5; last_cycle = 1'b1; end
            endcase
            SH_CALL: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b0; end
                3'd2: begin
                    kind = CY_RD;
                    // Four T-states only when the call is taken: the
                    // extra one is where the push begins, which is why
                    // `CALL cc,nn` is 17 taken and 10 not, and not 11.
                    tlen = (q || cond_met) ? 3'd4 : 3'd3;
                    last_cycle = q ? 1'b0 : ~cond_met;
                end
                3'd3: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b0; end
                default: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_RET_CC: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd5; last_cycle = ~cond_met; end
                3'd1: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b0; end
                default: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
            SH_IO: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b0; end
                default: begin
                    kind = (y == 3'd3) ? CY_IOR : CY_IOW;
                    tlen = 3'd4;
                    last_cycle = 1'b1;
                end
            endcase
            SH_EXSP: case (m)
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin kind = CY_RD; tlen = 3'd3; last_cycle = 1'b0; end
                3'd2: begin kind = CY_RD; tlen = 3'd4; last_cycle = 1'b0; end
                3'd3: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b0; end
                default: begin kind = CY_WR; tlen = 3'd5; last_cycle = 1'b1; end
            endcase
            default: case (m)   // SH_CB
                3'd0: begin kind = CY_M1; tlen = 3'd4; last_cycle = 1'b0; end
                3'd1: begin
                    kind = CY_M1; tlen = 3'd4;
                    last_cycle = ~cb_at_hl;
                end
                3'd2: begin
                    kind = CY_RD; tlen = 3'd4;
                    last_cycle = cb_is_bit;
                end
                default: begin kind = CY_WR; tlen = 3'd3; last_cycle = 1'b1; end
            endcase
        endcase
    end

    // A T-state ends unless WAIT holds it. The part samples WAIT in T2
    // of a memory or opcode cycle and in T3 — the wait state — of an
    // I/O cycle, and nowhere else.
    wire wait_point = ((kind == CY_M1) && (t == 3'd2))
                   || ((kind == CY_RD) && (t == 3'd2))
                   || ((kind == CY_WR) && (t == 3'd2))
                   || (((kind == CY_IOR) || (kind == CY_IOW)) && (t == 3'd3));
    wire stalled  = wait_point && !wait_n;
    wire cyc_end  = (t == tlen) && !stalled;

    // Where `din` is taken: the **rising edge of T3**, which is where
    // the part latches both an opcode and a memory read, and which in a
    // synchronous core is the edge that ends T2. It has to be that edge
    // and not the end of T3: the refresh address goes out during T3, so
    // a core that waited until T3 had ended would latch whatever the
    // refresh address points at instead of the opcode. An I/O read is
    // one T-state longer and latches at the end of its wait state.
    wire sample = !stalled && (((kind == CY_M1) && (t == 3'd2))
                            || ((kind == CY_RD) && (t == 3'd2))
                            || ((kind == CY_IOR) && (t == 3'd3)));

    // ------------------------------------------------------------------
    // INTERRUPTS
    // ------------------------------------------------------------------
    // Both are taken between instructions, which here is the M1 of the
    // next one: the shape becomes the interrupt's and the opcode just
    // fetched is discarded. NMI is the latched edge; INT is the level,
    // and only when EI has taken effect.
    wire at_instr_start = (m == 3'd0) && (t == 3'd1);
    wire take_nmi = at_instr_start && nmi_seen;
    wire take_int = at_instr_start && !nmi_seen && iff1 && !int_n && !ei_pending;

    // ------------------------------------------------------------------
    // THE ADDRESS AND DATA OUT
    // ------------------------------------------------------------------
    reg [15:0] addr_r;
    reg [7:0]  dout_r;

    // The byte a write puts on the bus, chosen by shape and M-cycle.
    always @(*) begin
        dout_r = a_reg;
        case (shape)
            SH_WR8: dout_r = (x == 2'd1) ? gp_rd           // LD (HL),r
                           : (p == 2'd0) ? a_reg           // LD (BC),A
                           :               a_reg;          // LD (DE),A
            SH_RD_WR:     dout_r = tmp;                    // LD (HL),n
            SH_ADDR_WR:   dout_r = a_reg;                  // LD (nn),A
            SH_ADDR_WR16: dout_r = (m == 3'd3) ? reg_l : reg_h;
            SH_RMW:       dout_r = step_res;
            SH_PUSH: begin
                if (z == 3'd7) dout_r = (m == 3'd1) ? pc[15:8] : pc[7:0];  // RST
                else case (p)
                    2'd0: dout_r = (m == 3'd1) ? reg_b : reg_c;
                    2'd1: dout_r = (m == 3'd1) ? reg_d : reg_e;
                    2'd2: dout_r = (m == 3'd1) ? reg_h : reg_l;
                    default: dout_r = (m == 3'd1) ? a_reg : f_reg;
                endcase
            end
            SH_CALL:  dout_r = (m == 3'd3) ? pc[15:8] : pc[7:0];
            SH_NMI, SH_INT: dout_r = (m == 3'd1) ? pc[15:8] : pc[7:0];
            SH_EXSP:  dout_r = (m == 3'd3) ? reg_h : reg_l;
            SH_IO:    dout_r = a_reg;
            SH_CB:    dout_r = cb_res;
            default:  dout_r = a_reg;
        endcase
    end

    always @(*) begin
        addr_r = pc;
        case (shape)
            SH_RD8: addr_r = (m == 3'd0) ? pc
                           : (x == 2'd0 && z == 3'd2) ? ((p == 2'd0) ? bc : de)
                           : ((x == 2'd1) || (x == 2'd2)) ? hl
                           : pc;
            SH_WR8: addr_r = (m == 3'd0) ? pc
                           : (x == 2'd1) ? hl
                           : ((p == 2'd0) ? bc : de);
            SH_RD_WR: addr_r = (m == 3'd1) ? pc : hl;
            SH_RMW:   addr_r = (m == 3'd0) ? pc : hl;
            SH_ADDR_RD, SH_ADDR_WR, SH_ADDR_RD16, SH_ADDR_WR16:
                addr_r = (m <= 3'd2) ? pc
                       : ((m == 3'd3) ? wz : (wz + 16'd1));
            SH_PUSH, SH_NMI, SH_INT:
                addr_r = (m == 3'd0) ? pc
                       : ((m == 3'd1) ? (sp - 16'd1) : (sp - 16'd2));
            SH_CALL: addr_r = (m <= 3'd2) ? pc
                            : ((m == 3'd3) ? (sp - 16'd1) : (sp - 16'd2));
            SH_RD16: addr_r = (m == 3'd0) ? pc
                            : (is_pop ? (sp + {15'd0, (m == 3'd2)}) : pc);
            SH_RET_CC: addr_r = (m == 3'd0) ? pc : (sp + {15'd0, (m == 3'd2)});
            SH_IO:   addr_r = (m <= 3'd1) ? pc : {a_reg, tmp};
            SH_EXSP: addr_r = (m == 3'd0) ? pc
                            : ((m == 3'd1) || (m == 3'd4)) ? sp : (sp + 16'd1);
            SH_CB:   addr_r = (m <= 3'd1) ? pc : hl;
            default: addr_r = pc;
        endcase
        // M1's second half puts the refresh address out instead.
        if ((kind == CY_M1) && (t >= 3'd3)) addr_r = {8'h00, rbit7, rreg};
    end

    assign addr    = addr_r;
    assign dout    = dout_r;
    assign dout_oe = ((kind == CY_WR) || (kind == CY_IOW)) && (t >= 3'd1);
    assign m1_n    = ~((kind == CY_M1) && (t <= 3'd2));
    assign rfsh_n  = ~((kind == CY_M1) && (t >= 3'd3));
    assign mreq_n  = ~(((kind == CY_M1) && ((t == 3'd2) || (t == 3'd3) || (t == 3'd4)))
                    || ((kind == CY_RD) && (t >= 3'd2) && (t <= 3'd3))
                    || ((kind == CY_WR) && (t >= 3'd2) && (t <= 3'd3)));
    assign rd_n    = ~(((kind == CY_M1) && (t >= 3'd2) && (t <= 3'd3))
                    || ((kind == CY_RD) && (t >= 3'd2) && (t <= 3'd3))
                    || ((kind == CY_IOR) && (t >= 3'd2) && (t <= 3'd4)));
    assign wr_n    = ~(((kind == CY_WR) && (t >= 3'd2) && (t <= 3'd3))
                    || ((kind == CY_IOW) && (t >= 3'd2) && (t <= 3'd4)));
    assign iorq_n  = ~(((kind == CY_IOR) || (kind == CY_IOW))
                    && (t >= 3'd2) && (t <= 3'd4));
    assign halt_n  = ~halted;

    assign dbg_pc     = retire_pc;
    assign dbg_retire = retire_q;
    assign dbg_trap   = trap;

    // ------------------------------------------------------------------
    // THE STATE
    // ------------------------------------------------------------------
    always @(posedge clk) begin
        if (!rst_n) begin
            // The state a Z80 comes up in, which is not all zeros.
            r_b <= 8'h00; r_c <= 8'h00; r_d <= 8'h00; r_e <= 8'h00;
            r_h <= 8'h00; r_l <= 8'h00; r_a <= 8'hff; r_f <= 8'hff;
            x_b <= 8'h00; x_c <= 8'h00; x_d <= 8'h00; x_e <= 8'h00;
            x_h <= 8'h00; x_l <= 8'h00; x_a <= 8'hff; x_f <= 8'hff;
            pc <= 16'h0000;
            sp <= 16'hffff;
            wz <= 16'h0000;
            ir <= 8'h00;
            tmp <= 8'h00;
            rreg <= 7'h00;
            rbit7 <= 1'b0;
            iff1 <= 1'b0;
            iff2 <= 1'b0;
            halted <= 1'b0;
            ei_pending <= 1'b0;
            nmi_seen <= 1'b0;
            nmi_last <= 1'b1;
            cbop <= 8'h00;
            in_nmi <= 1'b0;
            in_int <= 1'b0;
            trap <= 1'b0;
            retire_q <= 1'b0;
            retire_pc <= 16'h0000;
            op_pc <= 16'h0000;
            m <= 3'd0;
            t <= 3'd1;
        end else begin
            trap <= 1'b0;

            // NMI is an edge, and it is remembered until it is taken.
            if (nmi_last && !nmi_n) nmi_seen <= 1'b1;
            nmi_last <= nmi_n;

            // Taken at the start of the M1 that would have fetched the
            // next opcode; the fetch happens and its byte is discarded,
            // which is what the part does.
            if (at_instr_start) begin
                op_pc <= pc;
                if (take_nmi) in_nmi <= 1'b1;
                else if (take_int) in_int <= 1'b1;
            end
            retire_q <= cyc_end && last_cycle;
            if (cyc_end && last_cycle) retire_pc <= op_pc;
            if (cyc_end && last_cycle) begin
                in_nmi <= 1'b0;
                in_int <= 1'b0;
            end

            if (stalled) begin
                t <= t;
            end else if (t != tlen) begin
                t <= t + 3'd1;
            end else begin
                t <= 3'd1;
                m <= last_cycle ? 3'd0 : (m + 3'd1);
            end

            // The refresh counter steps once per opcode fetch.
            if ((kind == CY_M1) && (t == 3'd1) && !stalled) rreg <= rreg + 7'd1;

            // ----- what each M-cycle leaves behind -----
            if (sample) begin
                if ((kind == CY_M1) && (m == 3'd0)) begin
                    ir <= din;
                end else if ((kind == CY_M1) && (m == 3'd1)) begin
                    cbop <= din;           // the CB opcode
                end else begin
                    tmp <= din;
                    if (m == 3'd1) wz[7:0] <= din;
                    if (m == 3'd2) wz[15:8] <= din;
                end
            end

            if (cyc_end) exec;
        end
    end

    // ------------------------------------------------------------------
    // THE EXECUTION, ONE M-CYCLE AT A TIME
    // ------------------------------------------------------------------
    // Everything an instruction does happens at the end of one of its
    // M-cycles, which is where the part does it too.
    // The one write port of the `r[]` encoding. Every variable-index
    // write goes through it, which is what keeps the register file out
    // of a memory with a port per statement.
    task wr8;
        input [2:0] idx;
        input [7:0] v;
        begin
            case (idx)
                3'd0: r_b <= v;
                3'd1: r_c <= v;
                3'd2: r_d <= v;
                3'd3: r_e <= v;
                3'd4: r_h <= v;
                3'd5: r_l <= v;
                3'd7: r_a <= v;
                default: ;
            endcase
        end
    endtask

    task exec;
        begin
            case (shape)
                SH_ONE, SH_M1_6: exec_one;
                default: exec_long;
            endcase
        end
    endtask

    task exec_one;
        begin
            pc <= pc + 16'd1;
            // An undecoded prefix advances past itself and says so. Its
            // `x` and `z` reach no arm of the case below, so it is a NOP
            // in every other respect.
            if (ir_is_prefix) trap <= 1'b1;
            case (x)
                2'd0: case (z)
                    3'd0: if (y == 3'd1) begin                 // EX AF,AF'
                    r_a <= x_a; r_f <= x_f;
                    x_a <= r_a; x_f <= r_f;
                end
                    3'd3: begin                                // INC/DEC rp
                        case (p)
                            2'd0: {r_b, r_c} <= q ? (bc - 16'd1) : (bc + 16'd1);
                            2'd1: {r_d, r_e} <= q ? (de - 16'd1) : (de + 16'd1);
                            2'd2: {r_h, r_l} <= q ? (hl - 16'd1) : (hl + 16'd1);
                            default: sp <= q ? (sp - 16'd1) : (sp + 16'd1);
                        endcase
                    end
                    3'd4, 3'd5: begin                          // INC/DEC r
                        wr8(y, step_res);
                        r_f <= step_f;
                    end
                    default: exec_accops;                      // z == 7
                endcase
                2'd1: begin                                    // LD r,r' / HALT
                    if ((y == 3'd6) && (z == 3'd6)) halted <= 1'b1;
                    else wr8(y, gp_rd);
                end
                2'd2: begin                                    // ALU A,r
                    if (y != 3'd7) r_a <= alu_res;     // CP keeps A
                    r_f <= alu_f;
                end
                default: case (z)                              // x == 3
                    3'd1: if (q && (p == 2'd3)) sp <= hl;      // LD SP,HL
                    3'd3: case (y)
                        3'd5: begin                            // EX DE,HL
                            r_d <= reg_h;
                            r_e <= reg_l;
                            r_h <= reg_d;
                            r_l <= reg_e;
                        end
                        3'd6: begin iff1 <= 1'b0; iff2 <= 1'b0; end  // DI
                        3'd7: ei_pending <= 1'b1;                    // EI
                        default: ;
                    endcase
                    default: ;
                endcase
            endcase
            // `JP (HL)` is the one shape-one instruction that does not
            // advance PC past itself.
            if ((x == 2'd3) && (z == 3'd1) && q && (p == 2'd2)) pc <= hl;
            // EXX
            if ((x == 2'd3) && (z == 3'd1) && q && (p == 2'd1)) begin
                r_b <= x_b; r_c <= x_c; r_d <= x_d;
                r_e <= x_e; r_h <= x_h; r_l <= x_l;
                x_b <= r_b; x_c <= r_c; x_d <= r_d;
                x_e <= r_e; x_h <= r_h; x_l <= r_l;
            end
            // HALT holds PC on the HALT itself so an interrupt returns
            // to the instruction after it.
            if ((x == 2'd1) && (y == 3'd6) && (z == 3'd6)) pc <= pc;
            // EI takes effect after the next instruction, which is what
            // makes `EI / RET` safe at the end of a handler.
            if (ei_pending && !((x == 2'd3) && (z == 3'd3) && (y == 3'd7))) begin
                iff1 <= 1'b1;
                iff2 <= 1'b1;
                ei_pending <= 1'b0;
            end
        end
    endtask

    // The eight at the end of the first quadrant: the rotates of A,
    // DAA, CPL, SCF and CCF.
    task exec_accops;
        begin
            case (y)
                3'd0, 3'd1, 3'd2, 3'd3: begin
                    r_a <= rot_res;
                    // These four keep S, Z and P/V and take H and N
                    // clear, unlike their CB cousins.
                    r_f <= {f_reg[F_S], f_reg[F_Z],
                                      UNDOCUMENTED_FLAGS ? rot_res[5] : 1'b0,
                                      1'b0,
                                      UNDOCUMENTED_FLAGS ? rot_res[3] : 1'b0,
                                      f_reg[F_PV], 1'b0, rot_cout};
                end
                3'd4: begin                                   // DAA
                    r_a <= daa_res;
                    r_f <= {daa_res[7], daa_res == 8'h00,
                                      UNDOCUMENTED_FLAGS ? daa_res[5] : 1'b0,
                                      daa_h,
                                      UNDOCUMENTED_FLAGS ? daa_res[3] : 1'b0,
                                      ~(^daa_res), f_reg[F_N], daa_c};
                end
                3'd5: begin                                   // CPL
                    r_a <= ~a_reg;
                    r_f <= {f_reg[F_S], f_reg[F_Z],
                                      UNDOCUMENTED_FLAGS ? ~a_reg[5] : 1'b0,
                                      1'b1,
                                      UNDOCUMENTED_FLAGS ? ~a_reg[3] : 1'b0,
                                      f_reg[F_PV], 1'b1, f_reg[F_C]};
                end
                3'd6: begin                                   // SCF
                    r_f <= {f_reg[F_S], f_reg[F_Z],
                                      UNDOCUMENTED_FLAGS ? a_reg[5] : 1'b0,
                                      1'b0,
                                      UNDOCUMENTED_FLAGS ? a_reg[3] : 1'b0,
                                      f_reg[F_PV], 1'b0, 1'b1};
                end
                default: begin                                // CCF
                    r_f <= {f_reg[F_S], f_reg[F_Z],
                                      UNDOCUMENTED_FLAGS ? a_reg[5] : 1'b0,
                                      f_reg[F_C],
                                      UNDOCUMENTED_FLAGS ? a_reg[3] : 1'b0,
                                      f_reg[F_PV], 1'b0, ~f_reg[F_C]};
                end
            endcase
        end
    endtask

    task exec_long;
        begin
            case (shape)
                SH_RD8: exec_rd8;
                SH_WR8: exec_wr8;
                SH_RD_WR: exec_rd_wr;
                SH_RD16: exec_rd16;
                SH_ADDR_RD: exec_addr_rd;
                SH_ADDR_WR: exec_addr_wr;
                SH_ADDR_RD16: exec_addr_rd16;
                SH_ADDR_WR16: exec_addr_wr16;
                SH_RMW: exec_rmw;
                SH_ADD16: exec_add16;
                SH_PUSH: exec_push;
                SH_JR: exec_jr;
                SH_DJNZ: exec_djnz;
                SH_CALL: exec_call;
                SH_RET_CC: exec_ret_cc;
                SH_IO: exec_io;
                SH_EXSP: exec_exsp;
                SH_CB: exec_cb;
                SH_NMI: exec_nmi;
                default: exec_int;
            endcase
        end
    endtask

    task exec_rd8;
        begin
            if (m == 3'd0) begin
                pc <= pc + 16'd1;
            end else begin
                // The byte is in `din` this edge, not yet in `tmp`.
                if (x == 2'd0) begin
                    if (z == 3'd2) begin
                        r_a <= din;                   // LD A,(BC)/(DE)
                    end else begin
                        pc <= pc + 16'd1;                     // LD r,n
                        wr8(y, din);
                    end
                end else if (x == 2'd1) begin
                    wr8(y, din);                      // LD r,(HL)
                end else begin
                    if (x == 2'd3) pc <= pc + 16'd1;          // ALU A,n
                    if (y != 3'd7) r_a <= alu_res;
                    r_f <= alu_f;
                end
            end
        end
    endtask

    task exec_wr8;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
        end
    endtask

    task exec_rd_wr;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
            else if (m == 3'd1) pc <= pc + 16'd1;
        end
    endtask

    task exec_rd16;
        begin
            if (m == 3'd0) begin
                pc <= pc + 16'd1;
            end else if (m == 3'd1) begin
                if (!is_pop) pc <= pc + 16'd1;
            end else begin
                if (is_pop) begin
                    sp <= sp + 16'd2;
                    if (q) pc <= {din, wz[7:0]};              // RET
                    else case (p)
                        2'd0: begin r_c <= wz[7:0]; r_b <= din; end
                        2'd1: begin r_e <= wz[7:0]; r_d <= din; end
                        2'd2: begin r_l <= wz[7:0]; r_h <= din; end
                        default: begin r_f <= wz[7:0]; r_a <= din; end
                    endcase
                end else if (x == 2'd0) begin
                    pc <= pc + 16'd1;                         // LD rp,nn
                    case (p)
                        2'd0: begin r_c <= wz[7:0]; r_b <= din; end
                        2'd1: begin r_e <= wz[7:0]; r_d <= din; end
                        2'd2: begin r_l <= wz[7:0]; r_h <= din; end
                        default: sp <= {din, wz[7:0]};
                    endcase
                end else begin
                    // JP nn and JP cc,nn: both operand bytes are read
                    // either way, and the condition only decides PC.
                    pc <= pc + 16'd1;
                    if ((z == 3'd3) || cond_met) pc <= {din, wz[7:0]};
                end
            end
        end
    endtask

    task exec_addr_rd;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
            else if (m <= 3'd2) pc <= pc + 16'd1;
            else r_a <= din;
        end
    endtask

    task exec_addr_wr;
        begin
            if (m <= 3'd2) pc <= pc + 16'd1;
        end
    endtask

    task exec_addr_rd16;
        begin
            if (m <= 3'd2) pc <= pc + 16'd1;
            else if (m == 3'd3) r_l <= din;
            else r_h <= din;
        end
    endtask

    task exec_addr_wr16;
        begin
            if (m <= 3'd2) pc <= pc + 16'd1;
        end
    endtask

    // INC (HL) and DEC (HL): the flags come from the byte on `din`, and
    // the write-back in M3 takes the same byte out of `tmp`.
    task exec_rmw;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
            else if (m == 3'd1) r_f <= step_f;
        end
    endtask

    task exec_add16;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
            else if (m == 3'd2) begin
                r_h <= add16_sum[15:8];
                r_l <= add16_sum[7:0];
                r_f <= {f_reg[F_S], f_reg[F_Z],
                                  UNDOCUMENTED_FLAGS ? add16_sum[13] : 1'b0,
                                  add16_h,
                                  UNDOCUMENTED_FLAGS ? add16_sum[11] : 1'b0,
                                  f_reg[F_PV], 1'b0, add16_sum[16]};
            end
        end
    endtask

    // PUSH rp, and RST n, which pushes the address after itself and
    // jumps to eight times its row.
    task exec_push;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
            else if (m == 3'd2) begin
                sp <= sp - 16'd2;
                if (z == 3'd7) pc <= {10'd0, y, 3'd0};
            end
        end
    endtask

    task exec_jr;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
            else if (m == 3'd1) pc <= pc + 16'd1;
            else pc <= pc + {{8{tmp[7]}}, tmp};
        end
    endtask

    task exec_djnz;
        begin
            if (m == 3'd0) begin
                pc <= pc + 16'd1;
                r_b <= reg_b - 8'd1;
            end else if (m == 3'd1) pc <= pc + 16'd1;
            else pc <= pc + {{8{tmp[7]}}, tmp};
        end
    endtask

    task exec_call;
        begin
            if (m <= 3'd2) pc <= pc + 16'd1;
            else if (m == 3'd4) begin
                sp <= sp - 16'd2;
                pc <= wz;
            end
        end
    endtask

    task exec_ret_cc;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
            else if (m == 3'd2) begin
                sp <= sp + 16'd2;
                pc <= {din, wz[7:0]};
            end
        end
    endtask

    task exec_io;
        begin
            if (m <= 3'd1) pc <= pc + 16'd1;
            else if (y == 3'd3) r_a <= din;    // IN A,(n)
        end
    endtask

    task exec_exsp;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
            else if (m == 3'd2) begin
                r_l <= wz[7:0];
                r_h <= din;
            end
        end
    endtask

    // The CB quadrant. A register form finishes on the second M1; an
    // `(HL)` form reads in M3, and everything but `BIT` writes in M4.
    task exec_cb;
        begin
            if (m == 3'd0) pc <= pc + 16'd1;
            else if (m == 3'd1) begin
                pc <= pc + 16'd1;
                if (cbz != 3'd6) begin
                    if (cbx != 2'd1) begin                 // not BIT
                        wr8(cbz, cb_res);
                    end
                    r_f <= cb_f;
                end
            end else if (m == 3'd2) begin
                r_f <= cb_f;
            end
        end
    endtask

    task exec_nmi;
        begin
            if (m == 3'd0) begin
                halted <= 1'b0;
                nmi_seen <= 1'b0;
                iff2 <= iff1;
                iff1 <= 1'b0;
            end else if (m == 3'd2) begin
                sp <= sp - 16'd2;
                pc <= 16'h0066;
            end
        end
    endtask

    task exec_int;
        begin
            if (m == 3'd0) begin
                halted <= 1'b0;
                iff1 <= 1'b0;
                iff2 <= 1'b0;
            end else if (m == 3'd2) begin
                sp <= sp - 16'd2;
                pc <= 16'h0038;            // mode 1
            end
        end
    endtask

endmodule
